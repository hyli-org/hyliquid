use std::{fs, path::PathBuf, sync::Arc};

use alloy_primitives::{B256, Bytes};
use bincode;
use eyre::{Result, WrapErr, eyre};
use futures_util::StreamExt;
use alloy_primitives::address;
use reth_db::test_utils::create_test_rw_db_with_path;
use reth_ethereum::{
    node::{
        EthereumNode,
        builder::{NodeBuilder, NodeHandle, components::NoopNetworkBuilder},
        node::EthereumAddOns,
    },
    provider::CanonStateNotificationStream,
    tasks::TaskManager,
};
use reth_ethereum_primitives as _;
use reth_primitives_traits::Block as BlockTrait;
use reth_stateless::{ExecutionWitness, StatelessInput};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tracing::{debug, info};

use reth_ethereum::chainspec::ChainSpec;
use reth_ethereum::node::core::{
    args::{DatadirArgs, RpcServerArgs},
    dirs::MaybePlatformPath,
    node_config::NodeConfig,
};

pub mod support;

const CHAIN_ID: u64 = 2600;

/// Simplified harness wrapping an embedded Reth devnode plus helpers to submit
/// transactions and build stateless proof inputs from execution witnesses.
pub struct RethHarness {
    _datadir: TempDir,
    node_handle: NodeHandle<EthereumNode>,
    notifications: CanonStateNotificationStream,
    pub chain_spec: Arc<ChainSpec>,
    pub evm_config: reth_ethereum::evm::EthEvmConfig,
    pub next_block_number: u64,
}

impl RethHarness {
    pub async fn new() -> Result<Self> {
        let tasks = TaskManager::current();
        let PreparedDevNode {
            node_config,
            datadir,
            datadir_path: _,
            db_path,
            static_path: _,
        } = prepare_dev_node().wrap_err("failed to prepare dev node")?;

        let mut node_handle = NodeBuilder::new(node_config)
            .with_database(create_test_rw_db_with_path(db_path.clone()))
            .with_launch_context(tasks.executor())
            .with_types::<EthereumNode>()
            .with_components(EthereumNode::components().network(NoopNetworkBuilder::eth()))
            .with_add_ons(EthereumAddOns::default())
            .launch_with_debug_capabilities()
            .await?;

        let notifications = node_handle.node.provider.canonical_state_stream();
        let chain_spec = node_handle.node.chain_spec();
        let evm_config = node_handle.node.evm_config.clone();

        Ok(Self {
            _datadir: datadir,
            node_handle,
            notifications,
            chain_spec,
            evm_config,
            next_block_number: 1,
        })
    }

    /// Submit a raw EIP-1559 transaction, wait for inclusion, and return the tx
    /// hash plus a serialized stateless input (block + execution witness).
    pub async fn submit_raw_tx(&mut self, raw_tx: Vec<u8>) -> Result<(String, Vec<u8>)> {
        let eth_api = self.node_handle.node.rpc_registry.eth_api();
        let debug_api = self.node_handle.node.rpc_registry.debug_api();

        let tx_hash = eth_api
            .send_raw_transaction(Bytes::from(raw_tx.clone()))
            .await
            .wrap_err("failed to submit raw transaction to embedded reth")?;

        let block = wait_for_transaction(
            &mut self.notifications,
            tx_hash,
            &mut self.next_block_number,
        )
        .await?;
        let block_number = block.header().number;
        let block_hash = block.hash();
        let witness = debug_api
            .debug_execution_witness_by_block_hash(block_hash)
            .await
            .wrap_err_with(|| format!("failed to fetch execution witness for block #{block_number}"))?;

        let stateless = StatelessInput {
            block: block.clone().into_block(),
            witness,
        };
        let stateless_bytes = bincode::serialize(&stateless)
            .wrap_err("failed to serialize stateless input")?;

        Ok((format!("0x{tx_hash:x}"), stateless_bytes))
    }

}

/// Configuration bundle for spinning up a dev node.
pub struct PreparedDevNode {
    pub node_config: NodeConfig<ChainSpec>,
    pub datadir: TempDir,
    pub datadir_path: PathBuf,
    pub db_path: PathBuf,
    pub static_path: PathBuf,
}

pub fn prepare_dev_node() -> Result<PreparedDevNode> {
    let mut rpc_args = RpcServerArgs::default();
    rpc_args.http = false;
    rpc_args.ws = false;
    rpc_args.ipcdisable = true;
    rpc_args.disable_auth_server = true;

    let temp_dir = tempfile::tempdir().wrap_err("failed to create temporary datadir")?;
    let datadir_path = temp_dir.path().to_path_buf();
    let static_path = datadir_path.join("static");
    fs::create_dir_all(&static_path).wrap_err("failed to create static files directory")?;
    info!(
        ?datadir_path,
        ?static_path,
        "prepared ephemeral datadir for dev node"
    );

    let db_path = datadir_path.join("db");
    fs::create_dir_all(&db_path).wrap_err("failed to create database directory")?;

    let mut node_config = NodeConfig::test()
        .dev()
        .with_datadir_args(DatadirArgs {
            datadir: MaybePlatformPath::from(datadir_path.clone()),
            static_files_path: Some(static_path.clone()),
        })
        .with_rpc(rpc_args)
        .with_chain(support::custom_chain());
    node_config.dev.block_time = None;
    node_config.dev.block_max_transactions = Some(1);

    Ok(PreparedDevNode {
        node_config,
        datadir: temp_dir,
        datadir_path,
        db_path,
        static_path,
    })
}

pub async fn wait_for_transaction(
    notifications: &mut CanonStateNotificationStream,
    tx_hash: B256,
    next_block_number: &mut u64,
) -> Result<reth_primitives_traits::RecoveredBlock<reth_ethereum_primitives::Block>> {
    loop {
        let notification = notifications
            .next()
            .await
            .ok_or_else(|| eyre!("canonical state stream closed unexpectedly"))?;

        let tip = notification.tip_checked();
        let tip_number = tip.as_ref().map(|block| block.header().number);
        let tip_hash = tip.as_ref().map(|block| block.hash());
        debug!(
            ?tip_number,
            ?tip_hash,
            ?tx_hash,
            "received canonical state notification"
        );

        let Some(block) = tip else {
            debug!("notification contained no canonical tip; waiting for next update");
            continue;
        };

        let block_number = block.header().number;
        if block_number != *next_block_number {
            continue;
        }

        let mut txs = block.body().transactions();
        let mined_tx = txs
            .next()
            .ok_or_else(|| eyre!("expected at least one transaction in block"))?;
        eyre::ensure!(
            txs.next().is_none(),
            "block #{} contained more than one transaction",
            block_number
        );
        eyre::ensure!(
            *mined_tx.tx_hash() == tx_hash,
            "block #{} mined unexpected transaction. expected {tx_hash:?}, got {:?}",
            block_number,
            mined_tx.tx_hash()
        );

        *next_block_number += 1;
        return Ok(block.clone());
    }
}
