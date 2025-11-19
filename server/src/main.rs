use anyhow::{Context, Result};
use axum::Router;
use clap::Parser;
use client_sdk::helpers::sp1::SP1Prover;
use contracts::{ORDERBOOK_ELF, ORDERBOOK_VK};
use hyli_modules::{
    bus::{metrics::BusMetrics, SharedMessageBus},
    modules::{
        da_listener::{DAListener, DAListenerConf},
        rest::{RestApi, RestApiRunContext},
        BuildApiContextInner, ModulesHandler,
    },
    utils::logger::setup_tracing,
};
use prometheus::Registry;
use sdk::{api::NodeInfo, info};
use server::{
    api::{ApiModule, ApiModuleCtx},
    app::{OrderbookModule, OrderbookModuleCtx},
    bridge::{BridgeModule, BridgeModuleCtx},
    conf::Conf,
    database::{DatabaseModule, DatabaseModuleCtx},
    prover::{OrderbookProverCtx, OrderbookProverModule},
    setup::{init_tracing, setup_database, setup_services, ServiceContext},
};
use sp1_sdk::{Prover, ProverClient};
use std::sync::Arc;
use tracing::error;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "orderbook")]
    pub orderbook_cn: String,

    #[arg(long, default_value = "oranj")] // This should be USDC contract or so
    pub collateral_token_cn: String,

    #[arg(long, default_value = "false")]
    pub clean_db: bool,

    #[arg(long, default_value = "false")]
    pub no_check: bool,

    #[arg(long, default_value = "false")]
    pub no_prover: bool,

    #[arg(long, default_value = "false")]
    pub no_bridge: bool,

    #[arg(long, default_value = "false")]
    pub no_blobs: bool,

    #[arg(long, default_value = "false")]
    pub tracing: bool,

    /// Clean the data directory before starting the server
    /// Argument used by hylix tests commands
    #[arg(long, default_value = "false")]
    pub clean_data_directory: bool,

    /// Server port (overrides config)
    /// Argument used by hylix tests commands
    #[arg(long)]
    pub server_port: Option<u16>,
}

fn main() -> Result<()> {
    server::init::install_rustls_crypto_provider();
    let args = Args::parse();
    let config = Conf::new(args.config_file.clone()).context("reading config file")?;

    if args.tracing {
        init_tracing();
    } else {
        setup_tracing(&config.log_format, "hyliquid".to_string())?;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        // Results in poor threading performance otherwise.
        .disable_lifo_slot()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(actual_main(args, config))
}

async fn actual_main(args: Args, config: Conf) -> Result<()> {
    let config = Arc::new(config);

    if args.clean_data_directory && std::fs::exists(&config.data_directory).unwrap_or(false) {
        info!("Cleaning data directory: {:?}", &config.data_directory);
        std::fs::remove_dir_all(&config.data_directory).context("cleaning data directory")?;
    }

    info!("Starting orderbook with config: {:?}", &config);

    let pool = setup_database(&config, args.clean_db).await?;
    let ServiceContext {
        user_service,
        asset_service,
        book_service,
        node_client,
        indexer_client,
        validator_lane_id,
        bridge_service,
    } = setup_services(&config, pool.clone()).await?;

    // TODO: make a proper secret management
    let secret = vec![1, 2, 3];

    let (light_state, full_state) = server::init::init_orderbook_from_database(
        validator_lane_id.clone(),
        secret.clone(),
        asset_service.clone(),
        user_service.clone(),
        book_service.clone(),
        &node_client,
        &indexer_client,
        &args.orderbook_cn.clone().into(),
        !args.no_check,
    )
    .await
    .map_err(|e| anyhow::Error::msg(e.1))?;

    let contracts = vec![server::init::ContractInit {
        name: args.orderbook_cn.clone().into(),
        program_id: ORDERBOOK_VK.into(),
        initial_state: full_state.commit(),
    }];

    match server::init::init_node(node_client.clone(), contracts, !args.no_check).await {
        Ok(_) => {}
        Err(e) => {
            error!("Error initializing node: {:?}", e);
            return Ok(());
        }
    }
    let bus = SharedMessageBus::new(BusMetrics::global(config.id.clone()));

    std::fs::create_dir_all(&config.data_directory).context("creating data directory")?;

    let registry = Registry::new();
    // Init global metrics meter we expose as an endpoint
    let provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
        .with_reader(
            opentelemetry_prometheus::exporter()
                .with_registry(registry.clone())
                .build()
                .context("starting prometheus exporter")?,
        )
        .build();

    opentelemetry::global::set_meter_provider(provider.clone());

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
        client: node_client.clone(),
    });

    let database_ctx = Arc::new(DatabaseModuleCtx {
        pool: pool.clone(),
        user_service: user_service.clone(),
        asset_service: asset_service.clone(),
        client: node_client.clone(),
        no_blobs: args.no_blobs,
        metrics: server::database::DatabaseMetrics::new(),
    });

    let api_module_ctx = Arc::new(ApiModuleCtx {
        api: api_ctx.clone(),
        book_service,
        user_service,
        contract1_cn: args.orderbook_cn.clone().into(),
    });

    handler
        .build_module::<DAListener>(DAListenerConf {
            start_block: None,
            data_directory: config.data_directory.clone(),
            da_read_from: config.da_read_from.clone(),
            timeout_client_secs: 10,
        })
        .await?;

    handler
        .build_module::<OrderbookModule>(orderbook_ctx.clone())
        .await?;

    if !args.no_prover {
        info!("Setup sp1 prover client");
        let local_client = ProverClient::builder().cpu().build();
        let (pk, _) = local_client.setup(ORDERBOOK_ELF);

        info!("Building Proving Key");
        let prover = SP1Prover::new(pk).await;

        let orderbook_prover_ctx = Arc::new(OrderbookProverCtx {
            api: api_ctx.clone(),
            node_client: node_client.clone(),
            orderbook_cn: args.orderbook_cn.clone().into(),
            prover: Arc::new(prover),
            lane_id: validator_lane_id,
            initial_orderbook: full_state,
            pool: pool.clone(),
        });

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

    if !args.no_bridge {
        handler
            .build_module::<BridgeModule>(Arc::new(BridgeModuleCtx {
                api: api_ctx.clone(),
                collateral_token_cn: args.collateral_token_cn.clone().into(),
                bridge_config: config.bridge.clone(),
                pool: pool.clone(),
                asset_service: asset_service.clone(),
                bridge_service: bridge_service.clone(),
                orderbook_cn: args.orderbook_cn.clone().into(),
            }))
            .await?;
    }

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
            registry,
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
