use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use client_sdk::rest_client::{NodeApiClient, NodeApiHttpClient};
use hyli_modules::{
    bus::{BusMessage, SharedMessageBus},
    log_error, module_bus_client, module_handle_messages,
    modules::Module,
};
use opentelemetry::Context;
use opentelemetry::{
    metrics::{Histogram, Meter, UpDownCounter},
    KeyValue,
};
use orderbook::model::{OrderId, OrderbookEvent, UserInfo};
use reqwest::StatusCode;
use sdk::{BlobTransaction, TxHash};
use sqlx::PgPool;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, Instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::services::user_service::UserService;
use crate::{prover::OrderbookProverRequest, services::asset_service::AssetService};

/// Metrics for tracking database operation durations
#[derive(Clone)]
pub struct DatabaseMetrics {
    /// Total duration of write_events function
    pub write_events_duration: Histogram<f64>,
    /// Duration of transaction begin
    pub transaction_begin_duration: Histogram<f64>,
    /// Duration of commit insert
    pub commit_insert_duration: Histogram<f64>,
    /// Duration of event processing by event type
    pub event_processing_duration: Histogram<f64>,
    /// Duration of balance update operations
    pub balance_update_duration: Histogram<f64>,
    /// Duration of order creation operations
    pub order_create_duration: Histogram<f64>,
    /// Duration of order cancel operations
    pub order_cancel_duration: Histogram<f64>,
    /// Duration of order execution operations
    pub order_execute_duration: Histogram<f64>,
    /// Duration of order update operations
    pub order_update_duration: Histogram<f64>,
    /// Duration of user operations
    pub user_ops_duration: Histogram<f64>,
    /// Duration of prover request insert
    pub prover_request_insert_duration: Histogram<f64>,
    /// Duration of contract events insert
    pub contract_events_insert_duration: Histogram<f64>,
    /// Duration of notifications
    pub notification_duration: Histogram<f64>,
    /// Duration of transaction commit
    pub transaction_commit_duration: Histogram<f64>,
    /// Duration of blob transaction sending
    pub blob_send_duration: Histogram<f64>,
    /// Number of pending requests in worker queues
    pub worker_queue_depth: UpDownCounter<i64>,
    /// Total number of active workers
    pub worker_count: UpDownCounter<i64>,
}

impl DatabaseMetrics {
    /// Create a new DatabaseMetrics instance with the global meter provider
    pub fn new() -> Self {
        let meter = opentelemetry::global::meter("database");
        Self::with_meter(meter)
    }

