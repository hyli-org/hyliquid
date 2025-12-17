use anyhow::{Context, Result};
use axum::Router;
use clap::Parser;
use client_sdk::helpers::sp1::SP1Prover;
use contracts::ORDERBOOK_ELF;
use hyli_modules::{
    bus::{metrics::BusMetrics, SharedMessageBus},
    modules::{
        contract_listener::{ContractListener, ContractListenerConf},
        rest::{RestApi, RestApiRunContext},
        BuildApiContextInner, ModulesHandler,
    },
    utils::logger::setup_tracing,
};
use prometheus::Registry;
use sdk::{api::NodeInfo, info};
use server::{
    conf::Conf,
    prover::{OrderbookProverCtx, OrderbookProverModule},
    setup::{init_tracing, setup_database, setup_services, ServiceContext},
};
use sp1_sdk::{Prover, ProverClient};
use std::{collections::HashSet, sync::Arc, time::Duration};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "false")]
    pub tracing: bool,

    #[arg(long, default_value = "false")]
    pub no_check: bool,

    #[arg(long, default_value = "orderbook")]
    pub orderbook_cn: String,
}

fn main() -> Result<()> {
    server::init::install_rustls_crypto_provider();
    let args = Args::parse();
    let config = Conf::new(args.config_file.clone()).context("reading config file")?;

    let _tracing_provider = if args.tracing {
        Some(init_tracing())
    } else {
        setup_tracing(&config.log_format, "hyliquid".to_string())?;
        None
    };

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
    info!("Starting autoprover with config: {:?}", &config);

    let pool = setup_database(&config, false).await?;
    let ServiceContext {
        user_service,
        asset_service,
        bridge_service: _,
        book_service,
        node_client,
        indexer_client,
        validator_lane_id,
    } = setup_services(&config, pool.clone(), false, false).await?;

    let secret = config.secret.clone();

    let last_settled_tx = server::init::get_last_settled_tx(
        asset_service.clone(),
        false,
        &args.orderbook_cn.clone().into(),
        &indexer_client,
    )
    .await?;

    let (_, full_state) = server::init::init_orderbook_from_database(
        validator_lane_id.clone(),
        secret.clone(),
        asset_service.clone(),
        user_service.clone(),
        book_service.clone(),
        &node_client,
        !args.no_check,
        &last_settled_tx,
        false,
    )
    .await
    .map_err(|e| anyhow::Error::msg(e.1))?;

    info!("Setup sp1 prover client");
    let local_client = ProverClient::builder().cpu().build();
    let (pk, _) = local_client.setup(ORDERBOOK_ELF);

    info!("Building Proving Key");
    let prover = SP1Prover::new(pk).await;

    let bus = SharedMessageBus::new(BusMetrics::global(config.id.clone()));
    std::fs::create_dir_all(&config.data_directory).context("creating data directory")?;

    let api_ctx = Arc::new(BuildApiContextInner {
        router: std::sync::Mutex::new(Some(Router::new())),
        openapi: Default::default(),
    });

    let orderbook_prover_ctx = Arc::new(OrderbookProverCtx {
        node_client: node_client.clone(),
        orderbook_cn: args.orderbook_cn.clone().into(),
        prover: Arc::new(prover),
        lane_id: validator_lane_id,
        initial_orderbook: full_state,
        pool: pool.clone(),
    });

    let mut handler = ModulesHandler::new(&bus).await;

    handler
        .build_module::<ContractListener>(ContractListenerConf {
            database_url: config.indexer_database_url.clone(),
            contracts: HashSet::from([args.orderbook_cn.clone().into()]),
            poll_interval: Duration::from_secs(5),
        })
        .await?;

    handler
        .build_module::<OrderbookProverModule>(orderbook_prover_ctx.clone())
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
            port: config.rest_server_port + 1,
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
    handler.exit_process().await?;

    Ok(())
}
