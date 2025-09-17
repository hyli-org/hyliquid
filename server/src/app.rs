use std::{sync::Arc, time::Duration};

use anyhow::Result;
use axum::{
    extract::{Json, State},
    http::{HeaderMap, Method},
    response::IntoResponse,
    routing::get,
    Router,
};
use client_sdk::{
    contract_indexer::AppError,
    rest_client::{NodeApiClient, NodeApiHttpClient},
};
use hyli_modules::{
    bus::{BusClientReceiver, SharedMessageBus},
    module_bus_client, module_handle_messages,
    modules::{
        contract_state_indexer::CSIBusEvent,
        prover::AutoProverEvent,
        websocket::{WsInMessage, WsTopicMessage},
        BuildApiContextInner, Module,
    },
};
use orderbook::{
    orderbook::{OrderType, Orderbook, OrderbookEvent, TokenPair},
    OrderbookAction,
};
use reqwest::StatusCode;
use sdk::{Blob, BlobTransaction, ContractName, Identity};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

pub struct OrderbookModule {
    bus: OrderbookModuleBusClient,
    orderbook_cn: ContractName,
    contract: Arc<RwLock<Orderbook>>,
}

pub struct OrderbookModuleCtx {
    pub api: Arc<BuildApiContextInner>,
    pub node_client: Arc<NodeApiHttpClient>,
    pub orderbook_cn: ContractName,
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
        let contract = Arc::new(RwLock::new(ctx.default_state.clone()));

        let state = RouterCtx {
            client: ctx.node_client.clone(),
            orderbook_cn: ctx.orderbook_cn.clone(),
            contract: contract.clone(),
        };

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(vec![Method::GET, Method::POST])
            .allow_headers(Any);

        let api = Router::new()
            .route("/_health", get(health))
            .route("/api/config", get(get_config))
            .route("/create_order", get(create_order))
            .with_state(state)
            .layer(cors);

        if let Ok(mut guard) = ctx.api.router.lock() {
            if let Some(router) = guard.take() {
                guard.replace(router.merge(api));
            }
        }
        let bus = OrderbookModuleBusClient::new_from_bus(bus.new_handle()).await;

        Ok(OrderbookModule {
            bus,
            contract,
            orderbook_cn: ctx.orderbook_cn.clone(),
        })
    }

    async fn run(&mut self) -> Result<()> {
        module_handle_messages! {
            on_self self,

            // listen<RollupExecutorEvent> event => {
            //     self.handle_rollup_executor_event(event).await?;
            // }

        };

        Ok(())
    }
}

impl OrderbookModule {
    // async fn handle_rollup_executor_event(&mut self, event: RollupExecutorEvent) -> Result<()> {
    // match event {
    //     RollupExecutorEvent::TxExecutionSuccess(_, hyli_outputs, optimistic_contracts) => {
    //         tracing::error!("received TxExecutionSuccess");
    //         let mut events = vec![];
    //         for (hyli_output, contract_name) in hyli_outputs {
    //             if contract_name != self.orderbook_cn {
    //                 continue;
    //             }
    //             let evts: Vec<OrderbookEvent> = borsh::from_slice(&hyli_output.program_outputs)
    //                 .expect("output comes from contract, should always be valid");

    //             for event in evts {
    //                 events.push(event);
    //             }
    //         }

    //         // Update contract state for optimistic RestAPI
    //         // TODO: il faudra retirer l'indexer de l'app, et le mettre direct dans le module RollupExecutor
    //         // TODO: cela permettra de ne pas avoir à envoyer le state de l'orderbook à chaque transaction successful
    //         {
    //             if let Some(orderbook_contract) = optimistic_contracts
    //                 .get(&self.orderbook_cn)
    //                 .expect("Orderbook contract not found")
    //                 .downcast::<Orderbook>()
    //             {
    //                 let mut contract_guard = self.contract.write().await;
    //                 *contract_guard = orderbook_contract.clone();
    //             }
    //         }

