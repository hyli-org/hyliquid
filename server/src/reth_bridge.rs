use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use axum::{
    extract::{Extension, Path},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use client_sdk::{contract_indexer::AppError, rest_client::NodeApiHttpClient};
use anyhow::anyhow;
use hex;
use hyli_modules::modules::{BuildApiContextInner, Module};
use sdk::Identity;
use rand;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, warn};

use hyli_modules::{
    bus::{BusClientSender, SharedMessageBus},
    module_bus_client,
};

use sdk::ContractName;
use reth_harness::RethHarness;
use tokio::sync::Mutex;

use crate::app::OrderbookRequest;

/// Alternate bridge module built around an embedded Reth flow.
///
/// This keeps the same bus interface as the existing bridge and only wires the
/// API surface plus in-memory job tracking for now. The Reth-driven execution
/// and proof submission follow the plan in `README.bridge.md`.
pub struct RethBridgeModule {
    #[allow(dead_code)]
    bus: RethBridgeBusClient,
    #[allow(dead_code)]
    orderbook_cn: ContractName,
    #[allow(dead_code)]
    collateral_token_cn: ContractName,
    client: Arc<NodeApiHttpClient>,
    job_store: Arc<RwLock<HashMap<String, BridgeJobStatus>>>,
    job_tx: mpsc::UnboundedSender<BridgeJob>,
    job_rx: Option<mpsc::UnboundedReceiver<BridgeJob>>,
    reth: Arc<Mutex<RethHarness>>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeJobStatus {
    pub job_id: String,
    pub status: String,
    pub l1_tx_hash: Option<String>,
    pub hyli_tx_hash: Option<String>,
    pub error: Option<String>,
    pub evm_proof_hex: Option<String>,
}

#[derive(Debug)]
struct BridgeJob {
    job_id: String,
    identity: Identity,
    raw_tx: Vec<u8>,
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
            .route("/reth_bridge/status/:job_id", get(status))
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
            job_tx,
            job_rx: Some(job_rx),
            reth: Arc::new(Mutex::new(
                RethHarness::new().await.map_err(|err| anyhow!(err))?,
            )),
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

#[axum::debug_handler]
async fn deposit(
    Extension(router_ctx): Extension<RouterCtx>,
    Json(request): Json<BridgeDepositRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Validate identity parsing early.
    let identity = Identity(request.identity.clone());
    let raw_tx = hex::decode(request.signed_tx_hex.trim_start_matches("0x"))
        .map_err(|err| AppError(StatusCode::BAD_REQUEST, anyhow::anyhow!(err)))?;

    // Create a simple in-memory job entry.
    let job_id = {
        // Use a short numeric id to avoid new dependencies.
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!("job-{}", COUNTER.fetch_add(1, Ordering::Relaxed))
    };

    let status = BridgeJobStatus {
        job_id: job_id.clone(),
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
    }) {
        warn!(job_id = %job_id, error = %err, "failed to enqueue bridge job");
    }

    info!(
        job_id = %job_id,
        identity = %request.identity,
        "accepted reth bridge deposit request"
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
        return Ok((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "job not found"})),
        ));
    };

    Ok((StatusCode::OK, Json(status.clone())))
}

impl RethBridgeModule {
    async fn process_job(&self, job: BridgeJob) {
        // Placeholder processing: future work will submit to embedded Reth and Hyli.
        {
            let mut store = self.job_store.write().await;
            if let Some(status) = store.get_mut(&job.job_id) {
                status.status = "processing".to_string();
            }
        }

        // TODO: replace with embedded Reth submission + proof construction + Hyli blob/proof submission.
        let result = self
            .submit_and_prove(job.identity.clone(), job.raw_tx.clone())
            .await;

        {
            let mut store = self.job_store.write().await;
            if let Some(status) = store.get_mut(&job.job_id) {
                match result {
                    Ok((l1_hash, hyli_hash, evm_proof_hex)) => {
                        status.l1_tx_hash = Some(l1_hash);
                        status.hyli_tx_hash = Some(hyli_hash);
                        status.evm_proof_hex = Some(evm_proof_hex);
                        status.status = "completed".to_string();
                    }
                    Err(err) => {
                        status.status = "failed".to_string();
                        status.error = Some(err.to_string());
                    }
                }
            }
        }

        info!(job_id = %job.job_id, "completed reth bridge job");
    }

    async fn submit_and_prove(
        &self,
        identity: Identity,
        raw_tx: Vec<u8>,
    ) -> anyhow::Result<(String, String, String)> {
        // 1. Submit raw_tx to embedded Reth (placeholder).
        let (l1_tx_hash, evm_proof) = {
            let mut reth = self.reth.lock().await;
            reth.submit_raw_tx(raw_tx.clone()).await.map_err(|err| anyhow!(err))?
        };

        // 3. Craft two-blob Hyli tx (ERC20 blob + Orderbook blob) and submit paired proofs (placeholder).
        let hyli_tx_hash = self.submit_hyli(identity, evm_proof).await?;

        let evm_proof_hex = format!("0x{}", hex::encode(evm_proof));
        Ok((l1_tx_hash, hyli_tx_hash, evm_proof_hex))
    }

    async fn submit_hyli(&self, _identity: Identity, _evm_proof: Vec<u8>) -> anyhow::Result<String> {
        // TODO: craft two-blob Hyli transaction (ERC20 blob + Orderbook blob with caller/callee)
        // and submit EVM proof + orderbook prover proof via self.client.
        warn!("submit_hyli is not yet implemented; returning placeholder hash");
        Ok(format!(
            "0xhyli_{}",
            hex::encode(rand::random::<[u8; 4]>())
        ))
    }
}

/// Minimal harness placeholder for the embedded Reth devnode. This isolates the
/// eventual eth_api/debug_api plumbing used to submit transactions and produce
/// stateless proofs (mirroring the deposit demo helpers).
#[derive(Clone, Default)]
struct RethHarness;

impl RethHarness {
    fn new() -> Self {
        // TODO: initialize embedded Reth devnode and capture eth_api/debug_api handles.
        Self
    }

    async fn submit_raw_tx(&self, raw_tx: Vec<u8>) -> Option<String> {
        // TODO: wire to embedded Reth eth_api.send_raw_transaction and wait for inclusion.
        warn!("submit_raw_tx is not yet implemented; using placeholder hash");
        Some(format!("0xl1_{}", hex::encode(&raw_tx[..4.min(raw_tx.len())])))
    }

    async fn build_stateless_proof(&self) -> anyhow::Result<Vec<u8>> {
        // TODO: fetch block + execution witness from debug API and run stateless validation.
        warn!("build_stateless_proof is not yet implemented; returning placeholder proof bytes");
        Ok(vec![0u8; 1])
    }
}
