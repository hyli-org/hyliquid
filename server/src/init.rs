use anyhow::{bail, Result};
use client_sdk::{
    contract_indexer::AppError,
    rest_client::{IndexerApiHttpClient, NodeApiClient, NodeApiHttpClient},
};
use orderbook::{
    orderbook::{ExecutionMode, Orderbook, PairInfo, TokenName, TokenPair},
    smt_values::{Balance, BorshableH256, UserInfo},
};
use reqwest::StatusCode;
use sdk::{
    api::APIRegisterContract, info, ContractName, LaneId, ProgramId, StateCommitment, ZkContract,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};
use tokio::{sync::RwLock, time::timeout};
use tracing::{error, warn};

use crate::services::{
    asset_service::AssetService, book_service::BookService, user_service::UserService,
};

pub struct ContractInit {
    pub name: ContractName,
    pub program_id: Vec<u8>,
    pub initial_state: StateCommitment,
}

pub async fn init_node(
    node: Arc<NodeApiHttpClient>,
    indexer: Arc<IndexerApiHttpClient>,
    contracts: Vec<ContractInit>,
    check_commitment: bool,
) -> Result<()> {
    for contract in contracts {
        init_contract(&node, &indexer, contract, check_commitment).await?;
    }
    Ok(())
}

async fn init_contract(
    node: &NodeApiHttpClient,
    indexer: &IndexerApiHttpClient,
    contract: ContractInit,
    check_commitment: bool,
) -> Result<()> {
    match indexer.get_indexer_contract(&contract.name).await {
        Ok(existing) => {
            let onchain_program_id = hex::encode(existing.program_id.as_slice());
            let program_id = hex::encode(contract.program_id);
            if onchain_program_id != program_id {
                bail!(
                    "Invalid program_id for {}. On-chain version is {}, expected {}",
                    contract.name,
                    onchain_program_id,
                    program_id
                );
            }
            info!("✅ {} contract is up to date", contract.name);
            if check_commitment && contract.initial_state.0 != existing.state_commitment {
                bail!("Invalid state commitment for {}.", contract.name);
            }
        }
        Err(_) => {
            info!("🚀 Registering {} contract", contract.name);
            node.register_contract(APIRegisterContract {
                verifier: sdk::verifiers::SP1_4.into(),
                program_id: ProgramId(contract.program_id.to_vec()),
                state_commitment: contract.initial_state,
                contract_name: contract.name.clone(),
                ..Default::default()
            })
            .await?;
            wait_contract_state(indexer, &contract.name).await?;
        }
    }
    Ok(())
}

async fn wait_contract_state(
    indexer: &IndexerApiHttpClient,
    contract: &ContractName,
) -> anyhow::Result<()> {
    timeout(Duration::from_secs(30), async {
        loop {
            let resp = indexer.get_indexer_contract(contract).await;
            if resp.is_err() {
                info!("⏰ Waiting for contract {contract} state to be ready");
                tokio::time::sleep(Duration::from_millis(500)).await;
            } else {
                return Ok(());
            }
        }
    })
    .await?
}

