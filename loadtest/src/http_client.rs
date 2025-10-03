use anyhow::{Context, Result};
use goose::prelude::*;
use orderbook::orderbook::{Order, OrderSide, OrderType};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::auth::UserAuth;
use crate::config::Config;
use server::services::user_service::UserBalances;

/// HTTP client wrapper with proper headers and auth
pub struct OrderbookClient {
    client: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrderbookResponse {
    pub bids: Vec<OrderbookEntry>,
    pub asks: Vec<OrderbookEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrderbookEntry {
    pub price: u32,
    pub quantity: u32,
}

impl OrderbookResponse {
    pub fn best_bid(&self) -> Option<&OrderbookEntry> {
        self.bids.first()
    }

    pub fn best_ask(&self) -> Option<&OrderbookEntry> {
        self.asks.last()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreatePairRequest {
    pub pair: (String, String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DepositRequest {
    pub token: String,
    pub amount: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CancelOrderRequest {
    pub order_id: String,
}

impl OrderbookClient {
    pub fn new(config: &Config) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.http.timeout_ms))
            .connect_timeout(Duration::from_millis(config.http.connect_timeout_ms))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(OrderbookClient {
            client,
            base_url: config.server.base_url.clone(),
        })
    }

    /// Get balance for a user

    pub async fn get_balances(
        &self,
        user: &mut GooseUser,
        auth: &UserAuth,
    ) -> Result<UserBalances, Box<TransactionError>> {
        let path = "/api/balances".to_string();

        let builder = user
            .get_request_builder(&GooseMethod::Get, path.as_str())?
            .header("x-identity", &auth.identity)
            .body("");

        let request = GooseRequest::builder().set_request_builder(builder).build();

        let response = user.request(request).await?;

        let balance = response.response?.json::<UserBalances>().await?;

        Ok(balance)
    }

    /// Get nonce for a user
    pub async fn get_nonce(&self, auth: &UserAuth) -> Result<u32> {
        let url = format!("{}/api/nonce", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("x-identity", &auth.identity)
            .send()
            .await
            .context("Failed to send get_nonce request")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("get_nonce failed with status {}: {}", status, error_text);
        }

        let nonce = response.json::<u32>().await?;

        Ok(nonce)
    }

    /// Add session key for authentication
    pub async fn add_session_key(
        &self,
        user: &mut GooseUser,
        auth: &UserAuth,
    ) -> TransactionResult {
        let path = "/add_session_key";

        // Build custom request with headers
        let builder = user
            .get_request_builder(&GooseMethod::Post, path)?
            .header("x-identity", &auth.identity)
            .header("x-public-key", &auth.public_key_hex)
            .body("");

        let request = GooseRequest::builder().set_request_builder(builder).build();

        let _response = user.request(request).await?;

        Ok(())
    }

    /// Deposit tokens
    pub async fn deposit(
        &self,
        user: &mut GooseUser,
        auth: &UserAuth,
        token: &str,
        amount: u64,
    ) -> TransactionResult {
        let path = "/deposit";

        let request_body = DepositRequest {
            token: token.to_string(),
            amount,
        };

        let body = serde_json::to_vec(&request_body).unwrap();

        // Build custom request with headers
        let builder = user
            .get_request_builder(&GooseMethod::Post, path)?
            .header("x-identity", &auth.identity)
            .header("Content-Type", "application/json")
            .body(body);

        let request = GooseRequest::builder().set_request_builder(builder).build();

        let _response = user.request(request).await?;

        Ok(())
    }

    /// Create a new order
    pub async fn create_order(
        &self,
        user: &mut GooseUser,
        auth: &UserAuth,
        order: &Order,
        signature: &str,
    ) -> TransactionResult {
        let path = "/create_order";

        let body = serde_json::to_vec(&order).unwrap();

        // Build custom request with headers
        let builder = user
            .get_request_builder(&GooseMethod::Post, path)?
            .header("x-identity", &auth.identity)
            .header("x-public-key", &auth.public_key_hex)
            .header("x-signature", signature)
            .header("Content-Type", "application/json")
            .body(body);

        let request = GooseRequest::builder().set_request_builder(builder).build();

        let _response = user.request(request).await?;

        Ok(())
    }

    /// Cancel an order
    pub async fn cancel_order(
        &self,
        user: &mut GooseUser,
        auth: &UserAuth,
        order_id: &str,
        signature: &str,
    ) -> TransactionResult {
        let path = "/cancel_order";

        let request_body = CancelOrderRequest {
            order_id: order_id.to_string(),
        };

        let body = serde_json::to_vec(&request_body).unwrap();

        // Build custom request with headers
        let builder = user
            .get_request_builder(&GooseMethod::Post, path)?
            .header("x-identity", &auth.identity)
            .header("x-public-key", &auth.public_key_hex)
            .header("x-signature", signature)
            .header("Content-Type", "application/json")
            .body(body);

        let request = GooseRequest::builder().set_request_builder(builder).build();

        let _response = user.request(request).await?;

        Ok(())
    }

    /// Get orderbook (for reading best bid/ask)
    pub async fn get_orderbook(
        &self,
        user: &mut GooseUser,
        base_asset: &str,
        quote_asset: &str,
        levels: u32,
    ) -> Result<OrderbookResponse, Box<TransactionError>> {
        let path = format!(
            "/api/book/{}/{}?levels={}&group_ticks=1",
            base_asset, quote_asset, levels
        );

        let goose_request = GooseRequest::builder()
            .path(path.as_str())
            .method(GooseMethod::Get)
            .name("get_orderbook")
            .build();

        let goose_response = user.request(goose_request).await?;

        let orderbook: OrderbookResponse = goose_response
            .response?
            .json()
            .await
            .map_err(|e| Box::new(TransactionError::Reqwest(e)))?;

        Ok(orderbook)
    }

    /// Create a pair (typically done once at setup)
    pub async fn create_pair(
        &self,
        user: &mut GooseUser,
        auth: &UserAuth,
        pair: (String, String),
    ) -> TransactionResult {
        let path = "/create_pair";

        let request_body = CreatePairRequest { pair };

        let body = serde_json::to_vec(&request_body).unwrap();

        // Build custom request with headers
        let builder = user
            .get_request_builder(&GooseMethod::Post, path)?
            .header("x-identity", &auth.identity)
            .header("Content-Type", "application/json")
            .body(body);

        let request = GooseRequest::builder().set_request_builder(builder).build();

        // Pair might already exist, which is okay - don't fail on error
        let _ = user.request(request).await;

        Ok(())
    }
}

/// Helper to create an Order struct
pub fn build_order(
    order_id: String,
    side: OrderSide,
    order_type: OrderType,
    price: Option<u64>,
    pair: (String, String),
    quantity: u64,
) -> Order {
    Order {
        order_id,
        order_side: side,
        order_type,
        price,
        pair,
        quantity,
    }
}
