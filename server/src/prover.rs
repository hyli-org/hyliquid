use std::sync::Arc;

use alloy::{
    consensus::{TxEip4844Variant, TxEnvelope},
    eips::eip2718::Decodable2718,
    primitives::{Address, TxKind},
};
use anyhow::{anyhow, bail, Result};
use axum::{extract::State, response::IntoResponse, routing::get, Json, Router};
use client_sdk::{
    contract_indexer::AppError, helpers::ClientSdkProver, rest_client::NodeApiClient,
};
use eyre::Result as EyreResult;
use hyli_modules::{
    bus::SharedMessageBus,
    log_error, module_bus_client, module_handle_messages,
    modules::{BuildApiContextInner, Module},
};
use orderbook::{
    model::{AssetInfo, OrderbookEvent, UserInfo},
    transaction::{OrderbookAction, PermissionnedOrderbookAction, PermissionnedPrivateInput},
    zk::FullState,
    ORDERBOOK_ACCOUNT_IDENTITY,
};
use reqwest::Method;
use reth_harness::{RethHarness, SubmittedTx};
use sdk::{
    BlobIndex, BlockHeight, Calldata, ContractName, Hashed, IndexedBlobs, LaneId, NodeStateBlock,
    NodeStateEvent, ProofData, ProofTransaction, StructuredBlob, TransactionData, TxHash, Verifier,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};
use tracing::{debug, error, info, warn};

use crate::{app::ExecuteStateAPI, reth_utils::derive_program_pubkey};

#[derive(Debug, Clone)]
pub struct PendingTx {
    pub commitment_metadata: Vec<u8>,
    pub calldata: Calldata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookProverRequest {
    pub user_info: UserInfo,
    pub events: Vec<OrderbookEvent>,
    pub orderbook_action: PermissionnedOrderbookAction,
    pub nonce: u32,
    pub action_private_input: Vec<u8>,
    pub tx_hash: TxHash,
    pub blobs: Option<Vec<sdk::Blob>>,
    pub tx_blob_count: Option<usize>,
    pub orderbook_blob_index: Option<BlobIndex>,
    pub asset_info: Option<(String, AssetInfo)>,
}

module_bus_client! {
    #[derive(Debug)]
    struct OrderbookProverBusClient {
        receiver(NodeStateEvent),
    }
}

pub struct OrderbookProverCtx {
    // TO BE REMOVED
    pub api: Arc<BuildApiContextInner>,
    pub prover: Arc<dyn ClientSdkProver<Vec<Calldata>> + Send + Sync>,
    pub orderbook_cn: ContractName,
    pub collateral_token_cn: Option<ContractName>,
    pub lane_id: LaneId,
    pub node_client: Arc<dyn NodeApiClient + Send + Sync>,
    pub initial_orderbook: FullState,
    pub pool: PgPool,
    pub reth_harness: Option<Arc<tokio::sync::Mutex<RethHarness>>>,
    pub reth_chain_id: u64,
}

#[derive(Clone)]
pub struct Ctx {
    pub orderbook: Arc<Mutex<FullState>>,
}

pub struct OrderbookProverModule {
    ctx: Arc<OrderbookProverCtx>,
    bus: OrderbookProverBusClient,
    orderbook: Arc<Mutex<FullState>>,
    settled_block_height: Option<BlockHeight>,
}

impl Module for OrderbookProverModule {
    type Context = Arc<OrderbookProverCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let bus = OrderbookProverBusClient::new_from_bus(bus.new_handle()).await;
        let orderbook = Arc::new(Mutex::new(ctx.initial_orderbook.clone()));

        let router_ctx = Ctx {
            orderbook: orderbook.clone(),
        };

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(vec![Method::GET, Method::POST])
            .allow_headers(Any);

        // FIXME: to be removed. Only here  for debugging purposes
        let api = Router::new()
            .route("/prover/state", get(get_state))
            .with_state(router_ctx.clone())
            .layer(cors);

