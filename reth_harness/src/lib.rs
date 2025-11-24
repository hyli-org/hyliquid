use std::{fs, path::PathBuf, str::FromStr, sync::Arc};

use alloy_consensus::{SignableTransaction, TxEip1559, TxEip4844Variant, TxEnvelope};
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_genesis::Genesis;
use alloy_primitives::{address, Address, Bytes, TxKind, B256, U256};
use alloy_signer::{Signer, SignerSync};
use alloy_signer_local::PrivateKeySigner;
use eyre::{eyre, Result, WrapErr};
use futures_util::StreamExt;
use reth_db::{
    test_utils::{create_test_rw_db_with_path, TempDatabase},
    DatabaseEnv,
};
use reth_ethereum::{
    chainspec::ChainSpec,
    evm::{revm::primitives::alloy_primitives::TxHash, EthEvmConfig},
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
use serde::Serialize;
use serde_json;
use tempfile::TempDir;
use tracing::{debug, error, info};

pub mod support;

pub struct PreparedDevNode {
    pub node_config: reth_ethereum::node::core::node_config::NodeConfig<ChainSpec>,
    pub datadir: TempDir,
    pub datadir_path: PathBuf,
    pub db_path: PathBuf,
    pub static_path: PathBuf,
    pub chain_spec: Arc<ChainSpec>,
}

pub fn prepare_dev_node(chain_id: u64, extra_alloc: &[Address]) -> Result<PreparedDevNode> {
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

    let chain_spec = support::custom_chain(chain_id, extra_alloc);

    let mut node_config = reth_ethereum::node::core::node_config::NodeConfig::test()
        .dev()
        .with_datadir_args(reth_ethereum::node::core::args::DatadirArgs {
            datadir: reth_ethereum::node::core::dirs::MaybePlatformPath::from(datadir_path.clone()),
            static_files_path: Some(static_path.clone()),
        })
        .with_chain(chain_spec.clone());
    node_config.dev.block_time = None;
    node_config.dev.block_max_transactions = Some(1);

    Ok(PreparedDevNode {
        node_config,
        datadir: temp_dir,
        datadir_path,
        db_path,
        static_path,
        chain_spec,
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
const DEFAULT_PREFUNDED_ACCOUNTS: &[Address] = &[
    address!("f39Fd6e51aad88F6F4ce6aB8827279cffFb92266"),
    address!("70997970C51812dc3A010C7d01b50e0d17dc79C8"),
    address!("3C44CdDdB6a900fa2b585dd299e03d12FA4293BC"),
];

const COLLATERAL_DEPLOY_GAS_LIMIT: u64 = 3_000_000;
const COLLATERAL_DEPLOY_MAX_FEE: u128 = 2_000_000_000;
const COLLATERAL_DEPLOY_PRIORITY_FEE: u128 = 1_500_000_000;
const COLLATERAL_DEPLOY_NONCE: u64 = 0;

#[derive(Clone)]
pub struct CollateralContractInit {
    private_key: String,
    deployer: Address,
    bytecode: Bytes,
    gas_limit: u64,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
    nonce: u64,
}

impl CollateralContractInit {
    pub fn test_erc20(private_key_hex: &str) -> Result<Self> {
        let cleaned = private_key_hex.trim_start_matches("0x");
        let signer = PrivateKeySigner::from_str(cleaned)
            .map_err(|err| eyre!("parsing collateral deployer key: {err}"))?;
        let deployer = signer.address();

        let bytecode = load_test_erc20_bytecode()?;

        Ok(Self {
            private_key: cleaned.to_string(),
            deployer,
            bytecode,
            gas_limit: COLLATERAL_DEPLOY_GAS_LIMIT,
            max_fee_per_gas: COLLATERAL_DEPLOY_MAX_FEE,
            max_priority_fee_per_gas: COLLATERAL_DEPLOY_PRIORITY_FEE,
            nonce: COLLATERAL_DEPLOY_NONCE,
        })
    }

    fn signer(&self, chain_id: u64) -> Result<PrivateKeySigner> {
        let mut signer = PrivateKeySigner::from_str(&self.private_key)
            .map_err(|err| eyre!("parsing collateral deployer key: {err}"))?;
        signer.set_chain_id(Some(chain_id));
        Ok(signer)
    }

    pub fn deployer_address(&self) -> Address {
        self.deployer
    }
}

pub struct SubmittedTx {
    pub tx_hash: String,
    pub stateless_input: Vec<u8>,
    pub block_hash: B256,
    pub block_number: u64,
    pub previous_state_root: B256,
    pub state_root: B256,
    pub contract_address: Option<Address>,
    pub evm_summary: Vec<u8>,
}

pub struct RethHarness {
    #[allow(dead_code)]
    datadir: TempDir,
    handle: NodeHandleFor<EthereumNode, Arc<TempDatabase<DatabaseEnv>>>,
    next_block_number: u64,
    chain_id: u64,
    prefunded_accounts: Vec<Address>,
    collateral_init: Option<CollateralContractInit>,
    collateral_contract: Option<Address>,
    collateral_metadata: Option<CollateralDeployMetadata>,
    chain_spec: Arc<ChainSpec>,
    evm_config: EthEvmConfig,
    previous_state_root: B256,
    _tasks: TaskManager,
}

#[derive(Clone)]
pub struct CollateralDeployMetadata {
    pub contract_address: Address,
    pub block_hash: B256,
    pub block_number: u64,
    pub previous_state_root: B256,
    pub state_root: B256,
}

impl RethHarness {
    pub async fn new(chain_id: u64, prefunded_accounts: Vec<Address>) -> Result<Self> {
        Self::new_internal(chain_id, prefunded_accounts, None).await
    }

    pub async fn new_with_collateral(
        chain_id: u64,
        prefunded_accounts: Vec<Address>,
        collateral: CollateralContractInit,
    ) -> Result<Self> {
        Self::new_internal(chain_id, prefunded_accounts, Some(collateral)).await
    }

    async fn new_internal(
        chain_id: u64,
        prefunded_accounts: Vec<Address>,
        collateral: Option<CollateralContractInit>,
    ) -> Result<Self> {
        let tasks = TaskManager::current();
        let mut combined_accounts = DEFAULT_PREFUNDED_ACCOUNTS.to_vec();
        combined_accounts.extend(prefunded_accounts);
        combined_accounts.sort();
        combined_accounts.dedup();
        let PreparedDevNode {
            node_config,
            datadir,
            datadir_path: _,
            db_path,
            static_path: _,
            chain_spec,
        } = prepare_dev_node(chain_id, &combined_accounts)
            .wrap_err("failed to prepare dev node")?;

        let handle = NodeBuilder::new(node_config)
            .with_database(create_test_rw_db_with_path(db_path.clone()))
            .with_launch_context(tasks.executor())
            .node(EthereumNode::default())
            .launch_with_debug_capabilities()
            .await?;

        let evm_config = handle.node.evm_config.clone();
        let previous_state_root = chain_spec.genesis_header().state_root;

        let mut harness = Self {
            datadir,
            handle,
            next_block_number: 1,
            chain_id,
            prefunded_accounts: combined_accounts,
            collateral_init: collateral.clone(),
            collateral_contract: None,
            collateral_metadata: None,
            chain_spec,
            evm_config,
            previous_state_root,
            _tasks: tasks,
        };

        if let Some(config) = collateral {
            let address = harness.deploy_collateral_contract(&config).await?;
            harness.collateral_contract = Some(address);
        }

        Ok(harness)
    }

    pub async fn submit_raw_tx(&mut self, raw_tx: Vec<u8>) -> Result<SubmittedTx> {
        let eth_api = self.handle.node.rpc_registry.eth_api();
        if let Ok(envelope) = decode_envelope(&raw_tx) {
            if let Ok(sender) = recover_sender(&envelope) {
                let required = estimate_required_balance(&envelope);
                match eth_api.balance(sender, None).await {
                    Ok(balance) => {
                        info!(
                            ?sender,
                            ?required,
                            ?balance,
                            "reth harness replaying collateral tx"
                        );
                    }
                    Err(err) => {
                        error!("failed to fetch balance for {sender:?}: {err}");
                    }
                }
            }
        }
        let debug_api = self.handle.node.rpc_registry.debug_api();

        let tx_hash = match eth_api
            .send_raw_transaction(reth_ethereum::evm::revm::primitives::Bytes(
                reth_ethereum::evm::revm::primitives::bytes::Bytes::copy_from_slice(&raw_tx),
            ))
            .await
        {
            Ok(hash) => hash,
            Err(err) => {
                error!("embedded reth rejected raw tx: {err:#}");
                return Err(eyre!(
                    "failed to submit raw transaction to embedded reth: {err:#}"
                ));
            }
        };

        let mut notifications = self.handle.node.provider.canonical_state_stream();
        let block =
            wait_for_transaction(&mut notifications, tx_hash, &mut self.next_block_number).await?;
        let block_number = block.header().number;
        let block_hash = block.hash();
        let state_root = block.header().state_root;
        let previous_state_root = self.previous_state_root;
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

        info!(
            ?tx_hash,
            block_number, "embedded reth produced execution witness for collateral tx"
        );

        let receipt = eth_api
            .transaction_receipt(tx_hash)
            .await
            .wrap_err("failed to fetch transaction receipt")?
            .ok_or_else(|| eyre!("missing transaction receipt for {tx_hash:?}"))?;
        let contract_address = receipt.contract_address;

        let evm_summary = self
            .build_evm_summary_bytes(previous_state_root, state_root)
            .wrap_err("building evm config summary")?;
        self.previous_state_root = state_root;

        Ok(SubmittedTx {
            tx_hash: format!("0x{tx_hash:x}"),
            stateless_input: stateless_bytes,
            block_hash,
            block_number,
            previous_state_root,
            state_root,
            evm_summary,
            contract_address,
        })
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn prefunded_accounts(&self) -> &[Address] {
        &self.prefunded_accounts
    }

    pub fn collateral_config(&self) -> Option<CollateralContractInit> {
        self.collateral_init.clone()
    }

    pub fn collateral_contract_address(&self) -> Option<Address> {
        self.collateral_contract
    }

    pub fn collateral_metadata(&self) -> Option<CollateralDeployMetadata> {
        self.collateral_metadata.clone()
    }

    async fn deploy_collateral_contract(
        &mut self,
        init: &CollateralContractInit,
    ) -> Result<Address> {
        let signer = init.signer(self.chain_id)?;
        let tx = TxEip1559 {
            chain_id: self.chain_id,
            nonce: init.nonce,
            gas_limit: init.gas_limit,
            max_fee_per_gas: init.max_fee_per_gas,
            max_priority_fee_per_gas: init.max_priority_fee_per_gas,
            to: TxKind::Create,
            value: U256::ZERO,
            access_list: Default::default(),
            input: init.bytecode.clone(),
        };
        let signature = signer
            .sign_hash_sync(&tx.signature_hash())
            .map_err(|err| eyre!("signing collateral deployment tx: {err}"))?;
        let envelope: TxEnvelope = tx.into_signed(signature).into();
        let raw = envelope.encoded_2718().to_vec();

        let submitted = self.submit_raw_tx(raw).await?;
        let contract_address = submitted
            .contract_address
            .ok_or_else(|| eyre!("collateral deployment missing contract address"))?;

        self.collateral_metadata = Some(CollateralDeployMetadata {
            contract_address,
            block_hash: submitted.block_hash,
            block_number: submitted.block_number,
            previous_state_root: submitted.previous_state_root,
            state_root: submitted.state_root,
        });
        self.collateral_contract = Some(contract_address);

        info!(
            ?contract_address,
            deployer = ?init.deployer_address(),
            block = submitted.block_number,
            "deployed collateral contract on embedded reth"
        );

        Ok(contract_address)
    }

    fn build_evm_summary_bytes(
        &self,
        previous_state_root: B256,
        next_state_root: B256,
    ) -> Result<Vec<u8>> {
        let summary = HarnessEvmConfigSummary {
            chain_id: self.chain_spec.chain().id(),
            extra_data: format!(
                "0x{}",
                hex::encode(self.evm_config.block_assembler.extra_data.as_ref())
            ),
            initial_state_root: format_b256(&previous_state_root),
            next_state_root: format_b256(&next_state_root),
            genesis: self.chain_spec.genesis().clone(),
        };

        serde_json::to_vec(&summary).map_err(|err| eyre!("serializing evm config summary: {err}"))
    }
}

#[derive(Serialize)]
struct HarnessEvmConfigSummary {
    chain_id: u64,
    extra_data: String,
    initial_state_root: String,
    next_state_root: String,
    genesis: Genesis,
}

fn format_b256(value: &B256) -> String {
    format!("0x{}", hex::encode(value.as_slice()))
}

fn decode_envelope(raw_tx: &[u8]) -> Result<TxEnvelope> {
    let mut slice = raw_tx;
    TxEnvelope::decode_2718(&mut slice).map_err(|err| eyre!("decoding tx envelope: {err}"))
}

fn recover_sender(envelope: &TxEnvelope) -> Result<Address> {
    let sender = match envelope {
        TxEnvelope::Legacy(tx) => tx.recover_signer(),
        TxEnvelope::Eip2930(tx) => tx.recover_signer(),
        TxEnvelope::Eip1559(tx) => tx.recover_signer(),
        TxEnvelope::Eip4844(tx) => tx.recover_signer(),
        TxEnvelope::Eip7702(tx) => tx.recover_signer(),
    }
    .map_err(|err| eyre!("recovering sender: {err}"))?;
    Ok(sender)
}

fn estimate_required_balance(envelope: &TxEnvelope) -> U256 {
    match envelope {
        TxEnvelope::Legacy(tx) => {
            U256::from(tx.tx().gas_limit) * U256::from(tx.tx().gas_price) + tx.tx().value
        }
        TxEnvelope::Eip2930(tx) => {
            U256::from(tx.tx().gas_limit) * U256::from(tx.tx().gas_price) + tx.tx().value
        }
        TxEnvelope::Eip1559(tx) => {
            U256::from(tx.tx().gas_limit) * U256::from(tx.tx().max_fee_per_gas) + tx.tx().value
        }
        TxEnvelope::Eip4844(tx) => match tx.tx() {
            TxEip4844Variant::TxEip4844(inner) => {
                U256::from(inner.gas_limit) * U256::from(inner.max_fee_per_gas) + inner.value
            }
            TxEip4844Variant::TxEip4844WithSidecar(inner) => {
                U256::from(inner.tx.gas_limit) * U256::from(inner.tx.max_fee_per_gas)
                    + inner.tx.value
            }
        },
        TxEnvelope::Eip7702(tx) => {
            U256::from(tx.tx().gas_limit) * U256::from(tx.tx().max_fee_per_gas) + tx.tx().value
        }
    }
}

const TEST_ERC20_BYTECODE: &str = include_str!("test_erc20_bytecode.hex");

fn load_test_erc20_bytecode() -> Result<Bytes> {
    let trimmed = TEST_ERC20_BYTECODE.trim();
    let hex_bytes = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    let decoded = hex::decode(hex_bytes)
        .map_err(|err| eyre!("decoding embedded collateral bytecode: {err}"))?;
    Ok(Bytes::from(decoded))
}
