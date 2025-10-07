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
};
use orderbook::orderbook::OrderbookEvent;
use prometheus::Registry;
use sdk::{api::NodeInfo, info, Calldata, ZkContract};
use server::{
    api::{ApiModule, ApiModuleCtx},
    app::{OrderbookModule, OrderbookModuleCtx, OrderbookWsInMessage},
    conf::Conf,
    database::{DatabaseModule, DatabaseModuleCtx},
    prover::{OrderbookProverCtx, OrderbookProverModule},
};
use sp1_sdk::{Prover, ProverClient};
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, level_filters::LevelFilter};
use tracing_perfetto_sdk_schema as schema;
use tracing_perfetto_sdk_schema::trace_config;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "orderbook")]
    pub orderbook_cn: String,

    #[arg(long, default_value = "false")]
    pub clean_db: bool,

    #[arg(long, default_value = "false")]
    pub no_check: bool,

    #[arg(long, default_value = "false")]
    pub no_prover: bool,

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

pub fn minimal_trace_config() -> schema::TraceConfig {
    schema::TraceConfig {
        buffers: vec![trace_config::BufferConfig {
            size_kb: Some(1024),
            ..Default::default()
        }],
        data_sources: vec![trace_config::DataSource {
            config: Some(schema::DataSourceConfig {
                name: Some("rust_tracing".into()),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn init_tracing() {
    use opentelemetry::{global, trace::TracerProvider as _};
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::{
        trace::{self, BatchConfigBuilder, SdkTracerProvider},
        Resource,
    };
    use tracing_opentelemetry::OpenTelemetryLayer;
    use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    // Set up W3C trace context propagator
    global::set_text_map_propagator(opentelemetry_sdk::propagation::TraceContextPropagator::new());

    // Configure resource with service name
    let resource = Resource::builder_empty()
        .with_service_name("hyliquid-orderbook")
        .build();

    // Build OTLP exporter using tonic (grpc)
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint("http://localhost:4317")
        .build()
        .expect("Failed to create OTLP exporter");

    // Create batch span processor
    let batch_config = BatchConfigBuilder::default().build();
    let batch_processor = trace::BatchSpanProcessor::builder(otlp_exporter)
        .with_batch_config(batch_config)
        .build();

    // Create tracer provider
    let tracer_provider = SdkTracerProvider::builder()
        .with_span_processor(batch_processor)
        .with_resource(resource)
        .build();

    // Get tracer before setting as global
    let tracer = tracer_provider.tracer("hyliquid-orderbook");

    // Set as global tracer provider
    let _ = global::set_tracer_provider(tracer_provider);

    // Configure tracing subscriber with env filter
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // Initialize the tracing subscriber with both console output and OTLP
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(OpenTelemetryLayer::new(tracer))
        .init();

    tracing::info!("Tracing initialized with OTLP exporter to http://localhost:4317");
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Conf::new(args.config_file).context("reading config file")?;

    init_tracing();

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
    let book_service = Arc::new(RwLock::new(
        server::services::book_service::BookService::new(pool.clone()),
    ));

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

    let (light_state, full_state) = server::init::init_orderbook_from_database(
        validator_lane_id.clone(),
        secret.clone(),
        asset_service.clone(),
        user_service.clone(),
        book_service.clone(),
        &node_client,
        !args.no_check,
    )
    .await
    .map_err(|e| anyhow::Error::msg(e.1))?;

    info!("Setup sp1 prover client");
    let local_client = ProverClient::builder().cpu().build();
    let (pk, _) = local_client.setup(ORDERBOOK_ELF);

    info!("Building Proving Key");
    let prover = SP1Prover::new(pk).await;

    let contracts = vec![server::init::ContractInit {
        name: args.orderbook_cn.clone().into(),
        program_id: <SP1Prover as ClientSdkProver<Calldata>>::program_id(&prover).0,
        initial_state: light_state.commit(),
    }];

    match server::init::init_node(
        node_client.clone(),
        indexer_client.clone(),
        contracts,
        !args.no_check,
    )
    .await
    {
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
        api: api_ctx.clone(),
        orderbook_cn: args.orderbook_cn.clone().into(),
        lane_id: validator_lane_id.clone(),
        default_state: light_state.clone(),
        asset_service: asset_service.clone(),
    });

    let database_ctx = Arc::new(DatabaseModuleCtx {
        pool: pool.clone(),
        user_service: user_service.clone(),
        asset_service: asset_service.clone(),
        client: node_client.clone(),
    });

    let orderbook_prover_ctx = Arc::new(OrderbookProverCtx {
        node_client: node_client.clone(),
        orderbook_cn: args.orderbook_cn.clone().into(),
        prover: Arc::new(prover),
        lane_id: validator_lane_id,
        initial_orderbook: full_state,
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

    if !args.no_prover {
        handler
            .build_module::<OrderbookProverModule>(orderbook_prover_ctx.clone())
            .await?;
    }

    handler
        .build_module::<DatabaseModule>(database_ctx.clone())
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
