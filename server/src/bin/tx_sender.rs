use anyhow::{Context, Result};
use clap::{command, Parser, Subcommand};
use hyli_modules::utils::logger::setup_tracing;
use k256::{
    ecdsa::{signature::Signer, Signature, SigningKey},
    SecretKey,
};
use orderbook::orderbook::{OrderSide, OrderType};
use reqwest::Client;
use server::{
    app::{CancelOrderRequest, CreateOrderRequest, CreatePairRequest, DepositRequest},
    conf::Conf,
};
use sha2::{Digest, Sha256};

#[derive(Parser, Debug)]
#[command(version, about = "Send transactions to a server", long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "http://localhost:9002")]
    pub server_url: String,

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
        pair_token1: String,
        #[arg(long)]
        pair_token2: String,
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
        pair_token1: String,
        #[arg(long)]
        pair_token2: String,
        #[arg(long)]
        quantity: u64,
    },
    /// Add a session key for user authentication
    AddSessionKey,
    /// Deposit tokens
    Deposit {
        #[arg(long)]
        token: String,
        #[arg(long)]
        amount: u64,
    },
    // /// Cancel an existing order
    Cancel {
        #[arg(long)]
        order_id: String,
    },
    // /// Withdraw tokens
    Withdraw {
        #[arg(long)]
        token: String,
        #[arg(long)]
        amount: u64,
    },
}

// Helper function to create a signature for the given data
fn create_signature(signing_key: &SigningKey, data: &str) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let hash = hasher.finalize();

    let signature: Signature = signing_key.sign(&hash);
    Ok(hex::encode(signature.to_bytes()))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Conf::new(args.config_file).context("reading config file")?;

    setup_tracing(&config.log_format, "tx_sender".to_string()).context("setting up tracing")?;

    // Generate the key pair once for all operations
    let mut hasher = Sha256::new();
    hasher.update(args.identity.as_bytes());
    let derived_key = hasher.finalize();
    let private_key_bytes = derived_key.to_vec();

    let secret_key = SecretKey::from_slice(&private_key_bytes).context("Invalid private key")?;
    let signing_key = SigningKey::from(secret_key);
    let public_key = signing_key.verifying_key();
    let public_key_bytes = public_key.to_encoded_point(false).as_bytes().to_vec();
    let public_key_hex = hex::encode(public_key_bytes);

    let client = Client::new();

    let mut nonce: u32 = {
        let response = client
            .get(format!("{}/nonce", args.server_url))
            .header("x-identity", args.identity.clone())
            .send()
            .await
            .context("Failed to send request to server")?;

        if response.status().is_success() {
            let nonce_str = response.text().await?;
            nonce_str.trim().parse::<u32>().unwrap_or_default()
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Server returned error {status}: {error_text}");
        }
    };
    nonce += 1;

    match args.command {
        Commands::CreatePair {
            pair_token1,
            pair_token2,
        } => {
            let request = CreatePairRequest {
                pair: (pair_token1, pair_token2),
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
        Commands::Deposit { token, amount } => {
            let request = DepositRequest {
                token: token.clone(),
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
        Commands::CreateOrder {
            order_id,
            order_side,
            order_type,
            price,
            pair_token1,
            pair_token2,
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

            let request = CreateOrderRequest {
                order_id: order_id.clone(),
                order_side,
                order_type,
                price,
                pair: (pair_token1, pair_token2),
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
        Commands::Withdraw { token, amount } => {
            tracing::info!(
                "Sending withdraw request for token: {}, amount: {}",
                token,
                amount
            );

            // Create signature using the format: {user}:{nonce}:withdraw:{token}:{amount}
            let data_to_sign = format!("{}:{}:withdraw:{}:{}", args.identity, nonce, token, amount);
            let signature = create_signature(&signing_key, &data_to_sign)?;

            let response = client
                .post(format!("{}/withdraw", args.server_url))
                .header("x-identity", args.identity)
                .header("x-public-key", &public_key_hex)
                .header("x-signature", &signature)
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "token": token, "amount": amount }))
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
    }

    Ok(())
}
