use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::{BorshableMerkleProof, SHA256Hasher};
use sparse_merkle_tree::traits::Value;
use sparse_merkle_tree::MerkleProof;
use std::collections::{HashMap, HashSet};

use crate::{
    order_manager::OrderManager,
    orderbook::{ExecutionMode, ExecutionState, Order, Orderbook, OrderbookEvent, Symbol},
    orderbook_state::ZkVmState,
    smt_values::{Balance, BorshableH256 as H256, UserInfo},
    OrderbookAction, PermissionnedOrderbookAction,
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
        symbol: &Symbol,
        users: &[UserInfo],
    ) -> Result<ZkVmWitness<HashMap<H256, Balance>>, String> {
        let (balances, proof) = self.get_balances_with_proof(users, symbol)?;
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
        symbol: &Symbol,
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
                    let balance = self.get_balance(user_info, symbol);
                    balances_map.insert(user_info.clone(), balance);
                }

                let users: Vec<UserInfo> = balances_map.keys().cloned().collect();
                let tree = state
                    .balances_mt
                    .get(symbol)
                    .ok_or_else(|| format!("No balances tree found for {symbol}"))?;
                let proof = BorshableMerkleProof(
                    tree.merkle_proof(users.iter().map(|u| u.get_key().into()).collect::<Vec<_>>())
                        .map_err(|e| {
                            format!(
                                "Failed to create merkle proof for {symbol} and users {:?}: {e}",
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
                for (symbol, witness) in &state.balances {
                    // Verify that users balance are correct
                    let symbol_root = self
                        .balances_merkle_roots
                        .get(symbol.as_str())
                        .ok_or(format!("{symbol} not found in balances merkle roots"))?;

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
                            &TryInto::<[u8; 32]>::try_into(symbol_root.as_slice())
                                .map_err(|e| format!("Failed to cast proof root to H256: {e}"))?
                                .into(),
                            leaves,
                        )
                        .map_err(|e| {
                            format!("Failed to verify balances proof for {symbol}: {e}")
                        })?;

                    if !is_valid {
                        return Err(format!("Invalid balances proof for {symbol}"));
                    }
                }
                Ok(())
            }
        }
    }

    pub fn verify_orders_owners(&self, action: &OrderbookAction) -> Result<(), String> {
        for (order_id, user_info_key) in &self.order_manager.orders_owner {
            // Verify that the order exists
            if !self.order_manager.orders.contains_key(order_id)
                // If the action is creating this order, it's expected to not find it in orders
                && !matches!(
                    action,
                    OrderbookAction::PermissionnedOrderbookAction(
                        PermissionnedOrderbookAction::CreateOrder(Order {
                            order_id: create_order_id,
                            ..
                        })
                    ) if create_order_id == order_id
                )
            {
                return Err(format!("Order with id {order_id} does not exist"));
            }
            // Verify that user info exists
            if !self.has_user_info_key(*user_info_key).map_err(|e| {
                format!(
                    "Failed to get user info for key {}: {e}",
                    hex::encode(user_info_key)
                )
            })? {
                return Err(format!(
                    "Missing user info for user {}",
                    hex::encode(user_info_key)
                ));
            }
        }
        Ok(())
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

    pub fn for_zkvm(
        &self,
        user_info: &UserInfo,
        events: &[OrderbookEvent],
        action: &PermissionnedOrderbookAction,
    ) -> Result<(ZkVmState, OrderManager), String> {
        // Atm, we copy everything (will be merklized in a future version)
        let mut zkvm_order_manager = self.order_manager.clone();

        // We clear orders_owner and re-populate it based on events with only needed values
        zkvm_order_manager.orders_owner.clear();

        let mut users_info_needed: HashSet<UserInfo> = HashSet::new();
        let mut balances_needed: HashMap<String, HashMap<H256, Balance>> = HashMap::new();

        // Track all users, their balances per symbol, and order-user mapping for executed/updated orders
        for event in events {
            match event {
                OrderbookEvent::OrderExecuted { order_id, .. }
                | OrderbookEvent::OrderUpdate { order_id, .. }
                | OrderbookEvent::OrderCancelled { order_id, .. } => {
                    if let Some(order_owner) = self.order_manager.orders_owner.get(order_id) {
                        zkvm_order_manager
                            .orders_owner
                            .insert(order_id.clone(), *order_owner);
                    } else if let PermissionnedOrderbookAction::CreateOrder(Order {
                        order_id: create_order_id,
                        ..
                    }) = action
                    {
                        if create_order_id == order_id {
                            // Special case: the order was created in the same tx, we can use the user_info
                            zkvm_order_manager
                                .orders_owner
                                .insert(order_id.clone(), user_info.get_key());
                        }
                    } else {
                        return Err(format!(
                            "Order with id {order_id} does not have an owner in orders_owner mapping"
                        ));
                    }
                }
                OrderbookEvent::BalanceUpdated {
                    user,
                    symbol,
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
                        .entry(symbol.clone())
                        .or_default()
                        .insert(ui.get_key(), Balance(*amount));
                }
                OrderbookEvent::SessionKeyAdded { user, .. } => {
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
                _ => {}
            }
        }

        let mut balances: HashMap<Symbol, ZkVmWitness<HashMap<H256, Balance>>> = HashMap::new();
        for (symbol, symbol_balances) in balances_needed.iter() {
            let users: Vec<UserInfo> = symbol_balances
                .keys()
                .filter_map(|key| self.get_user_info_from_key(key).ok())
                .collect();

            let witness = self.create_balances_witness(symbol, &users)?;
            balances.insert(symbol.clone(), witness);
        }

        let users_info = self.create_users_info_witness(&users_info_needed)?;

        Ok((
            ZkVmState {
                users_info,
                balances,
            },
            zkvm_order_manager,
        ))
    }

    pub fn derive_zkvm_commitment_metadata_from_events(
        &self,
        user_info: &UserInfo,
        events: &[OrderbookEvent],
        action: &PermissionnedOrderbookAction,
    ) -> Result<Vec<u8>, String> {
        if !matches!(self.execution_state.mode(), ExecutionMode::Full) {
            return Err("Can only generate zkvm commitment in FullMode".to_string());
        }

        let (zkvm_state, order_manager) = self.for_zkvm(user_info, events, action)?;

        let zk_orderbook = Orderbook {
            hashed_secret: self.hashed_secret,
            assets_info: self.assets_info.clone(),
            lane_id: self.lane_id.clone(),
            last_block_number: self.last_block_number,
            balances_merkle_roots: self.balances_merkle_roots.clone(),
            users_info_merkle_root: self.users_info_merkle_root,
            order_manager,
            execution_state: ExecutionState::ZkVm(zkvm_state),
        };

        borsh::to_vec(&zk_orderbook)
            .map_err(|e| format!("Failed to serialize ZkVm orderbook metadata: {e}"))
    }
}
