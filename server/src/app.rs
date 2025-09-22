use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
    vec,
};

use anyhow::Result;
use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, Method},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use borsh::BorshSerialize;
use client_sdk::{
    contract_indexer::AppError,
    rest_client::{NodeApiClient, NodeApiHttpClient},
};
use hyli_modules::{
    bus::{BusClientSender, SharedMessageBus},
    module_bus_client, module_handle_messages,
    modules::{
        websocket::{WsInMessage, WsTopicMessage},
        BuildApiContextInner, Module,
    },
};
use orderbook::{
    orderbook::{Order, OrderSide, OrderType, Orderbook, OrderbookEvent, TokenPair, UserInfo},
    AddSessionKeyPrivateInput, CancelOrderPrivateInput, CreateOrderPrivateInput, OrderbookAction,
    PermissionnedOrderbookAction, PermissionnedPrivateInput, WithdrawPrivateInput,
};
use reqwest::StatusCode;
use sdk::{BlobTransaction, Calldata, ContractName, LaneId, TxContext, TxHash, ZkContract};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};

use crate::{prover::OrderbookProverRequest, services::book_service::BookWriterService};

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
    sender(OrderbookProverRequest),
    receiver(WsInMessage<OrderbookWsInMessage>),
}
}

impl Module for OrderbookModule {
    type Context = Arc<OrderbookModuleCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let orderbook = Arc::new(Mutex::new(ctx.default_state.clone()));

        let bus = OrderbookModuleBusClient::new_from_bus(bus.new_handle()).await;

