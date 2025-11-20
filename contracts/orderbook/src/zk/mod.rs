use std::collections::{BTreeMap, HashMap, HashSet};

use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::BorshableMerkleProof;
use sdk::{BlockHeight, LaneId, StateCommitment};
use sha3::{Digest, Sha3_256};
use sparse_merkle_tree::traits::Value;

use crate::model::{AssetInfo, ExecuteState, Symbol, UserInfo};
use crate::zk::order_merkle::OrderManagerWitnesses;
use crate::zk::smt::{GetKey, SHA3_256Hasher, UserBalance};

pub use smt::BorshableH256 as H256;
pub use smt::SMT;

mod commitment_metadata;
mod contract;
mod order_merkle;
pub mod smt;

pub use order_merkle::{OrderManagerMerkles, OrderManagerRoots};

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
enum Proof {
    Some(BorshableMerkleProof),
    CurrentRootHash(H256),
}

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkWitnessSet<
    T: BorshDeserialize
        + BorshSerialize
        + sparse_merkle_tree::traits::Value
        + GetKey
        + Ord
        + Eq
        + std::hash::Hash
        + Clone,
> {
    // TODO: we might want to use initial_values and updated_values
    // Could we then say that all values that have not been updated will be reset to 0 (and hence removed from the tree)?
    values: HashSet<T>,
    proof: Proof,
}

impl<
        T: BorshDeserialize
            + BorshSerialize
            + sparse_merkle_tree::traits::Value
            + GetKey
            + Ord
            + Eq
            + std::hash::Hash
            + std::fmt::Debug
            + Clone,
    > ZkWitnessSet<T>
{
    fn compute_root(&self) -> Result<H256, String> {
        match &self.proof {
            Proof::CurrentRootHash(root_hash) => Ok(*root_hash),
            Proof::Some(proof) => {
                let leaves: Vec<(_, _)> = self
                    .values
                    .iter()
                    .map(|v| (v.get_key().into(), v.to_h256()))
                    .collect();

                if leaves.is_empty() {
                    return Err("No leaves in merkle proof, proof should be empty".to_string());
                }

                let derived_root = proof
                    .0
                    .clone()
                    .compute_root::<SHA3_256Hasher>(leaves)
                    .map_err(|e| format!("Failed to compute users_info proof root: {e}"))?;

                Ok(derived_root.into())
            }
        }
    }
}

impl<
        T: BorshDeserialize
            + BorshSerialize
            + sparse_merkle_tree::traits::Value
            + GetKey
            + Ord
            + Eq
            + std::hash::Hash
            + Clone,
    > Default for ZkWitnessSet<T>
{
    fn default() -> Self {
        ZkWitnessSet {
            values: HashSet::new(),
            proof: Proof::CurrentRootHash(H256::zero()),
        }
    }
}

// Full state with commitment structures
#[derive(Default, Debug)]
pub struct FullState {
    pub users_info_mt: SMT<UserInfo>,
    pub balances_mt: HashMap<String, SMT<UserBalance>>,
    pub order_manager_mt: OrderManagerMerkles,
    pub state: ExecuteState,
    pub hashed_secret: [u8; 32],
    pub lane_id: LaneId,
    pub last_block_number: BlockHeight,
}

impl FullState {
    fn resolve_user_from_state(&self, fallback: &UserInfo, user: &str) -> Result<UserInfo, String> {
        match self.state.get_user_info(user) {
            Ok(ui) => Ok(ui),
            Err(_) if fallback.user == user => Ok(fallback.clone()),
            Err(e) => Err(e),
        }
    }

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
                        user_key: *user_info_key,
                        balance: balance.clone(),
                    }),
            )
            .map_err(|e| format!("Failed to update balances on symbol {symbol}: {e}"))?;
            balances_mt.insert(symbol.clone(), tree);
        }
        let hashed_secret: [u8; 32] = Sha3_256::digest(secret).into();

        let order_manager_mt = OrderManagerMerkles::from_order_manager(&light.order_manager)
            .map_err(|e| format!("Failed to build order manager SMTs from execute state: {e}"))?;

        Ok(FullState {
            users_info_mt,
            balances_mt,
            order_manager_mt,
            state: light.clone(),
            hashed_secret,
            lane_id,
            last_block_number,
        })
    }

    pub fn balance_roots(&self) -> BTreeMap<Symbol, H256> {
        self.balances_mt
            .iter()
            .filter_map(|(symb, user_balances)| {
                let root = user_balances.root();
                if root == H256::zero() {
                    None
                } else {
                    Some((symb.clone(), root))
                }
            })
            .collect()
    }

    pub fn commit(&self) -> StateCommitment {
        let order_manager_roots = self.order_manager_mt.commitment();
        StateCommitment(
            borsh::to_vec(&ParsedStateCommitment {
                users_info_root: self.users_info_mt.root(),
                balances_roots: self.balance_roots(),
                assets: self.state.assets_info.iter().collect::<BTreeMap<_, _>>(),
                order_manager_roots,
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
    pub balances_roots: BTreeMap<Symbol, H256>,
    pub assets: BTreeMap<&'a Symbol, &'a AssetInfo>,
    pub order_manager_roots: OrderManagerRoots,
    pub hashed_secret: [u8; 32],
    pub lane_id: &'a LaneId,
    pub last_block_number: &'a BlockHeight,
}

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkVmState {
    pub users_info: ZkWitnessSet<UserInfo>,
    pub balances: HashMap<Symbol, ZkWitnessSet<UserBalance>>,
    pub lane_id: LaneId,
    pub hashed_secret: [u8; 32],
    pub last_block_number: BlockHeight,
    pub order_manager: OrderManagerWitnesses,
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

        let order_manager_mt = OrderManagerMerkles::from_order_manager(&self.state.order_manager)
            .expect("clone order manager merkle trees");

        Self {
            users_info_mt,
            balances_mt,
            order_manager_mt,
            state: self.state.clone(),
            hashed_secret: self.hashed_secret,
            lane_id: self.lane_id.clone(),
            last_block_number: self.last_block_number,
        }
    }
}
