use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::{BorshableMerkleProof, SHA256Hasher};
use sparse_merkle_tree::traits::Value;
use sparse_merkle_tree::MerkleProof;
use std::collections::{HashMap, HashSet};

use crate::{
    orderbook::{ExecutionMode, ExecutionState, Order, Orderbook, OrderbookEvent, TokenName},
    orderbook_state::ZkVmState,
    smt_values::{Balance, BorshableH256 as H256, UserInfo},
};

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

/// impl of functions for zkvm state generation and verification
impl Orderbook {
    fn create_users_info_witness(
        &self,
        users: &HashSet<UserInfo>,
    ) -> Result<ZkVmWitness<HashSet<UserInfo>>, String> {
        let proof = self.get_users_info_proofs(users)?;
        let mut set = HashSet::new();
        for user in users {
            set.insert(user.clone());
        }
        Ok(ZkVmWitness { value: set, proof })
    }

    fn create_balances_witness(
        &self,
        token: &TokenName,
        users: &[UserInfo],
    ) -> Result<ZkVmWitness<HashMap<H256, Balance>>, String> {
        let (balances, proof) = self.get_balances_with_proof(users, token)?;
        let mut map = HashMap::new();
        for (user_info, balance) in balances {
            map.insert(user_info.get_key(), balance);
        }
        Ok(ZkVmWitness { value: map, proof })
    }

    fn get_users_info_proofs(
        &self,
        users_info: &HashSet<UserInfo>,
    ) -> Result<BorshableMerkleProof, String> {
        if users_info.is_empty() {
            return Ok(BorshableMerkleProof(MerkleProof::new(vec![], vec![])));
        }
        match &self.execution_state {
            ExecutionState::Full(state) => Ok(BorshableMerkleProof(
                state
                    .users_info_mt
                    .merkle_proof(
                        users_info
                            .iter()
                            .map(|u| u.get_key().into())
                            .collect::<Vec<_>>(),
                    )
                    .map_err(|e| {
                        format!("Failed to create merkle proof for users {users_info:?}: {e}")
                    })?,
            )),
            ExecutionState::Light(_) => {
                Err("Light execution mode does not maintain merkle proofs".to_string())
            }
            ExecutionState::ZkVm(_) => {
                Err("ZkVm execution mode cannot generate merkle proofs".to_string())
            }
        }
    }

    fn get_balances_with_proof(
        &self,
        users_info: &[UserInfo],
        token: &TokenName,
    ) -> Result<(HashMap<UserInfo, Balance>, BorshableMerkleProof), String> {
        if users_info.is_empty() {
            return Ok((
                HashMap::new(),
                BorshableMerkleProof(MerkleProof::new(vec![], vec![])),
            ));
        }
        match &self.execution_state {
            ExecutionState::Full(state) => {
                let mut balances_map = HashMap::new();
                for user_info in users_info {
                    let balance = self.get_balance(user_info, token);
                    balances_map.insert(user_info.clone(), balance);
                }

                let users: Vec<UserInfo> = balances_map.keys().cloned().collect();
                let tree = state
                    .balances_mt
                    .get(token)
                    .ok_or_else(|| format!("No balances tree found for token {token}"))?;
                let proof = BorshableMerkleProof(
                    tree.merkle_proof(users.iter().map(|u| u.get_key().into()).collect::<Vec<_>>())
                        .map_err(|e| {
                            format!(
                                "Failed to create merkle proof for token {token} and users {:?}: {e}",
                                users_info
                                    .iter()
                                    .map(|u| u.user.clone())
                                    .collect::<Vec<_>>()
                            )
                        })?,
                );

                Ok((balances_map, proof))
            }
            ExecutionState::Light(_) => {
                Err("Light execution mode does not maintain merkle proofs".to_string())
            }
            ExecutionState::ZkVm(_) => {
                Err("ZkVm execution mode cannot generate merkle proofs".to_string())
            }
        }
    }

