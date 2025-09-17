use anyhow::{Context, Result};
use clap::{command, Parser, Subcommand};
use hyli_modules::utils::logger::setup_tracing;
use k256::{elliptic_curve::sec1::ToEncodedPoint, SecretKey};
use orderbook::orderbook::{OrderType, TokenPair};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use server::conf::Conf;

#[derive(Parser, Debug)]
#[command(version, about = "Send transactions to a server", long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "http://localhost:9002")]
    pub server_url: String,

    #[arg(long, default_value = "txsender@orderbook")]
    pub identity: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a new order
    CreateOrder {
        #[arg(long)]
        order_id: String,
        #[arg(long)]
        order_type: String,
        #[arg(long)]
        price: Option<u32>,
        #[arg(long)]
        pair_token1: String,
        #[arg(long)]
        pair_token2: String,
        #[arg(long)]
        quantity: u32,
    },
    /// Add a session key for user authentication
    AddSessionKey,
    // /// Cancel an existing order
    // Cancel {
    //     #[arg(long)]
    //     order_id: String,
    // },
    // /// Deposit tokens
    // Deposit {
    //     #[arg(long)]
    //     token: String,
    //     #[arg(long)]
    //     amount: u32,
    // },
    // /// Withdraw tokens
    // Withdraw {
    //     #[arg(long)]
    //     token: String,
    //     #[arg(long)]
    //     amount: u32,
    // },
}

#[derive(Serialize, Deserialize, Debug)]
struct CreateOrderRequest {
    order_id: String,
    order_type: OrderType,
    price: Option<u32>,
    pair: TokenPair,
    quantity: u32,
}

#[derive(Serialize, Deserialize, Debug)]
struct AddSessionKeyRequest {
    public_key: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Conf::new(args.config_file).context("reading config file")?;
    const HARDCODED_PRIVATE_KEY: &str =
        "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";

    setup_tracing(&config.log_format, "tx_sender".to_string()).context("setting up tracing")?;

    let client = Client::new();

    match args.command {
        Commands::CreateOrder {
            order_id,
            order_type,
            price,
            pair_token1,
            pair_token2,
            quantity,
        } => {
            let order_type = match order_type.to_lowercase().as_str() {
                "buy" => OrderType::Buy,
                "sell" => OrderType::Sell,
                _ => anyhow::bail!("Invalid order type. Must be 'buy' or 'sell'"),
            };

            let request = CreateOrderRequest {
                order_id,
                order_type,
                price,
                pair: (pair_token1, pair_token2),
                quantity,
            };

            tracing::info!("Sending create order request: {:?}", request);

            let response = client
                .post(format!("{}/create_order", args.server_url))
                .header("x-identity", args.identity)
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
        Commands::AddSessionKey => {
            let private_key_bytes =
                hex::decode(HARDCODED_PRIVATE_KEY).context("Impossible to decode private key")?;

            let secret_key =
                SecretKey::from_slice(&private_key_bytes).context("Invalid private key")?;

            let public_key = secret_key.public_key();
            let public_key_bytes = public_key.to_encoded_point(false).as_bytes().to_vec();
            let public_key_hex = hex::encode(public_key_bytes);

            let request = AddSessionKeyRequest {
                public_key: public_key_hex.clone(),
            };

            tracing::info!(
                "Sending add session key request with derived public key: {}",
                public_key_hex
            );

            let response = client
                .post(format!("{}/add_session_key", args.server_url))
                .header("x-identity", args.identity)
                .header("Content-Type", "application/json")
                .json(&request)
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
    }

    Ok(())
}