    /// Create a new DatabaseMetrics instance with a specific meter
    pub fn with_meter(meter: Meter) -> Self {
        // Custom buckets for millisecond-level latencies
        // Covers: 10μs, 50μs, 100μs, 500μs, 1ms, 5ms, 10ms, 50ms, 100ms, 250ms, 500ms, 1000ms, 2500ms, 5000ms, 10000ms, 25000ms, 50000ms, 100000ms
        let latency_buckets = vec![
            0.00001, 0.00005, 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
            5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0, 25000.0,
            50000.0, 100000.0,
        ];

        Self {
            write_events_duration: meter
                .f64_histogram("db.write_events.duration")
                .with_description("Total duration of write_events function in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            transaction_begin_duration: meter
                .f64_histogram("db.transaction.begin.duration")
                .with_description("Duration of transaction begin in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            commit_insert_duration: meter
                .f64_histogram("db.commit.insert.duration")
                .with_description("Duration of commit insert in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            event_processing_duration: meter
                .f64_histogram("db.event.processing.duration")
                .with_description("Duration of event processing by type in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            balance_update_duration: meter
                .f64_histogram("db.balance.update.duration")
                .with_description("Duration of balance update operations in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            order_create_duration: meter
                .f64_histogram("db.order.create.duration")
                .with_description("Duration of order creation operations in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            order_cancel_duration: meter
                .f64_histogram("db.order.cancel.duration")
                .with_description("Duration of order cancel operations in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            order_execute_duration: meter
                .f64_histogram("db.order.execute.duration")
                .with_description("Duration of order execution operations in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            order_update_duration: meter
                .f64_histogram("db.order.update.duration")
                .with_description("Duration of order update operations in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            user_ops_duration: meter
                .f64_histogram("db.user.ops.duration")
                .with_description("Duration of user operations in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            prover_request_insert_duration: meter
                .f64_histogram("db.prover_request.insert.duration")
                .with_description("Duration of prover request insert in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            contract_events_insert_duration: meter
                .f64_histogram("db.contract_events.insert.duration")
                .with_description("Duration of contract events insert in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            notification_duration: meter
                .f64_histogram("db.notification.duration")
                .with_description("Duration of PostgreSQL notifications in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            transaction_commit_duration: meter
                .f64_histogram("db.transaction.commit.duration")
                .with_description("Duration of transaction commit in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            blob_send_duration: meter
                .f64_histogram("db.blob.send.duration")
                .with_description("Duration of blob transaction sending in seconds")
                .with_unit("s")
                .with_boundaries(latency_buckets.clone())
                .build(),
            worker_queue_depth: meter
                .i64_up_down_counter("db.worker.queue.depth")
                .with_description("Number of pending requests in worker queues")
                .with_unit("requests")
                .build(),
            worker_count: meter
                .i64_up_down_counter("db.worker.count")
                .with_description("Total number of active database workers")
                .with_unit("workers")
                .build(),
        }
    }

    /// Record the duration of an operation
    #[inline]
    fn record(&self, histogram: &Histogram<f64>, start: Instant, labels: &[KeyValue]) {
        let duration = start.elapsed().as_secs_f64();
        histogram.record(duration, labels);
    }
}

impl Default for DatabaseMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub enum DatabaseRequest {
    WriteEvents {
        user: UserInfo,
        tx_hash: TxHash,
        blob_tx: BlobTransaction,
        prover_request: OrderbookProverRequest,
        context: Context,
    },
}

impl BusMessage for DatabaseRequest {
    const CAPACITY: usize = 10000000;
}

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
    pub metrics: DatabaseMetrics,
}

/// Service for database operations that can be called directly
#[derive(Clone)]
pub struct DatabaseService {
    ctx: Arc<DatabaseModuleCtx>,
}

impl DatabaseService {
    pub fn new(ctx: Arc<DatabaseModuleCtx>) -> Self {
        Self { ctx }
    }

    /// Write events to the database and optionally send blob transaction
    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self,)))]
    pub async fn write_events(
        &self,
        user: UserInfo,
        tx_hash: TxHash,
        blob_tx: BlobTransaction,
        prover_request: OrderbookProverRequest,
        context: Context,
    ) -> Result<()> {
        tracing::Span::current().set_parent(context);
        log_error!(
            self.write_events_internal(&user, tx_hash.clone(), &prover_request)
                .await,
            "Failed to write events"
        )?;
        Ok(())
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    async fn write_events_internal(
        &self,
        user_info: &UserInfo,
        tx_hash: TxHash,
        prover_request: &OrderbookProverRequest,
    ) -> Result<()> {
        let write_events_start = Instant::now();
        let user = &user_info.user;
        debug!("Writing events for user {user} with tx hash {tx_hash:#}");
        use crate::services::asset_service::MarketStatus;

        let mut reload_instrument_map = false;

        let tx_begin_start = Instant::now();
        let mut tx = log_error!(
            self.ctx
                .pool
                .begin()
                .instrument(tracing::info_span!("begin_transaction"))
                .await,
            "Failed to begin transaction"
        )?;
        self.ctx.metrics.record(
            &self.ctx.metrics.transaction_begin_duration,
            tx_begin_start,
            &[],
        );

        let commit_insert_start = Instant::now();
        // Use the global nonce provided by the request as the commit identifier to preserve ordering across workers.
        let commit_id: i64 = prover_request.nonce as i64;

        log_error!(
            sqlx::query("INSERT INTO commits (commit_id, tx_hash) VALUES ($1, $2)")
                .bind(commit_id)
                .bind(tx_hash.0.clone())
                .execute(&mut *tx)
                .instrument(tracing::info_span!("create_commit"))
                .await,
            "Failed to create commit"
        )?;
        self.ctx.metrics.record(
            &self.ctx.metrics.commit_insert_duration,
            commit_insert_start,
            &[],
        );

        debug!("Created commit with id {}", commit_id);

        for event in prover_request.events.clone() {
            let event_start = Instant::now();
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
                        .instrument(tracing::info_span!("create_pair"))
                        .await,
                        "Failed to create pair"
                    )?;
                    reload_instrument_map = true;
                    self.ctx.metrics.record(
                        &self.ctx.metrics.event_processing_duration,
                        event_start,
                        &[KeyValue::new("event_type", "pair_created")],
                    );
                }
                OrderbookEvent::BalanceUpdated {
                    user,
                    symbol,
                    amount,
                } => {
                    if user == "orderbook" {
                        continue;
                    }
                    let balance_start = Instant::now();
                    let asset_service = self.ctx.asset_service.read().await;
                    let asset = asset_service
                        .get_asset(&symbol)
                        .ok_or_else(|| anyhow::anyhow!("Asset not found: {symbol}"))?;

                    debug!(
                        "Updating balance for user {} with asset {:?} and amount {}",
                        user, asset, amount
                    );

                    log_error!(
                        sqlx::query("INSERT INTO balance_events (commit_id, identity, asset_id, total, kind) VALUES ($1, $2, $3, $4, 'transfer')")
                        .bind(commit_id)
                        .bind(user)
                        .bind(asset.asset_id)
                        .bind(amount as i64)
                        .execute(&mut *tx)
                        .instrument(tracing::info_span!("create_balance_event"))
                        .await,
                        "Failed to create balance event"
                    )?;
                    self.ctx.metrics.record(
                        &self.ctx.metrics.balance_update_duration,
                        balance_start,
                        &[],
                    );
                    self.ctx.metrics.record(
                        &self.ctx.metrics.event_processing_duration,
                        event_start,
                        &[KeyValue::new("event_type", "balance_updated")],
                    );
                }
                OrderbookEvent::OrderCreated { order } => {
                    let order_create_start = Instant::now();

                    let symbol = format!("{}/{}", order.pair.0, order.pair.1);
                    let asset_service = self.ctx.asset_service.read().await;
                    let instrument = asset_service
                        .get_instrument(&symbol)
                        .ok_or_else(|| anyhow::anyhow!("Instrument not found: {symbol}"))?;

                    debug!(
                        "Creating order for user {} with instrument {:?} and order {:?}",
                        user, instrument, order
                    );

                    log_error!(
                        sqlx::query("INSERT INTO orders (order_id, instrument_id, identity, side, type, price, qty)
                                     VALUES ($1, $2, $3, $4, $5, $6, $7)")
                        .bind(order.order_id.clone())
                        .bind(instrument.instrument_id)
                        .bind(user.clone())
                        .bind(order.order_side.clone())
                        .bind(order.order_type.clone())
                        .bind(order.price.map(|p| p as i64))
                        .bind(order.quantity as i64)
                        .execute(&mut *tx)
                        .instrument(tracing::info_span!("create_order"))
                        .await,
                        "Failed to create order"
                    )?;

                    log_error!(
                        sqlx::query(
                            "INSERT INTO order_events (commit_id, order_id, identity, instrument_id, side, type, price, qty, qty_filled, status)
                            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, 'open')"
                        )
                        .bind(commit_id)
                        .bind(order.order_id)
                        .bind(user)
                        .bind(instrument.instrument_id)
                        .bind(order.order_side)
                        .bind(order.order_type)
                        .bind(order.price.map(|p| p as i64))
                        .bind(order.quantity as i64)
                        .execute(&mut *tx)
                        .instrument(tracing::info_span!("create_order_event"))
                        .await,
                        "Failed to create order event"
                    )?;
                    self.ctx.metrics.record(
                        &self.ctx.metrics.order_create_duration,
                        order_create_start,
                        &[],
                    );
                    self.ctx.metrics.record(
                        &self.ctx.metrics.event_processing_duration,
                        event_start,
                        &[KeyValue::new("event_type", "order_created")],
                    );
                }
                OrderbookEvent::OrderCancelled { order_id, pair } => {
                    debug!(
                        "Cancelling order for user {} with order id {:?} and pair {:?}",
                        user, order_id, pair
                    );
                    let order_cancel_start = Instant::now();

                    log_error!(
                        sqlx::query(
                            "
                            INSERT INTO order_events (commit_id, order_id, identity, instrument_id, side, type, price, qty, qty_filled, status)
                            VALUES select $1, order_id, identity, instrument_id, side, type, price, qty, qty_filled, status from orders where order_id = $2"
                        )
                        .bind(commit_id)
                        .bind(order_id)
                        .execute(&mut *tx)
                        .instrument(tracing::info_span!("create_order_event"))
                        .await,
                        "Failed to create order event"
                    )?;
                    self.ctx.metrics.record(
                        &self.ctx.metrics.order_cancel_duration,
                        order_cancel_start,
                        &[],
                    );
                    self.ctx.metrics.record(
                        &self.ctx.metrics.event_processing_duration,
                        event_start,
                        &[KeyValue::new("event_type", "order_cancelled")],
                    );
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
                    let order_execute_start = Instant::now();

                    let asset_service = self.ctx.asset_service.read().await;
                    let instrument = asset_service
                        .get_instrument(&format!("{}/{}", pair.0, pair.1))
                        .ok_or_else(|| {
                            anyhow::anyhow!("Instrument not found: {}/{}", pair.0, pair.1)
                        })?;

                    // TODO:have more data in the event to avoid the SELECT here
                    log_error!(
                        sqlx::query(
                            "
                            INSERT INTO order_events (commit_id, order_id, identity, instrument_id, side, type, price, qty, qty_filled, status)
                            SELECT $1, order_id, identity, instrument_id, side, type, price, qty, qty_filled, status FROM orders WHERE order_id = $2
                            "
                        )
                        .bind(commit_id)
                        .bind(order_id.clone())
                        .execute(&mut *tx)
                        .instrument(tracing::info_span!("create_order_event"))
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
                            INSERT INTO trade_events (commit_id, maker_order_id, taker_order_id, instrument_id, price, qty, side, maker_identity, taker_identity)
                            SELECT $1, $2, $3, $4, maker_order.price, maker_order.qty, get_other_side(maker_order.side), maker_order.identity, $5
                            FROM maker_order
                            "
                        )
                        .bind(commit_id)
                        .bind(order_id)
                        .bind(taker_order_id)
                        .bind(instrument.instrument_id)
                        .bind(user)
                        .execute(&mut *tx)
                        .instrument(tracing::info_span!("insert_trade_event"))
                        .await,
                        "Failed to insert trade event"
                    )?;
                    self.ctx.metrics.record(
                        &self.ctx.metrics.order_execute_duration,
                        order_execute_start,
                        &[],
                    );
                    self.ctx.metrics.record(
                        &self.ctx.metrics.event_processing_duration,
                        event_start,
                        &[KeyValue::new("event_type", "order_executed")],
                    );
                }
                OrderbookEvent::OrderUpdate {
                    order_id,
                    taker_order_id,
                    remaining_quantity: _remaining_quantity,
                    executed_quantity,
                    pair,
                } => {
                    debug!(
                        "Updating order for user {} with order id {:?} and taker order id {:?} on pair {:?}",
                        user, order_id, taker_order_id, pair
                    );
                    let order_update_start = Instant::now();

                    let asset_service = self.ctx.asset_service.read().await;
                    let instrument = asset_service
                        .get_instrument(&format!("{}/{}", pair.0, pair.1))
                        .ok_or_else(|| {
                            anyhow::anyhow!("Instrument not found: {}/{}", pair.0, pair.1)
                        })?;

                    log_error!(
                        sqlx::query(
                            "
                            INSERT INTO order_events (commit_id, order_id, identity, instrument_id, side, type, price, qty, qty_filled, status)
                            SELECT $1, order_id, identity, instrument_id, side, type, price, qty, qty_filled, status FROM orders WHERE order_id = $2
                            "
                        )
                        .bind(commit_id)
                        .bind(order_id.clone())
                        .execute(&mut *tx)
                        .instrument(tracing::info_span!("create_order_event"))
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
                            INSERT INTO trade_events (commit_id, maker_order_id, taker_order_id, instrument_id, price, qty, side, maker_identity, taker_identity)
                            SELECT $1, $2, $3, $4, maker_order.price, $5, get_other_side(maker_order.side), maker_order.identity, $6
                            FROM maker_order
                            "
                        )
                        .bind(commit_id)
                        .bind(order_id.clone())
                        .bind(taker_order_id)
                        .bind(instrument.instrument_id)
                        .bind(executed_quantity as i64)
                        .bind(user)
                        .execute(&mut *tx)
                        .instrument(tracing::info_span!("insert_trade_event"))
                        .await,
                        "Failed to insert trade event"
                    )?;
                    self.ctx.metrics.record(
                        &self.ctx.metrics.order_update_duration,
                        order_update_start,
                        &[],
                    );
                    self.ctx.metrics.record(
                        &self.ctx.metrics.event_processing_duration,
                        event_start,
                        &[KeyValue::new("event_type", "order_update")],
                    );
                }
                OrderbookEvent::SessionKeyAdded {
                    user,
                    salt,
                    nonce,
                    session_keys,
                } => {
                    let user_ops_start = Instant::now();
                    let fetched_user_id = self.ctx.user_service.read().await.get_nonce(&user).await;

                    if let Err(e) = fetched_user_id {
                        if e.0 == StatusCode::NOT_FOUND {
                            info!("Creating user {}", user);
                            log_error!(
                                sqlx::query(
                                    "INSERT INTO users (commit_id, identity, salt, nonce) VALUES ($1, $2, $3, $4) ON CONFLICT (identity) DO UPDATE SET nonce = EXCLUDED.nonce"
                                )
                                .bind(commit_id)
                                .bind(user.clone())
                                .bind(salt)
                                .bind(nonce as i64)
                                .execute(&mut *tx)
                                .await,
                                "Failed to create user"
                            )?;
                        } else {
                            return Err(anyhow::anyhow!("{}", e.1));
                        }
                    }

                    debug!("Setting user session keys for user {}", user);

                    log_error!(
                        sqlx::query("INSERT INTO user_session_keys (commit_id, identity, session_keys) VALUES ($1, $2, $3)")
                        .bind(commit_id)
                        .bind(user)
                        .bind(session_keys)
                        .execute(&mut *tx)
                        .instrument(tracing::info_span!("create_user_session_key"))
                        .await,
                        "Failed to create user session key"
                    )?;
                    self.ctx.metrics.record(
                        &self.ctx.metrics.user_ops_duration,
                        user_ops_start,
                        &[KeyValue::new("operation", "session_key_added")],
                    );
                    self.ctx.metrics.record(
                        &self.ctx.metrics.event_processing_duration,
                        event_start,
                        &[KeyValue::new("event_type", "session_key_added")],
                    );
                }
                OrderbookEvent::NonceIncremented { user, nonce } => {
                    debug!("Incrementing nonce for user {}", user);
                    let user_ops_start = Instant::now();
                    log_error!(
                        sqlx::query("UPDATE users SET nonce = $1 WHERE identity = $2")
                            .bind(nonce as i64)
                            .bind(user.clone())
                            .execute(&mut *tx)
                            .instrument(tracing::info_span!("increment_nonce"))
                            .await,
                        "Failed to increment nonce"
                    )?;

                    log_error!(
                        sqlx::query("INSERT INTO user_events_nonces (commit_id, identity, nonce) VALUES ($1, $2, $3)")
                            .bind(commit_id)
                            .bind(user)
                            .bind(nonce as i64)
                            .execute(&mut *tx)
                            .instrument(tracing::info_span!("insert_user_event_nonce"))
                            .await,
                        "Failed to insert user event nonce"
                    )?;
                    self.ctx.metrics.record(
                        &self.ctx.metrics.user_ops_duration,
                        user_ops_start,
                        &[KeyValue::new("operation", "nonce_incremented")],
                    );
                    self.ctx.metrics.record(
                        &self.ctx.metrics.event_processing_duration,
                        event_start,
                        &[KeyValue::new("event_type", "nonce_incremented")],
                    );
                }
            }
        }

        let prover_insert_start = Instant::now();
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
            .instrument(tracing::info_span!("insert_prover_request"))
            .await,
            "Failed to insert prover request"
        )?;

        self.ctx.metrics.record(
            &self.ctx.metrics.prover_request_insert_duration,
            prover_insert_start,
            &[],
        );

        let contract_events_start = Instant::now();
        let events_data = log_error!(
            borsh::to_vec(&prover_request.events),
            "Failed to serialize events"
        )?;

        let user_info_data =
            log_error!(borsh::to_vec(&user_info), "Failed to serialize user info")?;

        log_error!(
            sqlx::query(
                "INSERT INTO contract_events (commit_id, user_info, events) VALUES ($1, $2, $3)"
            )
            .bind(commit_id)
            .bind(user_info_data)
            .bind(events_data)
            .execute(&mut *tx)
            .instrument(tracing::info_span!("insert_contract_events"))
            .await,
            "Failed to insert contract events"
        )?;
        self.ctx.metrics.record(
            &self.ctx.metrics.contract_events_insert_duration,
            contract_events_start,
            &[],
        );

        let commit_start = Instant::now();
        log_error!(
            tx.commit()
                .instrument(tracing::info_span!("commit_transaction"))
                .await,
            "Failed to commit transaction"
        )?;
        self.ctx.metrics.record(
            &self.ctx.metrics.transaction_commit_duration,
            commit_start,
            &[],
        );
        debug!("Committed transaction with commit id {}", commit_id);

        if reload_instrument_map {
            let notify_start = Instant::now();
            log_error!(
                sqlx::query("select pg_notify('instruments', 'instruments')")
                    .execute(&self.ctx.pool)
                    .instrument(tracing::info_span!("notify_instruments"))
                    .await,
                "Failed to notify 'instruments'"
            )?;
            self.ctx.metrics.record(
                &self.ctx.metrics.notification_duration,
                notify_start,
                &[KeyValue::new("channel", "instruments")],
            );
            let mut asset_service = self.ctx.asset_service.write().await;
            asset_service
                .reload_instrument_map()
                .instrument(tracing::info_span!("reload_instrument_map"))
                .await
                .map_err(|e| anyhow::anyhow!("{}", e.1))?;
        }

        // Record the total duration of write_events
        self.ctx.metrics.record(
            &self.ctx.metrics.write_events_duration,
            write_events_start,
            &[],
        );

        Ok(())
    }
}

#[derive(Default)]
pub struct DatabaseAggregator {
    executed_orders: HashSet<OrderId>,
    cancelled_orders: HashSet<OrderId>,
    updated_orders: HashMap<OrderId, u64>,
    trigger_notify_trades: bool,
    trigger_notify_orders: bool,
    symbol_book_updated: HashSet<String>,
    pub updated_balances: HashMap<(String, i64), u64>,
}

impl DatabaseAggregator {
    pub fn create_order(&mut self, symbol: String) {
        self.trigger_notify_orders = true;
        self.symbol_book_updated.insert(symbol);
    }

    pub fn cancel_order(&mut self, order_id: OrderId, symbol: String) {
        self.cancelled_orders.insert(order_id);
        self.trigger_notify_orders = true;
        self.symbol_book_updated.insert(symbol);
    }

    pub fn execute_order(&mut self, order_id: OrderId, symbol: String) {
        self.updated_orders.remove(&order_id);
        self.executed_orders.insert(order_id);
        self.trigger_notify_trades = true;
        self.trigger_notify_orders = true;
        self.symbol_book_updated.insert(symbol);
    }
    pub fn update_order(&mut self, order_id: OrderId, remaining_quantity: u64, symbol: String) {
        self.updated_orders.insert(order_id, remaining_quantity);
        self.trigger_notify_trades = true;
        self.trigger_notify_orders = true;
        self.symbol_book_updated.insert(symbol);
    }
    pub fn update_balance(&mut self, user: String, asset_id: i64, amount: u64) {
        self.updated_balances.insert((user, asset_id), amount);
    }

    #[cfg_attr(
        feature = "instrumentation",
        tracing::instrument(skip(self, pool, metrics))
    )]
    pub async fn dump_to_db(&mut self, pool: &PgPool, metrics: &DatabaseMetrics) -> Result<()> {
        if self.symbol_book_updated.is_empty()
            && self.updated_orders.is_empty()
            && self.executed_orders.is_empty()
            && self.cancelled_orders.is_empty()
            && self.updated_balances.is_empty()
        {
            return Ok(());
        }

        info!("Dumping database aggregator to db with {} orders, {} trades, {} cancelled orders, {} symbol book updated, {} balances updated", self.updated_orders.len(), self.executed_orders.len(), self.cancelled_orders.len(), self.symbol_book_updated.len(), self.updated_balances.len());
        let mut tx = pool.begin().await?;
        for order_id in self.executed_orders.drain() {
            log_error!(
                sqlx::query(
                    "UPDATE orders SET status = 'filled', qty_filled = qty WHERE order_id = $1"
                )
                .bind(order_id.clone())
                .execute(&mut *tx)
                .instrument(tracing::info_span!("update_order_as_filled"))
                .await,
                "Failed to update order as filled"
            )?;
        }

        for order_id in self.cancelled_orders.drain() {
            log_error!(
                sqlx::query("UPDATE orders SET status = 'cancelled' WHERE order_id = $1")
                    .bind(order_id.clone())
                    .execute(&mut *tx)
                    .instrument(tracing::info_span!("update_order_as_cancelled"))
                    .await,
                "Failed to update order as cancelled"
            )?;
        }

        for (order_id, remaining_quantity) in self.updated_orders.drain() {
            log_error!(
                sqlx::query(
                    "
                UPDATE orders SET status = 'partially_filled', qty_filled = qty - $1 WHERE order_id = $2
                ",
                )
                .bind(remaining_quantity as i64)
                .bind(order_id.clone())
                .execute(&mut *tx)
                .instrument(tracing::info_span!("update_order_as_partially_filled"))
                .await,
                "Failed to update order as partially filled"
            )?;
        }
        for ((user, asset_id), amount) in self.updated_balances.drain() {
            log_error!(
                sqlx::query(
                    "INSERT INTO balances (identity, asset_id, total) VALUES ($1, $2, $3) ON CONFLICT (identity, asset_id) DO UPDATE SET total = $3"
                )
                .bind(user)
                .bind(asset_id)
                .bind(amount as i64)
                .execute(&mut *tx)
                .instrument(tracing::info_span!("update_balance"))
                .await,
                "Failed to update balance"
            )?;
        }

        tx.commit().await?;

        // Send notifications after committing the transaction
        if self.trigger_notify_trades {
            debug!("Notifying trades");
            let notify_start = Instant::now();
            log_error!(
                sqlx::query("select pg_notify('trades', 'trades')")
                    .execute(pool)
                    .instrument(tracing::info_span!("notify_trades"))
                    .await,
                "Failed to notify 'trades'"
            )?;
            metrics.record(
                &metrics.notification_duration,
                notify_start,
                &[KeyValue::new("channel", "trades")],
            );
        }

        if self.trigger_notify_orders {
            debug!("Notifying orders");
            let notify_start = Instant::now();
            log_error!(
                sqlx::query("select pg_notify('orders', 'orders')")
                    .execute(pool)
                    .instrument(tracing::info_span!("notify_orders"))
                    .await,
                "Failed to notify 'orders'"
            )?;
            metrics.record(
                &metrics.notification_duration,
                notify_start,
                &[KeyValue::new("channel", "orders")],
            );
        }

        for symbol in self.symbol_book_updated.drain() {
            debug!("Notifying book for symbol {}", symbol);
            let notify_start = Instant::now();
            log_error!(
                sqlx::query("select pg_notify('book', $1)")
                    .bind(symbol)
                    .execute(pool)
                    .instrument(tracing::info_span!("notify_book"))
                    .await,
                "Failed to notify 'book'"
            )?;
            metrics.record(
                &metrics.notification_duration,
                notify_start,
                &[KeyValue::new("channel", "book")],
            );
        }

        // Reset flags after sending notifications
        self.trigger_notify_trades = false;
        self.trigger_notify_orders = false;

        Ok(())
    }
}

pub struct DatabaseModule {
    ctx: Arc<DatabaseModuleCtx>,
    bus: DatabaseModuleBusClient,
    worker_txs: Vec<mpsc::UnboundedSender<DatabaseRequest>>,
    next_worker: std::sync::atomic::AtomicUsize,
    aggregator: DatabaseAggregator,
    pending_blob_txs: VecDeque<BlobTransaction>,
}

const MAX_PENDING_BLOB_TXS: usize = 10_000;

impl Module for DatabaseModule {
    type Context = Arc<DatabaseModuleCtx>;

    async fn build(bus: SharedMessageBus, ctx: Self::Context) -> Result<Self> {
        let bus = DatabaseModuleBusClient::new_from_bus(bus.new_handle()).await;
        let mut worker_txs = Vec::new();
        let mut worker_rxs = Vec::new();

        // Create 15 worker channels
        let worker_count = 35;
        for _ in 0..worker_count {
            let (tx, rx) = mpsc::unbounded_channel();
            worker_txs.push(tx);
            worker_rxs.push(rx);
        }

        // Set the worker count metric
        ctx.metrics.worker_count.add(worker_count as i64, &[]);

        // Spawn worker tasks
        for (worker_id, mut rx) in worker_rxs.into_iter().enumerate() {
            let ctx = ctx.clone();
            tokio::spawn(async move {
                while let Some(request) = rx.recv().await {
                    // Decrement queue depth when worker starts processing
                    ctx.metrics.worker_queue_depth.add(-1, &[]);

                    let service = DatabaseService::new(ctx.clone());
                    let result = match request {
                        DatabaseRequest::WriteEvents {
                            user,
                            tx_hash,
                            blob_tx,
                            prover_request,
                            context,
                        } => {
                            service
                                .write_events(
                                    user.clone(),
                                    tx_hash.clone(),
                                    blob_tx.clone(),
                                    prover_request.clone(),
                                    context,
                                )
                                .await
                        }
                    };
                    if let Err(e) = result {
                        tracing::error!(
                            "Worker {} failed to process database request: {}",
                            worker_id,
                            e
                        );
                    }
                }
            });
        }

        Ok(DatabaseModule {
            ctx,
            bus,
            worker_txs,
            next_worker: AtomicUsize::new(0),
            aggregator: DatabaseAggregator::default(),
            pending_blob_txs: VecDeque::new(),
        })
    }

    async fn run(&mut self) -> Result<()> {
        self.start().await?;
        Ok(())
    }
}

impl DatabaseModule {
    pub async fn start(&mut self) -> Result<()> {
        // Handle incoming messages and dispatch to workers

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        module_handle_messages! {
            on_self self,
            listen<DatabaseRequest> cmd => {
                _ = log_error!(self.dispatch_database_request(&cmd).await, "dispatch database request");
                log_error!(self.handle_database_request(cmd).await, "handle database request")?;
            }
             _ = interval.tick() => {
                _ = log_error!(self.aggregator.dump_to_db(&self.ctx.pool, &self.ctx.metrics).await, "dump database aggregator to db");
                if !self.ctx.no_blobs {
                    log_error!(self.flush_blob_queue().await, "flush blob queue from tick")?;
                }
            }
        };
        Ok(())
    }

    async fn dispatch_database_request(&mut self, request: &DatabaseRequest) -> Result<()> {
        // Round-robin distribution to workers
        let worker_index = self
            .next_worker
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.worker_txs.len();
        self.worker_txs[worker_index].send(request.clone())?;
        // Increment queue depth when dispatching a request
        self.ctx.metrics.worker_queue_depth.add(1, &[]);
        Ok(())
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self, request)))]
    async fn handle_database_request(&mut self, request: DatabaseRequest) -> Result<()> {
        match request {
            DatabaseRequest::WriteEvents {
                prover_request,
                blob_tx,
                context,
                ..
            } => {
                tracing::Span::current().set_parent(context);
                for event in prover_request.events {
                    match event {
                        OrderbookEvent::OrderCreated { order } => {
                            let symbol = format!("{}/{}", order.pair.0, order.pair.1);
                            self.aggregator.create_order(symbol);
                        }
                        OrderbookEvent::OrderCancelled { order_id, pair, .. } => {
                            let symbol = format!("{}/{}", pair.0, pair.1);
                            self.aggregator.cancel_order(order_id, symbol);
                        }
                        OrderbookEvent::OrderExecuted { order_id, pair, .. } => {
                            let symbol = format!("{}/{}", pair.0, pair.1);
                            self.aggregator.execute_order(order_id, symbol);
                        }
                        OrderbookEvent::OrderUpdate {
                            order_id,
                            remaining_quantity,
                            pair,
                            ..
                        } => {
                            let symbol = format!("{}/{}", pair.0, pair.1);
                            self.aggregator
                                .update_order(order_id, remaining_quantity, symbol);
                        }
                        OrderbookEvent::BalanceUpdated {
                            user,
                            symbol,
                            amount,
                        } => {
                            let asset_service = self.ctx.asset_service.read().await;
                            let asset = asset_service
                                .get_asset(&symbol)
                                .ok_or_else(|| anyhow::anyhow!("Asset not found: {symbol}"))?;
                            self.aggregator.update_balance(user, asset.asset_id, amount);
                        }
                        _ => {}
                    }
                }

                // TODO: sending blob tx should be done only if the write_events succeeded
                // and blob tx must be sent in the same order as the DatabaseRequest::WriteEvents
                // arrived.
                // But a broader question exists: how do we handle a failure in write_events ?
                // If we don't send the blob tx, we might write blob txs that refer to non-existing
                // orders or do balance updates that should not happen...
                // By sending the blob tx here, we guarantee the order, but we might send blob txs
                // that can't be proved if the database write fails.
                if !self.ctx.no_blobs {
                    if self.pending_blob_txs.len() >= MAX_PENDING_BLOB_TXS {
                        // TODO: handle this more gracefully: serializing + resending later
                        panic!(
                            "Pending blob transaction queue is full ({}). Time to die.",
                            MAX_PENDING_BLOB_TXS
                        );
                    }

                    self.pending_blob_txs.push_back(blob_tx);
                    log_error!(
                        self.flush_blob_queue().await,
                        "flush blob queue from request"
                    )?;
                }
            }
        }
        Ok(())
    }

    async fn flush_blob_queue(&mut self) -> Result<()> {
        while let Some(blob_tx) = self.pending_blob_txs.front() {
            let blob_send_start = Instant::now();
            let send_res = log_error!(
                self.ctx.client.send_tx_blob(blob_tx.clone()).await,
                "Failed to send blob tx"
            );

            self.ctx
                .metrics
                .record(&self.ctx.metrics.blob_send_duration, blob_send_start, &[]);

            if let Err(e) = send_res {
                tracing::warn!(
                    "Failed to send blob transaction ({} pending, will retry): {:#}",
                    self.pending_blob_txs.len(),
                    e
                );
                return Err(e);
            }

            self.pending_blob_txs.pop_front();
        }

        Ok(())
    }
}
