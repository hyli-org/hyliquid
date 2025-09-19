use client_sdk::contract_indexer::AppError;
use orderbook::orderbook::OrderbookEvent;
use serde::Serialize;
use sqlx::{PgPool, Row};

pub struct BookWriterService {
    pool: PgPool,
}

impl BookWriterService {
    pub fn new(pool: PgPool) -> Self {
        BookWriterService { pool }
    }

    pub async fn write_events(&self, _events: Vec<OrderbookEvent>) -> Result<(), AppError> {
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

        // let rows = sqlx::query("SELECT * FROM get_orderbook_grouped_by_ticks($1, $2, $3);")
        //     .bind(symbol)
        //     .bind(levels)
        //     .bind(group_ticks)
        //     .fetch_all(&self.pool)
        //     .await?;

        let mut bids = Vec::new();
        let mut asks = Vec::new();

        // for row in rows {
        //     let side: OrderSide = row.get("side");
        //     let price: i64 = row.get("price");
        //     let qty: i64 = row.get("qty");

        //     let entry = OrderbookAPIEntry {
        //         price: price as u32,
        //         quantity: qty as u32,
        //     };

        //     match side {
        //         OrderSide::Bid => {
        //             bids.push(entry);
        //         }
        //         OrderSide::Ask => {
        //             asks.push(entry);
        //         }
        //     }
        // }

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
