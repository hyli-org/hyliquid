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
use client_sdk::rest_client::{NodeApiClient, NodeApiHttpClient};
use hex::encode;
use sdk::{Blob, BlobData, BlobTransaction, ContractName, Identity, StructuredBlobData};
use server::conf::Conf;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Crafts a Hyli blob containing a mint transaction for the ERC20 collateral"
)]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config_file: Vec<String>,
    #[arg(long, help = "Hyli identity that will submit the blob transaction")]
    identity: String,
    #[arg(long, help = "Hex-encoded private key allowed to mint the ERC20")]
    private_key: String,
    #[arg(
        long = "recipient",
        required = true,
        help = "Recipient and amount formatted as address:amount (repeatable)"
    )]
    recipients: Vec<String>,
    #[arg(long, help = "Override the RPC URL from the config")]
    rpc_url: Option<String>,
    #[arg(long, help = "Override the Hyli node URL from the config")]
    node_url: Option<String>,
    #[arg(long, help = "Override the ERC20 contract address")]
    contract_address: Option<String>,
    #[arg(
        long,
        default_value = "oranj",
        help = "Collateral contract name on Hyli"
    )]
    contract_name: String,
    #[arg(
        long,
        default_value = "200000",
        help = "Gas limit for each mint (default 200k)"
    )]
    gas_limit: u64,
    #[arg(long, default_value = "2000000000", help = "Max fee per gas (wei)")]
    max_fee_per_gas: u64,
    #[arg(
        long,
        default_value = "1500000000",
        help = "Max priority fee per gas (wei)"
    )]
    max_priority_fee_per_gas: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let conf = Conf::new(args.config_file.clone()).context("loading config")?;

    let rpc_url = args
        .rpc_url
        .as_deref()
        .unwrap_or(&conf.bridge.eth_rpc_http_url);
    let node_url = args.node_url.as_deref().unwrap_or(&conf.node_url);
    let hyli_client =
        NodeApiHttpClient::new(node_url.to_string()).context("building Hyli client")?;

    let contract_address = args
        .contract_address
        .as_deref()
        .unwrap_or(&conf.bridge.eth_contract_address);
    let hyli_contract = ContractName(args.contract_name.clone());

    let contract = Address::from_str(contract_address)
        .with_context(|| format!("parsing contract address {contract_address}"))?;

    let provider = ProviderBuilder::new().connect_http(rpc_url.parse().context("parsing RPC URL")?);

    let chain_id: u64 = provider
        .get_chain_id()
        .await
        .context("fetching chain id")?
        .try_into()
        .map_err(|err| anyhow!("chain id too large: {err}"))?;

    let signer = PrivateKeySigner::from_str(args.private_key.trim_start_matches("0x"))
        .context("parsing mint signer private key")?;
    let from = signer.address();

    let mut nonce: u64 = provider
        .get_transaction_count(from)
        .await
        .context("fetching nonce")?
        .try_into()
        .map_err(|err| anyhow!("nonce too large: {err}"))?;

    let identity = Identity(args.identity.clone());

    for entry in &args.recipients {
        let target = parse_recipient(entry)?;
        let calldata = mintCall {
            to: target.address,
            amount: target.amount,
        }
        .abi_encode();

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
        nonce += 1;

        let signature = signer
            .sign_hash_sync(&tx.signature_hash())
            .context("signing mint transaction")?;
        let envelope: TxEnvelope = tx.into_signed(signature).into();
        let raw = envelope.encoded_2718();

        let blob = Blob {
            contract_name: hyli_contract.clone(),
            data: BlobData::from(StructuredBlobData {
                caller: None,
                callees: None,
                parameters: raw.clone(),
            }),
        };
        let blob_tx = BlobTransaction::new(identity.clone(), vec![blob]);

        let hyli_tx = hyli_client
            .send_tx_blob(blob_tx)
            .await
            .context("submitting mint blob to Hyli")?;

        println!(
            "Queued mint of {} tokens to {} (Hyli tx 0x{})",
            target.amount,
            target.address,
            encode(hyli_tx.0.as_bytes())
        );
    }

    Ok(())
}

struct MintTarget {
    address: Address,
    amount: U256,
}

fn parse_recipient(input: &str) -> Result<MintTarget> {
    let (address, amount) = input
        .split_once(':')
        .ok_or_else(|| anyhow!("recipient must be formatted as address:amount"))?;
    let addr = Address::from_str(address.trim())
        .with_context(|| format!("parsing recipient address {address}"))?;
    let amount = parse_u256(amount.trim())?;
    Ok(MintTarget {
        address: addr,
        amount,
    })
}

fn parse_u256(value: &str) -> Result<U256> {
    value
        .parse::<U256>()
        .map_err(|err| anyhow!("invalid amount {value}: {err}"))
}

sol! {
    #[allow(non_camel_case_types)]
    function mint(address to, uint256 amount);
}
