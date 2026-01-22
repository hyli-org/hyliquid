use std::{str::FromStr, sync::Arc};

use alloy::primitives::{Address, Signature, U256};
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use client_sdk::contract_indexer::AppError;
use futures::StreamExt;
use hyli_modules::{
    bus::{BusClientSender, SharedMessageBus},
    log_error, module_bus_client, module_handle_messages,
    modules::{BuildApiContextInner, Module},
};
use hyli_smt_token::SmtTokenAction;
use orderbook::{
    transaction::{OrderbookAction, PermissionedOrderbookAction},
    ORDERBOOK_ACCOUNT_IDENTITY,
};
use reqwest::Method;
use sdk::{
    BlobTransaction, ContractName, NodeStateEvent, StatefulEvent, StructuredBlob,
    UnsettledBlobTransaction,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing::{error, info, warn};

use crate::{
    app::{OrderbookRequest, PendingDeposit, PendingWithdraw},
    bridge::eth::{EthClient, EthListener, EthSendResult},
    conf::BridgeConfig,
    services::{asset_service::AssetService, bridge_service::BridgeService},
};

pub mod eth;
pub mod utils;

pub struct BridgeModule {
    bus: BridgeModuleBusClient,
    eth_ws_url: String,
    collateral_token_cn: ContractName, // Collateral token contract name on Hyli side
    eth_contract_address: Address,     // Collateral token address name on Ethereum side
    eth_contract_vault_address: Address,
    eth_client: Arc<EthClient>,
    bridge_service: Arc<RwLock<BridgeService>>,
    asset_service: Arc<RwLock<AssetService>>,
    orderbook_cn: ContractName,
}

pub struct BridgeModuleCtx {
    pub api: Arc<BuildApiContextInner>,
    pub collateral_token_cn: ContractName,
    pub bridge_config: BridgeConfig,
    pub pool: PgPool,
    pub bridge_service: Arc<RwLock<BridgeService>>,
    pub asset_service: Arc<RwLock<AssetService>>,
    pub orderbook_cn: ContractName,
}

#[derive(Clone)]
struct BridgeRouterCtx {
    bridge_service: Arc<RwLock<BridgeService>>,
    bus: RouterBusClient,
    collateral_token_cn: ContractName,
}

module_bus_client! {
#[derive(Debug)]
    pub struct BridgeModuleBusClient {
        sender(OrderbookRequest),
        receiver(NodeStateEvent),
    }
}

module_bus_client! {
#[derive(Debug)]
struct RouterBusClient {
    sender(OrderbookRequest),
    // No receiver here ! Because RouterBus is cloned
}
}

impl Module for BridgeModule {
    type Context = Arc<BridgeModuleCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let router_bus = RouterBusClient::new_from_bus(bus.new_handle()).await;
        let bus = BridgeModuleBusClient::new_from_bus(bus.new_handle()).await;

        let eth_contract_address = Address::from_str(&ctx.bridge_config.eth_contract_address)
            .context("parsing Ethereum contract address")?;
        let vault_address = Address::from_str(&ctx.bridge_config.eth_contract_vault_address)
            .context("parsing Ethereum vault address")?;

        let claim_state = BridgeRouterCtx {
            bridge_service: ctx.bridge_service.clone(),
            bus: router_bus,
            collateral_token_cn: ctx.collateral_token_cn.clone(),
        };

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(vec![Method::GET, Method::POST])
            .allow_headers(Any);

        let api = Router::new()
            .route("/bridge/claim", post(claim))
            .route("/bridge/claim/{identity}", get(claim_status))
            .layer(Extension(claim_state))
            .layer(cors);

        if let Ok(mut guard) = ctx.api.router.lock() {
            if let Some(router) = guard.take() {
                guard.replace(router.merge(api));
            }
        }

        let eth_client = Arc::new(
            EthClient::new(
                &ctx.bridge_config.eth_rpc_http_url,
                &ctx.bridge_config.eth_signer_private_key,
                eth_contract_address,
            )
            .await
            .context("initializing Ethereum client")?,
        );

        Ok(BridgeModule {
            bus,
            collateral_token_cn: ctx.collateral_token_cn.clone(),
            eth_ws_url: ctx.bridge_config.eth_rpc_ws_url.clone(),
            eth_contract_address,
            eth_contract_vault_address: vault_address,
            eth_client,
            asset_service: ctx.asset_service.clone(),
            bridge_service: ctx.bridge_service.clone(),
            orderbook_cn: ctx.orderbook_cn.clone(),
        })
    }

    async fn run(&mut self) -> Result<()> {
        let eth_listener =
            EthListener::connect(&self.eth_ws_url, self.eth_contract_address).await?;

        _ = log_error!(self.catch_up_eth(&eth_listener).await, "Catching up on Eth");

        info!("Connected to Ethereum node, listening for events...");

        let vault_address = self.eth_contract_vault_address;

        info!(
            "Listening for Transfer events to vault: {:?} on contract {:?}",
            self.eth_contract_vault_address, self.eth_contract_address
        );

        let mut to_vault_stream = eth_listener.stream_transfers_to(vault_address).await?;

        // There are actually three distinct flows:
        // - Flow 1: USDC token (on Eth) -> Orderbook (on Hyli): this only happens on one contract (say USDC).
        //   1. User sends token on eth to vault address
        //   2. We detect the transfer event, and create a corresponding tx on Hyli

        // - Flow 2: Any token from Orderbook (on Hyli) -> same token (on Hyli): this only happens for any supported token on the orderbook on Hyli
        //   1. User sends a withdraw action to the orderbook contract on Hyli, specifiying a Hyli identity
        //   2. We detect the settled tx event, and send a corresponding transfer on Hyli token contract

        // - Flow 3: USDC from Orderbook (on Hyli) -> USDC token (on Eth): this only happens on one contract (say USDC).
        //   1. User sends a withdraw action to the orderbook contract on Hyli, specifiying an Eth address
        //   2. We detect the settled tx event, and send a corresponding transfer on Eth

        module_handle_messages! {
            on_self self,

            // Receive Ethereum logs
            Some(msg) = to_vault_stream.next() => {
                match msg {
                    Ok(log) => {
                        // Flow 1
                        sdk::info!("Received ETH to vault log: {:?}", log);
                        self.handle_eth_to_vault_log(log.clone()).await?;
                    },
                    Err(e) => error!("Error (to vault): {}", e),
                }
            },

            // Flow 2 + Flow 3
            listen<NodeStateEvent> event => {
                _ = log_error!(self.handle_node_state_event(event).await, "handle node state event")
            }

        };

        Ok(())
    }
}

