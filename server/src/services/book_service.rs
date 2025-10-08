use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use client_sdk::contract_indexer::AppError;
use hyli_modules::log_error;
use orderbook::order_manager::OrderManager;
use orderbook::orderbook::{Order, OrderSide, OrderbookEvent};
use orderbook::smt_values::UserInfo;
use reqwest::StatusCode;
use sdk::TxHash;
use serde::Serialize;
use sqlx::{PgPool, Row};
use tokio::sync::RwLock;
use tracing::info;

use crate::services::asset_service::{AssetService, MarketStatus};
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
        _trigger_url: String,
    ) -> Self {
        BookWriterService {
            pool,
            user_service,
            asset_service,
        }
    }

    #[tracing::instrument(skip(self))]
    pub async fn write_events(
        &self,
        user: &str,
        tx_hash: TxHash,
        _events: Vec<OrderbookEvent>,
    ) -> Result<(), AppError> {
        // Transactionnaly write all events & update balances

        let mut symbol_book_updated = HashSet::<String>::new();
        let mut reload_instrument_map = false;
        let mut trigger_notify_trades = false;
        let mut trigger_notify_orders = false;

        let mut tx = self.pool.begin().await?;

        let row = log_error!(
            sqlx::query("INSERT INTO commits (tx_hash) VALUES ($1) RETURNING commit_id")
                .bind(tx_hash.0)
                .fetch_one(&mut *tx)
                .await,
            "Failed to create commit"
        )?;
        let commit_id: i64 = row.get("commit_id");

        for event in _events {
            match event {
                OrderbookEvent::PairCreated { pair, info: _ } => {
                    let asset_service = self.asset_service.read().await;
                    let base_asset = asset_service.get_asset(&pair.0).await.ok_or(AppError(
                        StatusCode::NOT_FOUND,
                        anyhow::anyhow!("Base asset not found: {}", pair.0),
                    ))?;
                    let quote_asset = asset_service.get_asset(&pair.1).await.ok_or(AppError(
                        StatusCode::NOT_FOUND,
                        anyhow::anyhow!("Quote asset not found: {}", pair.1),
                    ))?;
                    log_error!(
                        sqlx::query(
                            "INSERT INTO instruments 
                                (commit_id, symbol, tick_size, qty_step, base_asset_id, quote_asset_id, status) 
                                VALUES 
                                ($1, $2, $3, $4, $5, $6, $7) 
                            ON CONFLICT DO NOTHING"
                        )
                        .bind(commit_id)
                        .bind(format!("{}/{}", pair.0, pair.1))
                        .bind(1_i64)
                        .bind(1_i64)
                        .bind(base_asset.asset_id)
                        .bind(quote_asset.asset_id)
                        .bind(MarketStatus::Active)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create pair"
                    )?;
                    reload_instrument_map = true;
                }
                OrderbookEvent::BalanceUpdated {
                    user,
                    symbol,
                    amount,
                } => {
                    if user == "orderbook" {
                        continue;
                    }
                    let asset_service = self.asset_service.read().await;
                    let asset = asset_service.get_asset(&symbol).await.ok_or(AppError(
                        StatusCode::NOT_FOUND,
                        anyhow::anyhow!("Asset not found: {symbol}"),
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

                    log_error!(
                        sqlx::query("INSERT INTO balance_events (commit_id, user_id, asset_id, total, kind) VALUES ($1, $2, $3, $4, 'transfer')")
                        .bind(commit_id)
                        .bind(user_id)
                        .bind(asset.asset_id)
                        .bind(amount as i64)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create balance event"
                    )?;
                }
                OrderbookEvent::OrderCreated { order } => {
                    trigger_notify_orders = true;

                    let user_id = self.user_service.read().await.get_user_id(user).await?;
                    let symbol = format!("{}/{}", order.pair.0, order.pair.1);
                    let asset_service = self.asset_service.read().await;
                    let instrument =
                        asset_service.get_instrument(&symbol).await.ok_or(AppError(
                            StatusCode::NOT_FOUND,
                            anyhow::anyhow!("Instrument not found: {symbol}"),
                        ))?;

                    info!(
                        "Creating order for user {} with instrument {:?} and order {:?}",
                        user, instrument, order
                    );

                    symbol_book_updated.insert(symbol);

                    log_error!(
                        sqlx::query("INSERT INTO orders (order_id, instrument_id, user_id, side, type, price, qty)
                                     VALUES ($1, $2, $3, $4, $5, $6, $7)")
                        .bind(order.order_id.clone())
                        .bind(instrument.instrument_id)
                        .bind(user_id)
                        .bind(order.order_side.clone())
                        .bind(order.order_type.clone())
                        .bind(order.price.map(|p| p as i64))
                        .bind(order.quantity as i64)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create order"
                    )?;

                    log_error!(
                        sqlx::query(
                            "INSERT INTO order_events (commit_id, order_id, user_id, instrument_id, side, type, price, qty, qty_filled, status)
                            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, 'open')"
                        )
                        .bind(commit_id)
                        .bind(order.order_id.clone())
                        .bind(user_id)
                        .bind(instrument.instrument_id)
                        .bind(order.order_side)
                        .bind(order.order_type)
                        .bind(order.price.map(|p| p as i64))
                        .bind(order.quantity as i64)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create order event"
                    )?;
                }
                OrderbookEvent::OrderCancelled { order_id, pair } => {
                    info!(
                        "Cancelling order for user {} with order id {:?} and pair {:?}",
                        user, order_id, pair
                    );
                    trigger_notify_orders = true;

                    symbol_book_updated.insert(format!("{}/{}", pair.0, pair.1));

                    log_error!(
                        sqlx::query(
                            "
                        UPDATE orders SET status = 'cancelled' WHERE order_id = $1
                        ",
                        )
                        .bind(order_id.clone())
                        .execute(&mut *tx)
                        .await,
                        "Failed to update order as cancelled"
                    )?;

                    log_error!(
                        sqlx::query(
                            "
                            INSERT INTO order_events (commit_id, order_id, user_id, instrument_id, side, type, price, qty, qty_filled, status)
                            VALUES select $1, order_id, user_id, instrument_id, side, type, price, qty, qty_filled, status from orders where order_id = $2"
                        )
                        .bind(commit_id)
                        .bind(order_id)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create order event"
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
                    trigger_notify_orders = true;
                    trigger_notify_trades = true;

                    let user_id = self.user_service.read().await.get_user_id(user).await?;
                    let asset_service = self.asset_service.read().await;
                    let instrument = asset_service
                        .get_instrument(&format!("{}/{}", pair.0, pair.1))
                        .await
                        .ok_or(AppError(
                            StatusCode::NOT_FOUND,
                            anyhow::anyhow!("Instrument not found: {}/{}", pair.0, pair.1),
                        ))?;

                    symbol_book_updated.insert(format!("{}/{}", pair.0, pair.1));

                    log_error!(
                        sqlx::query(
                            "
                        UPDATE orders SET status = 'filled', qty_filled = qty WHERE order_id = $1 returning user_id
                        ",
                        )
                        .bind(order_id.clone())
                        .execute(&mut *tx)
                        .await,
                        "Failed to update order as filled"
                    )?;

                    // TODO:have more data in the event to avoid the SELECT here
                    log_error!(
                        sqlx::query(
                            "
                            INSERT INTO order_events (commit_id, order_id, user_id, instrument_id, side, type, price, qty, qty_filled, status)
                            SELECT $1, order_id, user_id, instrument_id, side, type, price, qty, qty_filled, status FROM orders WHERE order_id = $2
                            "
                        )
                        .bind(commit_id)
                        .bind(order_id.clone())
                        .execute(&mut *tx)
                        .await,
                        "Failed to create order event"
                    )?;

                    // TODO:have more data in the event to avoid the SELECT here
                    log_error!(
                        sqlx::query(
                            "
                            WITH maker_order AS (
                                SELECT * FROM orders WHERE order_id = $2
                            )
                            INSERT INTO trade_events (commit_id, maker_order_id, taker_order_id, instrument_id, price, qty, side, maker_user_id, taker_user_id)
                            SELECT $1, $2, $3, $4, maker_order.price, maker_order.qty, get_other_side(maker_order.side), maker_order.user_id, $5
                            FROM maker_order
                            "
                        )
                        .bind(commit_id)
                        .bind(order_id)
                        .bind(taker_order_id)
                        .bind(instrument.instrument_id)
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await,
                        "Failed to insert trade event"
                    )?;
                }
                OrderbookEvent::OrderUpdate {
                    order_id,
                    taker_order_id,
                    remaining_quantity,
                    executed_quantity: _,
                    pair,
                } => {
                    info!(
                        "Updating order for user {} with order id {:?} and taker order id {:?} on pair {:?}",
                        user, order_id, taker_order_id, pair
                    );
                    trigger_notify_trades = true;
                    trigger_notify_orders = true;

                    let user_id = self.user_service.read().await.get_user_id(user).await?;
                    let asset_service = self.asset_service.read().await;
                    let instrument = asset_service
                        .get_instrument(&format!("{}/{}", pair.0, pair.1))
                        .await
                        .ok_or(AppError(
                            StatusCode::NOT_FOUND,
                            anyhow::anyhow!("Instrument not found: {}/{}", pair.0, pair.1),
                        ))?;

                    symbol_book_updated.insert(format!("{}/{}", pair.0, pair.1));

                    log_error!(
                        sqlx::query(
                            "
                        UPDATE orders SET status = 'partially_filled', qty_filled = qty - $1 WHERE order_id = $2 returning user_id
                        ",
                        )
                        .bind(remaining_quantity as i64)
                        .bind(order_id.clone())
                        .execute(&mut *tx)
                        .await,
                        "Failed to update order as partially filled"
                    )?;

                    log_error!(
                        sqlx::query(
                            "
                            INSERT INTO order_events (commit_id, order_id, user_id, instrument_id, side, type, price, qty, qty_filled, status)
                            SELECT $1, order_id, user_id, instrument_id, side, type, price, qty, qty_filled, status FROM orders WHERE order_id = $2
                            "
                        )
                        .bind(commit_id)
                        .bind(order_id.clone())
                        .execute(&mut *tx)
                        .await,
                        "Failed to create order event"
                    )?;

                    // The trade insert query must be done before the order update query to be able to compute the executed quantity
                    log_error!(
                        sqlx::query(
                            "
                            WITH maker_order AS (
                                SELECT * FROM orders WHERE order_id = $2
                            )
                            INSERT INTO trade_events (commit_id, maker_order_id, taker_order_id, instrument_id, price, qty, side, maker_user_id, taker_user_id)
                            SELECT $1, $2, $3, $4, maker_order.price, maker_order.qty, get_other_side(maker_order.side), maker_order.user_id, $5
                            FROM maker_order
                            "
                        )
                        .bind(commit_id)
                        .bind(order_id.clone())
                        .bind(taker_order_id)
                        .bind(instrument.instrument_id)
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await,
                        "Failed to insert trade event"
                    )?;
                }
                OrderbookEvent::SessionKeyAdded {
                    user,
                    salt,
                    nonce,
                    session_keys,
                } => {
                    let fetched_user_id = self.user_service.read().await.get_user_id(&user).await;

                    let user_id = if let Err(e) = fetched_user_id {
                        if e.0 == StatusCode::NOT_FOUND {
                            info!("Creating user {}", user);
                            let row = log_error!(
                                sqlx::query(
                                    "INSERT INTO users (commit_id, identity, salt, nonce) VALUES ($1, $2, $3, $4) ON CONFLICT (identity) DO UPDATE SET nonce = EXCLUDED.nonce RETURNING user_id"
                                )
                                .bind(commit_id)
                                .bind(user.clone())
                                .bind(salt)
                                .bind(nonce as i64)
                                .fetch_one(&mut *tx)
                                .await,
                                "Failed to create user"
                            )?;
                            row.get::<i64, _>("user_id")
                        } else {
                            return Err(e);
                        }
                    } else {
                        fetched_user_id?
                    };

                    info!("Setting user session keys for user {}", user);

                    log_error!(
                        sqlx::query("INSERT INTO user_session_keys (commit_id, user_id, session_keys) VALUES ($1, $2, $3)")
                        .bind(commit_id)
                        .bind(user_id)
                        .bind(session_keys)
                        .execute(&mut *tx)
                        .await,
                        "Failed to create user session key"
                    )?;
                }
                OrderbookEvent::NonceIncremented { user, nonce } => {
                    info!("Incrementing nonce for user {}", user);
                    log_error!(
                        sqlx::query("UPDATE users SET nonce = $1 WHERE identity = $2")
                            .bind(nonce as i64)
                            .bind(user)
                            .execute(&mut *tx)
                            .await,
                        "Failed to increment nonce"
                    )?;
                }
            }
        }

        if trigger_notify_trades {
            info!("Notifying trades");
            log_error!(
                sqlx::query("select pg_notify('trades', 'trades')")
                    .execute(&mut *tx)
                    .await,
                "Failed to notify 'trades'"
            )?;
        }

        if trigger_notify_orders {
            info!("Notifying orders");
            log_error!(
                sqlx::query("select pg_notify('orders', 'orders')")
                    .execute(&mut *tx)
                    .await,
                "Failed to notify 'orders'"
            )?;
        }

        for symbol in symbol_book_updated {
            info!("Notifying book for symbol {}", symbol);
            log_error!(
                sqlx::query("select pg_notify('book', $1)")
                    .bind(symbol)
                    .execute(&mut *tx)
                    .await,
                "Failed to notify 'book'"
            )?;
        }

        tx.commit().await?;

        if reload_instrument_map {
            log_error!(
                sqlx::query("select pg_notify('instruments', 'instruments')")
                    .execute(&self.pool)
                    .await,
                "Failed to notify 'instruments'"
            )?;
            let mut asset_service = self.asset_service.write().await;
            asset_service.reload_instrument_map().await?;
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

    pub async fn get_order_manager(
        &self,
        users_info: &HashMap<String, UserInfo>,
    ) -> Result<OrderManager, AppError> {
        let rows = sqlx::query(
            "
        SELECT 
            o.order_id, 
            o.type, 
            o.side, 
            o.price, 
            o.qty_remaining, 
            u.identity,
            base_asset.symbol as base_asset_symbol,
            quote_asset.symbol as quote_asset_symbol
        FROM orders o
        JOIN instruments i ON o.instrument_id = i.instrument_id
        JOIN assets base_asset ON i.base_asset_id = base_asset.asset_id
        JOIN assets quote_asset ON i.quote_asset_id = quote_asset.asset_id
        JOIN users u ON o.user_id = u.user_id
        ORDER BY o.created_at ASC
        ",
        )
        .fetch_all(&self.pool)
        .await?;

        let orders: HashMap<String, (Order, String)> = rows
            .iter()
            .map(|row| {
                (
                    row.get("order_id"),
                    (
                        Order {
                            order_id: row.get("order_id"),
                            order_type: row.get("type"),
                            order_side: row.get("side"),
                            price: row.try_get("price").map(|p: i64| p as u64).ok(),
                            pair: (row.get("base_asset_symbol"), row.get("quote_asset_symbol")),
                            quantity: row.get::<i64, _>("qty_remaining") as u64,
                        },
                        row.get("identity"),
                    ),
                )
            })
            .collect();

        let buy_orders: BTreeMap<(String, String), VecDeque<String>> = rows
            .iter()
            .rev()
            .filter(|row| row.get::<OrderSide, _>("side") == OrderSide::Bid)
            .fold(BTreeMap::new(), |mut acc, row| {
                acc.entry((row.get("base_asset_symbol"), row.get("quote_asset_symbol")))
                    .or_default()
                    .push_back(row.get("order_id"));
                acc
            });

        let sell_orders: BTreeMap<(String, String), VecDeque<String>> = rows
            .iter()
            .filter(|row| row.get::<OrderSide, _>("side") == OrderSide::Ask)
            .fold(BTreeMap::new(), |mut acc, row| {
                acc.entry((row.get("base_asset_symbol"), row.get("quote_asset_symbol")))
                    .or_default()
                    .push_back(row.get("order_id"));
                acc
            });

        let orders_owner = orders
            .iter()
            .map(|(_, (order, user))| {
                (
                    order.order_id.clone(),
                    users_info.get(user).unwrap().get_key(),
                )
            })
            .collect();

        let orders = orders.into_iter().map(|(k, (o, _))| (k, o)).collect();

        Ok(OrderManager {
            orders,
            buy_orders,
            sell_orders,
            orders_owner,
        })
    }
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