    //         // Send events to all clients
    //         tracing::debug!("Sending events: {:?}", events);
    //         for event in events {
    //             let event_clone = event.clone();
    //             match &event {
    //                 OrderbookEvent::BalanceUpdated { user, .. } => {
    //                     _ = log_warn!(
    //                         self.bus.send(WsTopicMessage {
    //                             topic: user.clone(),
    //                             message: event_clone,
    //                         }),
    //                         "Failed to send balance update"
    //                     );
    //                 }
    //                 OrderbookEvent::OrderCancelled { pair, .. }
    //                 | OrderbookEvent::OrderExecuted { pair, .. }
    //                 | OrderbookEvent::OrderUpdate { pair, .. } => {
    //                     let pair = format!("{}-{}", pair.0, pair.1);
    //                     _ = log_warn!(
    //                         self.bus.send(WsTopicMessage {
    //                             topic: pair,
    //                             message: event_clone,
    //                         }),
    //                         "Failed to send order event"
    //                     );
    //                 }
    //                 OrderbookEvent::OrderCreated { order } => {
    //                     let pair = format!("{}-{}", order.pair.0, order.pair.1);
    //                     _ = log_warn!(
    //                         self.bus.send(WsTopicMessage {
    //                             topic: pair,
    //                             message: event_clone,
    //                         }),
    //                         "Failed to send order created event"
    //                     );
    //                 }
    //             }
    //         }
    //         Ok(())
    //     }
    //     RollupExecutorEvent::Rollback(optimistic_contracts) => {
    //         tracing::error!("received TxExecutionRollback");
    //         {
    //             if let Some(orderbook_contract) = optimistic_contracts
    //                 .get(&self.orderbook_cn)
    //                 .expect("Orderbook contract not found")
    //                 .downcast::<Orderbook>()
    //             {
    //                 let mut contract_guard = self.contract.write().await;
    //                 *contract_guard = orderbook_contract.clone();
    //             }
    //         }
    //         // Handle reverted transactions
    //         // We would probably just want to notify clients about the failure
    //         // todo!("Handle reverted transactions");
    //         Ok(())
    //     }
    //     RollupExecutorEvent::FailedTx(identity, tx_hash, message) => {
    //         tracing::error!("received FailedTx");
    //         self.bus.send(WsTopicMessage {
    //             topic: identity.to_string(),
    //             message: format!("Transaction {} failed: {}", tx_hash, message),
    //         })?;
    //         Ok(())
    //     }
    // }
    // }
}

#[derive(Clone)]
struct RouterCtx {
    pub client: Arc<NodeApiHttpClient>,
    pub orderbook_cn: ContractName,
    pub contract: Arc<RwLock<Orderbook>>,
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
    let mut blobs = vec![];

    match action {
        OrderbookAction::CreateOrder {
            order_id,
            order_type,
            price,
            pair,
            quantity,
        } => {
            // Assert that the auth headers contains the signature
            // Assert the signature is valid for that user

            blobs.push(
                OrderbookAction::CreateOrder {
                    order_id,
                    order_type,
                    price,
                    pair,
                    quantity,
                }
                .as_blob(ctx.orderbook_cn.clone()),
            );
        }
        _ => {
            todo!()
        }
    }

    execute_transaction(ctx, identity, blobs).await
}

async fn execute_transaction(
    ctx: RouterCtx,
    identity: Identity,
    blobs: Vec<Blob>,
) -> Result<impl IntoResponse, AppError> {
    // let tx_hash = ctx
    //     .client
    //     .send_tx_blob(BlobTransaction::new(identity.clone(), blobs))
    //     .await?;

    // let mut bus = {
    //     let app = ctx.contract.lock().await;
    //     OrderbookModuleBusClient::new_from_bus(app.bus.new_handle()).await
    // };

    // tokio::time::timeout(Duration::from_secs(5), async {
    //     loop {
    //         let event = bus.recv().await?;
    //         match event {
    //             CSIBusEvent {
    //                 event: AutoProverEvent::SuccessTx(sequenced_tx_hash, state),
    //             } => {
    //                 // if sequenced_tx_hash == tx_hash {
    //                 //     let balance = state.oranj_balances.get(&identity).copied().unwrap_or(0);
    //                 //     let mut table: ApiTable = state
    //                 //         .tables
    //                 //         .get(&identity)
    //                 //         .cloned()
    //                 //         .unwrap_or_default()
    //                 //         .into();
    //                 //     table.balance = balance;
    //                 return Ok(Json(Resp {
    //                     tx_hash: sequenced_tx_hash.to_string(),
    //                     table,
    //                 }));
    //                 // }
    //             }
    //             CSIBusEvent {
    //                 event: AutoProverEvent::FailedTx(sequenced_tx_hash, error),
    //             } => {
    //                 if sequenced_tx_hash == tx_hash {
    //                     return Err(AppError(StatusCode::BAD_REQUEST, anyhow::anyhow!(error)));
    //                 }
    //             }
    //         }
    //     }
    // })
    // .await?
    Ok(Json("ok"))
}
