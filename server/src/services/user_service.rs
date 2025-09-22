use client_sdk::contract_indexer::AppError;
use reqwest::StatusCode;
use serde::Serialize;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::RwLock;
use tracing::info;

pub struct UserService {
    pool: PgPool,
    user_id_map: RwLock<HashMap<String, i64>>,
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
    pub async fn new(pool: PgPool) -> Self {
        info!("Loading users into memory");
        // Fetch all users from the database and store them in the user_id_map
        let rows = sqlx::query("SELECT identity, user_id FROM users")
            .fetch_all(&pool)
            .await
            .unwrap();

        let user_id_map = rows
            .iter()
            .map(|row| (row.get("identity"), row.get("user_id")))
            .collect();

        UserService {
            pool,
            user_id_map: RwLock::new(user_id_map),
        }
    }

    /// Return the user_id for a given identity
    /// Store it in memory for faster access
    pub async fn get_user_id(&self, user: &str) -> Result<i64, AppError> {
        if let Some(user_id) = self.user_id_map.read().unwrap().get(user) {
            return Ok(*user_id);
        }

        let row = sqlx::query("SELECT user_id FROM users WHERE identity = $1")
            .bind(user)
            .fetch_one(&self.pool)
            .await
            .map_err(|_e| {
                AppError(
                    StatusCode::NOT_FOUND,
                    anyhow::anyhow!("User not found: {user}"),
                )
            })?;

        let user_id = row.get("user_id");
        self.user_id_map
            .write()
            .unwrap()
            .insert(user.to_string(), user_id);

        Ok(user_id)
    }

    pub async fn get_balances(&self, user: &str) -> Result<UserBalances, AppError> {
        let user_id = self.get_user_id(user).await?;
        let rows = sqlx::query(
            "
        SELECT 
            assets.symbol, balances.total, balances.reserved, balances.available 
        FROM 
            balances
        JOIN 
            assets ON balances.asset_id = assets.asset_id
        WHERE 
            balances.user_id = $1
        ;
        ",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        let balances = rows
            .iter()
            .map(|row| Balance {
                token: row.get("symbol"),
                total: row.get("total"),
                reserved: row.get("reserved"),
                available: row.get("available"),
            })
            .collect();

        Ok(UserBalances { balances })
    }
}
