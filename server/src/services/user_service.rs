use std::collections::HashMap;

use anyhow::Context;
use client_sdk::contract_indexer::AppError;
use orderbook::model::UserInfo;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::{PgConnection, PgPool, Row};
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

    pub async fn get_user_info(&self, user: &str) -> Result<UserInfo, AppError> {
        let row = sqlx::query(
            "
            SELECT 
                u.identity, 
                u.salt, 
                u.nonce, 
                (SELECT session_keys 
                 FROM user_session_keys 
                 WHERE identity = u.identity 
                 ORDER BY commit_id DESC 
                 LIMIT 1) as session_keys
            FROM users u
            WHERE u.identity = $1
            ",
        )
        .bind(user)
        .fetch_one(&self.pool)
        .await
        .map_err(|_e| {
            AppError(
                StatusCode::NOT_FOUND,
                anyhow::anyhow!("User not found: {user}"),
            )
        })?;

        Ok(UserInfo {
            user: row.get("identity"),
            salt: row.get("salt"),
            nonce: row.get::<i64, _>("nonce") as u32,
            session_keys: row
                .get::<Option<Vec<Vec<u8>>>, _>("session_keys")
                .unwrap_or_default(),
        })
    }

    pub async fn get_balances(&self, user: &str) -> Result<UserBalances, AppError> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "
        SELECT 
            assets.symbol, balances.total, balances.reserved, balances.available 
        FROM 
            balances
        JOIN 
            assets ON balances.asset_id = assets.asset_id
        WHERE 
            balances.identity = $1
        ;
        ",
        )
        .bind(user)
        .fetch_all(&mut *tx)
        .await?;

        tx.commit().await?;

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

    pub async fn get_balances_from_commit_id(
        &self,
        user: &str,
        commit_id: i64,
    ) -> Result<UserBalances, AppError> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "
            SELECT 
                assets.symbol, be.total, be.reserved, be.total - be.reserved as available
            FROM 
                balance_events as be
            JOIN 
                assets ON be.asset_id = assets.asset_id
            WHERE 
                be.identity = $1
                AND be.commit_id = 
                    (SELECT MAX(commit_id) FROM balance_events 
                        WHERE 
                            identity = be.identity 
                            AND asset_id = be.asset_id
                            AND commit_id <= $2
                    )
            ;
        ",
        )
        .bind(user)
        .bind(commit_id)
        .fetch_all(&mut *tx)
        .await?;

        tx.commit().await?;

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
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT nonce FROM users WHERE identity = $1")
            .bind(user)
            .fetch_optional(&mut *tx)
            .await
            .context("Failed to get nonce")?;

        tx.commit().await?;

        row.map(|row| row.get::<i64, _>("nonce") as u32)
            .ok_or_else(|| {
                AppError(
                    StatusCode::NOT_FOUND,
                    anyhow::anyhow!("User not found: {user}"),
                )
            })
    }

    /// Get all users from the database for a given commit_id
    pub async fn get_all_users(&self, commit_id: i64) -> HashMap<String, UserInfo> {
        // Fetch all users from the database and store them in the user_id_map
        debug!("Fetching all users from the database");
        // TODO this query might need to be optimized
        let rows = sqlx::query(
            "
            SELECT u.identity, u.salt, uen.nonce, 
                   usk.session_keys as session_keys
            FROM users u
            LEFT JOIN user_session_keys usk ON u.identity = usk.identity
            LEFT JOIN user_events_nonces uen ON u.identity = uen.identity
            WHERE 
                usk.commit_id = 
                    (SELECT MAX(commit_id) FROM user_session_keys 
                        WHERE identity = u.identity
                        AND commit_id <= $1
                    )
                AND uen.commit_id = 
                    (SELECT MAX(commit_id) FROM user_events_nonces 
                        WHERE identity = u.identity
                        AND commit_id <= $1
                    )
        ",
        )
        .bind(commit_id)
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
