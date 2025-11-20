use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    time::Instant,
    vec,
};

use anyhow::{anyhow, bail, Context, Result};
use axum::{
    extract::{Json, State},
    http::{HeaderMap, Method},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use borsh::BorshSerialize;
use client_sdk::{contract_indexer::AppError, rest_client::NodeApiHttpClient};
use hex;
use hyli_modules::{
    bus::{BusClientSender, BusMessage, SharedMessageBus},
    log_error, log_warn, module_bus_client, module_handle_messages,
    modules::{BuildApiContextInner, Module},
};
use hyli_smt_token::SmtTokenAction;
use opentelemetry::{
    metrics::{Counter, Histogram, Meter},
    KeyValue,
};
use orderbook::{
    model::{AssetInfo, Order, OrderbookEvent, PairInfo, UserInfo, WithdrawDestination},
    transaction::{
        AddSessionKeyPrivateInput, CancelOrderPrivateInput, CreateOrderPrivateInput,
        OrderbookAction, PermissionnedOrderbookAction, WithdrawPrivateInput,
    },
    zk::smt::GetKey,
    ORDERBOOK_ACCOUNT_IDENTITY,
};
use reqwest::StatusCode;
use sdk::{BlobTransaction, ContractAction, ContractName, Hashed, Identity, LaneId};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tracing::debug;

use crate::{
    database::DatabaseRequest, prover::OrderbookProverRequest,
    services::asset_service::AssetService,
};
use rand::RngCore;

/// Metrics for tracking HTTP request performance
#[derive(Clone)]
pub struct AppMetrics {
    /// Duration of HTTP requests by endpoint
    pub http_request_duration: Histogram<f64>,
    /// Count of HTTP requests by endpoint and status
    pub http_request_count: Counter<u64>,
    /// Duration of orderbook operations (overall including lock + method + apply)
    pub orderbook_operation_duration: Histogram<f64>,
    /// Duration of specific orderbook method calls (business logic only)
    pub orderbook_method_duration: Histogram<f64>,
    /// Count of orderbook lock acquisitions
    pub orderbook_lock_duration: Histogram<f64>,
    /// Event processing duration
    pub event_apply_duration: Histogram<f64>,
}

impl AppMetrics {
    pub fn new() -> Self {
        let meter = opentelemetry::global::meter("app");
        Self::with_meter(meter)
    }

    pub fn with_meter(meter: Meter) -> Self {
        // Custom buckets for millisecond-level latencies
        // Covers: 1ms, 2.5ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s, 10s, 25s, 50s, 100s, 250s, 500s, 1000s
        let latency_buckets = vec![
            0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 25.0,
            50.0, 100.0, 250.0, 500.0, 1000.0,
        ];

        // Tighter buckets for lock duration (expecting microsecond to low millisecond range)
        // Covers: 10μs, 50μs, 100μs, 500μs, 1ms, 5ms, 10ms, 50ms, 100ms, 250ms, 500ms, 1000ms, 2500ms, 5000ms, 10000ms, 25000ms, 50000ms, 100000ms
        let lock_buckets = vec![
            0.00001, 0.00005, 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
            5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0, 25000.0,
            50000.0, 100000.0,
        ];

        Self {
            http_request_duration: meter
                .f64_histogram("http.request.duration")
                .with_description("HTTP request duration in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            http_request_count: meter
                .u64_counter("http.request.count")
                .with_description("Total HTTP requests")
                .build(),
            orderbook_operation_duration: meter
                .f64_histogram("orderbook.operation.duration")
                .with_description("Orderbook operation duration in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            orderbook_method_duration: meter
                .f64_histogram("orderbook.method.duration")
                .with_description("Orderbook method call duration in seconds (business logic only)")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            orderbook_lock_duration: meter
                .f64_histogram("orderbook.lock.duration")
                .with_description("Duration of orderbook lock acquisition in seconds")
                .with_unit("s")
                .with_boundaries(lock_buckets)
                .build(),
            event_apply_duration: meter
                .f64_histogram("orderbook.event.apply.duration")
                .with_description("Duration of applying events to orderbook in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets)
                .build(),
        }
    }

    #[inline]
    fn record_request(&self, start: Instant, endpoint: &str, status: u16) {
        let duration = start.elapsed().as_secs_f64();
        self.http_request_duration.record(
            duration,
            &[
                KeyValue::new("endpoint", endpoint.to_string()),
                KeyValue::new("status", status.to_string()),
            ],
        );
        self.http_request_count.add(
            1,
            &[
                KeyValue::new("endpoint", endpoint.to_string()),
                KeyValue::new("status", status.to_string()),
            ],
        );
    }

    #[inline]
    fn record_operation(&self, start: Instant, operation: &str) {
        let duration = start.elapsed().as_secs_f64();
        self.orderbook_operation_duration.record(
            duration,
            &[KeyValue::new("operation", operation.to_string())],
        );
    }

    #[inline]
    fn record_lock(&self, start: Instant) {
        let duration = start.elapsed().as_secs_f64();
        self.orderbook_lock_duration.record(duration, &[]);
    }

    #[inline]
    fn record_event_apply(&self, start: Instant) {
        let duration = start.elapsed().as_secs_f64();
        self.event_apply_duration.record(duration, &[]);
    }

    #[inline]
    fn record_method(&self, start: Instant, method: &str) {
        let duration = start.elapsed().as_secs_f64();
        self.orderbook_method_duration
            .record(duration, &[KeyValue::new("method", method.to_string())]);
    }
}

impl Default for AppMetrics {
    fn default() -> Self {
        Self::new()
    }
}

pub struct OrderbookModule {
    bus: OrderbookModuleBusClient,
    router_ctx: RouterCtx,
}

pub struct OrderbookModuleCtx {
    pub api: Arc<BuildApiContextInner>,
    pub orderbook_cn: ContractName,
    pub lane_id: LaneId,
    pub default_state: orderbook::model::ExecuteState,
    pub client: Arc<NodeApiHttpClient>,
    pub asset_service: Arc<RwLock<AssetService>>,
}

#[derive(Debug, Clone)]
pub enum OrderbookRequest {
    PendingDeposit(PendingDeposit),
    PendingWithdraw(PendingWithdraw),
}

impl BusMessage for OrderbookRequest {}

#[derive(Debug, Clone)]
pub struct PendingDeposit {
    pub sender: Identity,
    pub contract_name: ContractName,
    pub amount: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingWithdraw {
    pub destination: WithdrawDestination,
    pub contract_name: ContractName,
    pub amount: u64,
}

module_bus_client! {
#[derive(Debug)]
pub struct OrderbookModuleBusClient {
    sender(DatabaseRequest),
    receiver(OrderbookRequest),
}
}

module_bus_client! {
#[derive(Debug)]
struct RouterBusClient {
    sender(DatabaseRequest),
    // No receiver here ! Because RouterBus is cloned
}
}

impl Module for OrderbookModule {
    type Context = Arc<OrderbookModuleCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let orderbook = Arc::new(Mutex::new(ctx.default_state.clone()));

        let router_bus = RouterBusClient::new_from_bus(bus.new_handle()).await;
        let bus = OrderbookModuleBusClient::new_from_bus(bus.new_handle()).await;

        let router_ctx = RouterCtx {
            orderbook_cn: ctx.orderbook_cn.clone(),
            default_state: ctx.default_state.clone(),
            bus: router_bus.clone(),
            orderbook: orderbook.clone(),
            lane_id: ctx.lane_id.clone(),
            asset_service: ctx.asset_service.clone(),
            client: ctx.client.clone(),
            action_id_counter: Arc::new(AtomicU32::new(0)),
            metrics: AppMetrics::new(),
        };

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(vec![Method::GET, Method::POST])
            .allow_headers(Any);

        let api = Router::new()
            .route("/create_pair", post(create_pair))
            .route("/add_session_key", post(add_session_key))
            .route("/deposit", post(deposit))
            .route("/create_order", post(create_order))
            .route("/cancel_order", post(cancel_order))
            .route("/withdraw", post(withdraw))
            .route("/nonce", get(get_nonce))
            // FIXME: to be removed. Only here for debugging purposes
            .route("/state", get(get_state))
            .with_state(router_ctx.clone())
            .layer(cors);

        if let Ok(mut guard) = ctx.api.router.lock() {
            if let Some(router) = guard.take() {
                guard.replace(router.merge(api));
            }
        }

        Ok(OrderbookModule { bus, router_ctx })
    }

    async fn run(&mut self) -> Result<()> {
        module_handle_messages! {
            on_self self,

            listen<OrderbookRequest> event => {
                match event {
                    OrderbookRequest::PendingDeposit(deposit) => {
                        _ = log_error!(self.execute_deposit(deposit)
                            .await, "could not deposit transfer")
                    }
                    OrderbookRequest::PendingWithdraw(withdraw) => {
                        _ =  log_error!(self.execute_withdraw(withdraw)
                            .await, "could not withdraw")
                    }
                }
            }
        };

        Ok(())
    }
}

impl OrderbookModule {
    async fn execute_deposit(&self, deposit: PendingDeposit) -> Result<()> {
        let PendingDeposit {
            sender,
            contract_name,
            amount,
        } = deposit;
        let asset_service = self.router_ctx.asset_service.read().await;

        let Identity(user) = sender;
        let Some(symbol) = asset_service
            .get_symbol_from_contract_name(&contract_name.0)
            .await
        else {
            bail!(
                "Could not deposit: Unknown contract name: {}",
                contract_name.0
            );
        };
        let amount_u64 =
            u64::try_from(amount).context("Deposit amount exceeds supported range (u64)")?;

        let (user_info, events) = {
            let mut orderbook = self.router_ctx.orderbook.lock().await;
            let user_info = orderbook.get_user_info(&user).unwrap_or_else(|_| {
                let mut salt = [0u8; 32];
                rand::rng().fill_bytes(&mut salt);
                UserInfo::new(user.clone(), salt.to_vec())
            });

            let events = orderbook
                .deposit(&symbol, amount_u64, &user_info)
                .map_err(|e| anyhow!("Failed to apply deposit on orderbook: {e}"))?;

            orderbook
                .apply_events(&user_info, &events)
                .map_err(|e| anyhow!("Failed to update orderbook state after deposit: {e}"))?;

            (user_info, events)
        };

        let action_private_input = Vec::<u8>::new();

        let orderbook_action = PermissionnedOrderbookAction::Deposit {
            symbol,
            amount: amount_u64,
        };

        let _ = process_orderbook_action(
            user_info,
            events,
            orderbook_action,
            &action_private_input,
            &self.router_ctx,
        )
        .await
        .map_err(|AppError(_, inner)| anyhow!("Failed to submit deposit action: {inner}"))?;

        Ok(())
    }

    async fn execute_withdraw(&self, withdraw: PendingWithdraw) -> Result<()> {
        let PendingWithdraw {
            destination,
            contract_name,
            amount,
        } = withdraw;

        if destination.network != "hyli" {
            // Non-Hyli withdraws are handled by the bridge module directly.
            tracing::info!(
                network = %destination.network,
                address = %destination.address,
                amount,
                "Skipping Hyli transfer for non-Hyli withdraw destination"
            );
            return Ok(());
        }

        let orderbook_id_action = PermissionnedOrderbookAction::Identify;

        let transfer_blob = SmtTokenAction::Transfer {
            sender: Identity(ORDERBOOK_ACCOUNT_IDENTITY.to_string()),
            recipient: Identity(destination.address.to_string()),
            amount: amount as u128,
        }
        .as_blob(contract_name, None, None);

        let action_id = self
            .router_ctx
            .action_id_counter
            .fetch_add(1, Ordering::Relaxed);
        let blob_tx = BlobTransaction::new(
            ORDERBOOK_ACCOUNT_IDENTITY,
            vec![
                OrderbookAction::PermissionnedOrderbookAction(
                    orderbook_id_action.clone(),
                    action_id,
                )
                .as_blob(self.router_ctx.orderbook_cn.clone()),
                transfer_blob,
            ],
        );

        let tx_hash = blob_tx.hashed();

        let mut bus = self.bus.clone();
        bus.send(DatabaseRequest::WriteEvents {
            user: UserInfo::new(ORDERBOOK_ACCOUNT_IDENTITY.to_string(), Vec::new()),
            tx_hash: tx_hash.clone(),
            blob_tx,
            prover_request: OrderbookProverRequest {
                events: vec![],
                user_info: UserInfo::default(),
                action_private_input: vec![],
                orderbook_action: orderbook_id_action,
                tx_hash: tx_hash.clone(),
                nonce: action_id,
            },
        })?;
        Ok(())
    }
}

#[derive(Clone)]
#[allow(dead_code)]
struct RouterCtx {
    pub bus: RouterBusClient,
    pub orderbook_cn: ContractName,
    pub default_state: orderbook::model::ExecuteState,
    pub orderbook: Arc<Mutex<orderbook::model::ExecuteState>>,
    pub lane_id: LaneId,
    pub asset_service: Arc<RwLock<AssetService>>,
    pub client: Arc<NodeApiHttpClient>,
    pub action_id_counter: Arc<AtomicU32>,
    pub metrics: AppMetrics,
}

// --------------------------------------------------------
//     Headers
// --------------------------------------------------------

const IDENTITY_HEADER: &str = "x-identity";
const PUBLIC_KEY_HEADER: &str = "x-public-key";
const SIGNATURE_HEADER: &str = "x-signature";

#[derive(Debug)]
struct AuthHeaders {
    identity: String,
    public_key: Option<Vec<u8>>,
    signature: Option<Vec<u8>>,
}

impl AuthHeaders {
    fn from_headers(headers: &HeaderMap) -> Result<Self, AppError> {
        let identity = headers
            .get(IDENTITY_HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError(
                    StatusCode::UNAUTHORIZED,
                    anyhow::anyhow!("Missing identity"),
                )
            })?
            .to_string();

        let public_key: Option<Vec<u8>> = headers
            .get(PUBLIC_KEY_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| hex::decode(s).ok());

        let signature = headers
            .get(SIGNATURE_HEADER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| hex::decode(s).ok());

        Ok(AuthHeaders {
            identity,
            public_key,
            signature,
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CreatePairRequest {
    pub base_contract: String,
    pub quote_contract: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DepositRequest {
    pub symbol: String,
    pub amount: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CancelOrderRequest {
    pub order_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WithdrawRequest {
    pub symbol: String,
    pub amount: u64,
    pub destination: WithdrawDestination,
}

// API-friendly representation of OrderManager for JSON serialization
#[derive(Debug, Clone, Serialize)]
pub struct OrderManagerAPI {
    pub orders: std::collections::BTreeMap<String, Order>,
    pub bid_orders: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, std::collections::VecDeque<String>>,
    >,
    pub ask_orders: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<String, std::collections::VecDeque<String>>,
    >,
    pub orders_owner: std::collections::BTreeMap<String, String>,
}

impl From<&orderbook::order_manager::OrderManager> for OrderManagerAPI {
    fn from(manager: &orderbook::order_manager::OrderManager) -> Self {
        let orders_owner = manager
            .orders_owner
            .iter()
            .map(|(order_id, owner_key)| (order_id.clone(), hex::encode(owner_key.0.as_slice())))
            .collect();

        // Convert u64 price keys to strings and pair tuples to strings for JSON serialization
        let bid_orders = manager
            .bid_orders
            .iter()
            .map(|(pair, price_map)| {
                let api_price_map = price_map
                    .iter()
                    .map(|(price, orders)| (price.to_string(), orders.clone()))
                    .collect();
                let pair_string = format!("{}-{}", pair.0, pair.1);
                (pair_string, api_price_map)
            })
            .collect();

        let ask_orders = manager
            .ask_orders
            .iter()
            .map(|(pair, price_map)| {
                let api_price_map = price_map
                    .iter()
                    .map(|(price, orders)| (price.to_string(), orders.clone()))
                    .collect();
                let pair_string = format!("{}-{}", pair.0, pair.1);
                (pair_string, api_price_map)
            })
            .collect();

        OrderManagerAPI {
            orders: manager.orders.clone(),
            bid_orders,
            ask_orders,
            orders_owner,
        }
    }
}

// API-friendly representation of the state for JSON serialization
#[derive(Debug, Clone, Serialize)]
pub struct ExecuteStateAPI {
    pub assets_info: BTreeMap<String, AssetInfo>,
    pub users_info: BTreeMap<String, UserInfo>,
    pub balances: BTreeMap<String, BTreeMap<String, orderbook::model::Balance>>,
    pub order_manager: OrderManagerAPI,
}

impl From<&orderbook::model::ExecuteState> for ExecuteStateAPI {
    fn from(state: &orderbook::model::ExecuteState) -> Self {
        let balances = state
            .balances
            .iter()
            .map(|(symbol, balance_map)| {
                let api_balance_map = balance_map
                    .iter()
                    .map(|(key, balance)| (hex::encode(key.0.as_slice()), balance.clone()))
                    .collect();
                (symbol.clone(), api_balance_map)
            })
            .collect();

        ExecuteStateAPI {
            assets_info: state.assets_info.clone(),
            users_info: state.users_info.clone(),
            balances,
            order_manager: OrderManagerAPI::from(&state.order_manager),
        }
    }
}

// --------------------------------------------------------
//     Routes
// --------------------------------------------------------
async fn get_state(State(ctx): State<RouterCtx>) -> Result<impl IntoResponse, AppError> {
    let request_start = Instant::now();
    let endpoint = "get_state";

    let result = async {
        let lock_start = Instant::now();
        let orderbook = ctx.orderbook.lock().await;
        ctx.metrics.record_lock(lock_start);

        let api_state = ExecuteStateAPI::from(&*orderbook);
        Ok(Json(api_state))
    }
    .await;

    let status = match &result {
        Ok(_) => 200,
        Err(AppError(status, _)) => status.as_u16(),
    };
    ctx.metrics.record_request(request_start, endpoint, status);

    result
}

async fn get_nonce(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let request_start = Instant::now();
    let endpoint = "get_nonce";

    let result = async {
        let auth = AuthHeaders::from_headers(&headers)?;
        let user = auth.identity;

        // TODO: do some checks on headers to verify identify the user

        let lock_start = Instant::now();
        let orderbook = ctx.orderbook.lock().await;
        ctx.metrics.record_lock(lock_start);

        let nonce = orderbook
            .get_user_info(&user)
            .map(|u| u.nonce)
            .unwrap_or_default();

        Ok(Json(nonce))
    }
    .await;

    let status = match &result {
        Ok(_) => 200,
        Err(AppError(status, _)) => status.as_u16(),
    };
    ctx.metrics.record_request(request_start, endpoint, status);

    result
}

#[axum::debug_handler]
// #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(ctx)))]
async fn create_pair(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<CreatePairRequest>,
) -> Result<impl IntoResponse, AppError> {
    let request_start = Instant::now();
    let endpoint = "create_pair";

    let result = async {
        let auth = AuthHeaders::from_headers(&headers)?;

        if request.base_contract == request.quote_contract {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                anyhow::anyhow!("Base and quote asset cannot be the same"),
            ));
        }

        let user = auth.identity;

        let CreatePairRequest {
            base_contract,
            quote_contract,
        } = request;

        let asset_service = ctx.asset_service.read().await;

        let base_asset = asset_service
            .get_asset_from_contract_name(&base_contract)
            .await
            .ok_or(AppError(
                StatusCode::NOT_FOUND,
                anyhow::anyhow!("Base asset not found: {base_contract}"),
            ))?;
        let quote_asset = asset_service
            .get_asset_from_contract_name(&quote_contract)
            .await
            .ok_or(AppError(
                StatusCode::NOT_FOUND,
                anyhow::anyhow!("Quote asset not found: {quote_contract}"),
            ))?;

        if base_asset.scale >= 20 {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                anyhow::anyhow!(
                    "Unsupported pair scale: base_scale >= 20: {}",
                    base_asset.scale
                ),
            ));
        }
        if quote_asset.scale >= 20 {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                anyhow::anyhow!(
                    "Unsupported pair scale: quote_scale >= 20: {}",
                    quote_asset.scale
                ),
            ));
        }

        let base_info = AssetInfo::new(base_asset.scale as u64, base_contract.into());
        let quote_info = AssetInfo::new(quote_asset.scale as u64, quote_contract.into());

        let info = PairInfo {
            base: base_info,
            quote: quote_info,
        };
        let pair = (base_asset.symbol.clone(), quote_asset.symbol.clone());
        drop(asset_service);

        let operation_start = Instant::now();
        let (user_info, events) = {
            let lock_start = Instant::now();
            let mut orderbook = ctx.orderbook.lock().await;
            ctx.metrics.record_lock(lock_start);

            // Get user_info if exists, otherwise create a new one with random salt
            let user_info = orderbook.get_user_info(&user).unwrap_or_else(|_| {
                let mut salt = [0u8; 32];
                rand::rng().fill_bytes(&mut salt);
                UserInfo::new(user.clone(), salt.to_vec())
            });

            let method_start = Instant::now();
            let events = orderbook
                .create_pair(&pair, &info)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
            ctx.metrics.record_method(method_start, "create_pair");

            let apply_start = Instant::now();
            orderbook
                .apply_events(&user_info, &events)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
            ctx.metrics.record_event_apply(apply_start);

            (user_info, events)
        };
        ctx.metrics.record_operation(operation_start, "create_pair");

        let action_private_input = Vec::<u8>::new();
        let orderbook_action = PermissionnedOrderbookAction::CreatePair { pair, info };

        process_orderbook_action(
            user_info,
            events,
            orderbook_action,
            &action_private_input,
            &ctx,
        )
        .await
    }
    .await;

    let status = match &result {
        Ok(_) => 200,
        Err(AppError(status, _)) => status.as_u16(),
    };
    ctx.metrics.record_request(request_start, endpoint, status);

    result
}

// #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(ctx)))]
async fn add_session_key(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let request_start = Instant::now();
    let endpoint = "add_session_key";

    let result = async {
        let auth = AuthHeaders::from_headers(&headers)?;
        let user = auth.identity;
        let public_key = auth.public_key.expect("Missing public key in headers");

        debug!(
            "Adding session key for user {user} with public key {}",
            hex::encode(&public_key)
        );

        let operation_start = Instant::now();
        // FIXME: locking here makes locking another time in execute_orderbook_action ...
        let (user_info, events) = {
            let lock_start = Instant::now();
            let mut orderbook = ctx.orderbook.lock().await;
            ctx.metrics.record_lock(lock_start);

            debug!(
                "Getting user info for user {user}. Orderbook users info: {:?}",
                orderbook.users_info
            );

            // Get user_info if exists, otherwise create a new one with random salt
            let user_info = orderbook.get_user_info(&user).unwrap_or_else(|_| {
                debug!("Creating new user info for user {user}");
                let mut salt = [0u8; 32];
                rand::rng().fill_bytes(&mut salt);
                UserInfo::new(user.clone(), salt.to_vec())
            });
            debug!("User info: {:?}", user_info);

            let method_start = Instant::now();
            let res = orderbook.add_session_key(user_info.clone(), &public_key);
            ctx.metrics.record_method(method_start, "add_session_key");
            let events = match res {
                Ok(events) => events,
                Err(e) => {
                    if e.contains("already exists") {
                        debug!("Session key already exists for user {user}. {e}");
                        return Err(AppError(StatusCode::NOT_MODIFIED, anyhow::anyhow!(e)));
                    } else {
                        return Err(AppError(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            anyhow::anyhow!(e),
                        ));
                    }
                }
            };

            let apply_start = Instant::now();
            orderbook
                .apply_events(&user_info, &events)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
            ctx.metrics.record_event_apply(apply_start);

            (user_info, events)
        };
        ctx.metrics
            .record_operation(operation_start, "add_session_key");

        let action_private_input = &AddSessionKeyPrivateInput {
            new_public_key: public_key,
        };

        let orderbook_action = PermissionnedOrderbookAction::AddSessionKey;

        process_orderbook_action(
            user_info,
            events,
            orderbook_action,
            action_private_input,
            &ctx,
        )
        .await
    }
    .await;

    let status = match &result {
        Ok(_) => 200,
        Err(AppError(status, _)) => status.as_u16(),
    };
    ctx.metrics.record_request(request_start, endpoint, status);

    result
}

// #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(ctx)))]
async fn deposit(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<DepositRequest>,
) -> Result<impl IntoResponse, AppError> {
    let request_start = Instant::now();
    let endpoint = "deposit";

    let result = async {
        let auth = AuthHeaders::from_headers(&headers)?;
        let user = auth.identity;
        // TODO: Check that the user actually has sent the funds to the contract before proceeding to deposit

        debug!(
            "Depositing {} {} for user {user}",
            request.amount, request.symbol
        );

        let operation_start = Instant::now();
        let (user_info, events) = {
            let lock_start = Instant::now();
            let mut orderbook = ctx.orderbook.lock().await;
            ctx.metrics.record_lock(lock_start);

            // Get user_info if exists, otherwise create a new one with random salt
            let user_info = orderbook.get_user_info(&user).unwrap_or_else(|_| {
                let mut salt = [0u8; 32];
                rand::rng().fill_bytes(&mut salt);
                UserInfo::new(user.clone(), salt.to_vec())
            });

            let method_start = Instant::now();
            let events = orderbook
                .deposit(&request.symbol, request.amount, &user_info)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
            ctx.metrics.record_method(method_start, "deposit");

            let apply_start = Instant::now();
            orderbook
                .apply_events(&user_info, &events)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
            ctx.metrics.record_event_apply(apply_start);

            (user_info, events)
        };
        ctx.metrics.record_operation(operation_start, "deposit");

        let action_private_input = Vec::<u8>::new();

        let orderbook_action = PermissionnedOrderbookAction::Deposit {
            symbol: request.symbol,
            amount: request.amount,
        };

        process_orderbook_action(
            user_info,
            events,
            orderbook_action,
            &action_private_input,
            &ctx,
        )
        .await
    }
    .await;

    let status = match &result {
        Ok(_) => 200,
        Err(AppError(status, _)) => status.as_u16(),
    };
    ctx.metrics.record_request(request_start, endpoint, status);

    result
}

// #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(ctx)))]
async fn create_order(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<Order>,
) -> Result<impl IntoResponse, AppError> {
    let request_start = Instant::now();
    let endpoint = "create_order";

    let result = async {
        let auth = AuthHeaders::from_headers(&headers)?;
        let user = auth.identity;
        let public_key = auth.public_key.expect("Missing public key in headers");
        let signature = auth.signature.expect("Missing signature in headers");

        debug!("Creating order for user {user}. Order: {:?}", request);

        let operation_start = Instant::now();
        let (user_info, events) = {
            let lock_start = Instant::now();
            let mut orderbook = ctx.orderbook.lock().await;
            ctx.metrics.record_lock(lock_start);

            let user_info = orderbook.get_user_info(&user).map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Could not find user {user}: {e}"),
                )
            })?;

            orderbook::utils::verify_user_signature_authorization(
                &user_info,
                &public_key,
                &format!(
                    "{}:{}:create_order:{}",
                    user_info.user, user_info.nonce, request.order_id
                ),
                &signature,
            )
            .map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Failed to verify user signature authorization: {e}"),
                )
            })?;

            let method_start = Instant::now();
            let events = log_warn!(
                orderbook
                    .execute_order(&user_info, request.clone())
                    .map_err(|e| anyhow::anyhow!(e)),
                "Failed to execute order"
            )
            .map_err(|e| AppError(StatusCode::BAD_REQUEST, e))?;
            ctx.metrics.record_method(method_start, "execute_order");

            let apply_start = Instant::now();
            log_error!(
                orderbook
                    .apply_events(&user_info, &events)
                    .map_err(|e| anyhow::anyhow!(e)),
                "Failed to apply events"
            )
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, e))?;
            ctx.metrics.record_event_apply(apply_start);

            (user_info, events)
        };
        ctx.metrics
            .record_operation(operation_start, "create_order");

        let action_private_input = &CreateOrderPrivateInput {
            public_key,
            signature,
        };

        let orderbook_action = PermissionnedOrderbookAction::CreateOrder(request);

        process_orderbook_action(
            user_info,
            events,
            orderbook_action,
            action_private_input,
            &ctx,
        )
        .await
    }
    .await;

    let status = match &result {
        Ok(_) => 200,
        Err(AppError(status, _)) => status.as_u16(),
    };
    ctx.metrics.record_request(request_start, endpoint, status);

    result
}

