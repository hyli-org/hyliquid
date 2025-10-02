use anyhow::{Context, Result};
use axum::Router;
use clap::Parser;
use client_sdk::{
    helpers::{sp1::SP1Prover, ClientSdkProver},
    rest_client::{IndexerApiHttpClient, NodeApiClient, NodeApiHttpClient},
};
use contracts::ORDERBOOK_ELF;
use hyli_modules::{
    bus::{metrics::BusMetrics, SharedMessageBus},
    modules::{
        da_listener::{DAListener, DAListenerConf},
        rest::{RestApi, RestApiRunContext},
        websocket::WebSocketModule,
        BuildApiContextInner, ModulesHandler,
    },
    utils::logger::setup_tracing,
};
use orderbook::orderbook::{ExecutionMode, Orderbook, OrderbookEvent};
use prometheus::Registry;
use sdk::{api::NodeInfo, info, Calldata, ZkContract};
use server::{
    api::{ApiModule, ApiModuleCtx},
    app::{OrderbookModule, OrderbookModuleCtx, OrderbookWsInMessage},
    conf::Conf,
    prover::{OrderbookProverCtx, OrderbookProverModule},
};
use sp1_sdk::{Prover, ProverClient};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::error;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "orderbook")]
    pub orderbook_cn: String,

    #[arg(long, default_value = "true")]
    pub clean_db: bool,

    /// Clean the data directory before starting the server
    /// Argument used by hylix tests commands
    #[arg(long, default_value = "false")]
    pub clean_data_directory: bool,

    /// Server port (overrides config)
    /// Argument used by hylix tests commands
    #[arg(long)]
    pub server_port: Option<u16>,
}

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./src/migrations");

async fn connect_database(config: &Conf) -> Result<PgPool> {
    info!("Connecting to database: {}", config.database_url);
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect(&config.database_url)
        .await
        .context("Failed to connect to the config database")?;

    if config.database_url.ends_with(&config.database_name) {
        return Ok(pool);
    }

    // Check if database exists
    let database_exists = sqlx::query(
        format!(
            "SELECT 1 FROM pg_database WHERE datname = '{}'",
            config.database_name
        )
        .as_str(),
    )
    .fetch_optional(&pool)
    .await?;

    if database_exists.is_none() {
        info!("Creating database: {}", config.database_name);
        sqlx::query(format!("CREATE DATABASE {}", config.database_name).as_str())
            .execute(&pool)
            .await?;
    }

    let database_url = format!("{}/{}", config.database_url, config.database_name);
    info!("Connecting to database: {}", database_url);

    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect(&database_url)
        .await
        .context("Failed to connect to the created database")?;

    Ok(pool)
}