        if let Ok(mut guard) = ctx.api.router.lock() {
            if let Some(router) = guard.take() {
                guard.replace(router.merge(api));
            }
        }

        let settled_block_height = ctx
            .node_client
            .get_settled_height(ctx.orderbook_cn.clone())
            .await
            .ok();
        if let Some(settled_block_height) = settled_block_height {
            info!("🔍 Settled block height: {}", settled_block_height);
        }

        Ok(OrderbookProverModule {
            ctx,
            bus,
            orderbook,
            settled_block_height,
        })
    }

    async fn run(&mut self) -> Result<()> {
        self.start().await?;
        Ok(())
    }
}

impl OrderbookProverModule {
    pub async fn start(&mut self) -> Result<()> {
        module_handle_messages! {
            on_self self,

            listen<NodeStateEvent> event => {
                if log_error!(self.handle_node_state_event(event).await, "handle node state event").is_err() {
                    error!("❌ Hard failure in handle_node_state_event");
                    error!("❌ Exiting prover module");
                    return Err(anyhow!("Hard failure in handle_node_state_event"));
                }
            }
        };
        Ok(())
    }

    async fn handle_prover_request(
        &mut self,
        request: OrderbookProverRequest,
    ) -> Result<PendingTx> {
        let OrderbookProverRequest {
            events,
            user_info,
            action_private_input,
            orderbook_action,
            tx_hash,
            nonce,
            blobs,
            tx_blob_count,
            orderbook_blob_index,
            asset_info,
        } = request;
        // The goal is to create commitment metadata that contains the proofs to be able to load the zkvm state into the zkvm

        // We generate the commitment metadata from the zkvm state
        // We then execute the action with the complete orderbook to compare the events and update the state

        let mut orderbook = self.orderbook.lock().await;

        if let Some((sym, info)) = asset_info {
            orderbook
                .state
                .assets_info
                .entry(sym)
                .or_insert(info);
        }

        let commitment_metadata = orderbook
            .derive_zkvm_commitment_metadata_from_events(&user_info, &events, &orderbook_action)
            .map_err(|e| anyhow!("Could not derive zkvm state for tx {tx_hash:#}: {e}"))?;

        debug!(
            tx_hash = %tx_hash,
            events = ?events,
            "Transaction processed for proving"
        );

        orderbook
            .apply_events_and_update_roots(&user_info, events)
            .map_err(|e| anyhow!("failed to execute orderbook tx: {e}"))?;

        let permissioned_private_input = PermissionnedPrivateInput {
            secret: vec![1, 2, 3],
            user_info: user_info.clone(),
            private_input: action_private_input.clone(),
        };

        let private_input = borsh::to_vec(&permissioned_private_input)?;

        let (blobs_vec, tx_blob_count, blob_index) = if let (Some(blobs), Some(count), Some(idx)) =
            (blobs, tx_blob_count, orderbook_blob_index)
        {
            (blobs, count, idx)
        } else {
            let single = vec![OrderbookAction::PermissionnedOrderbookAction(
                orderbook_action.clone(),
                nonce,
            )
            .as_blob(self.ctx.orderbook_cn.clone())];
            (single, 1, BlobIndex(0))
        };

        let calldata = Calldata {
            identity: ORDERBOOK_ACCOUNT_IDENTITY.into(),
            tx_hash: tx_hash.clone(),
            blobs: IndexedBlobs::from(blobs_vec),
            tx_blob_count,
            index: blob_index,
            private_input,
            tx_ctx: Default::default(), // Will be set when proving
        };

        let pending_tx = PendingTx {
            commitment_metadata,
            calldata,
        };

        Ok(pending_tx)
    }

