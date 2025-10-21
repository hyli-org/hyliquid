use anyhow::{bail, Result};
use borsh::BorshDeserialize;
use client_sdk::{
    contract_indexer::AppError,
    rest_client::{IndexerApiHttpClient, NodeApiClient, NodeApiHttpClient},
};
use orderbook::{
    model::{
        AssetInfo, Balance as OrderbookBalance, ExecuteState, Pair, PairInfo, Symbol, UserInfo,
    },
    order_manager::OrderManager,
    zk::{smt::GetKey, FullState, H256},
};
use reqwest::StatusCode;
use sdk::{
    api::{APIRegisterContract, TransactionStatusDb},
    info, BlockHeight, ContractName, LaneId, ProgramId, StateCommitment,
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
    pub program_id: ProgramId,
    pub initial_state: StateCommitment,
}

pub async fn init_node(
    node: Arc<NodeApiHttpClient>,
    contracts: Vec<ContractInit>,
    check_commitment: bool,
) -> Result<()> {
    for contract in contracts {
        init_contract(&node, contract, check_commitment).await?;
    }
    Ok(())
}

async fn init_contract(
    node: &NodeApiHttpClient,
    contract: ContractInit,
    check_commitment: bool,
) -> Result<()> {
    match node.get_contract(contract.name.clone()).await {
        Ok(existing) => {
            if existing.program_id != contract.program_id {
                bail!(
                    "Invalid program_id for {}. On-chain version is {}, expected {}",
                    contract.name,
                    hex::encode(existing.program_id.0.as_slice()),
                    hex::encode(contract.program_id.0.as_slice()),
                );
            }
            info!("✅ {} contract is up to date", contract.name);
            if check_commitment && contract.initial_state != existing.state_commitment {
                bail!("Invalid state commitment for {}.", contract.name);
            }
        }
        Err(_) => {
            info!("🚀 Registering {} contract", contract.name);
            node.register_contract(APIRegisterContract {
                verifier: sdk::verifiers::SP1_4.into(),
                program_id: contract.program_id,
                state_commitment: contract.initial_state,
                contract_name: contract.name.clone(),
                timeout_window: Some(0),
                ..Default::default()
            })
            .await?;
            wait_contract_state(node, &contract.name).await?;
        }
    }
    Ok(())
}