impl BridgeModule {
    async fn handle_node_state_event(&mut self, event: NodeStateEvent) -> Result<()> {
        match event {
            NodeStateEvent::NewBlock(block) => {
                for (_, stateful_event) in block.stateful_events.events.iter() {
                    if let StatefulEvent::SettledTx(unsettled) = stateful_event {
                        self.handle_settled_tx(unsettled).await?;
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_settled_tx(&mut self, tx: &UnsettledBlobTransaction) -> Result<()> {
        let transfers = self.extract_relevant_transfers(&tx.tx).await;
        let withdraws = self.extract_relevant_withdraws(&tx.tx).await;

        let tx_hash = tx.tx_id.1.clone();
        // TODO: do not re-process already processed txs
        // state.add_hyli_pending_transaction(tx_hash);

        // Handle deposits (transfers to orderbook)
        for transfer in transfers {
            sdk::info!(
                tx_hash = ?tx_hash.0,
                token = %transfer.contract_name,
                sender = ?transfer.sender.0,
                amount = transfer.amount,
                "Settled deposit transfer detected",
            );
            self.bus.send(OrderbookRequest::PendingDeposit(transfer))?;
        }

        // Handle withdraws (orderbook withdraw actions)
        for withdraw in withdraws {
            sdk::info!(
                tx_hash = ?tx_hash.0,
                token = %withdraw.contract_name,
                network = %withdraw.destination.network,
                address = %withdraw.destination.address,
                amount = withdraw.amount,
                "Settled withdraw action detected",
            );

            // Flow 3
            if withdraw.destination.network == "ethereum-mainnet"
                || withdraw.destination.network == "ethereum-sepolia"
            {
                // TODO: use outputed tx_hash to track the withdraw on Eth side
                // TODO: if the withdraw fails (e.g. insufficient balance), we need to handle it properly in order to redo it
                let _eth_send_result = log_error!(
                    self.handle_eth_withdraw(&withdraw).await,
                    "processing Ethereum withdraw"
                );
            } else {
                self.bus.send(OrderbookRequest::PendingWithdraw(withdraw))?;
            }
        }

        Ok(())
    }

    async fn extract_relevant_transfers(&self, tx: &BlobTransaction) -> Vec<PendingDeposit> {
        let mut transfers = Vec::new();
        for blob in tx.blobs.iter() {
            let Ok(structured) = StructuredBlob::<SmtTokenAction>::try_from(blob.clone()) else {
                continue;
            };

            if let SmtTokenAction::Transfer {
                sender,
                recipient,
                amount,
            } = structured.data.parameters
            {
                if recipient.0 != ORDERBOOK_ACCOUNT_IDENTITY {
                    continue;
                }

                transfers.push(PendingDeposit {
                    sender,
                    contract_name: blob.contract_name.clone(),
                    amount,
                });
            }
        }

        transfers
    }

    async fn extract_relevant_withdraws(&self, tx: &BlobTransaction) -> Vec<PendingWithdraw> {
        let asset_service = self.asset_service.read().await;

        let mut withdraws = Vec::new();
        for blob in tx.blobs.iter() {
            // Only look at orderbook contract blobs
            if blob.contract_name != self.orderbook_cn {
                continue;
            }

            let Ok(action) = borsh::from_slice::<OrderbookAction>(blob.data.0.as_slice()) else {
                continue;
            };

            if let OrderbookAction::PermissionedOrderbookAction(
                PermissionedOrderbookAction::Withdraw {
                    symbol,
                    amount,
                    destination,
                },
                _,
            ) = action
            {
                let Some(contract_name) =
                    asset_service.get_contract_name_from_symbol(&symbol).await
                else {
                    continue;
                };
                withdraws.push(PendingWithdraw {
                    destination,
                    contract_name,
                    amount,
                });
            }
        }

        withdraws
    }

    async fn handle_eth_withdraw(&self, withdraw: &PendingWithdraw) -> Result<EthSendResult> {
        let to = Address::from_str(&withdraw.destination.address).with_context(|| {
            format!("parsing Ethereum address {}", withdraw.destination.address)
        })?;

        let amount = U256::from(withdraw.amount);

        self.eth_client
            .get_token_balance(self.eth_contract_vault_address)
            .await
            .and_then(|balance| {
                if balance < amount {
                    Err(anyhow::anyhow!(
                        "insufficient bridge token balance on Ethereum: have {balance}, need {amount}"
                    ))
                } else {
                    Ok(())
                }
            })?;

        let result = self
            .eth_client
            .transfer(to, amount)
            .await
            .context("sending Ethereum transfer for withdraw")?;

        info!(
            address = %withdraw.destination.address,
            token = %withdraw.contract_name,
            amount = withdraw.amount,
            tx_hash = ?result.tx_hash,
            "Submitted Ethereum withdraw transfer"
        );

        Ok(result)
    }

    async fn handle_eth_to_vault_log(&mut self, log: alloy::rpc::types::Log) -> Result<()> {
        let eth_tx = utils::log_to_eth_transaction(log);
        if eth_tx.from == Address::ZERO {
            warn!(tx = ?eth_tx.tx_hash, "Skipping contract creation transaction");
            return Ok(());
        }

        info!(
            "🔵👀 ETH to vault detected: sender {} amount {} wei",
            format!("{:?}", eth_tx.from).get(0..6).unwrap_or(""),
            eth_tx.amount
        );

        let bridge_service = self.bridge_service.read().await;

        let already_tracked = bridge_service.is_eth_tracked(&eth_tx.tx_hash).await?;

        if already_tracked {
            info!(tx = ?eth_tx.tx_hash, "ETH transaction already tracked, skipping");
            return Ok(());
        }

        let hyli_identity = bridge_service.hyli_identity_for_eth(&eth_tx.from).await?;

        let Some(hyli_identity) = hyli_identity else {
            info!(
                "{} is not yet a claimed address. Waiting for the claim to process the deposit",
                eth_tx.from
            );
            bridge_service
                .add_eth_pending_transaction(eth_tx.clone())
                .await?;
            return Ok(());
        };

        let hyli_amount = u128::try_from(eth_tx.amount).expect("Amount too large");

        let deposit = PendingDeposit {
            sender: hyli_identity.into(),
            contract_name: self.collateral_token_cn.clone(),
            amount: hyli_amount,
        };
        self.bus.send(OrderbookRequest::PendingDeposit(deposit))?;
        // TODO: instead of marking as processed right away, wait for confirmation from orderbook settled txs
        bridge_service.mark_eth_processed(eth_tx.tx_hash).await?;
        Ok(())
    }

    async fn catch_up_eth(&mut self, listener: &EthListener) -> Result<()> {
        let (from_block, latest, vault) = {
            let bridge_service = self.bridge_service.read().await;
            let from_block = bridge_service.eth_last_block().await?.saturating_add(1);

            let latest = listener.latest_block_number().await?;
            if from_block > latest {
                info!(latest_block = latest, "No ETH catch-up needed");
                return Ok(());
            }

            let vault = self.eth_contract_vault_address;
            bridge_service.record_eth_block(latest).await?;
            (from_block, latest, vault)
        };

        // for loop from from_block to lastest with 10 step
        for chunk_start in (from_block..=latest).step_by(10) {
            let chunk_end = (chunk_start + 9).min(latest);
            info!(
                from_block = chunk_start,
                to_block = chunk_end,
                "Catching up on ETH events"
            );

            for log in listener
                .fetch_transfers_to_range(vault, chunk_start, chunk_end)
                .await?
            {
                self.handle_eth_to_vault_log(log).await?;
            }
        }

        info!(from_block, latest_block = latest, "ETH catch-up completed");
        Ok(())
    }
}

// --------------------------------------------------------
//     Routes
// --------------------------------------------------------
#[derive(Serialize, Deserialize, Debug)]
pub struct ClaimDepositRequest {
    chain: String,
    eth_address: String,
    user_identity: String,
    signature: String,
}

#[derive(Serialize, Debug)]
pub struct ClaimStatusResponse {
    claimed: bool,
    eth_address: Option<String>,
}

#[axum::debug_handler]
#[cfg_attr(feature = "instrumentation", tracing::instrument(skip(claim_state)))]
async fn claim_status(
    Extension(claim_state): Extension<BridgeRouterCtx>,
    Path(identity): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let bridge_service = claim_state.bridge_service.read().await;
    let addr = bridge_service
        .eth_address_for_hyli_identity(&identity)
        .await
        .map_err(|err| AppError(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    let response = ClaimStatusResponse {
        claimed: addr.is_some(),
        eth_address: addr.map(|a| format!("{a:#x}")),
    };

    Ok(Json(response))
}

#[axum::debug_handler]
#[cfg_attr(feature = "instrumentation", tracing::instrument(skip(claim_state)))]
async fn claim(
    Extension(mut claim_state): Extension<BridgeRouterCtx>,
    Json(request): Json<ClaimDepositRequest>,
) -> Result<impl IntoResponse, AppError> {
    let eth_address = Address::from_str(request.eth_address.as_str()).map_err(|err| {
        AppError(
            StatusCode::BAD_REQUEST,
            anyhow::anyhow!("invalid Ethereum address: {err}"),
        )
    })?;

    let raw_signature = request.signature.trim_start_matches("0x");
    let signature_bytes = hex::decode(raw_signature).map_err(|err| {
        AppError(
            StatusCode::BAD_REQUEST,
            anyhow::anyhow!("invalid signature format: {err}"),
        )
    })?;

    let signature = Signature::try_from(signature_bytes.as_slice()).map_err(|_| {
        AppError(
            StatusCode::BAD_REQUEST,
            anyhow::anyhow!("signature must be 65 bytes long"),
        )
    })?;

    let message = format!(
        "{}:{}:{}",
        request.chain, request.eth_address, request.user_identity
    );

    let recovered = signature.recover_address_from_msg(message).map_err(|err| {
        AppError(
            StatusCode::BAD_REQUEST,
            anyhow::anyhow!("failed to recover signer from signature: {err}"),
        )
    })?;

    if recovered != eth_address {
        return Err(AppError(
            StatusCode::UNAUTHORIZED,
            anyhow::anyhow!("signature does not match provided address"),
        ));
    }

    let bridge_service = claim_state.bridge_service.read().await;

    if let Some(existing_identity) = bridge_service
        .hyli_identity_for_eth(&eth_address)
        .await
        .map_err(|err| AppError(StatusCode::INTERNAL_SERVER_ERROR, err))?
    {
        if existing_identity != request.user_identity {
            return Err(AppError(
                StatusCode::CONFLICT,
                anyhow::anyhow!("address already associated with a different Hyli identity"),
            ));
        }
    }

    let pending_eth_txs = bridge_service
        .pending_eth_transactions_for_address(&eth_address)
        .await
        .map_err(|err| AppError(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    bridge_service
        .record_eth_identity_binding(eth_address, request.user_identity.clone())
        .await
        .map_err(|err| AppError(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    for eth_tx in pending_eth_txs {
        let hyli_amount = u128::try_from(eth_tx.amount).map_err(|_| {
            AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                anyhow::anyhow!("amount too large to fit into u128"),
            )
        })?;

        let deposit = PendingDeposit {
            sender: request.user_identity.clone().into(),
            contract_name: claim_state.collateral_token_cn.clone(),
            amount: hyli_amount,
        };

        sdk::info!(
            "Queuing pending deposit for eth tx {:?}: {:?}",
            eth_tx.tx_hash,
            deposit
        );

        claim_state
            .bus
            .send(OrderbookRequest::PendingDeposit(deposit))
            .map_err(|err| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    anyhow::anyhow!("failed to queue pending deposit after identity claim: {err}"),
                )
            })?;

        let tx_hash = eth_tx.tx_hash;
        bridge_service
            .mark_eth_processed(tx_hash)
            .await
            .map_err(|err| {
                AppError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    anyhow::anyhow!(
                        "failed to mark pending deposit as processed after identity claim: {err}"
                    ),
                )
            })?;
    }

    Ok(Json("ok"))
}
