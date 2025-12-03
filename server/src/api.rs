use anyhow::Result;
use axum::{
    extract::{Json, State},
    http::Method,
    response::IntoResponse,
    routing::get,
    Router,
};
use hyli_modules::{
    bus::SharedMessageBus,
    module_bus_client, module_handle_messages,
    modules::{BuildApiContextInner, Module},
};
use sdk::ContractName;
use serde::Serialize;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

pub struct ApiModule {
    bus: AppModuleBusClient,
}

pub struct ApiModuleCtx {
    pub api: Arc<BuildApiContextInner>,
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
        };

        // Créer un middleware CORS
        let cors = CorsLayer::new()
            .allow_origin(Any) // Permet toutes les origines (peut être restreint)
            .allow_methods(vec![Method::GET, Method::POST]) // Permet les méthodes nécessaires
            .allow_headers(Any); // Permet tous les en-têtes

        let api = Router::new()
            .route("/_health", get(health))
            .route("/api/config", get(get_config))
            // .route("/api/info", get(get_info))
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
    pub contract1_cn: ContractName,
}

async fn health() -> impl IntoResponse {
    Json("OK")
}

// --------------------------------------------------------
//     Headers
// --------------------------------------------------------

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
