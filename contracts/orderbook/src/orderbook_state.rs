use std::collections::{HashMap, HashSet};

use borsh::{BorshDeserialize, BorshSerialize};
use monotree::{hasher::Sha2, Monotree};
use monotree::{DefaultDatabase, Hash as MonotreeHash};
use sdk::merkle_utils::SHA256Hasher;
use sparse_merkle_tree::traits::Value;

use crate::orderbook::{OrderbookEvent, TokenName};
use crate::orderbook_witness::ZkVmWitness;
use crate::{
    orderbook::{ExecutionState, Orderbook},
    smt_values::{Balance, BorshableH256 as H256, UserInfo},
};

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct LightState {
    pub users_info: HashMap<String, UserInfo>,
    pub balances: HashMap<TokenName, HashMap<H256, Balance>>,
}

#[derive(Debug)]
pub struct FullState {
    pub users_info_mt: MonotreeMap<UserInfo>,
    pub balances_mt: HashMap<TokenName, MonotreeMap<Balance>>,
    pub users_info: HashMap<String, UserInfo>,
}

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkVmState {
    pub users_info: ZkVmWitness<HashSet<UserInfo>>,
    pub balances: HashMap<TokenName, ZkVmWitness<HashMap<H256, Balance>>>,
}

/// impl of functions for state management
impl Orderbook {
    pub fn fund_account(
        &mut self,
        token: &str,
        user_info: &UserInfo,
        amount: &Balance,
    ) -> Result<(), String> {
        let current_balance = self.get_balance(user_info, token);

        self.update_balances(
            token,
            vec![(user_info.get_key(), Balance(current_balance.0 + amount.0))],
        )
        .map_err(|e| e.to_string())
    }

    pub fn deduct_from_account(
        &mut self,
        token: &str,
        user_info: &UserInfo,
        amount: u64,
    ) -> Result<(), String> {
        let current_balance = self.get_balance(user_info, token);

        if current_balance.0 < amount {
            return Err(format!(
                "Insufficient balance: user {} has {} {} tokens, trying to remove {}",
                user_info.user, current_balance.0, token, amount
            ));
        }

        self.update_balances(
            token,
            vec![(user_info.get_key(), Balance(current_balance.0 - amount))],
        )
        .map_err(|e| e.to_string())
    }

    pub fn increment_nonce_and_save_user_info(
        &mut self,
        user_info: &UserInfo,
    ) -> Result<OrderbookEvent, String> {
        let mut updated_user_info = user_info.clone();
        updated_user_info.nonce = updated_user_info
            .nonce
            .checked_add(1)
            .ok_or("Nonce overflow")?;
        self.update_user_info_merkle_root(&updated_user_info)?;
        Ok(OrderbookEvent::NonceIncremented {
            user: user_info.user.clone(),
            nonce: updated_user_info.nonce,
        })
    }

    pub fn update_user_info_merkle_root(&mut self, user_info: &UserInfo) -> Result<(), String> {
        if user_info.nonce == 0 {
            return Err("User info nonce cannot be zero".to_string());
        }
        match &mut self.execution_state {
            ExecutionState::Full(state) => {
                state
                    .users_info_mt
                    .upsert(&user_info.get_key(), user_info.clone())
                    .map_err(|e| format!("Failed to update user info in monotree: {e}"))?;
                state
                    .users_info
                    .insert(user_info.user.clone(), user_info.clone());
                self.users_info_merkle_root = root_to_borshable(state.users_info_mt.root.as_ref());
            }
            ExecutionState::Light(state) => {
                state
                    .users_info
                    .insert(user_info.user.clone(), user_info.clone());
                state
                    .users_info
                    .entry(user_info.user.clone())
                    .or_insert_with(|| user_info.clone());
                self.users_info_merkle_root = sparse_merkle_tree::H256::zero().into();
            }
            ExecutionState::ZkVm(state) => {
                let users_info_proof = &state.users_info.proof;
                let leaves = state
                    .users_info
                    .value
                    .iter()
                    .map(|ui| {
                        if ui.user == user_info.user {
                            (ui.get_key().into(), user_info.to_h256())
                        } else {
                            (ui.get_key().into(), ui.to_h256())
                        }
                    })
                    .collect::<Vec<_>>();
                let new_root = users_info_proof
                    .0
                    .clone()
                    .compute_root::<SHA256Hasher>(leaves)
                    .unwrap_or_else(|e| {
                        panic!("Failed to compute new root on user_info merkle tree: {e}")
                    });
                self.users_info_merkle_root = new_root.into();
            }
        }
        Ok(())
    }

