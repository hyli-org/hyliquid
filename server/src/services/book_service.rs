use std::sync::Arc;

use client_sdk::contract_indexer::AppError;
use orderbook::orderbook::OrderbookEvent;
use reqwest::StatusCode;
use serde::Serialize;
use sqlx::{PgPool, Row};
use tokio::sync::RwLock;
use tracing::info;

use crate::services::asset_service::AssetService;
use crate::services::user_service::UserService;

pub struct BookWriterService {
    pool: PgPool,
    user_service: Arc<RwLock<UserService>>,
    asset_service: Arc<RwLock<AssetService>>,
}

impl BookWriterService {
    pub fn new(
        pool: PgPool,
        user_service: Arc<RwLock<UserService>>,
        asset_service: Arc<RwLock<AssetService>>,
    ) -> Self {
        BookWriterService {
            pool,
            user_service,
            asset_service,
        }
    }

    pub async fn write_events(
        &self,
        user: &str,
        _events: Vec<OrderbookEvent>,
    ) -> Result<(), AppError> {
        // Transactionnaly write all events & update balances

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
                    .await?;
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

                    sqlx::query(
                        "
                        INSERT INTO orders (
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
                            $6
                        )
                        ",
                    )
                    .bind(instrument.instrument_id)
                    .bind(user_id)
                    .bind(order.order_side)
                    .bind(order.order_type)
                    .bind(order.price.map(|p| p as i64))
                    .bind(order.quantity as i64)
                    .execute(&mut *tx)
                    .await?;
                }
                OrderbookEvent::OrderCancelled { order_id, pair } => {
                    sqlx::query(
                        "
                        UPDATE orders SET status = 'cancelled' WHERE order_id = $1
                        ",
                    )
                    .bind(order_id)
                    .execute(&mut *tx)
                    .await?;
                }
                OrderbookEvent::OrderExecuted { order_id, pair } => {
                    sqlx::query(
                        "
                        UPDATE orders SET status = 'executed' WHERE order_id = $1
                        ",
                    )
                    .bind(order_id)
                    .execute(&mut *tx)
                    .await?;
                }
                OrderbookEvent::OrderUpdate {
                    order_id,
                    remaining_quantity,
                    pair,
                } => {
                    sqlx::query(
                        "
                        UPDATE orders SET qty_remaining = $1 WHERE order_id = $2
                        ",
                    )
                    .bind(remaining_quantity as i64)
                    .bind(order_id)
                    .execute(&mut *tx)
                    .await?;
                }
                OrderbookEvent::SessionKeyAdded { user: _user } => {}
            }
        }
        tx.commit().await?;
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
