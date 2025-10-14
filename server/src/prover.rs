use std::{collections::BTreeMap, sync::Arc};

use anyhow::{anyhow, bail, Result};
use client_sdk::{helpers::ClientSdkProver, rest_client::NodeApiClient};
use hyli_modules::{
    bus::{BusMessage, SharedMessageBus},
    log_error, module_bus_client, module_handle_messages,
    modules::Module,
};
use orderbook::{
    model::{OrderbookEvent, UserInfo},
    transaction::{OrderbookAction, PermissionnedOrderbookAction, PermissionnedPrivateInput},
    zk::FullState,
    ORDERBOOK_ACCOUNT_IDENTITY,
};
use sdk::{BlobIndex, Calldata, ContractName, LaneId, NodeStateEvent, ProofTransaction, TxHash};
use tracing::{debug, error, info};

#[derive(Debug, Clone)]
pub struct PendingTx {
    pub commitment_metadata: Vec<u8>,
    pub calldata: Calldata,
}

#[derive(Debug, Clone)]
pub enum OrderbookProverRequest {
    TxToProve {
        user_info: UserInfo,
        events: Vec<OrderbookEvent>,
        orderbook_action: PermissionnedOrderbookAction,
        action_private_input: Vec<u8>,
        tx_hash: TxHash,
    },
}

impl BusMessage for OrderbookProverRequest {}

module_bus_client! {
    #[derive(Debug)]
    struct OrderbookProverBusClient {
        receiver(OrderbookProverRequest),
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
}

pub struct OrderbookProverModule {
    ctx: Arc<OrderbookProverCtx>,
    bus: OrderbookProverBusClient,
    pending_txs: BTreeMap<TxHash, PendingTx>,
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
            pending_txs: BTreeMap::new(),
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

            listen<OrderbookProverRequest> request => {
                _ = log_error!(self.handle_prover_request(request).await, "handle prover request")
            }

            listen<NodeStateEvent> event => {
                _ = log_error!(self.handle_node_state_event(event).await, "handle node state event")
            }
        };
        Ok(())
    }

    async fn handle_prover_request(&mut self, request: OrderbookProverRequest) -> Result<()> {
        match request {
            OrderbookProverRequest::TxToProve {
                events,
                user_info,
                action_private_input,
                orderbook_action,
                tx_hash,
            } => {
                // The goal is to create commitment metadata that contains the proofs to be able to load the zkvm state into the zkvm

                // We generate the commitment metadata from the zkvm state
                // We then execute the action with the complete orderbook to compare the events and update the state

                let commitment_metadata = self
                    .orderbook
                    .derive_zkvm_commitment_metadata_from_events(
                        &user_info,
                        &events,
                        &orderbook_action,
                    )
                    .map_err(|e| anyhow!("Could not derive zkvm state: {e}"))?;

                let permissioned_private_input = PermissionnedPrivateInput {
                    secret: vec![1, 2, 3],
                    user_info: user_info.clone(),
                    private_input: action_private_input.clone(),
                };

                let execution_events = self
                    .orderbook
                    .execute_permissionned_action(
                        user_info.clone(),
                        orderbook_action.clone(),
                        &permissioned_private_input.private_input,
                    )
                    .map_err(|e| anyhow!("failed to execute orderbook tx: {e}"))?;

                // This should NEVER happen. If it happens, it means there is a difference in execution logic between api and prover.
                // FIXME: we should compare elements and not lengths
                if events.len() != execution_events.len() {
                    bail!("The provided events do not match the executed events. This should NEVER happen. Provided: {events:#?}, Executed: {execution_events:#?}");
                }

                let private_input = borsh::to_vec(&permissioned_private_input)?;

                let calldata = Calldata {
                    identity: ORDERBOOK_ACCOUNT_IDENTITY.into(),
                    tx_hash: tx_hash.clone(),
                    blobs: vec![OrderbookAction::PermissionnedOrderbookAction(
                        orderbook_action.clone(),
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

                self.pending_txs.insert(tx_hash.clone(), pending_tx);

                debug!(
                    tx_hash = %tx_hash,
                    events = ?events,
                    "Transaction added to pending transactions queue"
                );
            }
        }
        Ok(())
    }

    async fn handle_node_state_event(&mut self, event: NodeStateEvent) -> Result<()> {
        match event {
            NodeStateEvent::NewBlock(block) => {
                tracing::debug!("New block received: {:?}", block);

                let first_tx_hash = match self.pending_txs.keys().next() {
                    Some(tx_hash) => {
                        if block
                            .parsed_block
                            .txs
                            .iter()
                            .any(|(id, _)| id.1 == *tx_hash)
                        {
                            tx_hash
                        } else {
                            // If the transaction is not in the block, we can't prove it.
                            return Ok(());
                        }
                    }
                    None => {
                        // There's no transaction to prove.
                        return Ok(());
                    }
                };

                // Compute tx_ctx that will be used to prove transactions
                let tx_ctx = block.parsed_block.build_tx_ctx(first_tx_hash)?;

                let tx_hashes: Vec<TxHash> = block
                    .parsed_block
                    .txs
                    .iter()
                    .map(|(tx_id, _)| tx_id.1.clone())
                    .collect();

                // Extract all transactions that need to be proved and that have been sequenced
                let mut txs_to_prove = Vec::new();
                for tx_hash in tx_hashes {
                    if let Some(pending_tx) = self.pending_txs.remove(&tx_hash) {
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
                Ok(())
            }
        }
    }
}
