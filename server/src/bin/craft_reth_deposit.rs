use std::str::FromStr;

use alloy::{
    consensus::{SignableTransaction, TxEip1559, TxEnvelope},
    eips::eip2718::Encodable2718,
    primitives::{Address, Bytes, TxKind, U256},
    providers::{Provider, ProviderBuilder},
    signers::{local::PrivateKeySigner, SignerSync},
    sol_types::{sol, SolCall},
};
use anyhow::{anyhow, Context, Result};
use clap::Parser;
use hex::ToHex;
use server::conf::Conf;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Craft a signed ERC20 transfer suitable for POST /reth_bridge/deposit"
)]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config_file: Vec<String>,
    #[arg(long, help = "Hex-encoded private key used to sign the ERC20 transfer")]
    private_key: String,
    #[arg(long, help = "Deposit amount in the ERC20 smallest unit")]
    amount: String,
    #[arg(long, help = "Hyli identity to include in the sample curl output")]
    identity: Option<String>,
    #[arg(long, help = "Override the RPC URL from the config")]
    rpc_url: Option<String>,
    #[arg(long, help = "Override the ERC20 contract address")]
    contract_address: Option<String>,
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
    #[arg(long, help = "Override the nonce used for signing")]
    nonce: Option<u64>,
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
    let vault_address = args
        .vault_address
        .as_deref()
        .unwrap_or(&conf.bridge.eth_contract_vault_address);

    let contract = Address::from_str(contract_address)
        .with_context(|| format!("parsing contract address {contract_address}"))?;
    let vault = Address::from_str(vault_address)
        .with_context(|| format!("parsing vault address {vault_address}"))?;
    let amount = parse_u256(&args.amount)?;

    let provider = ProviderBuilder::new().connect_http(rpc_url.parse().context("parsing RPC URL")?);

    let chain_id = args.chain_id.unwrap_or(conf.bridge.eth_chain_id);

    let signer = PrivateKeySigner::from_str(args.private_key.trim_start_matches("0x"))
        .context("parsing deposit private key")?;
    let from = signer.address();

    let nonce: u64 = if let Some(explicit) = args.nonce {
        explicit
    } else {
        provider
            .get_transaction_count(from)
            .await
            .context("fetching nonce")?
            .try_into()
            .map_err(|err| anyhow!("nonce too large: {err}"))?
    };

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

    if let Some(identity) = &args.identity {
        println!(
            "\nExample curl:\n  curl -X POST http://localhost:{}/reth_bridge/deposit \\\n    -H 'content-type: application/json' \\\n    -d '{{\"identity\":\"{}\",\"signed_tx_hex\":\"{}\",\"amount\":{}}}'",
            conf.rest_server_port, identity, raw_hex, amount
        );
    }

    Ok(())
}

fn parse_u256(value: &str) -> Result<U256> {
    value
        .parse::<U256>()
        .map_err(|err| anyhow!("invalid amount {value}: {err}"))
}

sol! {
    #[allow(non_camel_case_types)]
    function transfer(address to, uint256 amount) returns (bool);
}
