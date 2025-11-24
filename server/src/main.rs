use anyhow::{anyhow, Context, Result};
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
use reth_harness::{CollateralContractInit, RethHarness};
use sdk::{api::NodeInfo, info};
use server::{
    api::{ApiModule, ApiModuleCtx},
    app::{OrderbookModule, OrderbookModuleCtx},
    bridge::{BridgeModule, BridgeModuleCtx},
    conf::Conf,
    database::{DatabaseModule, DatabaseModuleCtx},
    prover::{OrderbookProverCtx, OrderbookProverModule},
    reth_bridge::{RethBridgeModule, RethBridgeModuleCtx},
    reth_utils::{
        derive_address_from_private_key, persist_collateral_metadata,
        register_reth_collateral_contract, register_reth_collateral_with_data,
        CollateralRegistrationData, COLLATERAL_METADATA_FILE,
    },
    setup::{init_tracing, setup_database, setup_services, ServiceContext},
};
use sp1_sdk::{Prover, ProverClient};
use std::sync::Arc;
use tracing::{error, warn};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "orderbook")]
    pub orderbook_cn: String,

    #[arg(long)]
    pub collateral_token_cn: Option<String>,

    #[arg(long, default_value = "false")]
    pub clean_db: bool,

    #[arg(long, default_value = "false")]
    pub no_check: bool,

    #[arg(long, default_value = "false")]
    pub no_prover: bool,

    #[arg(long, default_value = "false")]
    pub no_bridge: bool,

    #[arg(long, default_value = "false")]
    pub reth_bridge: bool,

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

    let collateral_token_cn = args
        .collateral_token_cn
        .clone()
        .unwrap_or_else(|| format!("reth-collateral-{}", args.orderbook_cn));

    let orderbook_contract = server::init::ContractInit {
        name: args.orderbook_cn.clone().into(),
        program_id: ORDERBOOK_VK.into(),
        initial_state: full_state.commit(),
        verifier: sdk::verifiers::SP1_4.into(),
    };
    let contracts = vec![orderbook_contract];

    if let Err(e) =
        server::init::init_node(node_client.clone(), contracts.clone(), !args.no_check).await
    {
        if args.reth_bridge {
            // retry without collateral and let the reth bridge register via API later
            if server::init::init_node(
                node_client.clone(),
                vec![contracts[0].clone()],
                !args.no_check,
            )
            .await
            .is_err()
            {
                error!("Error initializing node: {:?}", e);
                return Ok(());
            }
        } else {
            error!("Error initializing node: {:?}", e);
            return Ok(());
        }
    }
    if !args.reth_bridge {
        register_reth_collateral_contract(
            node_client.as_ref(),
            &collateral_token_cn.clone().into(),
            &args.orderbook_cn.clone().into(),
            &config.bridge,
        )
        .await?;
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
        client: node_client.clone(),
    });

    let database_ctx = Arc::new(DatabaseModuleCtx {
        pool: pool.clone(),
        user_service: user_service.clone(),
        asset_service: asset_service.clone(),
        client: node_client.clone(),
        no_blobs: args.no_blobs,
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
        let reth_harness = if args.reth_bridge {
            let mut prefunded = Vec::new();
            if let Ok(address) =
                derive_address_from_private_key(&config.bridge.eth_signer_private_key)
            {
                prefunded.push(address);
            }
            let collateral_init = CollateralContractInit::test_erc20(
                &config.bridge.eth_signer_private_key,
            )
            .map_err(|err| anyhow!("preparing collateral contract for reth harness: {err}"))?;
            Some(Arc::new(tokio::sync::Mutex::new(
                RethHarness::new_with_collateral(
                    config.bridge.eth_chain_id,
                    prefunded,
                    collateral_init,
                )
                .await
                .map_err(|err| anyhow!("initializing reth harness for prover: {err}"))?,
            )))
        } else {
            None
        };

        if args.reth_bridge {
            let harness_metadata = if let Some(harness) = &reth_harness {
                let guard = harness.lock().await;
                guard.collateral_metadata()
            } else {
                None
            };

            if let Some(meta) = harness_metadata {
                let registration = CollateralRegistrationData {
                    contract_address: meta.contract_address,
                    block_hash: meta.block_hash,
                    state_root: meta.state_root,
                };
                register_reth_collateral_with_data(
                    node_client.as_ref(),
                    &collateral_token_cn.clone().into(),
                    &args.orderbook_cn.clone().into(),
                    registration.clone(),
                )
                .await?;
                let metadata_path = config.data_directory.join(COLLATERAL_METADATA_FILE);
                if let Err(err) = persist_collateral_metadata(&metadata_path, &registration) {
                    warn!(error = %err, "failed to persist reth collateral metadata");
                }
            } else {
                register_reth_collateral_contract(
                    node_client.as_ref(),
                    &collateral_token_cn.clone().into(),
                    &args.orderbook_cn.clone().into(),
                    &config.bridge,
                )
                .await?;
            }
        }

        let orderbook_prover_ctx = Arc::new(OrderbookProverCtx {
            api: api_ctx.clone(),
            node_client: node_client.clone(),
            orderbook_cn: args.orderbook_cn.clone().into(),
            collateral_token_cn: args.reth_bridge.then(|| collateral_token_cn.clone().into()),
            prover: Arc::new(prover),
            lane_id: validator_lane_id,
            initial_orderbook: full_state,
            pool: pool.clone(),
            reth_harness,
            reth_chain_id: config.bridge.eth_chain_id,
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
        if args.reth_bridge {
            handler
                .build_module::<RethBridgeModule>(Arc::new(RethBridgeModuleCtx {
                    api: api_ctx.clone(),
                    orderbook_cn: args.orderbook_cn.clone().into(),
                    collateral_token_cn: collateral_token_cn.clone().into(),
                    client: node_client.clone(),
                }))
                .await?;
        } else {
            handler
                .build_module::<BridgeModule>(Arc::new(BridgeModuleCtx {
                    api: api_ctx.clone(),
                    collateral_token_cn: collateral_token_cn.clone().into(),
                    bridge_config: config.bridge.clone(),
                    pool: pool.clone(),
                    asset_service: asset_service.clone(),
                    bridge_service: bridge_service.clone(),
                    orderbook_cn: args.orderbook_cn.clone().into(),
                }))
                .await?;
        }
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