#[cfg_attr(feature = "instrumentation", tracing::instrument(skip(ctx)))]
async fn cancel_order(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<CancelOrderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let request_start = Instant::now();
    let endpoint = "cancel_order";

    let result = async {
        let auth = AuthHeaders::from_headers(&headers)?;
        let user = auth.identity;
        let public_key = auth.public_key.expect("Missing public key in headers");
        let signature = auth.signature.expect("Missing signature in headers");

        debug!(
            "Cancelling order for user {user}. Order ID: {}",
            request.order_id
        );

        let operation_start = Instant::now();
        let (user_info, events) = {
            let lock_start = Instant::now();
            let mut orderbook = ctx.orderbook.lock().await;
            ctx.metrics.record_lock(lock_start);

            let user_info = orderbook.get_user_info(&user).map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Could not find user {user}: {e}"),
                )
            })?;

            orderbook::utils::verify_user_signature_authorization(
                &user_info,
                &public_key,
                &format!(
                    "{}:{}:cancel:{}",
                    user_info.user, user_info.nonce, request.order_id
                ),
                &signature,
            )
            .map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Failed to verify user signature authorization: {e}"),
                )
            })?;

            let Some(order_owner) = orderbook.get_order_owner(&request.order_id) else {
                return Err(AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Order not found: {}", request.order_id),
                ));
            };
            if user_info.get_key() != *order_owner {
                return Err(AppError(
                    StatusCode::UNAUTHORIZED,
                    anyhow::anyhow!("You are not the owner of this order"),
                ));
            }

            let method_start = Instant::now();
            let events = orderbook
                .cancel_order(request.order_id.clone(), &user_info)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
            ctx.metrics.record_method(method_start, "cancel_order");

            let apply_start = Instant::now();
            orderbook
                .apply_events(&user_info, &events)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
            ctx.metrics.record_event_apply(apply_start);

            (user_info, events)
        };
        ctx.metrics
            .record_operation(operation_start, "cancel_order");

        let action_private_input = CancelOrderPrivateInput {
            public_key,
            signature,
        };

        let orderbook_action = PermissionnedOrderbookAction::Cancel {
            order_id: request.order_id.clone(),
        };

        process_orderbook_action(
            user_info,
            events,
            orderbook_action,
            &action_private_input,
            &ctx,
        )
        .await
    }
    .await;

    let status = match &result {
        Ok(_) => 200,
        Err(AppError(status, _)) => status.as_u16(),
    };
    ctx.metrics.record_request(request_start, endpoint, status);

    result
}

