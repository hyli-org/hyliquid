use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use client_sdk::rest_client::NodeApiHttpClient;
use sdk::ContractName;
use server::conf::Conf;
use server::reth_utils::{
    load_collateral_metadata, register_reth_collateral_contract,
    register_reth_collateral_with_data, COLLATERAL_METADATA_FILE,
};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Registers the ERC20 collateral contract with the Hyli node (one-time helper)"
)]
struct Args {
    #[arg(long, default_value = "config.toml")]
    config_file: Vec<String>,
    #[arg(long, default_value = "orderbook")]
    orderbook_cn: String,
    #[arg(long)]
    collateral_token_cn: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let conf = Arc::new(Conf::new(args.config_file.clone()).context("reading config file")?);

    let collateral = args
        .collateral_token_cn
        .clone()
        .unwrap_or_else(|| format!("reth-collateral-{}", args.orderbook_cn));

    let node_client =
        NodeApiHttpClient::new(conf.node_url.clone()).context("building Hyli client")?;

    let contract_name = ContractName(collateral.clone());
    let orderbook_name = ContractName(args.orderbook_cn.clone());
    let metadata_path = conf.data_directory.join(COLLATERAL_METADATA_FILE);

    if let Ok(metadata) = load_collateral_metadata(&metadata_path) {
        println!(
            "Using embedded collateral metadata from {}",
            metadata_path.display()
        );
        register_reth_collateral_with_data(&node_client, &contract_name, &orderbook_name, metadata)
            .await?;
    } else {
        register_reth_collateral_contract(
            &node_client,
            &contract_name,
            &orderbook_name,
            &conf.bridge,
        )
        .await?;
    }

    println!(
        "Registered collateral contract {} for orderbook {}",
        collateral, args.orderbook_cn
    );

    Ok(())
}
