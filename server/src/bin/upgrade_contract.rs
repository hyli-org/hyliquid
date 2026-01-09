use std::{
    env,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use client_sdk::{
    helpers::ClientSdkProver,
    rest_client::{NodeApiClient, NodeApiHttpClient},
};
use orderbook::{
    model::UserInfo,
    transaction::{OrderbookAction, PermissionedOrderbookAction},
    ORDERBOOK_ACCOUNT_IDENTITY,
};
use rand::Rng;
use sdk::{BlobTransaction, ContractName, Hashed, ProgramId};
use serde::Serialize;
use server::prover::OrderbookProverRequest;

#[derive(Parser, Debug)]
#[command(version, about = "Upgrade the orderbook contract", long_about = None)]
struct Args {
    /// Skip confirmation prompt
    #[arg(short = 'y', long = "yes")]
    yes: bool,
    /// Directory containing the ELF and VK files
    #[arg(long = "elf-dir", value_name = "DIR", default_value = "elf")]
    elf_dir: PathBuf,
}

#[derive(Serialize)]
struct SubmitProverRequest {
    secret: String,
    blob_tx: BlobTransaction,
    prover_request: OrderbookProverRequest,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    // Read node URL from environment variable or use default from conf_defaults.toml
    let node_url =
        env::var("HYLI_NODE_URL").unwrap_or_else(|_| "http://localhost:4321".to_string());
    let server_url =
        env::var("HYLI_SERVER_URL").unwrap_or_else(|_| "http://localhost:9002".to_string());

    env::var("HYLI_REGISTRY_URL").unwrap_or_else(|_| {
        env::set_var("HYLI_REGISTRY_URL", "http://localhost:9003");
        "http://localhost:9003".to_string()
    });
    env::var("HYLI_REGISTRY_API_KEY").unwrap_or_else(|_| {
        env::set_var("HYLI_REGISTRY_API_KEY", "dev");
        "dev".to_string()
    });

    let admin_secret = env::var("HYLI_ADMIN_SECRET").unwrap_or("admin_secret".to_string());

    println!("Connecting to node at: {}", node_url);

    // Create the node API client
    let client =
        NodeApiHttpClient::new(node_url.clone()).context("Failed to create NodeApiHttpClient")?;

    let (elf_bytes, vk_bytes) = read_contract_artifacts(&args.elf_dir)?;

    // Create the program ID from VK
    let program_id = ProgramId::from(vk_bytes.as_slice());

    // Get the contract name (default to "orderbook")
    let contract_name: ContractName = env::var("ORDERBOOK_CN")
        .unwrap_or_else(|_| "orderbook".to_string())
        .into();

    // Check if the contract is already up-to-date
    println!("Checking current contract state...");
    match client.get_contract(contract_name.clone()).await {
        Ok(existing) => {
            if existing.program_id == program_id {
                println!("✓ Contract is already up-to-date!");
                println!(
                    "  Current Program ID: {}",
                    hex::encode(&existing.program_id.0)
                );
                println!("  No upgrade needed.");
                return Ok(());
            }
            println!(
                "Current Program ID: {}",
                hex::encode(&existing.program_id.0)
            );
        }
        Err(e) => {
            println!("⚠️  Could not fetch current contract state: {:#}", e);
        }
    }

    println!(
        "\nUpgrading contract to program ID: {}",
        hex::encode(&program_id.0)
    );

    // Create the orderbook action
    let action = PermissionedOrderbookAction::UpgradeContract(program_id.clone());

    // Generate a random action_id
    let action_id = rand::rng().random::<u32>();
    println!("Using action_id: {}", action_id);

    // Wrap in OrderbookAction with random action_id
    let orderbook_action = OrderbookAction::PermissionedOrderbookAction(action.clone(), action_id);

    // Convert to blob
    let blob = orderbook_action.as_blob(contract_name.clone());

    // Create the blob transaction
    let blob_tx = BlobTransaction::new(ORDERBOOK_ACCOUNT_IDENTITY, vec![blob]);
    let tx_hash = blob_tx.hashed();

    // Ask for confirmation unless -y flag is passed
    if !args.yes {
        println!("\n⚠️  You are about to upgrade the contract!");
        println!("   Node: {}", node_url);
        println!(
            "   Contract: {}",
            env::var("ORDERBOOK_CN").unwrap_or_else(|_| "orderbook".to_string())
        );
        println!("   New Program ID: {}", hex::encode(&program_id.0));
        println!();
        print!("Do you want to proceed? (yes/no): ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim().to_lowercase();

        if input != "yes" && input != "y" {
            println!("Upgrade cancelled.");
            bail!("User cancelled the upgrade");
        }
        println!();
    }

    // Upload the ELF to the registry first
    println!("Uploading ELF to registry...");
    hyli_registry::upload_elf(
        &elf_bytes,
        &hex::encode(&program_id.0),
        &contract_name.to_string(),
        "sp1",
        None,
    )
    .await
    .context("Failed to upload ELF to registry")?;
    println!("✓ ELF uploaded successfully!");

    println!("Sending upgrade transaction...");

    let prover_request = OrderbookProverRequest {
        user_info: UserInfo::new(ORDERBOOK_ACCOUNT_IDENTITY.to_string(), vec![]),
        events: vec![],
        orderbook_action: action,
        nonce: action_id,
        action_private_input: vec![1, 2, 3],
        tx_hash: tx_hash.clone(),
    };

    let endpoint = format!(
        "{}/admin/submit_prover_request",
        server_url.trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .post(endpoint)
        .json(&SubmitProverRequest {
            secret: admin_secret,
            blob_tx,
            prover_request,
        })
        .send()
        .await
        .context("Failed to send request to server")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("Server returned {status}: {body}");
    }

    let response_body = response.text().await.unwrap_or_default();
    let tx_hash_display = serde_json::from_str::<serde_json::Value>(&response_body)
        .ok()
        .and_then(|value| value.as_str().map(|s| s.to_string()))
        .unwrap_or(response_body);

    println!("✓ Upgrade transaction sent successfully!");
    println!("Transaction Hash: {}", tx_hash_display);

    Ok(())
}

fn read_contract_artifacts(dir: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let entries = fs::read_dir(dir)
        .with_context(|| format!("Failed to read contract artifacts directory {}", dir.display()))?;

    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();
        if path.is_file() {
            files.push(path);
        }
    }

    if files.len() != 2 {
        bail!(
            "Expected exactly 2 files in {}, found {}",
            dir.display(),
            files.len()
        );
    }

    let mut vk_path = None;
    let mut elf_path = None;
    for path in files {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if file_name.ends_with("_vk") {
            vk_path = Some(path);
        } else {
            elf_path = Some(path);
        }
    }

    let vk_path = vk_path.ok_or_else(|| {
        anyhow!(
            "No VK file found in {} (expected a filename ending with _vk)",
            dir.display()
        )
    })?;
    let elf_path = elf_path.ok_or_else(|| {
        anyhow!(
            "No ELF file found in {} (expected a non-_vk filename)",
            dir.display()
        )
    })?;

    let elf_bytes = fs::read(&elf_path)
        .with_context(|| format!("Failed to read ELF file {}", elf_path.display()))?;
    let vk_bytes = fs::read(&vk_path)
        .with_context(|| format!("Failed to read VK file {}", vk_path.display()))?;

    Ok((elf_bytes, vk_bytes))
}
