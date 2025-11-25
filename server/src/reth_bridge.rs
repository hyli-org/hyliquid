use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use anyhow::anyhow;
use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use client_sdk::{
    contract_indexer::AppError,
    rest_client::{NodeApiClient, NodeApiHttpClient},
};
use hex;
use hyli_modules::modules::{BuildApiContextInner, Module};
use orderbook::{
    model::WithdrawDestination,
    transaction::{OrderbookAction, PermissionnedOrderbookAction},
};
use sdk::{Blob, BlobData, BlobTransaction, ContractName, Identity, StructuredBlobData, TxHash};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

use hyli_modules::{
    bus::{BusClientSender, SharedMessageBus},
    module_bus_client,
};

use crate::app::{OrderbookRequest, PendingDeposit, PendingWithdraw};

/// Alternate bridge module built around an embedded Reth flow.
///
/// This keeps the same bus interface as the existing bridge and only wires the
/// API surface plus in-memory job tracking for now. The Reth-driven execution
/// and proof submission follow the plan in `README.bridge.md`.
pub struct RethBridgeModule {
    bus: RethBridgeBusClient,
    orderbook_cn: ContractName,
    collateral_token_cn: ContractName,
    client: Arc<NodeApiHttpClient>,
    job_store: Arc<RwLock<HashMap<String, BridgeJobStatus>>>,
    job_rx: Option<mpsc::UnboundedReceiver<BridgeJob>>,
}

