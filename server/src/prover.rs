use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use client_sdk::{helpers::ClientSdkProver, rest_client::NodeApiClient};
use hyli_modules::{
    bus::SharedMessageBus, log_error, module_bus_client, module_handle_messages, modules::Module,
};
use orderbook::{
    model::{OrderbookEvent, UserInfo},
    transaction::{OrderbookAction, PermissionnedOrderbookAction, PermissionnedPrivateInput},
    zk::FullState,
    ORDERBOOK_ACCOUNT_IDENTITY,
};
use sdk::{BlobIndex, Calldata, ContractName, LaneId, NodeStateEvent, ProofTransaction, TxHash};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use tracing::{debug, error, info};

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
}

module_bus_client! {
    #[derive(Debug)]
    struct OrderbookProverBusClient {
        receiver(NodeStateEvent),
    }
}

pub struct OrderbookProverCtx {
    // TODO: Persist data
    // pub data_directory: PathBuf,
    pub prover: Arc<dyn ClientSdkProver<Vec<Calldata>> + Send + Sync>,
    pub orderbook_cn: ContractName,
    pub lane_id: LaneId,
    pub node_client: Arc<dyn NodeApiClient + Send + Sync>,
    pub initial_orderbook: FullState,
    pub pool: PgPool,
}

pub struct OrderbookProverModule {
    ctx: Arc<OrderbookProverCtx>,
    bus: OrderbookProverBusClient,
    orderbook: FullState,
}

impl Module for OrderbookProverModule {
    type Context = Arc<OrderbookProverCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let bus = OrderbookProverBusClient::new_from_bus(bus.new_handle()).await;
        let orderbook = ctx.initial_orderbook.clone();
        Ok(OrderbookProverModule {
            ctx,
            bus,
            orderbook,
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
                _ = log_error!(self.handle_node_state_event(event).await, "handle node state event")
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
        } = request;
        // The goal is to create commitment metadata that contains the proofs to be able to load the zkvm state into the zkvm

        // We generate the commitment metadata from the zkvm state
        // We then execute the action with the complete orderbook to compare the events and update the state

        let commitment_metadata = self
            .orderbook
            .derive_zkvm_commitment_metadata_from_events(&user_info, &events, &orderbook_action)
            .map_err(|e| anyhow!("Could not derive zkvm state: {e}"))?;

        debug!(
            tx_hash = %tx_hash,
            events = ?events,
            "Transaction processed for proving"
        );

        self.orderbook
            .apply_events_and_update_roots(&user_info, events)
            .map_err(|e| anyhow!("failed to execute orderbook tx: {e}"))?;

        let permissioned_private_input = PermissionnedPrivateInput {
            secret: vec![1, 2, 3],
            user_info: user_info.clone(),
            private_input: action_private_input.clone(),
        };

        let private_input = borsh::to_vec(&permissioned_private_input)?;

        let calldata = Calldata {
            identity: ORDERBOOK_ACCOUNT_IDENTITY.into(),
            tx_hash: tx_hash.clone(),
            blobs: vec![OrderbookAction::PermissionnedOrderbookAction(
                orderbook_action.clone(),
                nonce,
            )
            .as_blob(self.ctx.orderbook_cn.clone())]
            .into(),
            tx_blob_count: 1,
            index: BlobIndex(0),
            private_input,
            tx_ctx: Default::default(), // Will be set when proving
        };

        let pending_tx = PendingTx {
            commitment_metadata,
            calldata,
        };

        Ok(pending_tx)
    }

    async fn handle_node_state_event(&mut self, event: NodeStateEvent) -> Result<()> {
        match event {
            NodeStateEvent::NewBlock(block) => {
                tracing::debug!("New block received: {:?}", block);

                let tx_hashes: Vec<TxHash> = block
                    .parsed_block
                    .txs
                    .iter()
                    .map(|(tx_id, _)| tx_id.1.clone())
                    .collect();

                if tx_hashes.is_empty() {
                    // No transactions to prove
                    return Ok(());
                }

                // Get the first transaction hash for computing tx_ctx
                let first_tx_hash = &tx_hashes[0];

                // Compute tx_ctx that will be used to prove transactions
                let tx_ctx = block.parsed_block.build_tx_ctx(first_tx_hash)?;

                // Extract all transactions that need to be proved and that have been sequenced
                let mut txs_to_prove = Vec::new();
                for tx_hash in tx_hashes {
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
