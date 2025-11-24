use std::{fs, path::Path, str::FromStr};

use alloy::primitives::{keccak256, Address, B256};
use client_sdk::rest_client::{NodeApiClient, NodeApiHttpClient};
use k256::ecdsa::SigningKey;
use reqwest::Client;
use sdk::{api::APIRegisterContract, ContractName, ProgramId, StateCommitment, Verifier};
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

use crate::conf::BridgeConfig;
use alloy::signers::local::PrivateKeySigner;
use hex::ToHex;

pub const COLLATERAL_METADATA_FILE: &str = "reth_collateral_metadata.json";

#[derive(Clone)]
pub struct CollateralRegistrationData {
    pub contract_address: Address,
    pub block_hash: B256,
    pub state_root: B256,
}

#[derive(Serialize, Deserialize)]
struct CollateralMetadataFile {
    contract_address: String,
    block_hash: String,
    state_root: String,
}
use serde::{Deserialize, Serialize};

/// Deterministically derives a program id from the orderbook contract name.
/// Mirrors the derivation used in the reth deposit demo so verifier registration matches.
pub fn derive_program_pubkey(contract_name: &ContractName) -> ProgramId {
    let mut seed: [u8; 32] = keccak256(contract_name.0.as_bytes()).into();
    let signing_key = loop {
        match SigningKey::from_slice(&seed) {
            Ok(key) => break key,
            Err(_) => {
                seed = keccak256(seed).into();
            }
        }
    };
    let encoded = signing_key
        .verifying_key()
        .to_encoded_point(false)
        .as_bytes()
        .to_vec();
    ProgramId(encoded)
}

pub async fn register_reth_collateral_contract(
    client: &NodeApiHttpClient,
    contract_name: &ContractName,
    orderbook_cn: &ContractName,
    bridge_config: &BridgeConfig,
) -> anyhow::Result<()> {
    let (state_root, block_hash) = fetch_block_metadata(
        &bridge_config.eth_rpc_http_url,
        bridge_config.eth_contract_deploy_block,
    )
    .await?;

    let contract_address = Address::from_str(&bridge_config.eth_contract_address)
        .map_err(|err| anyhow::anyhow!("invalid collateral contract address: {err}"))?;
    let metadata = CollateralRegistrationData {
        contract_address,
        block_hash,
        state_root,
    };
    register_reth_collateral_with_data(client, contract_name, orderbook_cn, metadata).await
}

pub async fn register_reth_collateral_with_data(
    client: &NodeApiHttpClient,
    contract_name: &ContractName,
    orderbook_cn: &ContractName,
    metadata: CollateralRegistrationData,
) -> anyhow::Result<()> {
    let encoded_name = encode_contract_name(contract_name);
    if let Ok(existing) = client.get_contract(encoded_name.clone()).await {
        if existing.verifier.0 != "reth" {
            warn!(
                contract = %contract_name.0,
                verifier = %existing.verifier.0,
                "collateral contract already registered with different verifier; skipping registration"
            );
        }
        return Ok(());
    }

    let program_id = derive_program_pubkey(orderbook_cn);
    let mut constructor_metadata = Vec::new();
    constructor_metadata.extend_from_slice(metadata.contract_address.as_slice());
    constructor_metadata.extend_from_slice(metadata.block_hash.as_slice());
    constructor_metadata.extend_from_slice(program_id.0.as_slice());
    constructor_metadata.extend_from_slice(metadata.state_root.as_slice());

    let payload = APIRegisterContract {
        verifier: Verifier("reth".into()),
        program_id,
        state_commitment: StateCommitment(metadata.state_root.as_slice().to_vec()),
        contract_name: encoded_name.clone(),
        timeout_window: None,
        constructor_metadata: Some(constructor_metadata),
    };

    client
        .register_contract(payload)
        .await
        .map_err(|err| anyhow::anyhow!("registering collateral contract on Hyli: {err}"))?;

    wait_for_collateral_registration(client, encoded_name, contract_name).await?;

    Ok(())
}

