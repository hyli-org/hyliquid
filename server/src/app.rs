use std::{sync::Arc, vec};

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
    orderbook::{Order, OrderSide, Orderbook, OrderbookEvent, PairInfo, TokenPair},
    smt_values::UserInfo,
    AddSessionKeyPrivateInput, CancelOrderPrivateInput, CreateOrderPrivateInput,
    DepositPrivateInput, OrderbookAction, PermissionnedOrderbookAction, PermissionnedPrivateInput,
    WithdrawPrivateInput,
};
use reqwest::StatusCode;
use sdk::{
    merkle_utils::BorshableMerkleProof, BlobTransaction, Calldata, ContractName, Hashed, LaneId,
    TxContext, TxHash, ZkContract,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};
use tower_http::cors::{Any, CorsLayer};

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
            // To be removed later, temporary endpoint for testing
            .route("/nonce", get(get_nonce))
            .route("/temp/balances", get(get_balances))
            .route("/temp/balance/{user}", get(get_balance_for_account))
            .route("/temp/orders", get(get_orders))
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
    let balances = orderbook
        .get_balances_for_account(&user)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

    Ok(Json(balances))
}
async fn get_orders(State(ctx): State<RouterCtx>) -> Result<impl IntoResponse, AppError> {
    let orderbook = ctx.orderbook.lock().await;
    let orders = orderbook.get_orders();

    Ok(Json(orders))
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
    let nonce = orderbook
        .get_user_info(&user)
        .map(|u| u.nonce)
        .unwrap_or_default();

    Ok(Json(nonce))
}

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
    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };

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

    let (user_info, user_info_proof) = {
        let orderbook = ctx.orderbook.lock().await;

        // Get user_info if exists, otherwise create a new one with random salt
        let user_info = orderbook.get_user_info(&user).unwrap_or_else(|_| {
            let mut salt = [0u8; 32];
            rand::rng().fill_bytes(&mut salt);
            UserInfo::new(user.clone(), salt.to_vec())
        });

        let user_info_proof = orderbook
            .users_info_mt
            .merkle_proof(vec![user_info.get_key()])
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
        (user_info, BorshableMerkleProof::from(user_info_proof))
    };

    let private_input =
        create_permissioned_private_input(user_info, user_info_proof, &Vec::<u8>::new())
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

    let orderbook_blob =
        OrderbookAction::PermissionnedOrderbookAction(PermissionnedOrderbookAction::CreatePair {
            pair: request.pair,
            info,
        })
        .as_blob(ctx.orderbook_cn.clone());

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

    // FIXME: locking here makes locking another time in execute_orderbook_action ...
    let (user_info, user_info_proof) = {
        let orderbook = ctx.orderbook.lock().await;

        // Get user_info if exists, otherwise create a new one with random salt
        let user_info = orderbook.get_user_info(&user).unwrap_or_else(|_| {
            let mut salt = [0u8; 32];
            rand::rng().fill_bytes(&mut salt);
            UserInfo::new(user.clone(), salt.to_vec())
        });

        let user_info_proof = orderbook
            .users_info_mt
            .merkle_proof(vec![user_info.get_key()])
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
        (user_info, BorshableMerkleProof::from(user_info_proof))
    };

    let private_input = create_permissioned_private_input(
        user_info,
        user_info_proof,
        &AddSessionKeyPrivateInput {
            new_public_key: auth.public_key.expect("Missing public key in headers"),
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
            token: request.token.clone(),
            amount: request.amount,
        })
        .as_blob(ctx.orderbook_cn.clone());

    // FIXME: locking here makes locking another time in execute_orderbook_action ...
    let (user_info, user_info_proof, balance, balance_proof) = {
        let orderbook = ctx.orderbook.lock().await;

        let user_info = orderbook.get_user_info(&user).unwrap_or_else(|_| {
            let mut salt = [0u8; 32];
            rand::rng().fill_bytes(&mut salt);
            UserInfo::new(user.clone(), salt.to_vec())
        });

        let user_info_proof = orderbook
            .users_info_mt
            .merkle_proof(vec![user_info.get_key()])
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

        let user_balance = orderbook.get_balance(&user_info, &request.token);

        let balance_proof = orderbook
            .balances_mt
            .get(&request.token)
            .ok_or_else(|| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Deposit not allowed for this token"),
                )
            })?
            .merkle_proof(vec![user_info.get_key()])
            .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

        (
            user_info,
            BorshableMerkleProof::from(user_info_proof),
            user_balance,
            BorshableMerkleProof::from(balance_proof),
        )
    };

    let private_input = create_permissioned_private_input(
        user_info,
        user_info_proof,
        &DepositPrivateInput {
            balance,
            balance_proof,
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

async fn create_order(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<Order>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    let user = auth.identity;

    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };

    // FIXME: locking here makes locking another time in execute_orderbook_action ...
    let create_order_ctx = {
        let orderbook = ctx.orderbook.lock().await;

        let create_order_ctx = orderbook
            .get_create_order_ctx(&user, &request)
            .map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Could not generate create_order context: {e}"),
                )
            })?;
        create_order_ctx
    };

    let private_input = create_permissioned_private_input(
        create_order_ctx.user_info,
        create_order_ctx.user_info_proof,
        &CreateOrderPrivateInput {
            public_key: auth.public_key.expect("Missing public key in headers"),
            signature: auth.signature.expect("Missing signature in headers"),
            order_user_map: create_order_ctx.order_user_map,
            users_info: create_order_ctx.users_info,
            users_info_proof: create_order_ctx.users_info_proof,
            balances: create_order_ctx.balances,
            balances_proof: create_order_ctx.balances_proof,
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
            order_id: request.order_id.clone(),
        })
        .as_blob(ctx.orderbook_cn.clone());

    // FIXME: locking here makes locking another time in execute_orderbook_action ...
    // TODO: make it a function for readability
    let (user_info, user_info_proof, balance, balance_proof) = {
        let orderbook = ctx.orderbook.lock().await;

        // Verify that user is the owner of the order
        if orderbook.order_manager.get_order_owner(&request.order_id) != Some(&user) {
            return Err(AppError(
                StatusCode::BAD_REQUEST,
                anyhow::anyhow!("User is not the owner of the order"),
            ));
        }

        let (user_info, user_info_proof) =
            orderbook.get_user_info_with_proof(&user).map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Could not generate merkle proof for user {user}: {e}"),
                )
            })?;

        // Get the token name out of the order
        let order = orderbook
            .order_manager
            .get_order(&request.order_id)
            .ok_or_else(|| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Unknown order: {}", request.order_id),
                )
            })?;
        let token = match order.order_side {
            OrderSide::Bid => order.pair.1.clone(),
            OrderSide::Ask => order.pair.0.clone(),
        };

        let (balance, balance_proof) = orderbook
            .get_balance_with_proof(&user_info, &token)
            .map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Could not generate merkle proof of balance of token {token} for user {user}: {e}"),
                )
            })?;

        (user_info, user_info_proof, balance, balance_proof)
    };

    let private_input = create_permissioned_private_input(
        user_info,
        user_info_proof,
        &CancelOrderPrivateInput {
            public_key: auth.public_key.expect("Missing public key in headers"),
            signature: auth.signature.expect("Missing signature in headers"),
            balance,
            balance_proof,
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
            token: request.token.clone(),
            amount: request.amount,
        })
        .as_blob(ctx.orderbook_cn.clone());

    // FIXME: locking here makes locking another time in execute_orderbook_action ...
    let (user_info, user_info_proof, balances, balances_proof) = {
        let orderbook = ctx.orderbook.lock().await;

        let (user_info, user_info_proof) =
            orderbook.get_user_info_with_proof(&user).map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Could not generate merkle proof for user {user}: {e}"),
                )
            })?;

        let (balances, balances_proof) =
            orderbook.get_balances_with_proof(&[user_info.clone()], &request.token).map_err(|e| {
                AppError(
                    StatusCode::BAD_REQUEST,
                    anyhow::anyhow!("Could not generate merkle proof of balance of token {} for user {user}: {e}", request.token),
                )
            })?;

        (user_info, user_info_proof, balances, balances_proof)
    };

    let private_input = create_permissioned_private_input(
        user_info,
        user_info_proof,
        &WithdrawPrivateInput {
            public_key: auth.public_key.expect("Missing public key in headers"),
            signature: auth.signature.expect("Missing signature in headers"),
            balances,
            balances_proof,
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
    let book_service = ctx.book_writer_service.lock().await;

    // FIXME: This is not optimal. We should have a way to send the orderbook commitment metadata without blocking the entire process here
    let commitment_metadata = orderbook.as_bytes()?;

    let res = orderbook.execute(calldata);

    match &res {
        Ok((bytes, _, _)) => match borsh::from_slice::<Vec<OrderbookEvent>>(bytes) {
            Ok(events) => {
                tracing::info!("Orderbook execution results: {:?}", events);

                let tx = BlobTransaction::new(
                    calldata.identity.clone(),
                    calldata
                        .blobs
                        .iter()
                        .map(|(_, blob)| blob.clone())
                        .collect(),
                );

                // TODO: make tx.hashed() unique !!
                book_service.write_events(user, tx.hashed(), events).await?;
                let tx_hash = ctx.client.send_tx_blob(tx).await?;

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
    user_info: UserInfo,
    user_info_proof: BorshableMerkleProof,
    private_input: &T,
) -> Result<Vec<u8>> {
    let permissioned_private_input = PermissionnedPrivateInput {
        secret: vec![1, 2, 3],
        user_info,
        user_info_proof,
        private_input: borsh::to_vec(&private_input)?,
    };
    Ok(borsh::to_vec(&permissioned_private_input)?)
}
