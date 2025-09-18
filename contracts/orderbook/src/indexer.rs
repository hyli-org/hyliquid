use std::collections::BTreeMap;
use std::str;

use anyhow::{anyhow, Result};
use client_sdk::contract_indexer::{
    axum::{extract::State, http::StatusCode, response::IntoResponse, Json, Router},
    utoipa::openapi::OpenApi,
    utoipa_axum::{router::OpenApiRouter, routes},
    AppError, ContractHandler, ContractHandlerStore,
};
use sdk::hyli_model_utils::TimestampMs;
use serde::Serialize;

use crate::{orderbook::OrderId, *};
use client_sdk::contract_indexer::axum;
use client_sdk::contract_indexer::utoipa;

impl ContractHandler for Orderbook {
    async fn api(store: ContractHandlerStore<Orderbook>) -> (Router<()>, OpenApi) {
        let (router, api) = OpenApiRouter::default()
            .routes(routes!(get_orders))
            .routes(routes!(get_orders_by_pair))
            .routes(routes!(get_pair_history))
            .routes(routes!(get_pair_candles))
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
    buy_orders: Vec<Order>,
    sell_orders: Vec<Order>,
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

#[utoipa::path(
    get,
    path = "/orders/history/{base_token}/{quote_token}",
    tag = "Contract",
    params(
        ("base_token" = String, Path, description = "Base token of the pair"),
        ("quote_token" = String, Path, description = "Quote token of the pair")
    ),
    responses(
        (status = OK, description = "Get trading history for a specific token pair")
    )
)]
pub async fn get_pair_history(
    State(state): State<ContractHandlerStore<Orderbook>>,
    axum::extract::Path((base_token, quote_token)): axum::extract::Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let store = state.read().await;
    store
        .state
        .as_ref()
        .map(|state| Json(state.get_pair_history(&base_token, &quote_token)))
        .ok_or(AppError(
            StatusCode::NOT_FOUND,
            anyhow!(
                "No history found for pair '{}/{}' in contract '{}'",
                base_token,
                quote_token,
                store.contract_name
            ),
        ))
}

#[derive(Serialize)]
pub struct CandleStick {
    timestamp: TimestampMs,
    open: u32,
    high: u32,
    low: u32,
    close: u32,
    volume: u32,
}

#[utoipa::path(
    get,
    path = "/orders/candles/{base_token}/{quote_token}",
    tag = "Contract",
    params(
        ("base_token" = String, Path, description = "Base token of the pair"),
        ("quote_token" = String, Path, description = "Quote token of the pair"),
        ("from" = i64, Query, description = "Start timestamp in milliseconds"),
        ("to" = i64, Query, description = "End timestamp in milliseconds"),
        ("interval" = i64, Query, description = "Candle interval in milliseconds")
    ),
    responses(
        (status = OK, description = "Get OHLCV data for a specific token pair")
    )
)]
pub async fn get_pair_candles(
    State(state): State<ContractHandlerStore<Orderbook>>,
    axum::extract::Path((base_token, quote_token)): axum::extract::Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<BTreeMap<String, String>>,
) -> Result<impl IntoResponse, AppError> {
    let store = state.read().await;

    // Parse query parameters
    let from = params
        .get("from")
        .and_then(|s| s.parse::<u128>().ok())
        .map(TimestampMs)
        .ok_or(AppError(
            StatusCode::BAD_REQUEST,
            anyhow!("Missing or invalid 'from' parameter"),
        ))?;

    let to = params
        .get("to")
        .and_then(|s| s.parse::<u128>().ok())
        .map(TimestampMs)
        .ok_or(AppError(
            StatusCode::BAD_REQUEST,
            anyhow!("Missing or invalid 'to' parameter"),
        ))?;

    let interval = params
        .get("interval")
        .and_then(|s| s.parse::<u128>().ok())
        .ok_or(AppError(
            StatusCode::BAD_REQUEST,
            anyhow!("Missing or invalid 'interval' parameter"),
        ))?;

    store
        .state
        .as_ref()
        .map(|state| Json(state.get_pair_candles(&base_token, &quote_token, from, to, interval)))
        .ok_or(AppError(
            StatusCode::NOT_FOUND,
            anyhow!(
                "No candle data found for pair '{}/{}' in contract '{}'",
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

    pub fn get_pair_history(
        &self,
        base_token: &str,
        quote_token: &str,
    ) -> BTreeMap<TimestampMs, u32> {
        let pair = (base_token.to_string(), quote_token.to_string());
        self.orders_history.get(&pair).cloned().unwrap_or_default()
    }

    pub fn get_pair_candles(
        &self,
        base_token: &str,
        quote_token: &str,
        from: TimestampMs,
        to: TimestampMs,
        interval: u128,
    ) -> Vec<CandleStick> {
        let pair = (base_token.to_string(), quote_token.to_string());
        let history = self.orders_history.get(&pair).cloned().unwrap_or_default();

        let mut candles = Vec::new();
        let mut current_time = from;

        while current_time.0 < to.0 {
            let next_time = TimestampMs(current_time.0 + interval);

            let interval_trades: Vec<_> = history
                .iter()
                .filter(|(ts, _)| ts.0 >= current_time.0 && ts.0 < next_time.0)
                .collect();

            if !interval_trades.is_empty() {
                let prices: Vec<_> = interval_trades.iter().map(|(_, &price)| price).collect();
                let volume: u32 = prices.iter().sum();

                let candle = CandleStick {
                    timestamp: current_time,
                    open: *prices.first().unwrap_or(&0),
                    high: *prices.iter().max().unwrap_or(&0),
                    low: *prices.iter().min().unwrap_or(&0),
                    close: *prices.last().unwrap_or(&0),
                    volume,
                };

                candles.push(candle);
            }

            current_time = next_time;
        }

        candles
    }

    /// Returns a mapping from order IDs to user names
    pub fn get_order_user_map(
        &self,
        order_type: &OrderType,
        pair: &TokenPair,
    ) -> BTreeMap<OrderId, String> {
        let mut map = BTreeMap::new();
        let (base_token, quote_token) = pair.clone();
        let pair_key = (base_token.clone(), quote_token.clone());

        let relevant_orders = match order_type {
            OrderType::Buy => self.sell_orders.get(&pair_key),
            OrderType::Sell => self.buy_orders.get(&pair_key),
        };

        if let Some(order_ids) = relevant_orders {
            for order_id in order_ids {
                if let Some(user) = self.orders_owner.get(order_id) {
                    map.insert(order_id.clone(), user.clone());
                }
            }
        }
        map
    }
}
