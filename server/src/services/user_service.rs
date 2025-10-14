use client_sdk::contract_indexer::AppError;
use orderbook::model::UserInfo;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use tracing::debug;

pub struct UserService {
    pool: PgPool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Balance {
    pub symbol: String,
    pub total: i64,
    pub reserved: i64,
    pub available: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserBalances {
    pub balances: Vec<Balance>,
}

impl UserService {
    pub async fn new(pool: PgPool) -> Self {
        UserService { pool }
    }

    /// Return the user_id for a given identity
    /// Store it in memory for faster access
    pub async fn get_user_id(&self, user: &str) -> Result<i64, AppError> {
        let row = sqlx::query("SELECT user_id, salt, nonce FROM users WHERE identity = $1")
            .bind(user)
            .fetch_one(&self.pool)
            .await
            .map_err(|_e| {
                AppError(
                    StatusCode::NOT_FOUND,
                    anyhow::anyhow!("User not found: {user}"),
                )
            })?;

        let user_id = row.get::<i64, _>("user_id");

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
                symbol: row.get("symbol"),
                total: row.get("total"),
                reserved: row.get("reserved"),
                available: row.get("available"),
            })
            .collect();

        Ok(UserBalances { balances })
    }

    pub async fn get_nonce(&self, user: &str) -> Result<u32, AppError> {
        let user_id = self.get_user_id(user).await?;
        let row = sqlx::query("SELECT nonce FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&self.pool)
            .await?;

        Ok(row.get::<i64, _>("nonce") as u32)
    }

    pub async fn get_all_users(&self) -> HashMap<String, UserInfo> {
        // Fetch all users from the database and store them in the user_id_map
        debug!("Fetching all users from the database");
        // TODO this query might need to be optimized
        let rows = sqlx::query(
            "
            SELECT u.identity, u.user_id, u.salt, u.nonce, 
                   usk.session_keys as session_keys
            FROM users u
            LEFT JOIN user_session_keys usk ON u.user_id = usk.user_id
            WHERE usk.commit_id = (SELECT MAX(commit_id) FROM user_session_keys WHERE user_id = u.user_id)
        ",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap();

        let users_map = rows
            .iter()
            .map(|row| {
                (
                    row.get("identity"),
                    UserInfo {
                        user: row.get("identity"),
                        salt: row.get("salt"),
                        nonce: row.get::<i64, _>("nonce") as u32,
                        session_keys: row.get("session_keys"),
                    },
                )
            })
            .collect();

        users_map
    }
}
