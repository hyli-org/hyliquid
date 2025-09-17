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
        contract_state_indexer::{ContractStateIndexer, ContractStateIndexerCtx},
        da_listener::{DAListener, DAListenerConf},
        prover::{AutoProver, AutoProverCtx},
        rest::{RestApi, RestApiRunContext},
        websocket::WebSocketModule,
        BuildApiContextInner, ModulesHandler,
    },
    utils::logger::setup_tracing,
};
use orderbook::orderbook::{Orderbook, OrderbookEvent};
use prometheus::Registry;
use sdk::{api::NodeInfo, info, Calldata, ZkContract};
use server::{
    app::{OrderbookModule, OrderbookModuleCtx, OrderbookWsInMessage},
    conf::Conf,
};
use sp1_sdk::{Prover, ProverClient};
use std::sync::{Arc, Mutex};
use tracing::error;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "orderbook")]
    pub orderbook_cn: String,

    /// Clean the data directory before starting the server
    /// Argument used by hylix tests commands
    #[arg(long, default_value = "false")]
    pub clean_data_directory: bool,

    /// Server port (overrides config)
    /// Argument used by hylix tests commands
    #[arg(long)]
    pub server_port: Option<u16>,
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

    let default_state = Orderbook::init(validator_lane_id.clone());

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
        router: Mutex::new(Some(Router::new())),
        openapi: Default::default(),
    });

    let orderbook_ctx = Arc::new(OrderbookModuleCtx {
        node_client: node_client.clone(),
        api: api_ctx.clone(),
        orderbook_cn: args.orderbook_cn.clone().into(),
        default_state: default_state.clone(),
    });

    handler
        .build_module::<OrderbookModule>(orderbook_ctx.clone())
        .await?;

    handler
        .build_module::<WebSocketModule<OrderbookWsInMessage, OrderbookEvent>>(
            config.websocket.clone(),
        )
        .await?;

    handler
        .build_module::<ContractStateIndexer<Orderbook>>(ContractStateIndexerCtx {
            contract_name: args.orderbook_cn.clone().into(),
            data_directory: config.data_directory.clone(),
            api: api_ctx.clone(),
        })
        .await?;

    handler
        .build_module::<AutoProver<Orderbook>>(Arc::new(AutoProverCtx {
            data_directory: config.data_directory.clone(),
            prover: Arc::new(prover),
            contract_name: args.orderbook_cn.clone().into(),
            node: node_client.clone(),
            default_state,
            buffer_blocks: config.buffer_blocks,
            max_txs_per_proof: config.max_txs_per_proof,
            tx_working_window_size: config.tx_working_window_size,
            api: Some(api_ctx.clone()),
        }))
        .await?;

    // This module connects to the da_address and receives all the blocks²
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
