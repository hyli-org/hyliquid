use std::collections::{BTreeMap, HashMap, HashSet};

use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::{BorshableMerkleProof, SHA256Hasher};
use sdk::{BlockHeight, LaneId, StateCommitment};
use sha2::Sha256;
use sha3::Digest;
use sparse_merkle_tree::MerkleProof;

use crate::model::{AssetInfo, ExecuteState, Symbol, UserInfo};
use crate::order_manager::OrderManager;
use crate::zk::smt::{GetKey, UserBalance};

pub use smt::BorshableH256 as H256;
pub use smt::SMT;

mod commitment_metadata;
mod contract;
pub mod smt;

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkVmWitness<T: BorshDeserialize + BorshSerialize + Default> {
    pub value: T,
    pub proof: BorshableMerkleProof,
}

impl<T: BorshDeserialize + BorshSerialize + Default> Default for ZkVmWitness<T> {
    fn default() -> Self {
        ZkVmWitness {
            value: T::default(),
            proof: BorshableMerkleProof(MerkleProof::new(vec![], vec![])),
        }
    }
}

impl<V, T> ZkVmWitness<T>
where
    V: sparse_merkle_tree::traits::Value + GetKey + Clone,
    for<'b> &'b T: IntoIterator<Item = &'b V>,
    T: BorshDeserialize + BorshSerialize + Default,
{
    fn compute_root(&self) -> Result<H256, String> {
        let leaves: Vec<(_, _)> = self
            .value
            .into_iter()
            .map(|v| (v.get_key().into(), v.to_h256()))
            .collect();

        if leaves.is_empty() {
            return Err("No leaves in users_info proof, proof should be empty".to_string());
        }

        let derived_root = self
            .proof
            .0
            .clone()
            .compute_root::<SHA256Hasher>(leaves)
            .map_err(|e| format!("Failed to compute users_info proof root: {e}"))?;

        Ok(derived_root.into())
    }
}

// Full state with commitment structures
#[derive(Default, Debug)]
pub struct FullState {
    pub users_info_mt: SMT<UserInfo>,
    pub balances_mt: HashMap<String, SMT<UserBalance>>,
    pub state: ExecuteState,
    pub hashed_secret: [u8; 32],
    pub lane_id: LaneId,
    pub last_block_number: BlockHeight,
}

impl FullState {
    pub fn from_data(
        light: &ExecuteState,
        secret: Vec<u8>,
        lane_id: LaneId,
        last_block_number: BlockHeight,
    ) -> Result<FullState, String> {
        let mut users_info_mt = SMT::zero();

        users_info_mt
            .update_all_from_ref(light.users_info.values())
            .map_err(|e| format!("Failed to update users info in SMT: {e}"))?;

        let mut balances_mt = HashMap::new();
        for (symbol, symbol_balances) in light.balances.iter() {
            let mut tree = SMT::zero();
            tree.update_all(
                symbol_balances
                    .iter()
                    .map(|(user_info_key, balance)| UserBalance {
                        user_key: user_info_key.clone(),
                        balance: balance.clone(),
                    }),
            )
            .map_err(|e| format!("Failed to update balances on symbol {symbol}: {e}"))?;
            balances_mt.insert(symbol.clone(), tree);
        }

        let hashed_secret = *Sha256::digest(secret)
            .first_chunk::<32>()
            .ok_or("hashing secret failed".to_string())?;

        Ok(FullState {
            users_info_mt,
            balances_mt,
            state: light.clone(),
            hashed_secret,
            lane_id,
            last_block_number,
        })
    }

    pub fn balance_roots(&self) -> HashMap<Symbol, H256> {
        self.balances_mt
            .iter()
            .map(|(symb, user_balances)| (symb.clone(), user_balances.root()))
            .collect()
    }

    pub fn commit(&self) -> StateCommitment {
        StateCommitment(
            borsh::to_vec(&ParsedStateCommitment {
                users_info_root: self.users_info_mt.root(),
                balances_roots: &self.balance_roots(),
                assets: &self.state.assets_info,
                orders: &self.state.order_manager,
                hashed_secret: self.hashed_secret,
                lane_id: &self.lane_id,
                last_block_number: &self.last_block_number,
            })
            .expect("Could not encode onchain state into state commitment"),
        )
    }
}
// Committed state
#[derive(Debug, BorshSerialize, Eq, PartialEq)]
pub struct ParsedStateCommitment<'a> {
    pub users_info_root: H256,
    pub balances_roots: &'a HashMap<Symbol, H256>,
    pub assets: &'a HashMap<Symbol, AssetInfo>,
    pub orders: &'a OrderManager,
    pub hashed_secret: [u8; 32],
    pub lane_id: &'a LaneId,
    pub last_block_number: &'a BlockHeight,
}

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkVmState {
    pub users_info: ZkVmWitness<HashSet<UserInfo>>,
    pub balances: HashMap<Symbol, ZkVmWitness<HashSet<UserBalance>>>,
    pub lane_id: LaneId,
    pub hashed_secret: [u8; 32],
    pub last_block_number: BlockHeight,
    pub order_manager: OrderManager,
    pub assets: HashMap<Symbol, AssetInfo>,
}

/// impl of functions for state management
impl borsh::BorshSerialize for FullState {
    fn serialize<W: std::io::Write>(
        &self,
        _writer: &mut W,
    ) -> std::result::Result<(), std::io::Error> {
        panic!("FullState::serialize: todo!()")
    }
}

impl borsh::BorshDeserialize for FullState {
    fn deserialize_reader<R: std::io::Read>(
        _reader: &mut R,
    ) -> std::result::Result<Self, std::io::Error> {
        panic!("FullState::deserialize: todo!()")
    }
}

impl Clone for FullState {
    fn clone(&self) -> Self {
        let user_info_root = *self.users_info_mt.root();
        let user_info_store = self.users_info_mt.store().clone();
        let users_info_mt = SMT::from_store(user_info_root.into(), user_info_store);

        let mut balances_mt = HashMap::new();
        for (symbol, tree) in &self.balances_mt {
            let root = *tree.root();
            let store = tree.store().clone();
            let new_tree = SMT::from_store(root.into(), store);
            balances_mt.insert(symbol.clone(), new_tree);
        }

        Self {
            users_info_mt,
            balances_mt,
            state: self.state.clone(),
            hashed_secret: self.hashed_secret,
            lane_id: self.lane_id.clone(),
            last_block_number: self.last_block_number,
        }
    }
}

impl<'a> ParsedStateCommitment<'a> {
    // Implementation of functions that are only used by the server.
    // Detects differences between two orderbooks
    // It is used to detect differences between on-chain and db orderbooks
    pub fn diff(&self, other: &ParsedStateCommitment) -> BTreeMap<String, String> {
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

            for (symbol, info) in self.assets {
                match other.assets.get(symbol) {
                    Some(other_info) if other_info == info => {}
                    Some(other_info) => {
                        mismatches.push(format!("{symbol}: {info:?} != {other_info:?}"))
                    }
                    None => mismatches.push(format!("{symbol}: present only on self: {info:?}")),
                }
            }

            for (symbol, info) in other.assets {
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
