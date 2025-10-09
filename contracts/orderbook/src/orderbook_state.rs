use std::collections::{HashMap, HashSet};

use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::SHA256Hasher;
use sparse_merkle_tree::SparseMerkleTree;
use sparse_merkle_tree::{default_store::DefaultStore, traits::Value};

use crate::orderbook::{OrderbookEvent, Symbol};
use crate::orderbook_witness::ZkVmWitness;
use crate::{
    orderbook::{ExecutionState, Orderbook},
    smt_values::{Balance, BorshableH256 as H256, UserInfo},
};

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct LightState {
    pub users_info: HashMap<String, UserInfo>,
    pub balances: HashMap<Symbol, HashMap<H256, Balance>>,
}

#[derive(Default, Debug)]
pub struct FullState {
    pub users_info_mt: SparseMerkleTree<SHA256Hasher, UserInfo, DefaultStore<UserInfo>>,
    pub balances_mt:
        HashMap<Symbol, SparseMerkleTree<SHA256Hasher, Balance, DefaultStore<Balance>>>,
    pub users_info: HashMap<String, UserInfo>,
}

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkVmState {
    pub users_info: ZkVmWitness<HashSet<UserInfo>>,
    pub balances: HashMap<Symbol, ZkVmWitness<HashMap<H256, Balance>>>,
}

/// impl of functions for state management
impl Orderbook {
    pub fn fund_account(
        &mut self,
        symbol: &str,
        user_info: &UserInfo,
        amount: &Balance,
    ) -> Result<(), String> {
        let current_balance = self.get_balance(user_info, symbol);

        self.update_balances(
            symbol,
            vec![(user_info.get_key(), Balance(current_balance.0 + amount.0))],
        )
        .map_err(|e| e.to_string())
    }

    pub fn deduct_from_account(
        &mut self,
        symbol: &str,
        user_info: &UserInfo,
        amount: u64,
    ) -> Result<(), String> {
        let current_balance = self.get_balance(user_info, symbol);

        if current_balance.0 < amount {
            return Err(format!(
                "Insufficient balance: user {} has {} {}, trying to remove {}",
                user_info.user, current_balance.0, symbol, amount
            ));
        }

        self.update_balances(
            symbol,
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
                let new_root = state
                    .users_info_mt
                    .update(user_info.get_key().into(), user_info.clone())
                    .map_err(|e| format!("Failed to update user info in SMT: {e}"))?;
                state
                    .users_info
                    .entry(user_info.user.clone())
                    .or_insert_with(|| user_info.clone());
                self.users_info_merkle_root = (*new_root).into();
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
        symbol: &str,
        balances_to_update: Vec<(H256, Balance)>,
    ) -> Result<(), String> {
        match &mut self.execution_state {
            ExecutionState::Full(state) => {
                let tree = state
                    .balances_mt
                    .entry(symbol.to_string())
                    .or_insert_with(|| {
                        SparseMerkleTree::new(sparse_merkle_tree::H256::zero(), Default::default())
                    });
                let leaves = balances_to_update
                    .iter()
                    .map(|(user_info_key, balance)| ((*user_info_key).into(), balance.clone()))
                    .collect();
                let new_root = tree
                    .update_all(leaves)
                    .map_err(|e| format!("Failed to update balances on {symbol}: {e}"))?;
                self.balances_merkle_roots
                    .insert(symbol.to_string(), (*new_root).into());
            }
            ExecutionState::Light(state) => {
                let symbol_entry = state.balances.entry(symbol.to_string()).or_default();
                for (user_info_key, balance) in balances_to_update {
                    symbol_entry.insert(user_info_key, balance);
                }
                self.balances_merkle_roots
                    .entry(symbol.to_string())
                    .or_insert_with(|| sparse_merkle_tree::H256::zero().into());
            }
            ExecutionState::ZkVm(state) => {
                let witness = state.balances.get(symbol).ok_or_else(|| {
                    format!("No balance witness found for {symbol} while running in ZkVm mode")
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
                    .unwrap_or_else(|e| panic!("Failed to compute new root on {symbol}: {e}"));
                self.balances_merkle_roots
                    .insert(symbol.to_string(), (*new_root).into());
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

impl Clone for FullState {
    fn clone(&self) -> Self {
        let user_info_root = *self.users_info_mt.root();
        let user_info_store = self.users_info_mt.store().clone();
        let users_info_mt = SparseMerkleTree::new(user_info_root, user_info_store);

        let mut balances_mt = HashMap::new();
        for (symbol, tree) in &self.balances_mt {
            let root = *tree.root();
            let store = tree.store().clone();
            let new_tree = SparseMerkleTree::new(root, store);
            balances_mt.insert(symbol.clone(), new_tree);
        }

        Self {
            users_info_mt,
            balances_mt,
            users_info: self.users_info.clone(),
        }
    }
}