    pub fn get_user_info_from_key(&self, key: &H256) -> Result<UserInfo, String> {
        match &self.execution_state {
            ExecutionState::Full(state) => state.users_info_mt.get(key).map_err(|e| {
                format!(
                    "No user info found for key {:?}: {}",
                    hex::encode(key.as_slice()),
                    e
                )
            }),
            ExecutionState::Light(state) => state
                .users_info
                .iter()
                .find(|(_, info)| info.get_key() == *key)
                .map(|(_, info)| info.clone())
                .ok_or_else(|| {
                    format!(
                        "No user info found for key {:?}",
                        hex::encode(key.as_slice())
                    )
                }),
            ExecutionState::ZkVm(state) => state
                .users_info
                .value
                .iter()
                .find(|user_info| user_info.get_key() == *key)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "No user info found for key {:?}",
                        hex::encode(key.as_slice())
                    )
                }),
        }
    }

    pub fn verify_users_info_proof(&self) -> Result<(), String> {
        // Verification only needed in ZkVm mode
        match &self.execution_state {
            ExecutionState::Light(_) | ExecutionState::Full(_) => Ok(()),
            ExecutionState::ZkVm(state) => {
                if self.users_info_merkle_root == sparse_merkle_tree::H256::zero().into() {
                    return Ok(());
                }

                let leaves = state
                    .users_info
                    .value
                    .iter()
                    .map(|user_info| (user_info.get_key().into(), user_info.to_h256()))
                    .collect::<Vec<_>>();

                if leaves.is_empty() {
                    if state.users_info.proof.0 == MerkleProof::new(vec![], vec![]) {
                        return Ok(());
                    }
                    return Err("No leaves in users_info proof, proof should be empty".to_string());
                }

                let is_valid = state
                    .users_info
                    .proof
                    .0
                    .clone()
                    .verify::<SHA256Hasher>(
                        &TryInto::<[u8; 32]>::try_into(self.users_info_merkle_root.as_slice())
                            .map_err(|e| format!("Failed to cast proof root to H256: {e}"))?
                            .into(),
                        leaves,
                    )
                    .map_err(|e| format!("Failed to verify users_info proof: {e}"))?;

                if !is_valid {
                    return Err(format!(
                        "Invalid users_info proof; root is {}, value: {:?}",
                        hex::encode(self.users_info_merkle_root.as_slice()),
                        state.users_info.value
                    ));
                }
                Ok(())
            }
        }
    }

    pub fn verify_balances_proof(&self) -> Result<(), String> {
        match &self.execution_state {
            ExecutionState::Light(_) | ExecutionState::Full(_) => Ok(()),
            ExecutionState::ZkVm(state) => {
                for (token, witness) in &state.balances {
                    // Verify that users balance are correct
                    let token_root = self
                        .balances_merkle_roots
                        .get(token.as_str())
                        .ok_or(format!("Token {token} not found in balances merkle roots"))?;

                    let leaves = witness
                        .value
                        .iter()
                        .map(|(user_info_key, balance)| {
                            ((*user_info_key).into(), balance.to_h256())
                        })
                        .collect::<Vec<_>>();

                    if leaves.is_empty() {
                        if witness.proof.0 == MerkleProof::new(vec![], vec![]) {
                            return Ok(());
                        }
                        return Err(
                            "No leaves in users_info proof, proof should be empty".to_string()
                        );
                    }

                    let is_valid = &witness
                        .proof
                        .0
                        .clone()
                        .verify::<SHA256Hasher>(
                            &TryInto::<[u8; 32]>::try_into(token_root.as_slice())
                                .map_err(|e| format!("Failed to cast proof root to H256: {e}"))?
                                .into(),
                            leaves,
                        )
                        .map_err(|e| {
                            format!("Failed to verify balances proof for token {token}: {e}")
                        })?;

                    if !is_valid {
                        return Err(format!("Invalid balances proof for token {token}"));
                    }
                }
                Ok(())
            }
        }
    }

    pub fn has_user_info_key(&self, user_info_key: H256) -> Result<bool, String> {
        match &self.execution_state {
            ExecutionState::Full(_) | ExecutionState::Light(_) => Ok(true),
            ExecutionState::ZkVm(state) => Ok(state
                .users_info
                .value
                .iter()
                .any(|user_info| user_info.get_key() == user_info_key)),
        }
    }

    pub fn as_zkvm(
        &self,
        user_info: &UserInfo,
        events: &[OrderbookEvent],
    ) -> Result<ZkVmState, String> {
        let mut order_user_map = HashMap::new();
        let mut users_info_needed: HashSet<UserInfo> = HashSet::new();
        let mut balances_needed: HashMap<String, HashMap<H256, Balance>> = HashMap::new();

        // Track all users, their balances per token, and order-user mapping for executed/updated orders
        for event in events {
            match event {
                OrderbookEvent::OrderExecuted { order_id, .. }
                | OrderbookEvent::OrderUpdate { order_id, .. }
                | OrderbookEvent::OrderCancelled { order_id, .. } => {
                    if let Some(user_key) = self.order_manager.orders_owner.get(order_id) {
                        order_user_map.insert(order_id.clone(), *user_key);
                    }
                }
                OrderbookEvent::OrderCreated {
                    order: Order { order_id, .. },
                    ..
                } => {
                    order_user_map.insert(order_id.clone(), user_info.get_key());
                }
                OrderbookEvent::BalanceUpdated {
                    user,
                    token,
                    amount,
                } => {
                    // Get user_info (if available)
                    let ui = match self.get_user_info(user) {
                        Ok(ui) => ui,
                        Err(_) => {
                            if user_info.user == *user {
                                user_info.clone()
                            } else {
                                return Err(format!("User info not found for user '{user}'"));
                            }
                        }
                    };
                    users_info_needed.insert(ui.clone());

                    balances_needed
                        .entry(token.clone())
                        .or_default()
                        .insert(ui.get_key(), Balance(*amount));
                }
                OrderbookEvent::SessionKeyAdded { user } => {
                    // Get user_info (if available)
                    let ui = match self.get_user_info(user) {
                        Ok(ui) => ui,
                        Err(_) => {
                            if user_info.user == *user {
                                user_info.clone()
                            } else {
                                return Err(format!("User info not found for user '{user}'"));
                            }
                        }
                    };
                    users_info_needed.insert(ui);
                }
                OrderbookEvent::PairCreated { .. } => {}
            }
        }

        let mut balances: HashMap<TokenName, ZkVmWitness<HashMap<H256, Balance>>> = HashMap::new();
        for (token, token_balances) in balances_needed.iter() {
            let users: Vec<UserInfo> = token_balances
                .keys()
                .filter_map(|key| self.get_user_info_from_key(key).ok())
                .collect();

            let witness = self.create_balances_witness(token, &users)?;
            balances.insert(token.clone(), witness);
        }

        let users_info = self.create_users_info_witness(&users_info_needed)?;

        Ok(ZkVmState {
            users_info,
            balances,
        })
    }

    pub fn derive_zkvm_commitment_metadata_from_events(
        &self,
        user_info: &UserInfo,
        events: &[OrderbookEvent],
    ) -> Result<Vec<u8>, String> {
        if !matches!(self.execution_state.mode(), ExecutionMode::Full) {
            return Err("Can only generate zkvm commitment in FullMode".to_string());
        }

        let zk_orderbook = Orderbook {
            hashed_secret: self.hashed_secret,
            pairs_info: self.pairs_info.clone(),
            lane_id: self.lane_id.clone(),
            balances_merkle_roots: self.balances_merkle_roots.clone(),
            users_info_merkle_root: self.users_info_merkle_root,
            order_manager: self.order_manager.clone(),
            execution_state: ExecutionState::ZkVm(self.as_zkvm(user_info, events)?),
        };

        borsh::to_vec(&zk_orderbook)
            .map_err(|e| format!("Failed to serialize ZkVm orderbook metadata: {e}"))
    }
}
