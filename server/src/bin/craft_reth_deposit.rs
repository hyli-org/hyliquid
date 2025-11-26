use std::str::FromStr;

use alloy::{
    consensus::{SignableTransaction, TxEip1559, TxEnvelope},
    eips::eip2718::Encodable2718,
    primitives::{keccak256, Address, Bytes, TxKind, U256},
    providers::{Provider, ProviderBuilder},
    signers::{local::PrivateKeySigner, SignerSync},
    sol_types::{sol, SolCall},
};
use anyhow::{anyhow, Context, Result};
use clap::Parser;
use client_sdk::rest_client::NodeApiClient;
use hex::{self, ToHex};
use sdk::{ContractName, ProgramId};
use serde_json;
use server::{conf::Conf, nonce_store::NonceStore, reth_utils::derive_program_pubkey};
use reqwest::Client;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Craft a signed ERC20 transfer suitable for POST /reth_bridge/deposit"
)]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config_file: Vec<String>,
    #[arg(long, default_value = "orderbook", help = "Orderbook contract name")]
    orderbook_cn: String,
    #[arg(
        long,
        help = "Hex-encoded private key used to sign the ERC20 transfer (defaults to bridge.eth_signer_private_key)"
    )]
    private_key: Option<String>,
    #[arg(long, help = "Deposit amount in the ERC20 smallest unit")]
    amount: String,
    #[arg(long, help = "Hyli identity that will submit the blob transaction")]
    identity: Option<String>,
    #[arg(long, help = "Override the RPC URL from the config")]
    rpc_url: Option<String>,
    #[arg(long, help = "Override the ERC20 contract address")]
    contract_address: Option<String>,
    #[arg(
        long,
        default_value = "http://localhost:9002",
        help = "Hyli server URL (rest) to call /deposit_reth_bridge"
    )]
    server_url: String,
    #[arg(
        long,
        help = "Override the collateral contract name used for blobs (defaults to reth-collateral-<orderbook>)"
    )]
    collateral_token_cn: Option<String>,
    #[arg(long, help = "Override the enforced vault address")]
    vault_address: Option<String>,
    #[arg(long, default_value = "200000", help = "Gas limit (default 200k)")]
    gas_limit: u64,
    #[arg(long, default_value = "2000000000", help = "Max fee per gas (wei)")]
    max_fee_per_gas: u64,
    #[arg(
        long,
        default_value = "1500000000",
        help = "Max priority fee per gas (wei)"
    )]
    max_priority_fee_per_gas: u64,
    #[arg(long, help = "Override the chain id used for signing")]
    chain_id: Option<u64>,
    #[arg(long, help = "Use an explicit nonce for signing")]
    nonce: Option<u64>,
    #[arg(
        long,
        default_value_t = false,
        help = "Fetch the signer nonce from the RPC URL"
    )]
    use_rpc_nonce: bool,
    #[arg(
        long,
        default_value_t = false,
        help = "Reset locally tracked nonce before signing"
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
    let contract_address = args
        .contract_address
        .as_deref()
        .unwrap_or(&conf.bridge.eth_contract_address);

    let orderbook_contract: ContractName = args.orderbook_cn.clone().into();
    let program_id = derive_program_pubkey(&orderbook_contract);
    let derived_vault = program_address_from_program_id(&program_id);

    let vault = if let Some(override_addr) = args.vault_address.as_deref() {
        Address::from_str(override_addr)
            .with_context(|| format!("parsing vault address {override_addr}"))?
    } else {
        derived_vault
    };

    let contract = Address::from_str(contract_address)
        .with_context(|| format!("parsing contract address {contract_address}"))?;
    let amount = parse_u256(&args.amount)?;
    let amount_u64: u64 = amount
        .try_into()
        .map_err(|_| anyhow!("deposit amount does not fit into u64"))?;

    let collateral_contract = args
        .collateral_token_cn
        .clone()
        .unwrap_or_else(|| format!("reth-collateral-{}", args.orderbook_cn));
    let identity = args
        .identity
        .clone()
        .unwrap_or_else(|| format!("user@{}", args.orderbook_cn));

    let chain_id = args.chain_id.unwrap_or(conf.bridge.eth_chain_id);

    let key_hex = args
        .private_key
        .clone()
        .unwrap_or(conf.bridge.eth_signer_private_key.clone());
    let signer = PrivateKeySigner::from_str(key_hex.trim_start_matches("0x"))
        .context("parsing deposit private key")?;
    let from = signer.address();

    let nonce_store_path: PathBuf = conf.data_directory.join("reth_nonce_store.json");
    let mut nonce_store = NonceStore::load(nonce_store_path.clone())
        .with_context(|| format!("loading nonce store at {}", nonce_store_path.display()))?;
    let nonce_key = NonceStore::key(from, chain_id);

    if args.reset_nonce {
        nonce_store.reset(&nonce_key);
    }

    if let Some(explicit) = args.nonce {
        nonce_store.set(&nonce_key, explicit);
        println!(
            "Using explicit nonce {} for signer {:#x} on chain {}",
            explicit, from, chain_id
        );
    } else if args.use_rpc_nonce {
        let provider =
            ProviderBuilder::new().connect_http(rpc_url.parse().context("parsing RPC URL")?);
        let rpc_nonce: u64 = provider
            .get_transaction_count(from)
            .await
            .context("fetching nonce")?
            .try_into()
            .map_err(|err| anyhow!("nonce too large: {err}"))?;
        nonce_store.set(&nonce_key, rpc_nonce);
        println!(
            "Fetched nonce {} for signer {:#x} from RPC {}",
            rpc_nonce, from, rpc_url
        );
    }

    if nonce_store.get(&nonce_key).is_none() {
        nonce_store.ensure_default(&nonce_key, 1);
    }

    let nonce = nonce_store.next_nonce(&nonce_key);

    let calldata = transferCall { to: vault, amount }.abi_encode();

    let tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit: args.gas_limit.into(),
        max_fee_per_gas: args.max_fee_per_gas.into(),
        max_priority_fee_per_gas: args.max_priority_fee_per_gas.into(),
        to: TxKind::Call(contract),
        value: U256::ZERO,
        access_list: Default::default(),
        input: Bytes::from(calldata),
    };

    let signature = signer
        .sign_hash_sync(&tx.signature_hash())
        .context("signing transfer transaction")?;
    let envelope: TxEnvelope = tx.into_signed(signature).into();
    let raw = envelope.encoded_2718();

    let raw_hex = format!("0x{}", raw.encode_hex::<String>());
    println!("Signed ERC20 transfer:\n{raw_hex}");

    let client = Client::new();
    let response = client
        .post(format!("{}/deposit_reth_bridge", args.server_url))
        .json(&serde_json::json!({
            "signed_tx_hex": raw_hex,
            "identity": identity,
        }))
        .send()
        .await
        .context("sending deposit_reth_bridge request")?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("deposit_reth_bridge failed with status {status}: {body}");
    }

    println!("Submitted deposit_reth_bridge request: {body}");

    nonce_store.persist().context("saving nonce store")?;

    Ok(())
}

fn parse_u256(value: &str) -> Result<U256> {
    value
        .parse::<U256>()
        .map_err(|err| anyhow!("invalid amount {value}: {err}"))
}

fn program_address_from_program_id(program_id: &ProgramId) -> Address {
    let hash = keccak256(program_id.0.as_slice());
    Address::from_slice(&hash[12..])
}

sol! {
    #[allow(non_camel_case_types)]
    function transfer(address to, uint256 amount) returns (bool);
}
