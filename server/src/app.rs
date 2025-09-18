use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, Method},
    response::IntoResponse,
    routing::post,
    Router,
};
use client_sdk::{
    contract_indexer::AppError,
    rest_client::{NodeApiClient, NodeApiHttpClient},
};
use hyli_modules::{
    bus::SharedMessageBus,
    module_bus_client, module_handle_messages,
    modules::{
        websocket::{WsInMessage, WsTopicMessage},
        BuildApiContextInner, Module,
    },
};
use orderbook::{
    orderbook::{OrderSide, OrderType, Orderbook, OrderbookEvent, TokenPair},
    OrderbookAction,
};
use reqwest::StatusCode;
use sdk::{
    BlobTransaction, Calldata, ContractName, Identity, LaneId, TxContext, TxHash, ZkContract,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};

use crate::services::book_service::BookWriterService;

pub struct OrderbookModule {
    bus: OrderbookModuleBusClient,
}

pub struct OrderbookModuleCtx {
    pub api: Arc<BuildApiContextInner>,
    pub node_client: Arc<NodeApiHttpClient>,
    pub orderbook_cn: ContractName,
    pub lane_id: LaneId,
    pub default_state: Orderbook,
    pub book_service: Arc<Mutex<BookWriterService>>,
}

/// Messages received from WebSocket clients that will be processed by the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderbookWsInMessage();

module_bus_client! {
#[derive(Debug)]
pub struct OrderbookModuleBusClient {
    sender(WsTopicMessage<OrderbookEvent>),
    sender(WsTopicMessage<String>),
    receiver(WsInMessage<OrderbookWsInMessage>),
}
}

impl Module for OrderbookModule {
    type Context = Arc<OrderbookModuleCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let orderbook = Arc::new(Mutex::new(ctx.default_state.clone()));

        let state = RouterCtx {
            client: ctx.node_client.clone(),
            orderbook_cn: ctx.orderbook_cn.clone(),
            orderbook: orderbook.clone(),
            lane_id: ctx.lane_id.clone(),
            book_service: ctx.book_service.clone(),
        };

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(vec![Method::GET, Method::POST])
            .allow_headers(Any);

        let api = Router::new()
            .route("/create_order", post(create_order))
            .route("/add_session_key", post(add_session_key))
            .route("/deposit", post(deposit))
            // To be removed later, temporary endpoint for testing
            .route("/temp/balances", post(get_balances))
            .route("/temp/balance/{user}", post(get_balance_for_account))
            .route("/temp/orders", post(get_orders))
            .route("/temp/orders/{token1}/{token2}", post(get_orders_by_pair))
            .route("/temp/reset_state", post(reset_state))
            .with_state(state)
            .layer(cors);

        if let Ok(mut guard) = ctx.api.router.lock() {
            if let Some(router) = guard.take() {
                guard.replace(router.merge(api));
            }
        }
        let bus = OrderbookModuleBusClient::new_from_bus(bus.new_handle()).await;

        Ok(OrderbookModule { bus })
    }

    async fn run(&mut self) -> Result<()> {
        module_handle_messages! {
            on_self self,
        };

        Ok(())
    }
}