    async fn process_reth_blobs(&self, block: &NodeStateBlock) -> Result<()> {
        let Some(collateral) = &self.ctx.collateral_token_cn else {
            return Ok(());
        };
        let Some(reth) = &self.ctx.reth_harness else {
            return Ok(());
        };

        let program_id = derive_program_pubkey(&self.ctx.orderbook_cn);
        let verifier = Verifier("reth".into());

        for (_, tx) in block.parsed_block.txs.iter() {
            let TransactionData::Blob(blob_tx) = &tx.transaction_data else {
                continue;
            };

            let tx_hash = tx.hashed();
            let tx_ctx = block.parsed_block.build_tx_ctx(&tx_hash).ok();

            for (blob_index, blob) in blob_tx.blobs.iter().enumerate() {
                if &blob.contract_name != collateral {
                    continue;
                }

                let Ok(structured) = StructuredBlob::<Vec<u8>>::try_from(blob.clone()) else {
                    warn!(
                        contract = %blob.contract_name,
                        block_height = %block.signed_block.height().0,
                        "could not decode structured blob for collateral contract"
                    );
                    continue;
                };

                let raw_tx = structured.data.parameters;
                let envelope = match decode_tx_envelope(&raw_tx) {
                    Ok(env) => env,
                    Err(err) => {
                        error!(
                            contract = %blob.contract_name,
                            block_height = %block.signed_block.height().0,
                            "failed to decode collateral blob tx: {err:#}"
                        );
                        continue;
                    }
                };
                if let Err(err) =
                    validate_chain_id(&envelope, self.ctx.reth_chain_id, &blob.contract_name)
                {
                    error!(
                        contract = %blob.contract_name,
                        block_height = %block.signed_block.height().0,
                        "reth collateral blob rejected: {err:#}"
                    );
                    continue;
                }
                if let Ok(meta) = extract_tx_metadata(&envelope) {
                    let to_human = meta
                        .to
                        .map(|addr| format!("{addr:#x}"))
                        .unwrap_or_else(|| "contract creation".into());
                    info!(
                        contract = %blob.contract_name,
                        block_height = %block.signed_block.height().0,
                        nonce = meta.nonce,
                        to = to_human,
                        tx_type = meta.tx_type,
                        "reth collateral blob metadata"
                    );
                } else {
                    warn!(
                        contract = %blob.contract_name,
                        block_height = %block.signed_block.height().0,
                        "could not extract tx metadata for collateral blob"
                    );
                }
                info!(
                    contract = %blob.contract_name,
                    block_height = %block.signed_block.height().0,
                    blob_len = raw_tx.len(),
                    "processing reth collateral blob"
                );
                let harness = reth.clone();
                let node_client = self.ctx.node_client.clone();
                let contract_name = collateral.clone();
                let program_id = program_id.clone();
                let verifier = verifier.clone();
                let identity = blob_tx.identity.clone();
                let blobs = blob_tx.blobs.clone();
                let tx_blob_count = blobs.len();
                let blob_index = BlobIndex(blob_index);
                let calldata = Calldata {
                    tx_hash: tx_hash.clone(),
                    identity,
                    blobs: IndexedBlobs::from(blobs),
                    tx_blob_count,
                    index: blob_index,
                    tx_ctx: tx_ctx.clone(),
                    private_input: Vec::new(),
                };
                let calldata_bytes = match borsh::to_vec(&calldata) {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        error!(
                            contract = %blob.contract_name,
                            block_height = %block.signed_block.height().0,
                            "failed to serialize calldata for collateral blob: {err:#}"
                        );
                        continue;
                    }
                };

                tokio::spawn(async move {
                    match submit_raw_tx_with_restart(harness, raw_tx).await {
                        Ok(submission) => {
                            let proof_payload = build_reth_proof_payload(
                                calldata_bytes,
                                submission.stateless_input,
                                submission.evm_summary,
                            );
                            let proof_tx = ProofTransaction {
                                contract_name,
                                program_id,
                                verifier,
                                proof: ProofData(proof_payload),
                            };

                            if let Err(err) = node_client.send_tx_proof(proof_tx).await {
                                error!("failed to send reth proof: {err:#}");
                            } else {
                                info!("submitted reth collateral proof to Hyli");
                            }
                        }
                        Err(err) => {
                            error!(
                                "reth harness failed to build witness for collateral blob: {err:#}"
                            );
                        }
                    }
                });
            }
        }