async fn setup_database(config: &Conf, clean_db: bool) -> Result<PgPool> {
    let pool = connect_database(config).await?;

    if clean_db {
        info!("Cleaning database: {}", config.database_name);
        sqlx::query("DROP SCHEMA public CASCADE;")
            .execute(&pool)
            .await
            .context("cleaning database")?;
        sqlx::query("CREATE SCHEMA public;")
            .execute(&pool)
            .await
            .context("creating public schema")?;
    }

    info!("Running database migrations");
    MIGRATOR.run(&pool).await?;
    info!("Database migrations completed");

    Ok(pool)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Conf::new(args.config_file).context("reading config file")?;

    setup_tracing(
        &config.log_format,
        format!("{}(nopkey)", config.id.clone(),),
    )
    .context("setting up tracing")?;

    let config = Arc::new(config);

    if args.clean_data_directory && std::fs::exists(&config.data_directory).unwrap_or(false) {
        info!("Cleaning data directory: {:?}", &config.data_directory);
        std::fs::remove_dir_all(&config.data_directory).context("cleaning data directory")?;
    }

    info!("Starting orderbook with config: {:?}", &config);

    let node_client =
        Arc::new(NodeApiHttpClient::new(config.node_url.clone()).context("build node client")?);
    let indexer_client = Arc::new(
        IndexerApiHttpClient::new(config.indexer_url.clone()).context("build indexer client")?,
    );

    let pool = setup_database(&config, args.clean_db).await?;

    let user_service = Arc::new(RwLock::new(
        server::services::user_service::UserService::new(pool.clone()).await,
    ));
    let asset_service = Arc::new(RwLock::new(
        server::services::asset_service::AssetService::new(pool.clone()).await,
    ));
    let book_writer_service = Arc::new(Mutex::new(
        server::services::book_service::BookWriterService::new(
            pool.clone(),
            user_service.clone(),
            asset_service.clone(),
            config.trigger_url.clone(),
        ),
    ));
    let book_service = Arc::new(RwLock::new(
        server::services::book_service::BookService::new(pool.clone()),
    ));

    info!("Setup sp1 prover client");
    let local_client = ProverClient::builder().cpu().build();
    let (pk, _) = local_client.setup(ORDERBOOK_ELF);

    info!("Building Proving Key");
    let prover = SP1Prover::new(pk).await;

    let validator_lane_id = node_client
        .get_node_info()
        .await?
        .pubkey
        .map(sdk::LaneId)
        .ok_or_else(|| {
            error!("Validator lane id not found");
        })
        .ok();
    let Some(validator_lane_id) = validator_lane_id else {
        return Ok(());
    };

    // TODO: make a proper secret management
    let secret = vec![1, 2, 3];
    let default_state = Orderbook::init(
        validator_lane_id.clone(),
        ExecutionMode::Light,
        secret.clone(),
    )
    .map_err(anyhow::Error::msg)?;
    let prover_state = Orderbook::init(validator_lane_id.clone(), ExecutionMode::Full, secret)
        .map_err(anyhow::Error::msg)?;

    let contracts = vec![server::init::ContractInit {
        name: args.orderbook_cn.clone().into(),
        program_id: <SP1Prover as ClientSdkProver<Calldata>>::program_id(&prover).0,
        initial_state: default_state.commit(),
    }];

    match server::init::init_node(node_client.clone(), indexer_client.clone(), contracts).await {
        Ok(_) => {}
        Err(e) => {
            error!("Error initializing node: {:?}", e);
            return Ok(());
        }
    }
    let bus = SharedMessageBus::new(BusMetrics::global(config.id.clone()));

    std::fs::create_dir_all(&config.data_directory).context("creating data directory")?;

    let mut handler = ModulesHandler::new(&bus).await;

    let api_ctx = Arc::new(BuildApiContextInner {
        router: std::sync::Mutex::new(Some(Router::new())),
        openapi: Default::default(),
    });

    let orderbook_ctx = Arc::new(OrderbookModuleCtx {
        node_client: node_client.clone(),
        api: api_ctx.clone(),
        orderbook_cn: args.orderbook_cn.clone().into(),
        lane_id: validator_lane_id.clone(),
        default_state: default_state.clone(),
        book_writer_service,
        asset_service,
    });

    let orderbook_prover_ctx = Arc::new(OrderbookProverCtx {
        node_client: node_client.clone(),
        orderbook_cn: args.orderbook_cn.clone().into(),
        prover: Arc::new(prover),
        lane_id: validator_lane_id,
        initial_orderbook: prover_state,
    });

    let api_module_ctx = Arc::new(ApiModuleCtx {
        api: api_ctx.clone(),
        book_service,
        user_service,
        contract1_cn: args.orderbook_cn.clone().into(),
    });

    handler
        .build_module::<OrderbookModule>(orderbook_ctx.clone())
        .await?;
    handler
        .build_module::<OrderbookProverModule>(orderbook_prover_ctx.clone())
        .await?;

    handler
        .build_module::<ApiModule>(api_module_ctx.clone())
        .await?;

    handler
        .build_module::<WebSocketModule<OrderbookWsInMessage, OrderbookEvent>>(
            config.websocket.clone(),
        )
        .await?;

    handler
        .build_module::<DAListener>(DAListenerConf {
            start_block: None,
            data_directory: config.data_directory.clone(),
            da_read_from: config.da_read_from.clone(),
            timeout_client_secs: 10,
        })
        .await?;

    // Should come last so the other modules have nested their own routes.
    #[allow(clippy::expect_used, reason = "Fail on misconfiguration")]
    let router = api_ctx
        .router
        .lock()
        .expect("Context router should be available.")
        .take()
        .expect("Context router should be available.");
    #[allow(clippy::expect_used, reason = "Fail on misconfiguration")]
    let openapi = api_ctx
        .openapi
        .lock()
        .expect("OpenAPI should be available")
        .clone();

    handler
        .build_module::<RestApi>(RestApiRunContext {
            port: args.server_port.unwrap_or(config.rest_server_port),
            max_body_size: config.rest_server_max_body_size,
            registry: Registry::new(),
            router,
            openapi,
            info: NodeInfo {
                id: config.id.clone(),
                da_address: config.da_read_from.clone(),
                pubkey: None,
            },
        })
        .await?;

    handler.start_modules().await?;

    // Run until shut down or an error occurs
    handler.exit_process().await?;

    Ok(())
}
