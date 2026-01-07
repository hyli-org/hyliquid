use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hyli_modules::utils::logger::setup_tracing;
use k256::{
    ecdsa::{signature::DigestSigner, Signature, SigningKey},
    SecretKey,
};
use orderbook::model::{Order, OrderSide, OrderType};
use rand::Rng;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use server::{
    app::{CancelOrderRequest, CreatePairRequest, DepositRequest},
    conf::Conf,
    services::user_service::UserBalances,
};
use sha3::{Digest, Sha3_256};
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(version, about = "Send transactions to a server", long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "http://localhost:9002")]
    pub server_url: String,

    #[arg(long, default_value = "http://localhost:3000")]
    pub api_url: String,

    #[arg(long, default_value = "tx_sender")]
    pub identity: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a new pair
    CreatePair {
        #[arg(long)]
        contract_name1: String,
        #[arg(long)]
        contract_name2: String,
    },
    /// Create a new order
    CreateOrder {
        #[arg(long)]
        order_id: String,
        #[arg(long)]
        order_side: String,
        #[arg(long)]
        order_type: String,
        #[arg(long)]
        price: Option<u64>,
        #[arg(long)]
        asset_symbol1: String,
        #[arg(long)]
        asset_symbol2: String,
        #[arg(long)]
        quantity: u64,
    },
    /// Add a session key for user authentication
    AddSessionKey,
    /// Deposit
    Deposit {
        #[arg(long)]
        symbol: String,
        #[arg(long)]
        amount: u64,
    },
    // /// Cancel an existing order
    Cancel {
        #[arg(long)]
        order_id: String,
    },
    // /// Withdraw
    Withdraw {
        #[arg(long)]
        symbol: String,
        #[arg(long)]
        amount: u64,
    },
    /// Get identity balance
    GetBalances {},
    /// Simulate order creation for a given pair
    Simulate {
        #[arg(long)]
        asset_symbol1: String,
        #[arg(long)]
        asset_symbol2: String,
        #[arg(long)]
        middle_price: u64,
        #[arg(long, default_value = "100")]
        price_offset: u64,
        #[arg(long, default_value = "1")]
        interval_seconds: u64,
        #[arg(long, default_value = "100")]
        quantity: u64,
        #[arg(long, default_value = "10")]
        max_orders: u32,
        #[arg(long, default_value = "up")]
        trend: String,
        #[arg(long, default_value = "false")]
        fast: bool,
    },
}

// Helper function to create a signature for the given data
fn create_signature(signing_key: &SigningKey, data: &str) -> Result<String> {
    let mut hasher = Sha3_256::new();
    hasher.update(data.as_bytes());

    let signature: Signature = signing_key.sign_digest(hasher);
    Ok(hex::encode(signature.to_bytes()))
}

