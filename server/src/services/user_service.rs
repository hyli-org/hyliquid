use client_sdk::contract_indexer::AppError;
use serde::Serialize;
use sqlx::{PgPool, Row};

pub struct UserService {
    pool: PgPool,
}

#[derive(Debug, Serialize)]
pub struct Balance {
    pub token: String,
    pub total: i64,
    pub reserved: i64,
    pub available: i64,
}

#[derive(Debug, Serialize)]
pub struct UserBalances {
    pub balances: Vec<Balance>,
}

impl UserService {
    pub fn new(pool: PgPool) -> Self {
        UserService { pool }
    }

    pub async fn get_balances(&self, user_id: &str) -> Result<UserBalances, AppError> {
        let rows = sqlx::query("
        SELECT 
            assets.symbol, balances.total, balances.reserved, balances.available 
        FROM 
            balances
        JOIN 
            users ON balances.user_id = users.user_id
        WHERE 
            users.identity = $1
        JOIN 
            assets ON balances.asset_id = assets.asset_id
        ;
        ")
            .bind(user_id)
            .fetch_all(&self.pool)
            .await?;

        let balances = rows.iter().map(|row| Balance {
            token: row.get("symbol"),
            total: row.get("total"),
            reserved: row.get("reserved"),
            available: row.get("available"),
        }).collect();

        Ok(UserBalances { balances })
    }
}