        Ok(())
    }

    async fn handle_node_state_event(&mut self, event: NodeStateEvent) -> Result<()> {
        match event {
            NodeStateEvent::NewBlock(block) => {
                if let Some(settled_block_height) = self.settled_block_height {
                    if block.signed_block.height().0 < settled_block_height.0 {
                        if block.signed_block.height().0 % 1000 == 0 {
                            info!(
                                "⏭️ Skipping block {} because it is before the settled block height",
                                block.signed_block.height()
                            );
                        }
                        return Ok(());
                    }
                }
                if block.signed_block.height().0 % 1000 == 0 {
                    info!("Prover received block: {}", block.signed_block.height());
                }
                tracing::trace!("New block received: {:?}", block);

                self.process_reth_blobs(&block).await?;

                // Use signed_block to efficiently filter transactions by lane_id
                let tx_hashes: Vec<TxHash> = block
                    .signed_block
                    .iter_txs_with_id()
                    .filter_map(|(lane_id, tx_id, _)| {
                        if lane_id == self.ctx.lane_id {
                            Some(tx_id.1)
                        } else {
                            None
                        }
                    })
                    .collect();

                if tx_hashes.is_empty() {
                    // No transactions to prove on validator's lane
                    return Ok(());
                }

                // Get the first transaction hash for computing tx_ctx
                let first_tx_hash = &tx_hashes[0];

                // Compute tx_ctx that will be used to prove transactions
                let tx_ctx = block.parsed_block.build_tx_ctx(first_tx_hash)?;

                // Extract all transactions that need to be proved and that have been sequenced
                let mut txs_to_prove = Vec::new();
                for tx_hash in tx_hashes {
                    if let Some(settled_block_height) = self.settled_block_height {
                        if block.signed_block.height().0 == settled_block_height.0 {
                            let is_unsettled = self
                                .ctx
                                .node_client
                                .get_unsettled_tx(tx_hash.clone())
                                .await
                                .is_ok();
                            if !is_unsettled {
                                info!("⏭️ Skipping tx {tx_hash:#} because it is settled");
                                continue;
                            }
                        }
                    }
                    // Query the database for the prover request
                    let row = sqlx::query("SELECT request FROM prover_requests WHERE tx_hash = $1")
                        .bind(tx_hash.0.clone())
                        .fetch_optional(&self.ctx.pool)
                        .await?;

                    if let Some(row) = row {
                        let request_json: Vec<u8> = row.get("request");
                        let prover_request: OrderbookProverRequest =
                            serde_json::from_slice(&request_json)
                                .map_err(|e| anyhow!("Failed to parse prover request JSON: {e}"))?;

                        // Process the request to get the pending transaction
                        let pending_tx = self.handle_prover_request(prover_request).await?;
                        txs_to_prove.push((tx_hash, pending_tx));
                    }
                }

                // For each transaction to prove, spawn a new task to generate and send the proof
                for (tx_hash, pending_tx) in txs_to_prove {
                    let prover = self.ctx.prover.clone();
                    let contract_name = self.ctx.orderbook_cn.clone();
                    let node_client = self.ctx.node_client.clone();
                    let tx_context_cloned = tx_ctx.clone();

                    tokio::spawn(async move {
                        let mut calldata = pending_tx.calldata;

                        calldata.tx_ctx = Some(tx_context_cloned);

                        match prover
                            .prove(pending_tx.commitment_metadata, vec![calldata])
                            .await
                        {
                            Ok(proof) => {
                                let tx = ProofTransaction {
                                    contract_name: contract_name.clone(),
                                    program_id: prover.program_id(),
                                    verifier: prover.verifier(),
                                    proof: proof.data,
                                };

                                info!("Proof took {:?} cycles", proof.metadata.cycles);

                                match node_client.send_tx_proof(tx).await {
                                    Ok(proof_tx_hash) => {
                                        debug!("Successfully sent proof for {tx_hash:#}: {proof_tx_hash:#}");
                                    }
                                    Err(e) => {
                                        error!("Failed to send proof for {tx_hash:#}: {e:#}");
                                    }
                                }
                            }
                            Err(e) => {
                                bail!("failed to generate proof for {tx_hash:#}: {e:#}");
                            }
                        }
                        Ok(())
                    });
                }

                // Gather settled txs
                let mut settled_txs: Vec<&String> = block
                    .parsed_block
                    .successful_txs
                    .iter()
                    .map(|tx_hash| &tx_hash.0)
                    .collect();
                settled_txs.extend(
                    block
                        .parsed_block
                        .failed_txs
                        .iter()
                        .map(|tx_hash| &tx_hash.0),
                );
                settled_txs.extend(
                    block
                        .parsed_block
                        .timed_out_txs
                        .iter()
                        .map(|tx_hash| &tx_hash.0),
                );

                if !settled_txs.is_empty() {
                    // Delete settled txs from the database
                    log_error!(
                        sqlx::query("DELETE FROM prover_requests WHERE tx_hash = ANY($1)")
                            .bind(settled_txs)
                            .execute(&self.ctx.pool)
                            .await,
                        "Failed to delete settled txs from the database"
                    )?;
                }

                Ok(())
            }
        }
    }
}

