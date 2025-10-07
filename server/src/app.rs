use std::{sync::Arc, vec};

use anyhow::Result;
use axum::{
    extract::{Json, State},
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
    orderbook::{Order, Orderbook, OrderbookEvent, PairInfo, TokenPair},
    smt_values::UserInfo,
    AddSessionKeyPrivateInput, CancelOrderPrivateInput, CreateOrderPrivateInput, OrderbookAction,
    PermissionnedOrderbookAction, WithdrawPrivateInput,
};
use reqwest::StatusCode;
use sdk::{BlobTransaction, ContractName, LaneId};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tower_http::cors::{Any, CorsLayer};
use tracing::Instrument;

use crate::{
    prover::OrderbookProverRequest, services::asset_service::AssetService,
    services::book_service::BookWriterService,
};
use rand::RngCore;

pub struct OrderbookModule {
    bus: OrderbookModuleBusClient,
}

pub struct OrderbookModuleCtx {
    pub api: Arc<BuildApiContextInner>,
    pub node_client: Arc<NodeApiHttpClient>,
    pub orderbook_cn: ContractName,
    pub lane_id: LaneId,
    pub default_state: Orderbook,
    pub book_writer_service: Arc<Mutex<BookWriterService>>,
    pub asset_service: Arc<RwLock<AssetService>>,
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
            book_writer_service: ctx.book_writer_service.clone(),
            asset_service: ctx.asset_service.clone(),
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
#[allow(dead_code)]
struct RouterCtx {
    pub client: Arc<NodeApiHttpClient>,
    pub bus: OrderbookModuleBusClient,
    pub orderbook_cn: ContractName,
    pub default_state: Orderbook,
    pub orderbook: Arc<Mutex<Orderbook>>,
    pub lane_id: LaneId,
    pub book_writer_service: Arc<Mutex<BookWriterService>>,
    pub asset_service: Arc<RwLock<AssetService>>,
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
    pub pair: TokenPair,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DepositRequest {
    pub token: String,
    pub amount: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CancelOrderRequest {
    pub order_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WithdrawRequest {
    pub token: String,
    pub amount: u64,
}

// --------------------------------------------------------
//     Routes
// --------------------------------------------------------
async fn get_nonce(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;

    // TODO: do some checks on headers to verify identify the user

    let orderbook = ctx.orderbook.lock().await;
    let nonce = orderbook
        .get_user_info(&user)
        .map(|u| u.nonce)
        .unwrap_or_default();

    Ok(Json(nonce))
}

#[axum::debug_handler]
#[tracing::instrument(skip(ctx))]
async fn create_pair(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<CreatePairRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;

    if request.pair.0 == request.pair.1 {
        return Err(AppError(
            StatusCode::BAD_REQUEST,
            anyhow::anyhow!("Base and quote asset cannot be the same"),
        ));
    }

    let user = auth.identity;

    let asset_service = ctx.asset_service.read().await;
    let base_asset = asset_service
        .get_asset(&request.pair.0)
        .await
        .ok_or(AppError(
            StatusCode::NOT_FOUND,
            anyhow::anyhow!("Base asset not found: {}", request.pair.0),
        ))?;
    let quote_asset = asset_service
        .get_asset(&request.pair.1)
        .await
        .ok_or(AppError(
            StatusCode::NOT_FOUND,
            anyhow::anyhow!("Quote asset not found: {}", request.pair.1),
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

    let info = PairInfo {
        base_scale: base_asset.scale as u64,
        quote_scale: quote_asset.scale as u64,
    };
    drop(asset_service);

    let (user_info, events) = {
        let mut orderbook = ctx.orderbook.lock().await;

        // Get user_info if exists, otherwise create a new one with random salt
        let user_info = orderbook.get_user_info(&user).unwrap_or_else(|_| {
            let mut salt = [0u8; 32];
            rand::rng().fill_bytes(&mut salt);
            UserInfo::new(user.clone(), salt.to_vec())
        });

        let events = orderbook
            .create_pair(&request.pair, &info)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

        (user_info, events)
    };

    let action_private_input = Vec::<u8>::new();

    let orderbook_action = PermissionnedOrderbookAction::CreatePair {
        pair: request.pair,
        info,
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

#[tracing::instrument(skip(ctx))]
async fn add_session_key(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;
    let public_key = auth.public_key.expect("Missing public key in headers");

    // FIXME: locking here makes locking another time in execute_orderbook_action ...
    let (user_info, events) = {
        let mut orderbook = ctx.orderbook.lock().await;

        // Get user_info if exists, otherwise create a new one with random salt
        let user_info = orderbook.get_user_info(&user).unwrap_or_else(|_| {
            let mut salt = [0u8; 32];
            rand::rng().fill_bytes(&mut salt);
            UserInfo::new(user.clone(), salt.to_vec())
        });

        let events = orderbook
            .add_session_key(user_info.clone(), &public_key)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

        (user_info, events)
    };

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

#[tracing::instrument(skip(ctx))]
async fn deposit(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<DepositRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;
    // TODO: Check that the user actually has sent the funds to the contract before proceeding to deposit

    let (user_info, events) = {
        let mut orderbook = ctx.orderbook.lock().await;

        // Get user_info if exists, otherwise create a new one with random salt
        let user_info = orderbook.get_user_info(&user).unwrap_or_else(|_| {
            let mut salt = [0u8; 32];
            rand::rng().fill_bytes(&mut salt);
            UserInfo::new(user.clone(), salt.to_vec())
        });

        let events = orderbook
            .deposit(&request.token, request.amount, &user_info)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

        (user_info, events)
    };

    let action_private_input = Vec::<u8>::new();

    let orderbook_action = PermissionnedOrderbookAction::Deposit {
        token: request.token,
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

#[tracing::instrument(skip(ctx))]
async fn create_order(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<Order>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;

    let (user_info, events) = {
        let mut orderbook = ctx.orderbook.lock().await;

        let user_info = orderbook.get_user_info(&user).map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                anyhow::anyhow!("Could not find user {user}: {e}"),
            )
        })?;

        let events = orderbook
            .execute_order(&user_info, request.clone())
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

        (user_info, events)
    };

    let action_private_input = &CreateOrderPrivateInput {
        public_key: auth.public_key.expect("Missing public key in headers"),
        signature: auth.signature.expect("Missing signature in headers"),
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

#[tracing::instrument(skip(ctx))]
async fn cancel_order(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<CancelOrderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;

    let (user_info, events) = {
        let mut orderbook = ctx.orderbook.lock().await;

        let user_info = orderbook.get_user_info(&user).map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                anyhow::anyhow!("Could not find user {user}: {e}"),
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

        let events = orderbook
            .cancel_order(request.order_id.clone(), &user_info)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

        (user_info, events)
    };

    let action_private_input = CancelOrderPrivateInput {
        public_key: auth.public_key.expect("Missing public key in headers"),
        signature: auth.signature.expect("Missing signature in headers"),
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

#[tracing::instrument(skip(ctx))]
async fn withdraw(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<WithdrawRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;

    let (user_info, events) = {
        let mut orderbook = ctx.orderbook.lock().await;

        let user_info = orderbook.get_user_info(&user).map_err(|e| {
            AppError(
                StatusCode::BAD_REQUEST,
                anyhow::anyhow!("Could not find user {user}: {e}"),
            )
        })?;

        let balance = orderbook.get_balance(&user_info, &request.token);
        if balance.0 < request.amount {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                anyhow::anyhow!(
                    "Not enough balance: withdrawing {} {} while having {}",
                    request.amount,
                    request.token,
                    balance.0
                ),
            ));
        };

        let events = orderbook
            .withdraw(&request.token, &request.amount, &user_info)
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

        (user_info, events)
    };

    let action_private_input = WithdrawPrivateInput {
        public_key: auth.public_key.expect("Missing public key in headers"),
        signature: auth.signature.expect("Missing signature in headers"),
    };

    let orderbook_action = PermissionnedOrderbookAction::Withdraw {
        token: request.token.clone(),
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

#[tracing::instrument(skip(ctx, action_private_input))]
async fn process_orderbook_action<T: BorshSerialize>(
    user_info: UserInfo,
    events: Vec<OrderbookEvent>,
    orderbook_action: PermissionnedOrderbookAction,
    action_private_input: &T,
    ctx: &RouterCtx,
) -> Result<impl IntoResponse, AppError> {
    tracing::info!("Orderbook execution results: {:?}", events);
    let book_service = ctx.book_writer_service.lock().await;

    let tx_hash =
        ctx.client
            .send_tx_blob(BlobTransaction::new(
                "orderbook@orderbook",
                vec![
                    OrderbookAction::PermissionnedOrderbookAction(orderbook_action.clone())
                        .as_blob(ctx.orderbook_cn.clone()),
                ],
            ))
    .await?;

    book_service
        .write_events(&user_info.user, tx_hash.clone(), events.clone())
        .await?;

    let action_private_input = borsh::to_vec(action_private_input).map_err(|e| {
        AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            anyhow::anyhow!("Failed to serialize action private input: {e}"),
        )
    })?;

    // Send tx to prover
    let mut bus = ctx.bus.clone();
    bus.send(OrderbookProverRequest::TxToProve {
        events,
        user_info,
        action_private_input,
        orderbook_action,
        tx_hash: tx_hash.clone(),
    })?;

    Ok(Json(tx_hash))
}