#[cfg_attr(feature = "instrumentation", tracing::instrument(skip(ctx)))]
async fn withdraw(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<WithdrawRequest>,
) -> Result<impl IntoResponse, AppError> {
    let request_start = Instant::now();
    let endpoint = "withdraw";

    let result = async {
        let auth = AuthHeaders::from_headers(&headers)?;
        let user = auth.identity;
        let public_key = auth.public_key.expect("Missing public key in headers");
        let signature = auth.signature.expect("Missing signature in headers");

        debug!(
            "Withdrawing {} {} for user {user}",
            request.amount, request.symbol
        );

        let operation_start = Instant::now();
        let (user_info, events) = {
            let lock_start = Instant::now();
            let mut orderbook = ctx.orderbook.lock().await;
            ctx.metrics.record_lock(lock_start);

            let user_info = orderbook.get_user_info(&user).map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Could not find user {user}: {e}"),
                )
            })?;

            orderbook::utils::verify_user_signature_authorization(
                &user_info,
                &public_key,
                &format!(
                    "{}:{}:withdraw:{}:{}",
                    user_info.user, user_info.nonce, request.symbol, request.amount
                ),
                &signature,
            )
            .map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Failed to verify user signature authorization: {e}"),
                )
            })?;

            let balance = orderbook.get_balance(&user_info, &request.symbol);
            if balance.0 < request.amount {
                return Err(AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!(
                        "Not enough balance: withdrawing {} {} while having {}",
                        request.amount,
                        request.symbol,
                        balance.0
                    ),
                ));
            };

            let method_start = Instant::now();
            let events = orderbook
                .withdraw(&request.symbol, &request.amount, &user_info)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
            ctx.metrics.record_method(method_start, "withdraw");

            let apply_start = Instant::now();
            orderbook
                .apply_events(&user_info, &events)
                .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
            ctx.metrics.record_event_apply(apply_start);

            (user_info, events)
        };
        ctx.metrics.record_operation(operation_start, "withdraw");

        let action_private_input = WithdrawPrivateInput {
            public_key,
            signature,
        };

        let orderbook_action = PermissionnedOrderbookAction::Withdraw {
            symbol: request.symbol,
            amount: request.amount,
            destination: request.destination,
        };

        process_orderbook_action(
            user_info,
            events,
            orderbook_action,
            &action_private_input,
            &ctx,
        )
        .await
    }
    .await;

    let status = match &result {
        Ok(_) => 200,
        Err(AppError(status, _)) => status.as_u16(),
    };
    ctx.metrics.record_request(request_start, endpoint, status);

    result
}

