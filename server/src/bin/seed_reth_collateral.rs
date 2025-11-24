use std::{path::PathBuf, str::FromStr};

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
use server::{
    conf::Conf,
    nonce_store::NonceStore,
    reth_utils::{load_collateral_metadata, COLLATERAL_METADATA_FILE},
};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Crafts a Hyli blob containing a mint transaction for the ERC20 collateral"
)]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config_file: Vec<String>,
    #[arg(
        long,
        default_value = "orderbook",
        help = "Orderbook contract name to derive defaults"
    )]
    orderbook_cn: String,
    #[arg(
        long,
        help = "Hyli identity that will submit the blob transaction (defaults to seed@<contract>)"
    )]
    identity: Option<String>,
    #[arg(
        long,
        help = "Hex-encoded private key allowed to mint the ERC20 (defaults to the deployer key from the config)"
    )]
    private_key: Option<String>,
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
        help = "Collateral contract name on Hyli (defaults to derived orderbook-specific value)"
    )]
    contract_name: Option<String>,
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
    #[arg(long, help = "Override the chain id used for signing")]
    chain_id: Option<u64>,
    #[arg(long, help = "Override the starting nonce for the mint signer")]
    nonce: Option<u64>,
    #[arg(
        long,
        default_value_t = false,
        help = "Fetch the signer nonce from the RPC (requires access token if the node enforces auth)"
    )]
    use_rpc_nonce: bool,
    #[arg(
        long,
        default_value_t = false,
        help = "Reset the locally tracked nonce before minting"
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
    let node_url = args.node_url.as_deref().unwrap_or(&conf.node_url);
    let hyli_client =
        NodeApiHttpClient::new(node_url.to_string()).context("building Hyli client")?;

    let metadata_path = conf.data_directory.join(COLLATERAL_METADATA_FILE);
    let metadata_address = if args.contract_address.is_none() {
        load_collateral_metadata(&metadata_path)
            .map(|data| format!("{:#x}", data.contract_address))
            .ok()
    } else {
        None
    };

    let contract_address = args
        .contract_address
        .clone()
        .or_else(|| metadata_address.clone())
        .unwrap_or(conf.bridge.eth_contract_address.clone());
    let default_contract = format!("reth-collateral-{}", args.orderbook_cn);
    let hyli_contract = ContractName(
        args.contract_name
            .clone()
            .unwrap_or(default_contract.clone()),
    );
    let identity = Identity(
        args.identity
            .clone()
            .unwrap_or_else(|| format!("seed@{}", hyli_contract.0)),
    );

    if args.contract_address.is_none() && metadata_address.is_some() {
        println!(
            "Using embedded reth collateral contract address {contract_address} from {}",
            metadata_path.display()
        );
    }

    let contract = Address::from_str(&contract_address)
        .with_context(|| format!("parsing contract address {contract_address}"))?;

    let provider = ProviderBuilder::new().connect_http(rpc_url.parse().context("parsing RPC URL")?);

    let chain_id = args.chain_id.unwrap_or(conf.bridge.eth_chain_id);

    let deployer_key = args
        .private_key
        .as_deref()
        .unwrap_or(&conf.bridge.eth_signer_private_key);
    let signer = PrivateKeySigner::from_str(deployer_key.trim_start_matches("0x"))
        .context("parsing mint signer private key")?;
    let from = signer.address();
    println!(
        "Minting with signer {from:#x} on chain {chain_id} (gas fee {} / priority {})",
        args.max_fee_per_gas, args.max_priority_fee_per_gas
    );

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
        match provider.get_transaction_count(from).await {
            Ok(value) => {
                let rpc_nonce: u64 = value
                    .try_into()
                    .map_err(|err| anyhow!("nonce too large from RPC: {err}"))?;
                nonce_store.set(&nonce_key, rpc_nonce);
                println!(
                    "Fetched nonce {} for signer {:#x} from RPC {}",
                    rpc_nonce, from, rpc_url
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

    for entry in &args.recipients {
        let target = parse_recipient(entry)?;
        let nonce = nonce_store.next_nonce(&nonce_key);
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
            "Queued mint of {} tokens to {} with nonce {} (Hyli tx 0x{})",
            target.amount,
            target.address,
            nonce,
            encode(hyli_tx.0.as_bytes())
        );
    }

    nonce_store.persist().context("saving nonce store")?;

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
