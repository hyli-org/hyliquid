use anyhow::Result;
use axum::{
    extract::{Json, State},
    http::{HeaderMap, Method, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use client_sdk::contract_indexer::AppError;
use hyli_modules::{
    bus::SharedMessageBus,
    module_bus_client, module_handle_messages,
    modules::{BuildApiContextInner, Module},
};
use sdk::ContractName;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

use crate::services::{
    book_service::{self, BookService},
    user_service::UserService,
};

pub struct ApiModule {
    bus: AppModuleBusClient,
}

pub struct ApiModuleCtx {
    pub api: Arc<BuildApiContextInner>,
    pub book_service: Arc<RwLock<BookService>>,
    pub user_service: Arc<RwLock<UserService>>,
    pub contract1_cn: ContractName,
}

module_bus_client! {
#[derive(Debug)]
pub struct AppModuleBusClient {
}
}

impl Module for ApiModule {
    type Context = Arc<ApiModuleCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let state = RouterCtx {
            contract1_cn: ctx.contract1_cn.clone(),
            book_service: ctx.book_service.clone(),
            user_service: ctx.user_service.clone(),
        };

        // Créer un middleware CORS
        let cors = CorsLayer::new()
            .allow_origin(Any) // Permet toutes les origines (peut être restreint)
            .allow_methods(vec![Method::GET, Method::POST]) // Permet les méthodes nécessaires
            .allow_headers(Any); // Permet tous les en-têtes

        let api = Router::new()
            .route("/_health", get(health))
            .route("/api/config", get(get_config))
            .route("/api/info", get(get_info))
            .route("/api/book/{symbol}", get(get_book))
            .route("/api/balances", get(get_balance))
            .with_state(state)
            .layer(cors); // Appliquer le middleware CORS

        if let Ok(mut guard) = ctx.api.router.lock() {
            if let Some(router) = guard.take() {
                guard.replace(router.merge(api));
            }
        }
        let bus = AppModuleBusClient::new_from_bus(bus.new_handle()).await;

        Ok(ApiModule { bus })
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
    pub book_service: Arc<RwLock<BookService>>,
    pub user_service: Arc<RwLock<UserService>>,
    pub contract1_cn: ContractName,
}

async fn health() -> impl IntoResponse {
    Json("OK")
}

// --------------------------------------------------------
//     Headers
// --------------------------------------------------------

const USER_HEADER: &str = "x-user";

#[derive(Debug)]
struct AuthHeaders {
    user: String,
}

impl AuthHeaders {
    fn from_headers(headers: &HeaderMap) -> Result<Self, AppError> {
        let user = headers
            .get(USER_HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError(
                    StatusCode::UNAUTHORIZED,
                    anyhow::anyhow!("Missing signature"),
                )
            })?;

        Ok(AuthHeaders {
            user: user.to_string(),
        })
    }
}

#[derive(Serialize)]
struct ConfigResponse {
    contract_name: String,
}

// --------------------------------------------------------
//     Routes
// --------------------------------------------------------
//
async fn get_config(State(ctx): State<RouterCtx>) -> impl IntoResponse {
    Json(ConfigResponse {
        contract_name: ctx.contract1_cn.0,
    })
}

async fn get_info(State(_ctx): State<RouterCtx>) -> Result<impl IntoResponse, AppError> {
    let book_service = _ctx.book_service.read().await;

    let info = book_service.get_info().await?;

    Ok(Json(info))
}

async fn get_book(
    State(ctx): State<RouterCtx>,
    axum::extract::Path(symbol): axum::extract::Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let book_service = ctx.book_service.read().await;

    let book = book_service.get_order_book(&symbol).await?;

    Ok(Json(book))
}

async fn get_balance(
    State(ctx): State<RouterCtx>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth_headers = AuthHeaders::from_headers(&headers)?;
    let user_service = ctx.user_service.read().await;

    let balance = user_service.get_balances(&auth_headers.user).await?;

    Ok(Json(balance))
}