// --------------------------------------------------------
//     Routes
// --------------------------------------------------------
async fn get_state(State(ctx): State<Ctx>) -> Result<impl IntoResponse, AppError> {
    let orderbook_full_state = ctx.orderbook.lock().await;

    let api_state = ExecuteStateAPI::from(&orderbook_full_state.state);
    Ok(Json(api_state))
}

fn validate_chain_id(
    envelope: &TxEnvelope,
    expected_chain_id: u64,
    contract_name: &ContractName,
) -> Result<()> {
    match envelope_chain_id(envelope) {
        Ok(Some(chain_id)) => {
            if chain_id != expected_chain_id {
                bail!(
                    "raw tx for contract {} targets chain id {chain_id}, expected {expected_chain_id}",
                    contract_name.0
                );
            } else {
                info!(
                    contract = %contract_name.0,
                    chain_id,
                    "reth collateral blob chain id validated"
                );
            }
        }
        Ok(None) => {
            bail!(
                "raw tx for contract {} is missing a replay-protecting chain id",
                contract_name.0
            );
        }
        Err(err) => {
            bail!(
                "failed to decode raw tx for contract {}: {err:#}",
                contract_name.0
            );
        }
    }

    Ok(())
}

fn decode_tx_envelope(raw_tx: &[u8]) -> Result<TxEnvelope> {
    let mut slice = raw_tx;
    TxEnvelope::decode_2718(&mut slice).map_err(|err| anyhow!("decoding tx envelope: {err}"))
}

fn envelope_chain_id(envelope: &TxEnvelope) -> Result<Option<u64>> {
    let chain_id = match envelope {
        TxEnvelope::Legacy(tx) => tx.tx().chain_id,
        TxEnvelope::Eip2930(tx) => Some(tx.tx().chain_id),
        TxEnvelope::Eip1559(tx) => Some(tx.tx().chain_id),
        TxEnvelope::Eip4844(tx) => Some(match tx.tx() {
            TxEip4844Variant::TxEip4844(inner) => inner.chain_id,
            TxEip4844Variant::TxEip4844WithSidecar(inner) => inner.tx.chain_id,
        }),
        TxEnvelope::Eip7702(tx) => Some(tx.tx().chain_id),
    };
    Ok(chain_id)
}

