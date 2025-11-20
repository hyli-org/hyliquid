use std::{fs, path::PathBuf, sync::Arc};

use eyre::{eyre, Result, WrapErr};
use futures_util::StreamExt;
use reth_db::{
    test_utils::{create_test_rw_db_with_path, TempDatabase},
    DatabaseEnv,
};
use reth_ethereum::{
    chainspec::ChainSpec,
    evm::revm::primitives::alloy_primitives::TxHash,
    node::{
        builder::{NodeBuilder, NodeHandleFor},
        EthereumNode,
    },
    provider::{CanonStateNotificationStream, CanonStateSubscriptions},
    rpc::eth::EthApiServer,
    tasks::TaskManager,
    EthPrimitives,
};
use reth_primitives::Block;
use reth_primitives_traits::RecoveredBlock;
use reth_stateless::{ExecutionWitness, StatelessInput};
use tempfile::TempDir;
use tracing::{debug, info};

pub mod support;

pub struct PreparedDevNode {
    pub node_config: reth_ethereum::node::core::node_config::NodeConfig<ChainSpec>,
    pub datadir: TempDir,
    pub datadir_path: PathBuf,
    pub db_path: PathBuf,
    pub static_path: PathBuf,
}

pub fn prepare_dev_node() -> Result<PreparedDevNode> {
    let mut rpc_args = reth_ethereum::node::core::args::RpcServerArgs::default();
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

    let mut node_config = reth_ethereum::node::core::node_config::NodeConfig::test()
        .dev()
        .with_datadir_args(reth_ethereum::node::core::args::DatadirArgs {
            datadir: reth_ethereum::node::core::dirs::MaybePlatformPath::from(datadir_path.clone()),
            static_files_path: Some(static_path.clone()),
        })
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
    notifications: &mut CanonStateNotificationStream<EthPrimitives>,
    tx_hash: TxHash,
    next_block_number: &mut u64,
) -> Result<RecoveredBlock<Block>> {
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

/// Simple wrapper so consumers can hold onto a harness instance even while the
/// underlying helpers spin up new devnodes per submission. This keeps the API
/// surface open for future persistent-node wiring without changing callers.
pub struct RethHarness {
    #[allow(dead_code)]
    datadir: TempDir,
    handle: NodeHandleFor<EthereumNode, Arc<TempDatabase<DatabaseEnv>>>,
    next_block_number: u64,
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

        let handle = NodeBuilder::new(node_config)
            .with_database(create_test_rw_db_with_path(db_path.clone()))
            .with_launch_context(tasks.executor())
            .node(EthereumNode::default())
            .launch_with_debug_capabilities()
            .await?;

        Ok(Self {
            datadir,
            handle,
            next_block_number: 1,
        })
    }

    pub async fn submit_raw_tx(&mut self, raw_tx: Vec<u8>) -> Result<(String, Vec<u8>)> {
        let eth_api = self.handle.node.rpc_registry.eth_api();
        let debug_api = self.handle.node.rpc_registry.debug_api();

        let tx_hash = eth_api
            .send_raw_transaction(reth_ethereum::evm::revm::primitives::Bytes(
                reth_ethereum::evm::revm::primitives::bytes::Bytes::copy_from_slice(&raw_tx),
            ))
            .await
            .wrap_err("failed to submit raw transaction to embedded reth")?;

        let mut notifications = self.handle.node.provider.canonical_state_stream();
        let block =
            wait_for_transaction(&mut notifications, tx_hash, &mut self.next_block_number).await?;
        let block_number = block.header().number;
        let block_hash = block.hash();
        let witness: ExecutionWitness = debug_api
            .debug_execution_witness_by_block_hash(block_hash)
            .await
            .wrap_err_with(|| {
                format!("failed to fetch execution witness for block #{block_number}")
            })?;

        let stateless = StatelessInput {
            block: block.clone().into_block(),
            witness,
        };
        let stateless_bytes =
            bincode::serialize(&stateless).wrap_err("failed to serialize stateless input")?;

        Ok((format!("0x{tx_hash:x}"), stateless_bytes))
    }
}
