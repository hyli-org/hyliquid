use std::collections::HashSet;
use std::sync::Arc;

use anyhow::Result;
use client_sdk::rest_client::{NodeApiClient, NodeApiHttpClient};
use hyli_modules::{
    bus::{BusMessage, SharedMessageBus},
    log_error, module_bus_client, module_handle_messages,
    modules::Module,
};
use orderbook::model::OrderbookEvent;
use reqwest::StatusCode;
use sdk::{BlobTransaction, TxHash};
use sqlx::{PgPool, Row};
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use crate::services::user_service::UserService;
use crate::{prover::OrderbookProverRequest, services::asset_service::AssetService};

#[derive(Debug, Clone)]
pub enum DatabaseRequest {
    WriteEvents {
        user: String,
        tx_hash: TxHash,
        blob_tx: BlobTransaction,
        prover_request: OrderbookProverRequest,
    },
}

impl BusMessage for DatabaseRequest {}

module_bus_client! {
    #[derive(Debug)]
    struct DatabaseModuleBusClient {
        receiver(DatabaseRequest),
    }
}

pub struct DatabaseModuleCtx {
    pub pool: PgPool,
    pub user_service: Arc<RwLock<UserService>>,
    pub asset_service: Arc<RwLock<AssetService>>,
    pub client: Arc<NodeApiHttpClient>,
    pub no_blobs: bool,
}

pub struct DatabaseModule {
    ctx: Arc<DatabaseModuleCtx>,
    bus: DatabaseModuleBusClient,
}

impl Module for DatabaseModule {
    type Context = Arc<DatabaseModuleCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let bus = DatabaseModuleBusClient::new_from_bus(bus.new_handle()).await;
        Ok(DatabaseModule { ctx, bus })
    }

    async fn run(&mut self) -> Result<()> {
        self.start().await?;
        Ok(())
    }
}

