use std::collections::HashSet;
use std::sync::Arc;

use client_sdk::contract_indexer::AppError;
use hyli_modules::log_error;
use orderbook::orderbook::OrderbookEvent;
use reqwest::StatusCode;
use serde::Serialize;
use serde_json::json;
use sqlx::{PgPool, Row};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::services::asset_service::AssetService;
use crate::services::user_service::UserService;

pub struct BookWriterService {
    pool: PgPool,
    user_service: Arc<RwLock<UserService>>,
    asset_service: Arc<RwLock<AssetService>>,
    trigger_url: String,
}

impl BookWriterService {
    pub fn new(
        pool: PgPool,
        user_service: Arc<RwLock<UserService>>,
        asset_service: Arc<RwLock<AssetService>>,
        trigger_url: String,
    ) -> Self {
        BookWriterService {
            pool,
            user_service,
            asset_service,
            trigger_url,
        }
    }

    pub async fn write_events(
        &self,
        user: &str,
        _events: Vec<OrderbookEvent>,
    ) -> Result<(), AppError> {
        // Transactionnaly write all events & update balances

        let mut symbol_book_updated = HashSet::<String>::new();

        let mut tx = self.pool.begin().await?;
        for event in _events {
            match event {
                OrderbookEvent::BalanceUpdated {
                    user,
                    token,
                    amount,
                } => {
                    if user == "orderbook" {
                        continue;
                    }
                    let asset_service = self.asset_service.read().await;
                    let asset = asset_service.get_asset(&token).await.ok_or(AppError(
                        StatusCode::NOT_FOUND,
                        anyhow::anyhow!("Asset not found: {}", token),
                    ))?;
                    let user_id = self.user_service.read().await.get_user_id(&user).await?;

                    info!(
                        "Updating balance for user {} with asset {:?} and amount {}",
                        user, asset, amount
                    );

                    log_error!(
                        sqlx::query(
                            "
                        INSERT INTO balances (user_id, asset_id, total)
                        VALUES ($1, $2, $3)
                        ON CONFLICT (user_id, asset_id) DO UPDATE SET
                            total = $3
                        ",
                        )
                        .bind(user_id)
                        .bind(asset.asset_id)
                        .bind(amount as i64)
                        .execute(&mut *tx)
                        .await,
                        "Failed to update balance"
                    )?;
                }
                OrderbookEvent::OrderCreated { order } => {
                    let user_id = self.user_service.read().await.get_user_id(&user).await?;
                    let symbol = format!("{}/{}", order.pair.0, order.pair.1);
                    let asset_service = self.asset_service.read().await;
                    let instrument =
                        asset_service.get_instrument(&symbol).await.ok_or(AppError(
                            StatusCode::NOT_FOUND,
                            anyhow::anyhow!("Instrument not found: {}", symbol),
                        ))?;

                    info!(
                        "Creating order for user {} with instrument {:?} and order {:?}",
                        user, instrument, order
                    );

                    symbol_book_updated.insert(symbol);

                    log_error!(
                        sqlx::query(
                            "INSERT INTO order_signed_ids (order_signed_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
                        )
                        .bind(order.order_id.clone())
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create order signed id"
                    )?;

                    log_error!(
                        sqlx::query(
                            "
                        INSERT INTO orders (
                            order_signed_id,
                            instrument_id, 
                            user_id, 
                            side, 
                            type,
                            price, 
                            qty
                            )
                        VALUES (
                            $1, 
                            $2, 
                            $3, 
                            $4, 
                            $5, 
                            $6, 
                            $7
                        )
                        ",
                        )
                        .bind(order.order_id)
                        .bind(instrument.instrument_id)
                        .bind(user_id)
                        .bind(order.order_side)
                        .bind(order.order_type)
                        .bind(order.price.map(|p| p as i64))
                        .bind(order.quantity as i64)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create order"
                    )?;
                }
                OrderbookEvent::OrderCancelled { order_id, pair } => {
                    info!(
                        "Cancelling order for user {} with order id {:?} and pair {:?}",
                        user, order_id, pair
                    );

                    symbol_book_updated.insert(format!("{}/{}", pair.0, pair.1));

                    log_error!(
                        sqlx::query(
                            "
                        UPDATE orders SET status = 'cancelled' WHERE order_user_signed_id = $1
                        ",
                        )
                        .bind(order_id)
                        .execute(&mut *tx)
                        .await,
                        "Failed to cancel order"
                    )?;
                }
                OrderbookEvent::OrderExecuted {
                    order_id,
                    taker_order_id,
                    pair,
                } => {
                    info!(
                        "Executing order for user {} with order id {:?} and taker order id {:?} on pair {:?}",
                        user, order_id, taker_order_id, pair
                    );

                    let user_id = self.user_service.read().await.get_user_id(&user).await?;
                    let asset_service = self.asset_service.read().await;
                    let instrument = asset_service
                        .get_instrument(&format!("{}/{}", pair.0, pair.1))
                        .await
                        .ok_or(AppError(
                            StatusCode::NOT_FOUND,
                            anyhow::anyhow!(
                                "Instrument not found: {}",
                                format!("{}/{}", pair.0, pair.1)
                            ),
                        ))?;

                    symbol_book_updated.insert(format!("{}/{}", pair.0, pair.1));

                    log_error!(
                        sqlx::query(
                            "
                        UPDATE orders SET status = 'filled', qty_filled = qty WHERE order_signed_id = $1
                        ",
                        )
                        .bind(order_id.clone())
                        .execute(&mut *tx)
                        .await,
                        "Failed to execute order"
                    )?;

                    log_error!(
                        sqlx::query(
                            "INSERT INTO order_signed_ids (order_signed_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
                        )
                        .bind(taker_order_id.clone())
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create order signed id"
                    )?;

                    log_error!(
                        sqlx::query(
                            "
                            WITH maker_order AS (
                                SELECT * FROM orders WHERE order_signed_id = $1
                            )
                            INSERT INTO trades (maker_order_signed_id, taker_order_signed_id, instrument_id, price, qty, side)
                            SELECT $1, $2, $3, maker_order.price, maker_order.qty, get_other_side(maker_order.side)
                            FROM maker_order
                            "
                        )
                        .bind(order_id)
                        .bind(taker_order_id)
                        .bind(instrument.instrument_id)
                        .execute(&mut *tx)
                        .await,
                        "Failed to execute order"
                    )?;
                }
                OrderbookEvent::OrderUpdate {
                    order_id,
                    taker_order_id,
                    remaining_quantity,
                    pair,
                } => {
                    info!(
                        "Updating order for user {} with order id {:?} and taker order id {:?} on pair {:?}",
                        user, order_id, taker_order_id, pair
                    );

                    let user_id = self.user_service.read().await.get_user_id(&user).await?;
                    let asset_service = self.asset_service.read().await;
                    let instrument = asset_service
                        .get_instrument(&format!("{}/{}", pair.0, pair.1))
                        .await
                        .ok_or(AppError(
                            StatusCode::NOT_FOUND,
                            anyhow::anyhow!("Instrument not found: {}", format!("{}/{}", pair.0, pair.1)),
                        ))?;

                    symbol_book_updated.insert(format!("{}/{}", pair.0, pair.1));

                    log_error!(
                        sqlx::query(
                            "INSERT INTO order_signed_ids (order_signed_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING"
                        )
                        .bind(taker_order_id.clone())
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create order signed id"
                    )?;

                    // The trade insert query must be done before the order update query to be able to compute the executed quantity
                    log_error!(
                        sqlx::query(
                            "
                            WITH maker_order AS (
                                SELECT * FROM orders WHERE order_signed_id = $1
                            )
                            INSERT INTO trades (maker_order_signed_id, taker_order_signed_id, instrument_id, price, qty, side)
                            SELECT $1, $2, $3, maker_order.price, $4 - maker_order.qty, get_other_side(maker_order.side)
                            FROM maker_order
                            "
                        )
                        .bind(order_id.clone())
                        .bind(taker_order_id)
                        .bind(instrument.instrument_id)
                        .bind(remaining_quantity as i64)
                        .execute(&mut *tx)
                        .await,
                        "Failed to update order"
                    )?;

                    log_error!(
                        sqlx::query(
                            "
                        UPDATE orders SET qty_filled = qty - $1 WHERE order_user_signed_id = $2
                        ",
                        )
                        .bind(remaining_quantity as i64)
                        .bind(order_id)
                        .execute(&mut *tx)
                        .await,
                        "Failed to update order"
                    )?;
                }
                OrderbookEvent::SessionKeyAdded { user } => {
                    info!("Creating user {}", user);

                    log_error!(
                        sqlx::query(
                            "INSERT INTO users (identity) VALUES ($1) ON CONFLICT DO NOTHING"
                        )
                        .bind(user)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create user"
                    )?;
                }
            }
        }
        tx.commit().await?;

        self.send_order_book_update(symbol_book_updated).await?;

        Ok(())
    }

