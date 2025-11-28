use std::{path::PathBuf, str::FromStr};

use alloy::{
    primitives::Address,
    providers::{Provider, ProviderBuilder},
};
use anyhow::{anyhow, Context, Result};
use clap::Parser;
use reqwest::Client;
use sdk::ContractName;
use server::{
    conf::Conf,
    nonce_store::NonceStore,
    reth_utils::{derive_program_pubkey, program_address_from_program_id},
};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Craft a withdraw payload for POST /withdraw_reth_bridge with nonce tracking"
)]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config_file: Vec<String>,
    #[arg(long, default_value = "orderbook", help = "Orderbook contract name")]
    orderbook_cn: String,
    #[arg(
        long,
        help = "Hyli identity that will submit the withdraw action (defaults to user@<contract>)"
    )]
    identity: Option<String>,
    #[arg(long, help = "Destination Ethereum address for the withdraw")]
    eth_address: String,
    #[arg(long, help = "Withdraw amount in the ERC20 smallest unit")]
    amount: u64,
    #[arg(
        long,
        default_value = "http://localhost:9002",
        help = "Hyli server URL (rest) to call /withdraw_reth_bridge"
    )]
    server_url: String,
    #[arg(long, help = "Override the RPC URL from the config for nonce lookups")]
    rpc_url: Option<String>,
    #[arg(long, help = "Override the chain id used for nonce tracking")]
    chain_id: Option<u64>,
    #[arg(long, help = "Use an explicit nonce instead of the tracked value")]
    nonce: Option<u64>,
    #[arg(
        long,
        default_value_t = false,
        help = "Fetch the vault nonce from the RPC URL before sending"
    )]
    use_rpc_nonce: bool,
    #[arg(
        long,
        default_value_t = false,
        help = "Reset locally tracked nonce before sending"
    )]
    reset_nonce: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let conf = Conf::new(args.config_file.clone()).context("loading config")?;

    let rpc_url = args
        .rpc_url
        .as_deref()
        .unwrap_or(&conf.bridge.eth_rpc_http_url);
    let chain_id = args.chain_id.unwrap_or(conf.bridge.eth_chain_id);

    let orderbook_contract: ContractName = args.orderbook_cn.clone().into();
    let program_id = derive_program_pubkey(&orderbook_contract);
    let vault = program_address_from_program_id(&program_id);
    println!(
        "Derived vault signer {vault:#x} from orderbook contract {}",
        orderbook_contract.0
    );

    let identity = args
        .identity
        .clone()
        .unwrap_or_else(|| format!("user@{}", orderbook_contract.0));

    let destination = Address::from_str(args.eth_address.as_str())
        .with_context(|| format!("parsing eth_address {}", args.eth_address))?;

    let nonce_store_path: PathBuf = conf.data_directory.join("reth_nonce_store.json");
    let mut nonce_store = NonceStore::load(nonce_store_path.clone())
        .with_context(|| format!("loading nonce store at {}", nonce_store_path.display()))?;
    let nonce_key = NonceStore::key(vault, chain_id);

    if args.reset_nonce {
        nonce_store.reset(&nonce_key);
    }

    if let Some(explicit) = args.nonce {
        nonce_store.set(&nonce_key, explicit);
        println!(
            "Using explicit nonce {} for vault {:#x} on chain {}",
            explicit, vault, chain_id
        );
    } else if args.use_rpc_nonce {
        let provider =
            ProviderBuilder::new().connect_http(rpc_url.parse().context("parsing RPC URL")?);
        match provider.get_transaction_count(vault).await {
            Ok(value) => {
                let rpc_nonce: u64 = value
                    .try_into()
                    .map_err(|err| anyhow!("nonce too large from RPC: {err}"))?;
                nonce_store.set(&nonce_key, rpc_nonce);
                println!(
                    "Fetched nonce {} for vault {:#x} from RPC {}",
                    rpc_nonce, vault, rpc_url
                );
            }
            Err(err) => {
                eprintln!(
                    "Warning: failed to fetch nonce from RPC ({err}); falling back to local tracking"
                );
            }
        }
    }

    if nonce_store.get(&nonce_key).is_none() {
        nonce_store.ensure_default(&nonce_key, 0);
    }

    let nonce = nonce_store.next_nonce(&nonce_key);
    println!(
        "Sending withdraw for identity {} → {:#x} (amount {}) with nonce {} on chain {}",
        identity, destination, args.amount, nonce, chain_id
    );

    let client = Client::new();
    let response = client
        .post(format!("{}/withdraw_reth_bridge", args.server_url))
        .json(&serde_json::json!({
            "identity": identity,
            "eth_address": format!("{:#x}", destination),
            "amount": args.amount,
            "nonce": nonce,
        }))
        .send()
        .await
        .context("sending withdraw_reth_bridge request")?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("withdraw_reth_bridge failed with status {status}: {body}");
    }

    println!("Submitted withdraw_reth_bridge request: {body}");

    nonce_store.persist().context("saving nonce store")?;

    Ok(())
}
