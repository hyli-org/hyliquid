use std::sync::Arc;

use anyhow::{bail, Result};
use client_sdk::{helpers::ClientSdkProver, rest_client::NodeApiClient};
use hyli_modules::{
    bus::{BusMessage, SharedMessageBus},
    module_bus_client, module_handle_messages,
    modules::Module,
};
use hyli_net::logged_task::logged_task;
use sdk::{Calldata, ContractName, ProofTransaction, TxContext, TxHash};
use tracing::{debug, error};

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
    }
}

pub struct OrderbookProverCtx {
    // TODO: Persist data
    // pub data_directory: PathBuf,
    pub prover: Arc<dyn ClientSdkProver<Vec<Calldata>> + Send + Sync>,
    pub orderbook_cn: ContractName,
    pub node_client: Arc<dyn NodeApiClient + Send + Sync>,
}

pub struct OrderbookProverModule {
    ctx: Arc<OrderbookProverCtx>,
    bus: OrderbookProverBusClient,
}

impl Module for OrderbookProverModule {
    type Context = Arc<OrderbookProverCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let bus = OrderbookProverBusClient::new_from_bus(bus.new_handle()).await;
        Ok(OrderbookProverModule { ctx, bus })
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
                self.handle_prover_request(request).await?;
            }
        };
        Ok(())
    }

    async fn handle_prover_request(&self, request: OrderbookProverRequest) -> Result<()> {
        // TODO: Batch multiple calldatas into a single proof
        match request {
            OrderbookProverRequest::TxToProve {
                commitment_metadata,
                mut calldata,
                tx_hash,
            } => {
                let prover = self.ctx.prover.clone();
                let contract_name = self.ctx.orderbook_cn.clone();
                let node_client = self.ctx.node_client.clone();

                logged_task(async move {
                    // Reconstruct tx_context
                    // FIXME: We need to get the context info from somewhere... Either a nodecall or listing to new block events...
                    let tx_ctx = TxContext {
                        ..Default::default()
                    };

                    calldata.tx_hash = tx_hash.clone();
                    calldata.tx_ctx = Some(tx_ctx);

                    match prover.prove(commitment_metadata, vec![calldata]).await {
                        Ok(proof) => {
                            let tx = ProofTransaction {
                                contract_name: contract_name.clone(),
                                program_id: prover.program_id(),
                                verifier: prover.verifier(),
                                proof: proof.data,
                            };

                            match node_client.send_tx_proof(tx).await {
                                Ok(tx_hash) => {
                                    debug!("Successfully sent proof: {tx_hash:#}");
                                }
                                Err(e) => {
                                    error!("Failed to send proof: {e:#}");
                                }
                            }
                        }
                        Err(e) => {
                            bail!("failed to generate proof: {e:#}");
                        }
                    };
                    Ok(())
                });
            }
        }
        Ok(())
    }
}
