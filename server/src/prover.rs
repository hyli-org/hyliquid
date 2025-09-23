use std::{collections::BTreeMap, sync::Arc};

use anyhow::{bail, Result};
use client_sdk::{helpers::ClientSdkProver, rest_client::NodeApiClient};
use hyli_modules::{
    bus::{BusMessage, SharedMessageBus},
    log_error, module_bus_client, module_handle_messages,
    modules::Module,
};
use sdk::{Calldata, ContractName, LaneId, NodeStateEvent, ProofTransaction, TxHash};
use tracing::{debug, error};

#[derive(Debug, Clone)]
pub struct PendingTx {
    pub commitment_metadata: Vec<u8>,
    pub calldata: Calldata,
}

#[derive(Debug, Clone)]
pub enum OrderbookProverRequest {
    TxToProve {
        commitment_metadata: Vec<u8>,
        calldata: Calldata,
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
}

pub struct OrderbookProverModule {
    ctx: Arc<OrderbookProverCtx>,
    bus: OrderbookProverBusClient,
    pending_txs: BTreeMap<TxHash, PendingTx>,
}

impl Module for OrderbookProverModule {
    type Context = Arc<OrderbookProverCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let bus = OrderbookProverBusClient::new_from_bus(bus.new_handle()).await;
        Ok(OrderbookProverModule {
            ctx,
            bus,
            pending_txs: BTreeMap::new(),
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
                commitment_metadata,
                calldata,
                tx_hash,
            } => {
                let pending_tx = PendingTx {
                    commitment_metadata,
                    calldata,
                };

                self.pending_txs.insert(tx_hash.clone(), pending_tx);

                debug!("Transaction {tx_hash:#} added to pending transactions queue");
            }
        }
        Ok(())
    }

    async fn handle_node_state_event(&mut self, event: NodeStateEvent) -> Result<()> {
        match event {
            NodeStateEvent::NewBlock(block) => {
                tracing::debug!("New block received: {:?}", block);

                let first_tx_hash = match self.pending_txs.keys().next() {
                    Some(tx_hash) => tx_hash,
                    None => {
                        // There's not transaction to prove.
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

                        calldata.tx_hash = tx_hash.clone();
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