pub async fn init_orderbook_from_database(
    lane_id: LaneId,
    secret: Vec<u8>,
    asset_service: Arc<RwLock<AssetService>>,
    user_service: Arc<RwLock<UserService>>,
    book_service: Arc<RwLock<BookService>>,
    node: &NodeApiHttpClient,
    check_commitment: bool,
) -> Result<(Orderbook, Orderbook), AppError> {
    let asset_service = asset_service.read().await;
    let user_service = user_service.read().await;
    let book_service = book_service.read().await;

    let instruments = asset_service.get_all_instruments().await;
    let assets = asset_service.get_all_assets().await;

    let mut pairs_info: BTreeMap<TokenPair, PairInfo> = BTreeMap::new();
    for (_, instrument) in instruments.iter() {
        let base_asset_symbol = instrument.symbol.split('/').next().unwrap();
        let quote_asset_symbol = instrument.symbol.split('/').nth(1).unwrap();

        let base_asset = assets.get(base_asset_symbol).ok_or_else(|| {
            AppError(
                StatusCode::NOT_FOUND,
                anyhow::anyhow!("Base asset not found: {base_asset_symbol}"),
            )
        })?;

        let quote_asset = assets.get(quote_asset_symbol).ok_or_else(|| {
            AppError(
                StatusCode::NOT_FOUND,
                anyhow::anyhow!("Quote asset not found: {quote_asset_symbol}"),
            )
        })?;

        pairs_info.insert(
            (base_asset.symbol.clone(), quote_asset.symbol.clone()),
            PairInfo {
                base_scale: base_asset.scale as u64,
                quote_scale: quote_asset.scale as u64,
            },
        );
    }

    let users_info: HashMap<String, UserInfo> = user_service.get_all_users().await;
    let mut balances: HashMap<TokenName, HashMap<BorshableH256, Balance>> = HashMap::new();

    for user in users_info.values() {
        let balance = user_service.get_balances(&user.user).await?;
        for balance in balance.balances {
            balances
                .entry(balance.token.clone())
                .or_default()
                .insert(user.get_key(), Balance(balance.total as u64));
        }
    }

    let order_manager = book_service.get_order_manager(&users_info).await?;

    // Log some statistics about loaded data
    info!("✅ Users info loaded: {}", users_info.len());
    info!("✅ Balances loaded: {}", balances.len());
    info!("✅ Pairs info loaded: {}", pairs_info.len());
    info!(
        "✅ Orders loaded: {} (buy: {}, sell: {})",
        order_manager.orders.len(),
        order_manager
            .buy_orders
            .values()
            .map(|orders| orders.len())
            .sum::<usize>(),
        order_manager
            .sell_orders
            .values()
            .map(|orders| orders.len())
            .sum::<usize>(),
    );

    let light_orderbook = Orderbook::from_data(
        lane_id.clone(),
        ExecutionMode::Light,
        secret.clone(),
        pairs_info.clone().into_iter().collect(),
        order_manager.clone(),
        users_info.clone(),
        balances.clone(),
    )
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;
    let full_orderbook = Orderbook::from_data(
        lane_id,
        ExecutionMode::Full,
        secret,
        pairs_info,
        order_manager,
        users_info,
        balances,
    )
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

    if light_orderbook.commit() != full_orderbook.commit() {
        return Err(AppError(
            StatusCode::INTERNAL_SERVER_ERROR,
            anyhow::anyhow!("Business logic error: Light and full orderbook commitments mismatch"),
        ));
    }

    if !check_commitment {
        info!("🔍 Checking commitment is disabled, skipping");
        return Ok((light_orderbook, full_orderbook));
    }

    if let Ok(existing) = node.get_contract(ContractName::from("orderbook")).await {
        let onchain = Orderbook::from(existing.state_commitment.clone());
        // Log existing & new orderbook and spot diff
        let diff = onchain.diff(&light_orderbook);
        if !diff.is_empty() {
            warn!("⚠️ Differences (onchain vs db):");
            for (key, value) in diff.iter() {
                warn!("  {}: {}", key, value);
            }

            return Err(AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                anyhow::anyhow!("Differences found"),
            ));
        }
        info!("✅ No differences found between onchain and db");

        let commit = light_orderbook.commit();
        if commit != existing.state_commitment {
            error!("No differences found, but commitment mismatch! Diff algo is broken!");
            return Err(AppError(
                StatusCode::INTERNAL_SERVER_ERROR,
                anyhow::anyhow!("Commitment mismatch"),
            ));
        }
        info!("✅ Commitment matches");
    } else {
        info!("🔍 No onchain contract found, can't check for differences");
    }

    Ok((light_orderbook, full_orderbook))
}