#[cfg_attr(
    feature = "instrumentation",
    tracing::instrument(skip(ctx, action_private_input))
)]
async fn process_orderbook_action<T: BorshSerialize>(
    user_info: UserInfo,
    events: Vec<OrderbookEvent>,
    orderbook_action: PermissionnedOrderbookAction,
    action_private_input: &T,
    ctx: &RouterCtx,
) -> Result<impl IntoResponse, AppError> {
    let action_id = ctx.action_id_counter.fetch_add(1, Ordering::Relaxed);
    let blob_tx = BlobTransaction::new(
        ORDERBOOK_ACCOUNT_IDENTITY,
        vec![
            OrderbookAction::PermissionnedOrderbookAction(orderbook_action.clone(), action_id)
                .as_blob(ctx.orderbook_cn.clone()),
        ],
    );
    let tx_hash = blob_tx.hashed();

    let action_private_input = borsh::to_vec(action_private_input).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            anyhow::anyhow!("Failed to serialize action private input: {e}"),
        )
    })?;

    let prover_request = OrderbookProverRequest {
        events,
        user_info: user_info.clone(),
        action_private_input,
        orderbook_action,
        tx_hash: tx_hash.clone(),
        nonce: action_id,
    };

    // Send write events request to database module
    // Database module will send the blob tx to the node
    debug!("Sending write events request to database module for tx {tx_hash:#}");
    let mut bus = ctx.bus.clone();
    bus.send(DatabaseRequest::WriteEvents {
        user: user_info,
        tx_hash: tx_hash.clone(),
        blob_tx,
        prover_request,
    })?;

    Ok(Json(tx_hash))
}
