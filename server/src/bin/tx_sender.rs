use anyhow::{Context, Result};
use clap::{command, Parser, Subcommand};
use client_sdk::rest_client::{NodeApiClient, NodeApiHttpClient};
use hyli_modules::utils::logger::setup_tracing;
use orderbook::{orderbook::OrderType, OrderbookAction};
use sdk::{BlobTransaction, ContractName};
use server::conf::Conf;

#[derive(Parser, Debug)]
#[command(version, about = "Send transactions to a node", long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "orderbook")]
    pub orderbook_cn: String,

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
    /// Cancel an existing order
    Cancel {
        #[arg(long)]
        order_id: String,
    },
    /// Deposit tokens
    Deposit {
        #[arg(long)]
        token: String,
        #[arg(long)]
        amount: u32,
    },
    /// Withdraw tokens
    Withdraw {
        #[arg(long)]
        token: String,
        #[arg(long)]
        amount: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Conf::new(args.config_file).context("reading config file")?;

    setup_tracing(&config.log_format, "tx_sender".to_string()).context("setting up tracing")?;

    let client = NodeApiHttpClient::new(config.node_url).context("build node client")?;

    let action = match args.command {
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

            OrderbookAction::CreateOrder {
                order_id,
                order_type,
                price,
                pair: (pair_token1, pair_token2),
                quantity,
            }
        }
        Commands::Cancel { order_id } => OrderbookAction::Cancel { order_id },
        Commands::Deposit { token, amount } => OrderbookAction::Deposit { token, amount },
        Commands::Withdraw { token, amount } => OrderbookAction::Withdraw { token, amount },
    };

    tracing::info!("Action to be sent: {:?}", action);

    // Create the blob for the action
    let blob = action.as_blob(ContractName(args.orderbook_cn));

    let blob_tx = BlobTransaction::new("txsender@orderbook", vec![blob]);

    // Send transaction
    let tx_hash = client.send_tx_blob(blob_tx).await?;

    println!("Transaction sent successfully! Hash: {tx_hash}");

    Ok(())
}