impl DatabaseModule {
    pub async fn start(&mut self) -> Result<()> {
        module_handle_messages! {
            on_self self,

            listen<DatabaseRequest> request => {
                _ = log_error!(self.handle_database_request(request).await, "handle database request")
            }
        };
        Ok(())
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    async fn handle_database_request(&mut self, request: DatabaseRequest) -> Result<()> {
        match request {
            DatabaseRequest::WriteEvents {
                user,
                tx_hash,
                blob_tx,
                prover_request,
            } => {
                match self
                    .write_events(&user, tx_hash.clone(), prover_request)
                    .await
                {
                    Ok(()) => {
                        println!(
                            "Events written successfully for user {user} with tx hash {tx_hash:#}"
                        );
                    }
                    Err(e) => {
                        println!(
                            "Error writing events for user {user} with tx hash {tx_hash:#}: {}",
                            e
                        );
                        error!(
                            "Error writing events for user {user} with tx hash {tx_hash:#}: {}",
                            e
                        );
                        return Err(anyhow::anyhow!("Failed to write events: {}", e));
                    }
                }
                if !self.ctx.no_blobs {
                    self.ctx.client.send_tx_blob(blob_tx).await?;
                }
            }
        }
        Ok(())
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    async fn write_events(
        &self,
        user: &str,
        tx_hash: TxHash,
        prover_request: OrderbookProverRequest,
    ) -> Result<()> {
        debug!("Writing events for user {user} with tx hash {tx_hash:#}");
        use crate::services::asset_service::MarketStatus;

        let mut symbol_book_updated = HashSet::<String>::new();
        let mut reload_instrument_map = false;
        let mut trigger_notify_trades = false;
        let mut trigger_notify_orders = false;

        let mut tx = log_error!(self.ctx.pool.begin().await, "Failed to begin transaction")?;

        debug!("Transaction started");
        println!("Transaction started");

        let row = sqlx::query("INSERT INTO commits (tx_hash) VALUES ($1) RETURNING commit_id")
            .bind(tx_hash.0.clone())
            .fetch_one(&mut *tx)
            .await;
        if let Err(e) = row {
            println!("Error creating commit: {}", e);
            return Err(anyhow::anyhow!("Failed to create commit: {}", e));
        } else {
            println!("Commit created");
        }
        let row = row.unwrap();
        println!("Row unwrapped");
        let commit_id: i64 = row.get("commit_id");
        println!("Commit id: {}", commit_id);
        debug!("Created commit with id {}", commit_id);

        for event in prover_request.events.clone() {
            println!("Processing event: {:?}", event);
            match event {
                OrderbookEvent::PairCreated { pair, info: _ } => {
                    let asset_service = self.ctx.asset_service.read().await;
                    let base_asset = asset_service
                        .get_asset(&pair.0)
                        .ok_or_else(|| anyhow::anyhow!("Base asset not found: {}", pair.0))?;
                    let quote_asset = asset_service
                        .get_asset(&pair.1)
                        .ok_or_else(|| anyhow::anyhow!("Quote asset not found: {}", pair.1))?;
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
                    let asset_service = self.ctx.asset_service.read().await;
                    let asset = asset_service
                        .get_asset(&symbol)
                        .ok_or_else(|| anyhow::anyhow!("Asset not found: {symbol}"))?;
                    let user_id = self
                        .ctx
                        .user_service
                        .read()
                        .await
                        .get_user_id(&user, &mut tx)
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e.1))?;

                    debug!(
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

                    let symbol = format!("{}/{}", order.pair.0, order.pair.1);
                    let asset_service = self.ctx.asset_service.read().await;
                    let instrument = asset_service
                        .get_instrument(&symbol)
                        .ok_or_else(|| anyhow::anyhow!("Instrument not found: {symbol}"))?;

                    debug!(
                        "Creating order for user {} with instrument {:?} and order {:?}",
                        user, instrument, order
                    );

                    let events_user_id = self
                        .ctx
                        .user_service
                        .read()
                        .await
                        .get_user_id(user, &mut tx)
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e.1))?;

                    symbol_book_updated.insert(symbol);

                    log_error!(
                        sqlx::query("INSERT INTO orders (order_id, instrument_id, user_id, side, type, price, qty)
                                     VALUES ($1, $2, $3, $4, $5, $6, $7)")
                        .bind(order.order_id.clone())
                        .bind(instrument.instrument_id)
                        .bind(events_user_id)
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
                        .bind(events_user_id)
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
                    debug!(
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
                    debug!(
                        "Executing order for user {} with order id {:?} and taker order id {:?} on pair {:?}",
                        user, order_id, taker_order_id, pair
                    );
                    trigger_notify_orders = true;
                    trigger_notify_trades = true;

                    let asset_service = self.ctx.asset_service.read().await;
                    let instrument = asset_service
                        .get_instrument(&format!("{}/{}", pair.0, pair.1))
                        .ok_or_else(|| {
                            anyhow::anyhow!("Instrument not found: {}/{}", pair.0, pair.1)
                        })?;

                    let events_user_id = self
                        .ctx
                        .user_service
                        .read()
                        .await
                        .get_user_id(user, &mut tx)
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e.1))?;

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
                        .bind(events_user_id)
                        .execute(&mut *tx)
                        .await,
                        "Failed to insert trade event"
                    )?;
                }
                OrderbookEvent::OrderUpdate {
                    order_id,
                    taker_order_id,
                    remaining_quantity,
                    executed_quantity,
                    pair,
                } => {
                    debug!(
                        "Updating order for user {} with order id {:?} and taker order id {:?} on pair {:?}",
                        user, order_id, taker_order_id, pair
                    );
                    trigger_notify_trades = true;
                    trigger_notify_orders = true;

                    let asset_service = self.ctx.asset_service.read().await;
                    let instrument = asset_service
                        .get_instrument(&format!("{}/{}", pair.0, pair.1))
                        .ok_or_else(|| {
                            anyhow::anyhow!("Instrument not found: {}/{}", pair.0, pair.1)
                        })?;

                    let events_user_id = self
                        .ctx
                        .user_service
                        .read()
                        .await
                        .get_user_id(user, &mut tx)
                        .await
                        .map_err(|e| anyhow::anyhow!("{}", e.1))?;

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
                            SELECT $1, $2, $3, $4, maker_order.price, $5, get_other_side(maker_order.side), maker_order.user_id, $6
                            FROM maker_order
                            "
                        )
                        .bind(commit_id)
                        .bind(order_id.clone())
                        .bind(taker_order_id)
                        .bind(instrument.instrument_id)
                        .bind(executed_quantity as i64)
                        .bind(events_user_id)
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
                    let fetched_user_id = self
                        .ctx
                        .user_service
                        .read()
                        .await
                        .get_user_id(&user, &mut tx)
                        .await;

                    let user_id = if let Err(e) = fetched_user_id {
                        if e.0 == StatusCode::NOT_FOUND {
                            println!("User not found. Creating user {}", user);
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
                            println!("User not found. Error: {}", e.1);
                            return Err(anyhow::anyhow!("{}", e.1));
                        }
                    } else {
                        let user_id = fetched_user_id.map_err(|e| anyhow::anyhow!("{}", e.1))?;
                        println!("User found. User id: {}", user_id);
                        user_id
                    };

                    println!("Setting user session keys for user {}", user);
                    debug!("Setting user session keys for user {}", user);

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
                    println!("Incrementing nonce for user {}", user);
                    debug!("Incrementing nonce for user {}", user);
                    let row = log_error!(
                        sqlx::query(
                            "UPDATE users SET nonce = $1 WHERE identity = $2 RETURNING user_id"
                        )
                        .bind(nonce as i64)
                        .bind(user)
                        .fetch_one(&mut *tx)
                        .await,
                        "Failed to increment nonce"
                    )?;
                    let user_id = row.get::<i64, _>("user_id");
                    log_error!(
                        sqlx::query("INSERT INTO user_events_nonces (commit_id, user_id, nonce) VALUES ($1, $2, $3)")
                            .bind(commit_id)
                            .bind(user_id)
                            .bind(nonce as i64)
                            .execute(&mut *tx)
                            .await,
                        "Failed to insert user event nonce"
                    )?;
                }
            }
        }

        let json_data = log_error!(
            serde_json::to_vec(&prover_request),
            "Failed to serialize prover request"
        )?;

        log_error!(
            sqlx::query(
                "INSERT INTO prover_requests (commit_id, tx_hash, request) VALUES ($1, $2, $3)"
            )
            .bind(commit_id)
            .bind(tx_hash.0.clone())
            .bind(json_data)
            .execute(&mut *tx)
            .await,
            "Failed to insert prover request"
        )?;

        if trigger_notify_trades {
            debug!("Notifying trades");
            log_error!(
                sqlx::query("select pg_notify('trades', 'trades')")
                    .execute(&mut *tx)
                    .await,
                "Failed to notify 'trades'"
            )?;
        }

        if trigger_notify_orders {
            debug!("Notifying orders");
            log_error!(
                sqlx::query("select pg_notify('orders', 'orders')")
                    .execute(&mut *tx)
                    .await,
                "Failed to notify 'orders'"
            )?;
        }

        for symbol in symbol_book_updated {
            debug!("Notifying book for symbol {}", symbol);
            log_error!(
                sqlx::query("select pg_notify('book', $1)")
                    .bind(symbol)
                    .execute(&mut *tx)
                    .await,
                "Failed to notify 'book'"
            )?;
        }

        println!("Committing transaction");
        log_error!(tx.commit().await, "Failed to commit transaction")?;
        println!("Transaction committed");
        debug!("Committed transaction with commit id {}", commit_id);

        if reload_instrument_map {
            log_error!(
                sqlx::query("select pg_notify('instruments', 'instruments')")
                    .execute(&self.ctx.pool)
                    .await,
                "Failed to notify 'instruments'"
            )?;
            let mut asset_service = self.ctx.asset_service.write().await;
            asset_service
                .reload_instrument_map()
                .await
                .map_err(|e| anyhow::anyhow!("{}", e.1))?;
        }

        Ok(())
    }
}