pub fn derive_address_from_private_key(key_hex: &str) -> anyhow::Result<Address> {
    let signer = PrivateKeySigner::from_str(key_hex.trim_start_matches("0x"))
        .map_err(|err| anyhow::anyhow!("parsing private key for prefunding: {err}"))?;
    Ok(signer.address())
}

fn encode_contract_name(contract_name: &ContractName) -> ContractName {
    ContractName(contract_name.0.replace('/', "%2F"))
}

fn format_b256(value: &B256) -> String {
    format!("0x{}", value.as_slice().encode_hex::<String>())
}

fn parse_b256(hex_str: &str) -> anyhow::Result<B256> {
    B256::from_str(hex_str).map_err(|err| anyhow::anyhow!("invalid b256 {hex_str}: {err}"))
}

pub fn persist_collateral_metadata(
    metadata_path: &Path,
    data: &CollateralRegistrationData,
) -> anyhow::Result<()> {
    if let Some(parent) = metadata_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|err| anyhow::anyhow!("creating metadata dir: {err}"))?;
        }
    }

    let file = CollateralMetadataFile {
        contract_address: format!(
            "0x{}",
            data.contract_address.as_slice().encode_hex::<String>()
        ),
        block_hash: format_b256(&data.block_hash),
        state_root: format_b256(&data.state_root),
    };

    let contents = serde_json::to_vec_pretty(&file)
        .map_err(|err| anyhow::anyhow!("serializing collateral metadata: {err}"))?;
    fs::write(metadata_path, contents)
        .map_err(|err| anyhow::anyhow!("writing collateral metadata: {err}"))?;
    Ok(())
}

pub fn load_collateral_metadata(
    metadata_path: &Path,
) -> anyhow::Result<CollateralRegistrationData> {
    let bytes = fs::read(metadata_path)
        .map_err(|err| anyhow::anyhow!("reading collateral metadata: {err}"))?;
    let file: CollateralMetadataFile = serde_json::from_slice(&bytes)
        .map_err(|err| anyhow::anyhow!("parsing collateral metadata: {err}"))?;
    let contract_address = Address::from_str(file.contract_address.as_str())
        .map_err(|err| anyhow::anyhow!("invalid collateral metadata address: {err}"))?;
    let block_hash = parse_b256(&file.block_hash)?;
    let state_root = parse_b256(&file.state_root)?;
    Ok(CollateralRegistrationData {
        contract_address,
        block_hash,
        state_root,
    })
}

async fn fetch_block_metadata(rpc_url: &str, block_number: u64) -> anyhow::Result<(B256, B256)> {
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_getBlockByNumber",
        "params": [format!("0x{:x}", block_number), false],
        "id": 1,
    });

    let resp = Client::new()
        .post(rpc_url)
        .json(&payload)
        .send()
        .await
        .map_err(|err| anyhow::anyhow!("fetching block {block_number} from {rpc_url}: {err}"))?;
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|err| anyhow::anyhow!("parsing block response: {err}"))?;
    let block = value
        .get("result")
        .ok_or_else(|| anyhow::anyhow!("missing result in block response"))?;
    let hash_str = block
        .get("hash")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing block hash"))?;
    let state_root_str = block
        .get("stateRoot")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing stateRoot"))?;

    let block_hash = B256::from_str(hash_str)
        .map_err(|err| anyhow::anyhow!("invalid block hash {hash_str}: {err}"))?;
    let state_root = B256::from_str(state_root_str)
        .map_err(|err| anyhow::anyhow!("invalid state root {state_root_str}: {err}"))?;

    Ok((state_root, block_hash))
}

async fn wait_for_collateral_registration(
    client: &NodeApiHttpClient,
    encoded_name: ContractName,
    contract_name: &ContractName,
) -> anyhow::Result<()> {
    info!(
        contract = %contract_name.0,
        "waiting for Hyli node to register reth collateral contract"
    );
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(contract) = client.get_contract(encoded_name.clone()).await {
                if contract.verifier.0 == "reth" {
                    break;
                }
            }
            sleep(Duration::from_millis(500)).await;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "timed out waiting for reth collateral contract {} to register",
            contract_name.0
        )
    })??;

    info!(
        contract = %contract_name.0,
        "reth collateral contract registered on Hyli node"
    );
    Ok(())
}
