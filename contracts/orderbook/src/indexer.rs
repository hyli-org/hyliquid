use std::collections::BTreeMap;
use std::str;

use anyhow::{anyhow, Result};
use client_sdk::contract_indexer::{
    axum::{extract::State, http::StatusCode, response::IntoResponse, Json, Router},
    utoipa::openapi::OpenApi,
    utoipa_axum::{router::OpenApiRouter, routes},
    AppError, ContractHandler, ContractHandlerStore,
};
use serde::Serialize;

use crate::{orderbook::TokenName, *};
use client_sdk::contract_indexer::axum;
use client_sdk::contract_indexer::utoipa;

impl ContractHandler for Orderbook {
    async fn api(store: ContractHandlerStore<Orderbook>) -> (Router<()>, OpenApi) {
        let (router, api) = OpenApiRouter::default()
            .routes(routes!(get_orders))
            .routes(routes!(get_orders_by_pair))
            .split_for_parts();

        (router.with_state(store), api)
    }
}

#[utoipa::path(
    get,
    path = "/orders",
    tag = "Contract",
    responses(
        (status = OK, description = "Get json state of contract")
    )
)]
pub async fn get_orders(
    State(state): State<ContractHandlerStore<Orderbook>>,
) -> Result<impl IntoResponse, AppError> {
    let store = state.read().await;
    store
        .state
        .as_ref()
        .map(|state| Json(state.get_orders()))
        .ok_or(AppError(
            StatusCode::NOT_FOUND,
            anyhow!("No state found for contract '{}'", store.contract_name),
        ))
}

#[derive(Serialize)]
pub struct PairOrders {
    pub buy_orders: Vec<Order>,
    pub sell_orders: Vec<Order>,
}

#[utoipa::path(
    get,
    path = "/orders/pair/{base_token}/{quote_token}",
    tag = "Contract",
    params(
        ("base_token" = String, Path, description = "Base token of the pair"),
        ("quote_token" = String, Path, description = "Quote token of the pair")
    ),
    responses(
        (status = OK, description = "Get all orders for a specific token pair")
    )
)]
pub async fn get_orders_by_pair(
    State(state): State<ContractHandlerStore<Orderbook>>,
    axum::extract::Path((base_token, quote_token)): axum::extract::Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let store = state.read().await;
    store
        .state
        .as_ref()
        .map(|state| Json(state.get_orders_by_pair(&base_token, &quote_token)))
        .ok_or(AppError(
            StatusCode::NOT_FOUND,
            anyhow!(
                "No orders found for pair '{}/{}' in contract '{}'",
                base_token,
                quote_token,
                store.contract_name
            ),
        ))
}

/// Implementation for indexing purposes
impl Orderbook {
    pub fn get_state(&self) -> Self {
        self.clone()
    }

    pub fn get_balances(&self) -> BTreeMap<TokenName, BTreeMap<String, u64>> {
        self.balances.clone()
    }

    pub fn get_balance_for_account(&self, user: &str) -> BTreeMap<TokenName, u64> {
        self.balances
            .iter()
            .filter_map(|(token, balances)| {
                balances.get(user).map(|balance| (token.clone(), *balance))
            })
            .collect()
    }

    pub fn get_orders(&self) -> BTreeMap<String, Order> {
        self.orders.clone()
    }

    pub fn get_orders_by_pair(&self, base_token: &str, quote_token: &str) -> PairOrders {
        let pair = (base_token.to_string(), quote_token.to_string());

        let buy_orders = self
            .buy_orders
            .get(&pair)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.orders.get(id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let sell_orders = self
            .sell_orders
            .get(&pair)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.orders.get(id))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        PairOrders {
            buy_orders,
            sell_orders,
        }
    }
}
