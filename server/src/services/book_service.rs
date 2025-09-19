use client_sdk::contract_indexer::AppError;
use orderbook::orderbook::OrderbookEvent;
use sdk::Identity;
use serde::Serialize;
use sqlx::{PgPool, Row};

pub struct BookWriterService {
    pool: PgPool,
}

impl BookWriterService {
    pub fn new(pool: PgPool) -> Self {
        BookWriterService { pool }
    }

    pub async fn write_events(
        &self,
        user: Identity,
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
                    sqlx::query(
                        "
                        UPDATE 
                            balances SET total = $1 
                        JOIN users ON balances.user_id = users.user_id
                        JOIN assets ON balances.asset_id = assets.asset_id
                        WHERE 
                            users.identity = $1
                            AND assets.symbol = $2
                        ",
                    )
                    .bind(amount as i64)
                    .bind(user)
                    .bind(token)
                    .execute(&mut *tx)
                    .await?;
                }
                OrderbookEvent::OrderCreated { order } => {
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
                            SELECT instrument_id FROM instruments WHERE symbol = $1, 
                            $2, 
                            $3, 
                            $4, 
                            $5, 
                            $6
                        )
                        ",
                    )
                    .bind(&format!("{}/{}", order.pair.0, order.pair.1))
                    .bind(user.0.clone())
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