#[derive(Clone)]
struct RouterCtx {
    pub client: Arc<NodeApiHttpClient>,
    pub orderbook_cn: ContractName,
    pub orderbook: Arc<Mutex<Orderbook>>,
    pub lane_id: LaneId,
    pub book_service: Arc<Mutex<BookWriterService>>,
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

#[derive(serde::Deserialize)]
struct CreateOrderRequest {
    order_id: String,
    order_side: OrderSide,
    order_type: OrderType,
    price: Option<u32>,
    pair: TokenPair,
    quantity: u32,
}

#[derive(serde::Deserialize)]
struct AddSessionKeyRequest {
    public_key: String,
}

#[derive(serde::Deserialize)]
struct DepositRequest {
    token: String,
    amount: u32,
}

// --------------------------------------------------------
//     Routes
// --------------------------------------------------------
async fn get_balances(State(ctx): State<RouterCtx>) -> Result<impl IntoResponse, AppError> {
    let orderbook = ctx.orderbook.lock().await;
    let balances = orderbook.get_balances();

    Ok(Json(balances))
}
async fn get_balance_for_account(
    State(ctx): State<RouterCtx>,
    Path(user): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let orderbook = ctx.orderbook.lock().await;
    let balances = orderbook.get_balance_for_account(&user);

    Ok(Json(balances))
}
async fn get_orders(State(ctx): State<RouterCtx>) -> Result<impl IntoResponse, AppError> {
    let orderbook = ctx.orderbook.lock().await;
    let orders = orderbook.get_orders();

    Ok(Json(orders))
}
async fn get_orders_by_pair(
    State(ctx): State<RouterCtx>,
    Path((token1, token2)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let orderbook = ctx.orderbook.lock().await;
    let orders = orderbook.get_orders_by_pair(&token1, &token2);

    Ok(Json(orders))
}
async fn reset_state(State(ctx): State<RouterCtx>) -> Result<impl IntoResponse, AppError> {
    let mut orderbook = ctx.orderbook.lock().await;
    *orderbook = Orderbook::init(ctx.lane_id.clone(), true);

    Ok(Json("Orderbook state has been reset"))
}

async fn create_order(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<CreateOrderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let identity = Identity(auth.identity);

    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };

    // FIXME: locking here makes locking another time in execute_orderbook_action ...
    let order_user_map = {
        let orderbook = ctx.orderbook.lock().await;
        orderbook.get_order_user_map(&request.order_side, &request.pair)
    };

    let private_input = orderbook::CreateOrderPrivateInput {
        user: identity.to_string(),
        public_key: auth.public_key.expect("Missing public key in headers"),
        signature: auth.signature.expect("Missing signature in headers"),
        order_user_map,
    };

    let calldata = Calldata {
        identity,
        index: sdk::BlobIndex(0),
        blobs: vec![OrderbookAction::CreateOrder {
            order_id: request.order_id,
            order_side: request.order_side,
            order_type: request.order_type,
            price: request.price,
            pair: request.pair,
            quantity: request.quantity,
        }
        .as_blob(ctx.orderbook_cn.clone())]
        .into(),
        tx_blob_count: 1,
        tx_hash: TxHash::default(),
        tx_ctx: Some(tx_ctx),
        private_input: borsh::to_vec(&private_input)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?,
    };

    execute_orderbook_action(&calldata, &ctx).await
}

async fn add_session_key(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<AddSessionKeyRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;

    // Convert hex string to bytes
    let public_key_bytes = hex::decode(&request.public_key).map_err(|_| {
        AppError(
            StatusCode::BAD_REQUEST,
            anyhow::anyhow!("Invalid hex public key"),
        )
    })?;

    let identity = Identity(auth.identity);

    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };

    let orderbook_blob = OrderbookAction::AddSessionKey {}.as_blob(ctx.orderbook_cn.clone());

    let calldata = Calldata {
        identity,
        index: sdk::BlobIndex(0),
        blobs: vec![orderbook_blob].into(),
        tx_blob_count: 1,
        tx_hash: TxHash::default(),
        tx_ctx: Some(tx_ctx),
        private_input: public_key_bytes,
    };

    execute_orderbook_action(&calldata, &ctx).await
}

async fn deposit(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<DepositRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let identity = Identity(auth.identity);
    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };

    let orderbook_blob = OrderbookAction::Deposit {
        token: request.token,
        amount: request.amount,
    }
    .as_blob(ctx.orderbook_cn.clone());

    let calldata = Calldata {
        identity,
        index: sdk::BlobIndex(0),
        blobs: vec![orderbook_blob].into(),
        tx_blob_count: 1,
        tx_hash: TxHash::default(),
        tx_ctx: Some(tx_ctx),
        private_input: vec![],
    };

    execute_orderbook_action(&calldata, &ctx).await
}

async fn execute_orderbook_action(
    calldata: &Calldata,
    ctx: &RouterCtx,
) -> Result<impl IntoResponse, AppError> {
    let mut orderbook = ctx.orderbook.lock().await;
    let book_service = ctx.book_service.lock().await;
    let res = orderbook.execute(calldata);

    match &res {
        Ok((bytes, _, _)) => match borsh::from_slice::<Vec<OrderbookEvent>>(bytes) {
            Ok(events) => {
                tracing::info!("orderbook execute results: {:?}", events);
                book_service.write_events(events).await?;
                let tx_hash = ctx
                    .client
                    .send_tx_blob(BlobTransaction::new(
                        calldata.identity.clone(),
                        calldata
                            .blobs
                            .iter()
                            .map(|(_, blob)| blob.clone())
                            .collect(),
                    ))
                    .await?;

                Ok(Json(tx_hash))
            }
            Err(e) => {
                tracing::error!("Failed to deserialize events: {:?}", e);
                Err(AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Failed to deserialize events: {e}"),
                ))
            }
        },
        Err(e) => {
            tracing::error!("Could not execute the transaction: {:?}", e);
            Err(AppError(
                StatusCode::BAD_REQUEST,
                anyhow::anyhow!("Could not execute the transaction: {e}"),
            ))
        }
    }
}