async fn get_nonce(client: &Client, server_url: &str, identity: &str) -> Result<u32> {
    let response = client
        .get(format!("{}/nonce", server_url))
        .header("x-identity", identity)
        .send()
        .await
        .context("Failed to send request to server")?;
    if response.status().is_success() {
        let nonce_str = response.text().await?;
        Ok(nonce_str.trim().parse::<u32>().unwrap_or_default())
    } else {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        anyhow::bail!("Server returned error {status}: {error_text}");
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Conf::new(args.config_file).context("reading config file")?;

    setup_tracing(&config.log_format, "tx_sender".to_string()).context("setting up tracing")?;

    // Generate the key pair once for all operations
    let mut hasher = Sha3_256::new();
    hasher.update(args.identity.as_bytes());
    let derived_key = hasher.finalize();
    let private_key_bytes = derived_key.to_vec();

    let secret_key = SecretKey::from_slice(&private_key_bytes).context("Invalid private key")?;
    let signing_key = SigningKey::from(secret_key);
    let public_key = signing_key.verifying_key();
    let public_key_bytes = public_key.to_encoded_point(false).as_bytes().to_vec();
    let public_key_hex = hex::encode(public_key_bytes);

    let client = Client::new();

    let nonce = get_nonce(&client, &args.server_url, &args.identity).await?;

    match args.command {
        Commands::CreatePair {
            contract_name1,
            contract_name2,
        } => {
            let request = CreatePairRequest {
                base_contract: contract_name1,
                quote_contract: contract_name2,
            };

            tracing::info!("Sending create pair request: {:?}", request);

            let response = client
                .post(format!("{}/create_pair", args.server_url))
                .header("x-identity", args.identity)
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let response_text = response.text().await?;
                println!("Pair created successfully! Response: {response_text}");
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            }
        }
        Commands::AddSessionKey => {
            tracing::info!(
                "Sending add session key request with derived public key: {}",
                public_key_hex
            );

            let response = client
                .post(format!("{}/add_session_key", args.server_url))
                .header("x-identity", args.identity)
                .header("x-public-key", &public_key_hex)
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let response_text = response.text().await?;
                println!("Session key added successfully! Response: {response_text}");
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            }
        }
        Commands::Deposit { symbol, amount } => {
            let request = DepositRequest {
                symbol: symbol.clone(),
                amount,
            };

            tracing::info!("Sending deposit request: {:?}", request);

            let response = client
                .post(format!("{}/deposit", args.server_url))
                .header("x-identity", args.identity)
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let response_text = response.text().await?;
                println!("Deposit successful! Response: {response_text}");
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            }
        }
        Commands::GetBalances {} => {
            tracing::info!("Sending get balance request");

            let response = client
                .get(format!("{}/api/user/balances", args.api_url))
                .header("x-identity", args.identity)
                .header("Content-Type", "application/json")
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let balances = response.json::<UserBalances>().await?;
                println!("Balances: {:#?}", balances);
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            }
        }
        Commands::CreateOrder {
            order_id,
            order_side,
            order_type,
            price,
            asset_symbol1,
            asset_symbol2,
            quantity,
        } => {
            let order_side = match order_side.to_lowercase().as_str() {
                "bid" => OrderSide::Bid,
                "ask" => OrderSide::Ask,
                _ => anyhow::bail!("Invalid order side. Must be 'bid' or 'ask'"),
            };

            let order_type = match order_type.to_lowercase().as_str() {
                "limit" => OrderType::Limit,
                "market" => OrderType::Market,
                _ => anyhow::bail!("Invalid order type. Must be 'limit' or 'market'"),
            };

            let request = Order {
                order_id: order_id.clone(),
                order_side,
                order_type,
                price,
                pair: (asset_symbol1, asset_symbol2),
                quantity,
            };

            tracing::info!("Sending create order request: {:?}", request);

            // Create signature using the format: {user}:{nonce}:create_order:{order_id}
            let data_to_sign = format!("{}:{}:create_order:{}", args.identity, nonce, order_id);
            tracing::info!("Data to sign: {}", data_to_sign);
            let signature = create_signature(&signing_key, &data_to_sign)?;

            let response = client
                .post(format!("{}/create_order", args.server_url))
                .header("x-identity", args.identity)
                .header("x-public-key", &public_key_hex)
                .header("x-signature", &signature)
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let response_text = response.text().await?;
                println!("Order created successfully! Response: {response_text}");
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            }
        }
        Commands::Cancel { order_id } => {
            let request = CancelOrderRequest {
                order_id: order_id.clone(),
            };
            tracing::info!("Sending cancel order request for order_id: {}", order_id);

            // Create signature using the format: {user}:{nonce}:cancel:{order_id}
            let data_to_sign = format!("{}:{}:cancel:{}", args.identity, nonce, order_id);
            let signature = create_signature(&signing_key, &data_to_sign)?;

            let response = client
                .post(format!("{}/cancel_order", args.server_url))
                .header("x-identity", args.identity)
                .header("x-public-key", &public_key_hex)
                .header("x-signature", &signature)
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let response_text = response.text().await?;
                println!("Order cancelled successfully! Response: {response_text}");
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            }
        }
        Commands::Withdraw { symbol, amount } => {
            tracing::info!(
                "Sending withdraw request for symbol: {}, amount: {}",
                symbol,
                amount
            );

            // Create signature using the format: {user}:{nonce}:withdraw:{symbol}:{amount}
            let data_to_sign =
                format!("{}:{}:withdraw:{}:{}", args.identity, nonce, symbol, amount);
            let signature = create_signature(&signing_key, &data_to_sign)?;

            let response = client
                .post(format!("{}/withdraw", args.server_url))
                .header("x-identity", args.identity)
                .header("x-public-key", &public_key_hex)
                .header("x-signature", &signature)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "symbol": symbol, "amount": amount }))
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let response_text = response.text().await?;
                println!("Withdraw successful! Response: {response_text}");
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            }
        }
        Commands::Simulate {
            asset_symbol1,
            asset_symbol2,
            middle_price,
            price_offset,
            interval_seconds,
            quantity,
            max_orders,
            trend,
            fast,
        } => {
            // Get assets info
            let response = client
                .get(format!("{}/api/info", args.api_url))
                .header("x-identity", args.identity.clone())
                .send()
                .await
                .context("Failed to send request to server")?;
            #[derive(Debug, Deserialize)]
            struct ApiAsset {
                contract_name: String,
                scale: u64,
                symbol: String,
            }

            #[derive(Debug, Deserialize)]
            struct ApiInfoResponse {
                assets: Vec<ApiAsset>,
            }

            let assets_info: ApiInfoResponse = if response.status().is_success() {
                response.json().await?
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            };

            let asset_1 = assets_info
                .assets
                .iter()
                .find(|asset| asset.symbol == asset_symbol1)
                .unwrap();

            let deposit_amount_1 = asset_1.scale * 100_000_000_000;

            let asset_2 = assets_info
                .assets
                .iter()
                .find(|asset| asset.symbol == asset_symbol2)
                .unwrap();

            let deposit_amount_2 = asset_2.scale * 100_000_000_000 * middle_price;

            // Add session key
            let response = client
                .post(format!("{}/add_session_key", args.server_url))
                .header("x-identity", args.identity.clone())
                .header("x-public-key", &public_key_hex)
                .header("Content-Length", "0")
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let response_text = response.text().await?;
                println!("Session key added successfully! Response: {response_text}");
            } else {
                let status = response.status();
                if status != StatusCode::NOT_MODIFIED {
                    let error_text = response.text().await.unwrap_or_default();
                    anyhow::bail!("Server returned error {status}: {error_text}");
                } else {
                    let response_text = response.text().await.unwrap_or_default();
                    println!(
                        "Session key already exists. Response: {response_text}. Status: {status}"
                    );
                }
            }

            // Deposit
            let response = client
                .post(format!("{}/deposit", args.server_url))
                .header("x-identity", args.identity.clone())
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "symbol": asset_symbol1, "amount": deposit_amount_1 }))
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let response_text = response.text().await?;
                println!(
                    "Deposit successful! Response: {response_text}. Amount: {deposit_amount_1}"
                );
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            }

            // Deposit
            let response = client
                .post(format!("{}/deposit", args.server_url))
                .header("x-identity", args.identity.clone())
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "symbol": asset_symbol2, "amount": deposit_amount_2 }))
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let response_text = response.text().await?;
                println!(
                    "Deposit successful! Response: {response_text}. Amount: {deposit_amount_2}"
                );
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            }

            // Create pair
            let response = client
                .post(format!("{}/create_pair", args.server_url))
                .header("x-identity", args.identity.clone())
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "base_contract": asset_1.contract_name.clone(), "quote_contract": asset_2.contract_name.clone() }))
                .send()
                .await
                .context("Failed to send request to server")?;

            if response.status().is_success() {
                let response_text = response.text().await?;
                println!("Pair created successfully! Response: {response_text}");
            } else {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                anyhow::bail!("Server returned error {status}: {error_text}");
            }

            // Wait 10 seconds before starting simulation
            tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

            let quantity = quantity * 10_u64.pow(asset_1.scale as u32);
            let middle_price = middle_price * 10_u64.pow(asset_2.scale as u32);
            let price_offset = price_offset * 10_u64.pow(asset_2.scale as u32);

            // Validate trend parameter
            match trend.to_lowercase().as_str() {
                "up" => {
                    tracing::info!("Trend: up");
                }
                "down" => {
                    tracing::info!("Trend: down");
                }
                "stale" => {
                    tracing::info!("Trend: stale");
                }
                "random" => {
                    tracing::info!("Trend: random");
                }
                _ => anyhow::bail!("Invalid trend. Must be 'up', 'down', 'stale', or 'random'"),
            };

            tracing::info!(
                "Starting simulation for pair {}/{} with middle_price: {}, offset: {}, interval: {}s, quantity: {}, max_orders: {}, trend: {}",
                asset_symbol1, asset_symbol2, middle_price / 10_u64.pow(asset_2.scale as u32), price_offset / 10_u64.pow(asset_2.scale as u32), interval_seconds, quantity / 10_u64.pow(asset_1.scale as u32), max_orders, trend
            );

            let mut order_count = 0;
            let mut middle_price = middle_price;
            let mut trend_direction = trend.as_str();

            while order_count < max_orders {
                let current_nonce = get_nonce(&client, &args.server_url, &args.identity).await?;
                tracing::info!("Current nonce: {}", current_nonce);
                // Calculate price progression based on trend

                match trend.as_str() {
                    "up" => {
                        trend_direction = "up";
                    }
                    "down" => {
                        trend_direction = "down";
                    }
                    "stale" => {
                        trend_direction = "stale";
                    }
                    "random" => {
                        if order_count % (3 * 60) == 0 {
                            trend_direction = match random_between(0, 2) {
                                0 => "up",
                                1 => "down",
                                2 => "stale",
                                _ => unreachable!(),
                            };
                            tracing::info!("Trend updated to: {}", trend_direction);
                        }
                    }
                    _ => {}
                }

                match trend_direction {
                    "up" => {
                        // Upward trend: price increases over time
                        if order_count % 10 == 0 {
                            middle_price += price_offset;
                            tracing::info!(
                                "Middle price updated to: {}",
                                middle_price as f64 / 10_u64.pow(asset_2.scale as u32) as f64
                            );
                        }
                    }
                    "down" => {
                        // Downward trend: price decreases over time
                        if order_count % 10 == 0 {
                            middle_price = middle_price.saturating_sub(price_offset);
                            tracing::info!(
                                "Middle price updated to: {}",
                                middle_price as f64 / 10_u64.pow(asset_2.scale as u32) as f64
                            );
                        }
                    }
                    "stale" => {
                        // Stale trend: price stays around middle with small variations
                    }
                    _ => {}
                };

                // Alternate between bid and ask orders
                let order_side = if order_count % 2 == 0 {
                    OrderSide::Bid
                } else {
                    OrderSide::Ask
                };

                // random price between middle_price - price_offset and middle_price + price_offset
                let price = random_between(
                    middle_price.saturating_sub(price_offset * 5),
                    middle_price.saturating_add(price_offset * 5),
                );

                let order_id = format!("sim_{}_{}", args.identity, Uuid::new_v4());
                let order = Order {
                    order_id: order_id.clone(),
                    order_side,
                    order_type: OrderType::Limit,
                    price: Some(price),
                    pair: (asset_symbol1.clone(), asset_symbol2.clone()),
                    quantity,
                };

                tracing::info!(
                    "Creating order #{} (price: {}): {:?}",
                    order_count + 1,
                    price as f64 / 10_u64.pow(asset_2.scale as u32) as f64,
                    order
                );

                // Create signature for this order
                let data_to_sign = format!(
                    "{}:{}:create_order:{}",
                    args.identity, current_nonce, order_id
                );
                let signature = create_signature(&signing_key, &data_to_sign)?;

                let response = client
                    .post(format!("{}/create_order", args.server_url))
                    .header("x-identity", args.identity.clone())
                    .header("x-public-key", &public_key_hex)
                    .header("x-signature", &signature)
                    .header("Content-Type", "application/json")
                    .json(&order)
                    .send()
                    .await
                    .context("Failed to send request to server")?;

                if response.status().is_success() {
                    let response_text = response.text().await?;
                    tracing::debug!(
                        "Order #{} created successfully! Price: {}, Response: {}",
                        order_count + 1,
                        price,
                        response_text
                    );
                } else {
                    let status = response.status();
                    let error_text = response.text().await.unwrap_or_default();
                    tracing::warn!(
                        "Order #{} failed with status {}: {}",
                        order_count + 1,
                        status,
                        error_text
                    );
                }

                order_count += 1;

                // Wait for the specified interval before creating the next order
                if order_count < max_orders && !fast {
                    tokio::time::sleep(tokio::time::Duration::from_secs(interval_seconds)).await;
                }
            }

            println!(
                "Simulation completed! Created {} orders with {} trend.",
                order_count, trend
            );
        }
    }

    Ok(())
}

fn random_between(min: u64, max: u64) -> u64 {
    let mut rng = rand::rng();
    rng.random_range(min..=max)
}