fn extract_tx_metadata(envelope: &TxEnvelope) -> Result<TxMetadata> {
    let (nonce, to, tx_type) = match envelope {
        TxEnvelope::Legacy(tx) => (tx.tx().nonce, txkind_to_address(tx.tx().to), "legacy"),
        TxEnvelope::Eip2930(tx) => (tx.tx().nonce, txkind_to_address(tx.tx().to), "eip2930"),
        TxEnvelope::Eip1559(tx) => (tx.tx().nonce, txkind_to_address(tx.tx().to), "eip1559"),
        TxEnvelope::Eip4844(tx) => match tx.tx() {
            TxEip4844Variant::TxEip4844(inner) => (inner.nonce, Some(inner.to), "eip4844"),
            TxEip4844Variant::TxEip4844WithSidecar(inner) => {
                (inner.tx.nonce, Some(inner.tx.to), "eip4844")
            }
        },
        TxEnvelope::Eip7702(tx) => (tx.tx().nonce, Some(tx.tx().to), "eip7702"),
    };

    Ok(TxMetadata { nonce, to, tx_type })
}

struct TxMetadata {
    nonce: u64,
    to: Option<Address>,
    tx_type: &'static str,
}

fn txkind_to_address(kind: TxKind) -> Option<Address> {
    match kind {
        TxKind::Call(addr) => Some(addr),
        TxKind::Create => None,
    }
}

async fn submit_raw_tx_with_restart(
    harness: Arc<Mutex<RethHarness>>,
    raw_tx: Vec<u8>,
) -> Result<SubmittedTx, anyhow::Error> {
    match submit_raw_tx_once(harness.clone(), raw_tx.clone()).await {
        Ok(res) => Ok(res),
        Err(err) => {
            let err_msg = err.to_string();
            if should_restart_harness(&err_msg) {
                warn!(
                    error = %err_msg,
                    "reth devnode became unhealthy; restarting harness"
                );
                restart_reth_harness(harness.clone())
                    .await
                    .map_err(|restart_err| anyhow!(restart_err))?;
                submit_raw_tx_once(harness, raw_tx)
                    .await
                    .map_err(|err| anyhow!(err))
            } else {
                Err(anyhow!(err))
            }
        }
    }
}

async fn submit_raw_tx_once(
    harness: Arc<Mutex<RethHarness>>,
    raw_tx: Vec<u8>,
) -> EyreResult<SubmittedTx> {
    let mut guard = harness.lock().await;
    guard.submit_raw_tx(raw_tx).await
}

async fn restart_reth_harness(harness: Arc<Mutex<RethHarness>>) -> EyreResult<()> {
    let (chain_id, prefunded, collateral) = {
        let guard = harness.lock().await;
        (
            guard.chain_id(),
            guard.prefunded_accounts().to_vec(),
            guard.collateral_config(),
        )
    };
    let new_harness = if let Some(config) = collateral {
        RethHarness::new_with_collateral(chain_id, prefunded, config).await?
    } else {
        RethHarness::new(chain_id, prefunded).await?
    };
    let mut guard = harness.lock().await;
    *guard = new_harness;
    Ok(())
}

fn should_restart_harness(error_msg: &str) -> bool {
    error_msg.contains("Batch transaction sender channel closed")
        || error_msg.contains("canonical state stream closed unexpectedly")
}

fn build_reth_proof_payload(
    calldata_bytes: Vec<u8>,
    stateless_bytes: Vec<u8>,
    evm_bytes: Vec<u8>,
) -> Vec<u8> {
    let mut proof_payload =
        Vec::with_capacity(12 + calldata_bytes.len() + stateless_bytes.len() + evm_bytes.len());
    proof_payload.extend_from_slice(&(calldata_bytes.len() as u32).to_le_bytes());
    proof_payload.extend_from_slice(&calldata_bytes);
    proof_payload.extend_from_slice(&(stateless_bytes.len() as u32).to_le_bytes());
    proof_payload.extend_from_slice(&stateless_bytes);
    proof_payload.extend_from_slice(&(evm_bytes.len() as u32).to_le_bytes());
    proof_payload.extend_from_slice(&evm_bytes);
    proof_payload
}