    pub async fn send_order_book_update(&self, symbols: HashSet<String>) -> Result<(), AppError> {
        // Send a POST request to localhost:3000/api/websocket/trigger with instrument in body
        let client = reqwest::Client::new();
        let response = client
            .post(self.trigger_url.clone())
            .body(serde_json::to_string(&json!({ "instruments": symbols })).unwrap())
            .header("Content-Type", "application/json")
            .send()
            .await;
        if let Err(e) = response {
            warn!("Failed to send order book update: {}", e);
        }
        Ok(())
    }
}

pub struct BookService {
    pool: PgPool,
}

impl BookService {
    pub fn new(pool: PgPool) -> Self {
        BookService { pool }
    }

    pub async fn get_info(&self) -> Result<String, AppError> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        // Dummy implementation for example purposes
        Ok("Order book info".to_string())
    }

    pub async fn get_order_book(
        &self,
        base_asset_symbol: &str,
        quote_asset_symbol: &str,
        levels: u32,
        group_ticks: u32,
    ) -> Result<OrderbookAPI, AppError> {
        let symbol = format!(
            "{}/{}",
            base_asset_symbol.to_uppercase(),
            quote_asset_symbol.to_uppercase()
        );

        let rows = sqlx::query("SELECT * FROM get_orderbook_grouped_by_ticks($1, $2, $3);")
            .bind(symbol)
            .bind(levels as i32)
            .bind(group_ticks as i32)
            .fetch_all(&self.pool)
            .await?;

        let mut bids = Vec::new();
        let mut asks = Vec::new();

        for row in rows {
            let side: OrderSide = row.get("side");
            let price: i64 = row.get("price");
            let qty: i64 = row.get("qty");

            let entry = OrderbookAPIEntry {
                price: price as u32,
                quantity: qty as u32,
            };

            match side {
                OrderSide::Bid => {
                    bids.push(entry);
                }
                OrderSide::Ask => {
                    asks.push(entry);
                }
            }
        }

        Ok(OrderbookAPI { bids, asks })
    }
}

#[derive(Debug, Serialize, sqlx::Type)]
#[sqlx(type_name = "order_side", rename_all = "lowercase")]
pub enum OrderSide {
    Bid, // Buy
    Ask, // Sell
}

#[derive(Debug, Serialize)]
pub struct OrderbookAPI {
    pub bids: Vec<OrderbookAPIEntry>,
    pub asks: Vec<OrderbookAPIEntry>,
}

#[derive(Debug, Serialize)]
pub struct OrderbookAPIEntry {
    pub price: u32,
    pub quantity: u32,
}
