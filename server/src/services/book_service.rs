use std::collections::{BTreeMap, VecDeque};

use client_sdk::contract_indexer::AppError;
use orderbook::model::{Order, OrderSide, UserInfo};
use orderbook::order_manager::OrderManager;
use orderbook::zk::smt::GetKey;
use serde::Serialize;
use sqlx::{PgPool, Row};

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
        users_info: &BTreeMap<String, UserInfo>,
        commit_id: i64,
    ) -> Result<OrderManager, AppError> {
        let rows = sqlx::query(
            "
        WITH last_commit AS (
        SELECT order_id, MAX(commit_id) AS commit_id
        FROM order_events
        WHERE commit_id <= $1
        GROUP BY order_id
        )
        SELECT 
            o.order_id,
            o.type,
            o.side,
            o.price,
            o.qty - o.qty_filled AS qty_remaining,
            u.identity,
            base_asset.symbol AS base_asset_symbol,
            quote_asset.symbol AS quote_asset_symbol
        FROM last_commit lc
        JOIN order_events o
        ON o.order_id = lc.order_id AND o.commit_id = lc.commit_id
        JOIN instruments i       ON o.instrument_id = i.instrument_id
        JOIN assets base_asset   ON i.base_asset_id = base_asset.asset_id
        JOIN assets quote_asset  ON i.quote_asset_id = quote_asset.asset_id
        JOIN users u             ON o.user_id = u.user_id
        WHERE o.status IN ('open','partially_filled')
        ORDER BY o.event_time asc
        ",
        )
        .bind(commit_id)
        .fetch_all(&self.pool)
        .await?;

        let orders: BTreeMap<String, (Order, String)> = rows
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

        let buy_orders: BTreeMap<(String, String), BTreeMap<u64, VecDeque<String>>> = rows
            .iter()
            .rev()
            .filter(|row| row.get::<OrderSide, _>("side") == OrderSide::Bid)
            .fold(BTreeMap::new(), |mut acc, row| {
                acc.entry((row.get("base_asset_symbol"), row.get("quote_asset_symbol")))
                    .and_modify(|v| {
                        v.entry(row.get::<i64, _>("price") as u64)
                            .and_modify(|v| {
                                v.push_front(row.get("order_id"));
                            })
                            .or_insert(VecDeque::from([row.get("order_id")]));
                    })
                    .or_insert(BTreeMap::from([(
                        row.get::<i64, _>("price") as u64,
                        VecDeque::from([row.get("order_id")]),
                    )]));
                acc
            });

        let sell_orders: BTreeMap<(String, String), BTreeMap<u64, VecDeque<String>>> = rows
            .iter()
            .filter(|row| row.get::<OrderSide, _>("side") == OrderSide::Ask)
            .fold(BTreeMap::new(), |mut acc, row| {
                acc.entry((row.get("base_asset_symbol"), row.get("quote_asset_symbol")))
                    .and_modify(|v| {
                        v.entry(row.get::<i64, _>("price") as u64)
                            .and_modify(|v| {
                                v.push_back(row.get("order_id"));
                            })
                            .or_insert(VecDeque::from([row.get("order_id")]));
                    })
                    .or_insert(BTreeMap::from([(
                        row.get::<i64, _>("price") as u64,
                        VecDeque::from([row.get("order_id")]),
                    )]));
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
            bid_orders: buy_orders,
            ask_orders: sell_orders,
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