async fn wait_contract_state(
    node: &NodeApiHttpClient,
    contract: &ContractName,
) -> anyhow::Result<()> {
    timeout(Duration::from_secs(30), async {
        loop {
            let resp = node.get_contract(contract.clone()).await;
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

fn init_empty_orderbook(secret: Vec<u8>, lane_id: LaneId) -> (ExecuteState, FullState) {
    let light = ExecuteState::default();
    let full = FullState::from_data(
        &light,
        secret.clone(),
        lane_id.clone(),
        BlockHeight::default(),
    )
    .expect("building full state");
    (light, full)
}

#[allow(clippy::too_many_arguments)]
pub async fn init_orderbook_from_database(
    lane_id: LaneId,
    secret: Vec<u8>,
    asset_service: Arc<RwLock<AssetService>>,
    user_service: Arc<RwLock<UserService>>,
    book_service: Arc<RwLock<BookService>>,
    node: &NodeApiHttpClient,
    indexer: &IndexerApiHttpClient,
    contract_name: &ContractName,
    check_commitment: bool,
) -> Result<(ExecuteState, FullState), AppError> {
    let asset_service = asset_service.read().await;
    let user_service = user_service.read().await;
    let book_service = book_service.read().await;

    let last_settled_tx = indexer
        .get_last_settled_txid_by_contract(contract_name, Some(vec![TransactionStatusDb::Success]))
        .await?;

    if last_settled_tx.is_none() {
        info!("🔍 No last settled success tx found, initializing orderbook with empty state");
        let (light_orderbook, full_orderbook) = init_empty_orderbook(secret, lane_id);
        return check(node, light_orderbook, full_orderbook).await;
    }
    let last_settled_tx = last_settled_tx.unwrap();

    info!("🔍 Last settled tx found: {}", last_settled_tx);

    let commit_id = asset_service
        .get_commit_id_from_tx_hash(&last_settled_tx.1)
        .await;

    if commit_id.is_none() {
        warn!("🔍 No commit id found for tx hash: {}", last_settled_tx.1);
        warn!("🔍 Initializing orderbook with empty state");
        let (light_orderbook, full_orderbook) = init_empty_orderbook(secret, lane_id);
        return check(node, light_orderbook, full_orderbook).await;
    }

    let commit_id = commit_id.unwrap();

    info!("🔍 Commit id: {}", commit_id);

    let instruments = asset_service.get_all_instruments(commit_id).await?;
    let assets = asset_service.get_all_assets().await;

    let mut pairs_info: BTreeMap<Pair, PairInfo> = BTreeMap::new();
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

        let base_info = AssetInfo::new(
            base_asset.scale as u64,
            ContractName(base_asset.contract_name.clone()),
        );

        let quote_info = AssetInfo::new(
            quote_asset.scale as u64,
            ContractName(quote_asset.contract_name.clone()),
        );

        pairs_info.insert(
            (base_asset.symbol.clone(), quote_asset.symbol.clone()),
            PairInfo {
                base: base_info,
                quote: quote_info,
            },
        );
    }

    let users_info: HashMap<String, UserInfo> = user_service.get_all_users(commit_id).await;
    let mut balances: HashMap<Symbol, HashMap<orderbook::zk::H256, OrderbookBalance>> =
        HashMap::new();

    for user in users_info.values() {
        let user_balances = user_service
            .get_balances_from_commit_id(&user.user, commit_id)
            .await?;
        for balance in user_balances.balances {
            balances
                .entry(balance.symbol.clone())
                .or_default()
                .insert(user.get_key(), OrderbookBalance(balance.total as u64));
        }
    }

    let order_manager = book_service
        .get_order_manager(&users_info, commit_id)
        .await?;

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

    info!("Users info: {:?}", users_info);

    // TODO: load properly the value
    let last_block_height = sdk::BlockHeight(0);

    let light_orderbook = orderbook::model::ExecuteState::from_data(
        pairs_info.clone(),
        order_manager.clone(),
        users_info.clone(),
        balances.clone(),
    )
    .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

    let full_orderbook = FullState::from_data(&light_orderbook, secret, lane_id, last_block_height)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, anyhow::anyhow!(e)))?;

    if !check_commitment {
        info!("🔍 Checking commitment is disabled, skipping");
        return Ok((light_orderbook, full_orderbook));
    }

    check(node, light_orderbook, full_orderbook).await
}

async fn check(
    node: &NodeApiHttpClient,
    light_orderbook: ExecuteState,
    full_orderbook: FullState,
) -> Result<(ExecuteState, FullState), AppError> {
    if let Ok(existing) = node.get_contract(ContractName::from("orderbook")).await {
        let onchain = DebugStateCommitment::from(existing.state_commitment.clone());
        // Log existing & new orderbook and spot diff
        let db_state = DebugStateCommitment::from(full_orderbook.commit());
        let diff = onchain.diff(&db_state);
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

        if db_state != onchain {
            error!("No differences found, but commitment mismatch! Diff algo is broken!");
            error!("Onchain commitment: {:?}", existing.state_commitment);
            error!("DB commitment: {:?}", db_state);
            error!("Onchain state: {:#?}", onchain);
            error!("DB state: {:#?}", db_state);
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

#[derive(Debug, BorshDeserialize, Eq, PartialEq)]
pub struct DebugStateCommitment {
    pub users_info_root: H256,
    pub balances_roots: HashMap<Symbol, H256>,
    pub assets: HashMap<Symbol, AssetInfo>,
    pub orders: OrderManager,
    pub hashed_secret: [u8; 32],
    pub lane_id: LaneId,
    pub last_block_number: BlockHeight,
}

impl From<StateCommitment> for DebugStateCommitment {
    fn from(value: StateCommitment) -> Self {
        borsh::from_slice(&value.0).expect("Failed to deser DebugStateCommitment")
    }
}

impl DebugStateCommitment {
    // Implementation of functions that are only used by the server.
    // Detects differences between two orderbooks
    // It is used to detect differences between on-chain and db orderbooks
    pub fn diff(&self, other: &DebugStateCommitment) -> BTreeMap<String, String> {
        let mut diff = BTreeMap::new();
        if self.hashed_secret != other.hashed_secret {
            diff.insert(
                "hashed_secret".to_string(),
                format!(
                    "{} != {}",
                    hex::encode(self.hashed_secret.as_slice()),
                    hex::encode(other.hashed_secret.as_slice())
                ),
            );
        }

        if self.assets != other.assets {
            let mut mismatches = Vec::new();

            for (symbol, info) in &self.assets {
                match other.assets.get(symbol) {
                    Some(other_info) if other_info == info => {}
                    Some(other_info) => {
                        mismatches.push(format!("{symbol}: {info:?} != {other_info:?}"))
                    }
                    None => mismatches.push(format!("{symbol}: present only on self: {info:?}")),
                }
            }

            for (symbol, info) in &other.assets {
                if !self.assets.contains_key(symbol) {
                    mismatches.push(format!("{symbol}: present only on other: {info:?}"));
                }
            }

            diff.insert("symbols_info".to_string(), mismatches.join("; "));
        }

        if self.lane_id != other.lane_id {
            diff.insert(
                "lane_id".to_string(),
                format!(
                    "{} != {}",
                    hex::encode(&self.lane_id.0 .0),
                    hex::encode(&other.lane_id.0 .0)
                ),
            );
        }

        if self.balances_roots != other.balances_roots {
            diff.insert(
                "balances_merkle_roots".to_string(),
                format!("{:?} != {:?}", self.balances_roots, other.balances_roots),
            );
        }

        if self.users_info_root != other.users_info_root {
            diff.insert(
                "users_info_merkle_root".to_string(),
                format!(
                    "{} != {}",
                    hex::encode(self.users_info_root.as_slice()),
                    hex::encode(other.users_info_root.as_slice())
                ),
            );
        }

        if self.orders != other.orders {
            diff.extend(self.orders.diff(&other.orders));
        }

        diff
    }
}