        let state = RouterCtx {
            client: ctx.node_client.clone(),
            orderbook_cn: ctx.orderbook_cn.clone(),
            default_state: ctx.default_state.clone(),
            bus: bus.clone(),
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
            .route("/cancel_order", post(cancel_order))
            .route("/withdraw", post(withdraw))
            .route("/nonce", get(get_nonce))
            // To be removed later, temporary endpoint for testing
            .route("/temp/balances", get(get_balances))
            .route("/temp/balance/{user}", get(get_balance_for_account))
            .route("/temp/orders", get(get_orders))
            .route("/temp/orders/{token1}/{token2}", get(get_orders_by_pair))
            .route("/temp/state", get(get_state))
            .route("/temp/reset_state", get(reset_state))
            .with_state(state)
            .layer(cors);

        if let Ok(mut guard) = ctx.api.router.lock() {
            if let Some(router) = guard.take() {
                guard.replace(router.merge(api));
            }
        }

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
    pub bus: OrderbookModuleBusClient,
    pub orderbook_cn: ContractName,
    pub default_state: Orderbook,
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

#[derive(Serialize, Deserialize, Debug)]
pub struct CreateOrderRequest {
    pub order_id: String,
    pub order_side: OrderSide,
    pub order_type: OrderType,
    pub price: Option<u32>,
    pub pair: TokenPair,
    pub quantity: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DepositRequest {
    pub token: String,
    pub amount: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CancelOrderRequest {
    pub order_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WithdrawRequest {
    pub token: String,
    pub amount: u32,
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
async fn get_state(State(ctx): State<RouterCtx>) -> Result<impl IntoResponse, AppError> {
    let orderbook = ctx.orderbook.lock().await;
    let serializable_state = SerializableOrderbook::from(&*orderbook);

    Ok(Json(serializable_state))
}
async fn reset_state(State(ctx): State<RouterCtx>) -> Result<impl IntoResponse, AppError> {
    let mut orderbook = ctx.orderbook.lock().await;
    *orderbook = ctx.default_state.clone();

    Ok(Json("Orderbook state has been reset"))
}

async fn get_nonce(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;

    // TODO: do some checks on headers to verify identify the user

    let orderbook = ctx.orderbook.lock().await;
    let nonce = orderbook.get_nonce(&user);

    Ok(Json(nonce))
}

async fn create_order(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<CreateOrderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;

    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };

    // FIXME: locking here makes locking another time in execute_orderbook_action ...
    let order_user_map = {
        let orderbook = ctx.orderbook.lock().await;
        orderbook.get_order_user_map(&request.order_side, &request.pair)
    };

    let private_input = create_permissioned_private_input(
        user.to_string(),
        &CreateOrderPrivateInput {
            public_key: auth.public_key.expect("Missing public key in headers"),
            signature: auth.signature.expect("Missing signature in headers"),
            order_user_map,
        },
    )
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

    let calldata = Calldata {
        identity: "orderbook@orderbook".into(),
        index: sdk::BlobIndex(0),
        blobs: vec![OrderbookAction::PermissionnedOrderbookAction(
            PermissionnedOrderbookAction::CreateOrder {
                order_id: request.order_id,
                order_side: request.order_side,
                order_type: request.order_type,
                price: request.price,
                pair: request.pair,
                quantity: request.quantity,
            },
        )
        .as_blob(ctx.orderbook_cn.clone())]
        .into(),
        tx_blob_count: 1,
        tx_hash: TxHash::default(),
        tx_ctx: Some(tx_ctx),
        private_input,
    };

    execute_orderbook_action(&user, &calldata, &ctx).await
}

async fn add_session_key(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;
    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };

    let orderbook_blob =
        OrderbookAction::PermissionnedOrderbookAction(PermissionnedOrderbookAction::AddSessionKey)
            .as_blob(ctx.orderbook_cn.clone());

    let private_input = create_permissioned_private_input(
        user.to_string(),
        &AddSessionKeyPrivateInput {
            public_key: auth.public_key.expect("Missing public key in headers"),
        },
    )
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

    let calldata = Calldata {
        identity: "orderbook@orderbook".into(),
        index: sdk::BlobIndex(0),
        blobs: vec![orderbook_blob].into(),
        tx_blob_count: 1,
        tx_hash: TxHash::default(),
        tx_ctx: Some(tx_ctx),
        private_input,
    };

    execute_orderbook_action(&user, &calldata, &ctx).await
}

async fn deposit(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<DepositRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;
    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };
    // TODO: Check that the user actually has sent the funds to the contract before proceeding to deposit

    let orderbook_blob =
        OrderbookAction::PermissionnedOrderbookAction(PermissionnedOrderbookAction::Deposit {
            token: request.token,
            amount: request.amount,
        })
        .as_blob(ctx.orderbook_cn.clone());

    let private_input = create_permissioned_private_input(user.to_string(), &Vec::<u8>::new())
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

    let calldata = Calldata {
        identity: "orderbook@orderbook".into(),
        index: sdk::BlobIndex(0),
        blobs: vec![orderbook_blob].into(),
        tx_blob_count: 1,
        tx_hash: TxHash::default(),
        tx_ctx: Some(tx_ctx),
        private_input,
    };

    execute_orderbook_action(&user, &calldata, &ctx).await
}

async fn cancel_order(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<CancelOrderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;
    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };

    let orderbook_blob =
        OrderbookAction::PermissionnedOrderbookAction(PermissionnedOrderbookAction::Cancel {
            order_id: request.order_id,
        })
        .as_blob(ctx.orderbook_cn.clone());

    let private_input = create_permissioned_private_input(
        user.to_string(),
        &CancelOrderPrivateInput {
            public_key: auth.public_key.expect("Missing public key in headers"),
            signature: auth.signature.expect("Missing signature in headers"),
        },
    )
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

    let calldata = Calldata {
        identity: "orderbook@orderbook".into(),
        index: sdk::BlobIndex(0),
        blobs: vec![orderbook_blob].into(),
        tx_blob_count: 1,
        tx_hash: TxHash::default(),
        tx_ctx: Some(tx_ctx),
        private_input,
    };

    execute_orderbook_action(&user, &calldata, &ctx).await
}

async fn withdraw(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<WithdrawRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;
    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };

    let orderbook_blob =
        OrderbookAction::PermissionnedOrderbookAction(PermissionnedOrderbookAction::Withdraw {
            token: request.token,
            amount: request.amount,
        })
        .as_blob(ctx.orderbook_cn.clone());

    // FIXME: locking here makes locking another time in execute_orderbook_action ...
    let user_nonce = {
        let orderbook = ctx.orderbook.lock().await;
        orderbook.get_nonce(&user)
    };

    let private_input = create_permissioned_private_input(
        user.to_string(),
        &WithdrawPrivateInput {
            public_key: auth.public_key.expect("Missing public key in headers"),
            signature: auth.signature.expect("Missing signature in headers"),
            nonce: user_nonce,
        },
    )
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

    let calldata = Calldata {
        identity: "orderbook@orderbook".into(),
        index: sdk::BlobIndex(0),
        blobs: vec![orderbook_blob].into(),
        tx_blob_count: 1,
        tx_hash: TxHash::default(),
        tx_ctx: Some(tx_ctx),
        private_input,
    };

    execute_orderbook_action(&user, &calldata, &ctx).await
}

async fn execute_orderbook_action(
    user: &str,
    calldata: &Calldata,
    ctx: &RouterCtx,
) -> Result<impl IntoResponse, AppError> {
    let mut orderbook = ctx.orderbook.lock().await;
    let book_service = ctx.book_service.lock().await;

    // FIXME: This is not optimal. We should have a way to send the orderbook commitment metadata without blocking the entire process here
    let commitment_metadata = orderbook.as_bytes()?;

    let res = orderbook.execute(calldata);

    match &res {
        Ok((bytes, _, _)) => match borsh::from_slice::<Vec<OrderbookEvent>>(bytes) {
            Ok(events) => {
                tracing::info!("Orderbook execution results: {:?}", events);
                book_service.write_events(user, events).await?;
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

                // Send tx to prover
                let mut bus = ctx.bus.clone();
                bus.send(OrderbookProverRequest::TxToProve {
                    commitment_metadata,
                    calldata: calldata.clone(),
                    tx_hash: tx_hash.clone(),
                })?;

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

fn create_permissioned_private_input<T: BorshSerialize>(
    user: String,
    private_input: &T,
) -> Result<Vec<u8>> {
    let permissioned_private_input = PermissionnedPrivateInput {
        secret: vec![1, 2, 3],
        user: user.to_string(),
        private_input: borsh::to_vec(&private_input)?,
    };
    Ok(borsh::to_vec(&permissioned_private_input)?)
}

// To be removed later, temporary struct for easier serialization
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SerializableOrderbook {
    pub secret: Vec<u8>,
    pub lane_id: LaneId,
    pub balances: BTreeMap<String, BTreeMap<String, u32>>,
    pub users_info: BTreeMap<String, UserInfo>,
    pub orders: BTreeMap<String, Order>,
    pub buy_orders: BTreeMap<String, VecDeque<String>>, // "token1-token2" format
    pub sell_orders: BTreeMap<String, VecDeque<String>>, // "token1-token2" format
    pub accepted_tokens: BTreeSet<ContractName>,
    pub orders_owner: BTreeMap<String, String>,
}

impl From<&Orderbook> for SerializableOrderbook {
    fn from(orderbook: &Orderbook) -> Self {
        SerializableOrderbook {
            secret: orderbook.secret.clone(),
            lane_id: orderbook.lane_id.clone(),
            balances: orderbook.balances.clone(),
            users_info: orderbook.users_info.clone(),
            orders: orderbook.orders.clone(),
            buy_orders: orderbook
                .buy_orders
                .iter()
                .map(|(pair, orders)| (format!("{}-{}", pair.0, pair.1), orders.clone()))
                .collect(),
            sell_orders: orderbook
                .sell_orders
                .iter()
                .map(|(pair, orders)| (format!("{}-{}", pair.0, pair.1), orders.clone()))
                .collect(),
            accepted_tokens: orderbook.accepted_tokens.clone(),
            orders_owner: orderbook.orders_owner.clone(),
        }
    }
}