pub struct RethBridgeModuleCtx {
    pub api: Arc<BuildApiContextInner>,
    pub orderbook_cn: ContractName,
    pub collateral_token_cn: ContractName,
    pub client: Arc<NodeApiHttpClient>,
    // Placeholder for future embedded Reth config (datadir, chain id, mnemonic, etc.)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeDepositRequest {
    pub identity: String,
    pub signed_tx_hex: String,
    pub amount: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeWithdrawRequest {
    pub identity: String,
    pub signed_tx_hex: String,
    pub destination: WithdrawDestination,
    pub amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeJobStatus {
    pub job_id: String,
    pub operation: String,
    pub status: String,
    pub l1_tx_hash: Option<String>,
    pub hyli_tx_hash: Option<String>,
    pub error: Option<String>,
    pub evm_proof_hex: Option<String>,
}

#[derive(Debug, Clone)]
enum BridgeJobKind {
    Deposit {
        amount: u128,
    },
    Withdraw {
        destination: WithdrawDestination,
        amount: u64,
    },
}

#[derive(Debug)]
struct BridgeJob {
    job_id: String,
    identity: Identity,
    raw_tx: Vec<u8>,
    kind: BridgeJobKind,
}

module_bus_client! {
#[derive(Debug)]
    pub struct RethBridgeBusClient {
        sender(OrderbookRequest),
        // No receiver: this module pushes into orderbook via bus once jobs complete.
    }
}

#[derive(Clone)]
struct RouterCtx {
    job_store: Arc<RwLock<HashMap<String, BridgeJobStatus>>>,
    job_tx: mpsc::UnboundedSender<BridgeJob>,
}

fn next_job_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!("job-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

impl Module for RethBridgeModule {
    type Context = Arc<RethBridgeModuleCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> anyhow::Result<Self> {
        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(vec![axum::http::Method::GET, axum::http::Method::POST])
            .allow_headers(Any);

        let job_store = Arc::new(RwLock::new(HashMap::new()));
        let (job_tx, job_rx) = mpsc::unbounded_channel::<BridgeJob>();
        let router_ctx = RouterCtx {
            job_store: job_store.clone(),
            job_tx: job_tx.clone(),
        };

        let api = Router::new()
            .route("/reth_bridge/deposit", post(deposit))
            .route("/reth_bridge/withdraw", post(withdraw))
            .route("/reth_bridge/status/{job_id}", get(status))
            .layer(Extension(router_ctx))
            .layer(cors);

        if let Ok(mut guard) = ctx.api.router.lock() {
            if let Some(router) = guard.take() {
                guard.replace(router.merge(api));
            }
        }

        Ok(Self {
            bus: RethBridgeBusClient::new_from_bus(bus.new_handle()).await,
            orderbook_cn: ctx.orderbook_cn.clone(),
            collateral_token_cn: ctx.collateral_token_cn.clone(),
            client: ctx.client.clone(),
            job_store,
            job_rx: Some(job_rx),
        })
    }

    async fn run(&mut self) -> anyhow::Result<()> {
        // Drain jobs sequentially for now. In the fuller implementation, this loop will:
        // 1) submit the raw tx into embedded Reth,
        // 2) build the stateless proof from the block witness,
        // 3) craft the two-blob Hyli tx and submit paired proofs,
        // 4) push `PendingDeposit`/`PendingWithdraw` into the bus.
        let mut job_rx = self
            .job_rx
            .take()
            .expect("job receiver should be initialized");
        while let Some(job) = job_rx.recv().await {
            self.process_job(job).await;
        }

        Ok(())
    }
}

async fn deposit(
    Extension(router_ctx): Extension<RouterCtx>,
    Json(request): Json<BridgeDepositRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Validate identity parsing early.
    let identity = Identity(request.identity.clone());
    let raw_tx = hex::decode(request.signed_tx_hex.trim_start_matches("0x"))
        .map_err(|err| AppError(StatusCode::BAD_REQUEST, anyhow::anyhow!(err)))?;

    // Create a simple in-memory job entry.
    let job_id = next_job_id();

    let status = BridgeJobStatus {
        job_id: job_id.clone(),
        operation: "deposit".to_string(),
        status: "queued".to_string(),
        l1_tx_hash: None,
        hyli_tx_hash: None,
        error: None,
        evm_proof_hex: None,
    };

    {
        let mut store = router_ctx.job_store.write().await;
        store.insert(job_id.clone(), status.clone());
    }

    // Enqueue for background processing.
    if let Err(err) = router_ctx.job_tx.send(BridgeJob {
        job_id: job_id.clone(),
        identity,
        raw_tx,
        kind: BridgeJobKind::Deposit {
            amount: request.amount,
        },
    }) {
        warn!(job_id = %job_id, error = %err, "failed to enqueue bridge job");
    }

    info!(
        job_id = %job_id,
        identity = %request.identity,
        amount = request.amount,
        "accepted reth bridge deposit request"
    );

    Ok((StatusCode::ACCEPTED, Json(status)))
}

async fn withdraw(
    Extension(router_ctx): Extension<RouterCtx>,
    Json(request): Json<BridgeWithdrawRequest>,
) -> Result<impl IntoResponse, AppError> {
    let identity = Identity(request.identity.clone());
    let raw_tx = hex::decode(request.signed_tx_hex.trim_start_matches("0x"))
        .map_err(|err| AppError(StatusCode::BAD_REQUEST, anyhow::anyhow!(err)))?;

    let job_id = next_job_id();

    let status = BridgeJobStatus {
        job_id: job_id.clone(),
        operation: "withdraw".to_string(),
        status: "queued".to_string(),
        l1_tx_hash: None,
        hyli_tx_hash: None,
        error: None,
        evm_proof_hex: None,
    };

    {
        let mut store = router_ctx.job_store.write().await;
        store.insert(job_id.clone(), status.clone());
    }

    if let Err(err) = router_ctx.job_tx.send(BridgeJob {
        job_id: job_id.clone(),
        identity,
        raw_tx,
        kind: BridgeJobKind::Withdraw {
            destination: request.destination.clone(),
            amount: request.amount,
        },
    }) {
        warn!(job_id = %job_id, error = %err, "failed to enqueue bridge withdraw job");
    }

    info!(
        job_id = %job_id,
        identity = %request.identity,
        network = %request.destination.network,
        address = %request.destination.address,
        amount = request.amount,
        "accepted reth bridge withdraw request"
    );

    Ok((StatusCode::ACCEPTED, Json(status)))
}

#[axum::debug_handler]
async fn status(
    Extension(router_ctx): Extension<RouterCtx>,
    Path(job_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let store = router_ctx.job_store.read().await;
    let Some(status) = store.get(&job_id) else {
        return Err(AppError(StatusCode::NOT_FOUND, anyhow!("job not found")));
    };

    Ok((StatusCode::OK, Json(status.clone())))
}

impl RethBridgeModule {
    async fn process_job(&mut self, job: BridgeJob) {
        // Placeholder processing: future work will submit to embedded Reth and Hyli.
        {
            let mut store = self.job_store.write().await;
            if let Some(status) = store.get_mut(&job.job_id) {
                status.status = "processing".to_string();
            }
        }

        // TODO: replace with embedded Reth submission + proof construction + Hyli blob/proof submission.
        let result = self.submit_hyli(&job).await;

        let mut notify_orderbook = false;
        {
            let mut store = self.job_store.write().await;
            if let Some(status) = store.get_mut(&job.job_id) {
                match result {
                    Ok(hyli_hash) => {
                        status.hyli_tx_hash = Some(hyli_hash);
                        status.status = "completed".to_string();
                        notify_orderbook = true;
                    }
                    Err(err) => {
                        status.status = "failed".to_string();
                        status.error = Some(err.to_string());
                    }
                }
            }
        }

        if notify_orderbook {
            if let Err(err) = self.forward_orderbook_request(&job).await {
                warn!(
                    job_id = %job.job_id,
                    error = %err,
                    "failed to forward job to orderbook"
                );
            }
        }

        info!(job_id = %job.job_id, "completed reth bridge job");
    }

    async fn submit_hyli(&self, job: &BridgeJob) -> anyhow::Result<String> {
        let collateral_blob = Blob {
            contract_name: self.collateral_token_cn.clone(),
            data: BlobData::from(StructuredBlobData {
                caller: None,
                callees: None,
                parameters: job.raw_tx.clone(),
            }),
        };

        let symbol = self.collateral_token_cn.0.clone();
        let orderbook_action = match &job.kind {
            BridgeJobKind::Deposit { amount } => {
                let deposit_amount = u64::try_from(*amount).map_err(|_| {
                    anyhow!("deposit amount {} exceeds supported range (u64)", amount)
                })?;
                PermissionnedOrderbookAction::DepositRethBridge {
                    symbol: symbol.clone(),
                    amount: deposit_amount,
                }
            }
            BridgeJobKind::Withdraw {
                destination,
                amount,
            } => PermissionnedOrderbookAction::Withdraw {
                symbol: symbol.clone(),
                amount: *amount,
                destination: destination.clone(),
            },
        };

        let orderbook_blob = OrderbookAction::PermissionnedOrderbookAction(orderbook_action, 0)
            .as_blob(self.orderbook_cn.clone());

        let blob_tx =
            BlobTransaction::new(job.identity.clone(), vec![collateral_blob, orderbook_blob]);
        let tx_hash: TxHash = self.client.send_tx_blob(blob_tx).await?;
        Ok(format!("0x{}", hex::encode(tx_hash.0.as_bytes())))
    }

    async fn forward_orderbook_request(&mut self, job: &BridgeJob) -> anyhow::Result<()> {
        match &job.kind {
            BridgeJobKind::Deposit { amount } => {
                let deposit = PendingDeposit {
                    sender: job.identity.clone(),
                    contract_name: self.collateral_token_cn.clone(),
                    amount: *amount,
                };
                self.bus
                    .send(OrderbookRequest::PendingDeposit(deposit))
                    .map_err(anyhow::Error::from)?;
            }
            BridgeJobKind::Withdraw {
                destination,
                amount,
            } => {
                let withdraw = PendingWithdraw {
                    destination: destination.clone(),
                    contract_name: self.collateral_token_cn.clone(),
                    amount: *amount,
                };
                self.bus
                    .send(OrderbookRequest::PendingWithdraw(withdraw))
                    .map_err(anyhow::Error::from)?;
            }
        }

        Ok(())
    }
}
