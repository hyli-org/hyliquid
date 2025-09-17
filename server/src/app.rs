use std::{collections::BTreeMap, sync::Arc};

use anyhow::Result;
use axum::{
    extract::{Json, State},
    http::{HeaderMap, Method},
    response::IntoResponse,
    routing::{get, post},
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
    orderbook::{OrderType, Orderbook, OrderbookEvent, TokenPair},
    OrderbookAction,
};
use reqwest::StatusCode;
use sdk::{
    BlobTransaction, Calldata, ContractName, Identity, LaneId, TxContext, TxHash, ZkContract,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tower_http::cors::{Any, CorsLayer};

pub struct OrderbookModule {
    bus: OrderbookModuleBusClient,
}

pub struct OrderbookModuleCtx {
    pub api: Arc<BuildApiContextInner>,
    pub node_client: Arc<NodeApiHttpClient>,
    pub orderbook_cn: ContractName,
    pub lane_id: LaneId,
    pub default_state: Orderbook,
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
        };

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(vec![Method::GET, Method::POST])
            .allow_headers(Any);

        let api = Router::new()
            .route("/_health", get(health))
            .route("/api/config", get(get_config))
            .route("/create_order", post(create_order))
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
}

async fn health() -> impl IntoResponse {
    Json("OK")
}

#[derive(Serialize)]
struct ConfigResponse {
    contract_name: String,
}

// --------------------------------------------------------
//     Headers
// --------------------------------------------------------

const IDENTITY_HEADER: &str = "x-identity";

#[derive(Debug)]
struct AuthHeaders {
    identity: String,
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

        Ok(AuthHeaders { identity })
    }
}

#[derive(serde::Deserialize)]
struct CreateOrderRequest {
    order_id: String,
    order_type: OrderType,
    price: Option<u32>,
    pair: TokenPair,
    quantity: u32,
}

// --------------------------------------------------------
//     Routes
// --------------------------------------------------------

async fn get_config(State(ctx): State<RouterCtx>) -> impl IntoResponse {
    Json(ConfigResponse {
        contract_name: ctx.orderbook_cn.0,
    })
}

async fn create_order(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
    Json(request): Json<CreateOrderRequest>,
) -> Result<impl IntoResponse, AppError> {
    let auth = AuthHeaders::from_headers(&headers)?;
    send(
        ctx,
        OrderbookAction::CreateOrder {
            order_id: request.order_id,
            order_type: request.order_type,
            price: request.price,
            pair: request.pair,
            quantity: request.quantity,
        },
        auth,
    )
    .await
}

async fn send(
    ctx: RouterCtx,
    action: OrderbookAction,
    auth: AuthHeaders,
) -> Result<impl IntoResponse, AppError> {
    let identity = Identity(auth.identity);
    let mut orderbook = ctx.orderbook.lock().await;

    let tx_ctx = TxContext {
        lane_id: ctx.lane_id.clone(),
        ..Default::default()
    };

    let calldata = match action {
        OrderbookAction::CreateOrder {
            order_id,
            order_type,
            price,
            pair,
            quantity,
        } => {
            // Assert that the auth headers contains the signature
            // Assert the signature is valid for that user

            let orderbook_blob = OrderbookAction::CreateOrder {
                order_id,
                order_type,
                price,
                pair,
                quantity,
            }
            .as_blob(ctx.orderbook_cn.clone());
            let private_input = orderbook::CreateOrderPrivateInput {
                user: identity.to_string(),
                public_key: vec![],
                signature: vec![],
                order_user_map: BTreeMap::default(),
            };

            Calldata {
                identity,
                index: sdk::BlobIndex(0),
                blobs: vec![orderbook_blob].into(),
                tx_blob_count: 1,
                tx_hash: TxHash::default(),
                tx_ctx: Some(tx_ctx),
                private_input: borsh::to_vec(&private_input)
                    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?,
            }
        }
        _ => {
            todo!()
        }
    };

    let res = orderbook.execute(&calldata);
    tracing::error!("orderbook execute result: {:?}", res);

    if res.is_ok() {
        let tx_hash = ctx
            .client
            .send_tx_blob(BlobTransaction::new(
                calldata.identity,
                calldata
                    .blobs
                    .iter()
                    .map(|(_, blob)| blob.clone())
                    .collect(),
            ))
            .await?;

        Ok(Json(tx_hash))
    } else {
        Err(AppError(
            StatusCode::BAD_REQUEST,
            anyhow::anyhow!("Something went wrong, could not execute the transaction"),
        ))
    }
}