    pub fn update_balances(
        &mut self,
        token: &str,
        balances_to_update: Vec<(H256, Balance)>,
    ) -> Result<(), String> {
        match &mut self.execution_state {
            ExecutionState::Full(state) => {
                let tree = state
                    .balances_mt
                    .entry(token.to_string())
                    .or_insert_with(MonotreeMap::default);
                tree.upsert_batch(&balances_to_update).map_err(|e| {
                    format!("Failed to update balances on token {token} in monotree: {e}")
                })?;
                self.balances_merkle_roots
                    .insert(token.to_string(), root_to_borshable(tree.root.as_ref()));
            }
            ExecutionState::Light(state) => {
                let token_entry = state
                    .balances
                    .get_mut(token)
                    .ok_or_else(|| format!("Token {token} is not found in allowed tokens"))?;
                for (user_info_key, balance) in balances_to_update {
                    token_entry.insert(user_info_key, balance);
                }
                self.balances_merkle_roots
                    .entry(token.to_string())
                    .or_insert_with(|| sparse_merkle_tree::H256::zero().into());
            }
            ExecutionState::ZkVm(state) => {
                let witness = state.balances.get(token).ok_or_else(|| {
                    format!("No balance witness found for token {token} while running in ZkVm mode")
                })?;
                let leaves = balances_to_update
                    .iter()
                    .map(|(user_info_key, balance)| ((*user_info_key).into(), balance.to_h256()))
                    .collect();

                let new_root = &witness
                    .proof
                    .0
                    .clone()
                    .compute_root::<SHA256Hasher>(leaves)
                    .unwrap_or_else(|e| panic!("Failed to compute new root on token {token}: {e}"));
                self.balances_merkle_roots
                    .insert(token.to_string(), (*new_root).into());
            }
        }

        Ok(())
    }
}

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

type Sha256Monotree = Monotree<DefaultDatabase, Sha2>;

fn root_to_borshable(root: Option<&MonotreeHash>) -> H256 {
    root.map(|bytes| sparse_merkle_tree::H256::from(*bytes).into())
        .unwrap_or_else(|| sparse_merkle_tree::H256::zero().into())
}

pub struct MonotreeMap<T: Value> {
    pub tree: Sha256Monotree,
    pub root: Option<MonotreeHash>,
    pub data: HashMap<H256, T>,
}

impl<T: Value> Default for MonotreeMap<T> {
    fn default() -> Self {
        Self::new("monotree")
    }
}

impl<T: Value> MonotreeMap<T> {
    pub fn new(namespace: &str) -> Self {
        Self {
            tree: Sha256Monotree::new(namespace),
            root: None,
            data: HashMap::new(),
        }
    }

    pub fn get(&self, key: &H256) -> Option<&T> {
        self.data.get(key)
    }

    pub fn upsert(&mut self, key: &H256, value: T) -> monotree::Result<()> {
        let key_bytes: [u8; 32] = (*key).into();
        let leaf_hash: [u8; 32] = value.to_h256().into();
        self.root = self
            .tree
            .insert(self.root.as_ref(), &key_bytes, &leaf_hash)?;
        self.data.insert(*key, value);
        Ok(())
    }

    pub fn upsert_batch(&mut self, entries: &[(H256, T)]) -> monotree::Result<()>
    where
        T: Clone,
    {
        if entries.is_empty() {
            return Ok(());
        }

        self.tree.prepare();
        let mut current_root = self.root;
        for (key, value) in entries.iter() {
            let key_bytes: [u8; 32] = (*key).into();
            let leaf_hash: [u8; 32] = value.to_h256().into();
            current_root = self
                .tree
                .insert(current_root.as_ref(), &key_bytes, &leaf_hash)?;
            self.data.insert(*key, value.clone());
        }
        self.tree.commit();
        self.root = current_root;
        Ok(())
    }
}
impl<T: Value + Clone> Clone for MonotreeMap<T> {
    fn clone(&self) -> Self {
        let mut clone = MonotreeMap::new("monotree-clone");
        clone.data = self.data.clone();

        let mut root = None;
        clone.tree.prepare();
        for (key, value) in &self.data {
            let key_bytes: [u8; 32] = (*key).into();
            let leaf_hash = value.to_h256();
            let leaf_bytes: [u8; 32] = leaf_hash.into();
            root = clone
                .tree
                .insert(root.as_ref(), &key_bytes, &leaf_bytes)
                .expect("Failed to insert into monotree clone");
        }
        clone.tree.commit();
        clone.root = root;
        clone
    }
}

impl<T: Value> std::fmt::Debug for MonotreeMap<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonotreeMap")
            .field("root", &self.root.map(hex::encode))
            .field("entries", &self.data.len())
            .finish()
    }
}

impl Default for FullState {
    fn default() -> Self {
        Self {
            users_info_mt: MonotreeMap::new("users-info"),
            balances_mt: HashMap::new(),
            users_info: HashMap::new(),
        }
    }
}

impl Clone for FullState {
    fn clone(&self) -> Self {
        let mut balances_mt = HashMap::new();
        for (token_name, tree) in &self.balances_mt {
            balances_mt.insert(token_name.clone(), tree.clone());
        }

        Self {
            users_info_mt: self.users_info_mt.clone(),
            balances_mt,
            users_info: self.users_info.clone(),
        }
    }
}
