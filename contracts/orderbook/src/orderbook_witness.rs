use borsh::{BorshDeserialize, BorshSerialize};
use sdk::tracing::debug;
use std::collections::{HashMap, HashSet};

use crate::{
    monotree_multi_proof::{MonotreeMultiProof, ProofStatus},
    order_manager::OrderManager,
    orderbook::{ExecutionMode, ExecutionState, Order, Orderbook, OrderbookEvent, TokenName},
    orderbook_state::{MonotreeCommitment, ZkVmState},
    smt_values::{Balance, BorshableH256 as H256, MonotreeValue, UserInfo},
    OrderbookAction, PermissionnedOrderbookAction,
};
use monotree::{hasher::Sha2, verify_proof, Hasher};

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkVmWitness<T: BorshDeserialize + BorshSerialize + Default> {
    pub value: T,
    pub proof: Option<MonotreeMultiProof>,
}
impl<T: BorshDeserialize + BorshSerialize + Default> Default for ZkVmWitness<T> {
    fn default() -> Self {
        ZkVmWitness {
            value: T::default(),
            proof: None,
        }
    }
}

/// impl of functions for zkvm state generation and verification
impl Orderbook {
    fn create_users_info_witness(
        &mut self,
        users: &HashSet<UserInfo>,
    ) -> Result<ZkVmWitness<HashSet<UserInfo>>, String> {
        let mut set = HashSet::new();
        let proof = self.get_users_info_proof(users)?;
        for user in users {
            set.insert(user.clone());
        }
        Ok(ZkVmWitness { value: set, proof })
    }

    fn create_balances_witness(
        &mut self,
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
    fn get_users_info_proof(
        &mut self,
        users_info: &HashSet<UserInfo>,
    ) -> Result<Option<MonotreeMultiProof>, String> {
        if users_info.is_empty() {
            return Ok(None);
        }
        match &mut self.execution_state {
            ExecutionState::Full(state) => {
                debug!("Merkle tree root: {:?}", self.users_info_merkle_root);
                debug!("Merkle tree state: {:?}", state.users_info_mt);
                let multi_proof = state
                    .users_info_mt
                    .build_multi_proof(users_info.iter().map(|u| u.get_key().into()))
                    .map_err(|e| format!("Failed to create users_info multi-proof: {e}"))?;

                debug!("Multi-proof: {:?}", multi_proof);

                Ok(Some(multi_proof))
            }
            ExecutionState::Light(_) => {
                Err("Light execution mode does not maintain merkle proofs".to_string())
            }
            ExecutionState::ZkVm(_) => {
                Err("ZkVm execution mode cannot generate merkle proofs".to_string())
            }
        }
    }

    fn get_balances_with_proof(
        &mut self,
        users_info: &[UserInfo],
        token: &TokenName,
    ) -> Result<(HashMap<UserInfo, Balance>, Option<MonotreeMultiProof>), String> {
        if users_info.is_empty() {
            return Ok((HashMap::new(), None));
        }
        match &mut self.execution_state {
            ExecutionState::Full(state) => {
                let mut balances_map = HashMap::new();
                for user_info in users_info {
                    let balance = state
                        .light
                        .balances
                        .get(token)
                        .and_then(|b| b.get(&user_info.get_key()))
                        .cloned()
                        .unwrap_or(Balance(0));
                    balances_map.insert(user_info.clone(), balance);
                }

                let token_balances_mt = state
                    .balances_mt
                    .get_mut(token)
                    .ok_or_else(|| format!("No balances data found for token {token}"))?;

                // Merge all balance proofs for this token into one multiproof payload.
                let multi_proof = token_balances_mt
                    .build_multi_proof(balances_map.keys().map(|u| u.get_key().into()))
                    .map_err(|e| format!("Failed to create balances multi-proof: {e}"))?;

                Ok((balances_map, Some(multi_proof)))
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
            ExecutionState::Full(state) => state
                .light
                .users_info
                .values()
                .find(|info| info.get_key() == *key)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "No user info found for key {:?}",
                        hex::encode(key.as_slice())
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
        debug!("Execution state: {:?}", self.execution_state);

        // Verification only needed in ZkVm mode
        match &self.execution_state {
            ExecutionState::Light(_) | ExecutionState::Full(_) => Ok(()),
            ExecutionState::ZkVm(state) => {
                if self.users_info_merkle_root.as_slice() == &[0u8; 32] {
                    return Ok(());
                }

                if state.users_info.value.is_empty() {
                    if state.users_info.proof.is_none() {
                        return Ok(());
                    }
                    return Err("No users provided but proofs were supplied".to_string());
                }

                debug!(
                    "Verifying users_info proof for {} users",
                    state.users_info.value.len()
                );

                debug!(
                    "Nb of leaves in users_info proof: {:?}",
                    state.users_info.value
                );

                let multi_proof = state
                    .users_info
                    .proof
                    .as_ref()
                    .ok_or_else(|| "Missing users_info multiproof".to_string())?;

                let root_bytes: [u8; 32] =
                    self.users_info_merkle_root.as_slice().try_into().unwrap();

                let leaves: Vec<([u8; 32], [u8; 32])> = state
                    .users_info
                    .value
                    .iter()
                    .map(|user_info| ((*user_info.get_key()).into(), user_info.to_hash_bytes()))
                    .collect();

                let derived_root = multi_proof
                    .derived_root(&Sha2::new(), leaves.clone().into_iter())
                    .map_err(|e| format!("Failed to compute users_info proof root: {}", e))?;

                debug!(
                    "Hex derived root: {}, expected root: {}",
                    hex::encode(derived_root.expect("derived root")),
                    hex::encode(root_bytes)
                );

                multi_proof
                    .verify(&Sha2::new(), Some(&root_bytes), leaves.into_iter())
                    .map_err(|e| format!("Invalid users_info proof: {}", e.to_string()))
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

                    for user_info_key in witness.value.keys() {
                        let has_user = state
                            .users_info
                            .value
                            .iter()
                            .any(|info| info.get_key() == *user_info_key);
                        if !has_user {
                            return Err(format!(
                                "Missing user info for user {} while verifying balances of token {token}",
                                hex::encode(user_info_key.as_slice())
                            ));
                        }
                    }

                    if witness.value.is_empty() {
                        if witness.proof.is_none() {
                            continue;
                        }
                        return Err(format!(
                            "No balances provided for token {token} but proofs were supplied"
                        ));
                    }

                    let multi_proof = witness
                        .proof
                        .as_ref()
                        .ok_or_else(|| format!("Missing balances multiproof for token {token}"))?;

                    let root_bytes: [u8; 32] = token_root.as_slice().try_into().unwrap();
                    let hasher = Sha2::new();
                    let mut verified_keys: HashSet<[u8; 32]> = HashSet::new();

                    for (user_info_key, balance) in witness.value.iter() {
                        let leaf_hash = balance.to_hash_bytes();
                        let key_bytes: [u8; 32] = (*user_info_key).into();
                        match multi_proof.proof_status(&key_bytes) {
                            Some(ProofStatus::Present(path)) => {
                                let is_valid = verify_proof(
                                    &hasher,
                                    Some(&root_bytes),
                                    &leaf_hash,
                                    Some(&path),
                                );

                                if !is_valid {
                                    return Err(format!(
                                        "Invalid balances proof for token {token} and user key {}",
                                        hex::encode(user_info_key.as_slice())
                                    ));
                                }

                                verified_keys.insert(key_bytes);
                            }
                            Some(ProofStatus::Absent) | None => {
                                return Err(format!(
                                    "Missing balance proof for token {token} and user key {}",
                                    hex::encode(user_info_key.as_slice())
                                ));
                            }
                        }
                    }

                    let unused = multi_proof
                        .entries()
                        .filter(|(key, status)| {
                            let key_bytes = **key;
                            matches!(status, ProofStatus::Present(_))
                                && !verified_keys.contains(&key_bytes)
                        })
                        .count();

                    if unused > 0 {
                        return Err(format!("Unused balance proofs detected for token {token}"));
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
        &mut self,
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

        // Track all users, their balances per token, and order-user mapping for executed/updated orders
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
                    if *user == user_info.user {
                        users_info_needed.insert(ui.clone());
                    }

                    balances_needed
                        .entry(token.clone())
                        .or_default()
                        .insert(ui.get_key(), Balance(*amount));
                }
                OrderbookEvent::SessionKeyAdded { user, .. } => {
                    // Get user_info (if available)

                    debug!("SessionKeyAdded event for user {}", user);
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

        debug!("Zk witness for zkvm: users_info: {:?}", users_info);

        Ok((
            ZkVmState {
                users_info,
                balances,
            },
            zkvm_order_manager,
        ))
    }

    pub fn derive_zkvm_commitment_metadata_from_events(
        &mut self,
        user_info: &UserInfo,
        events: &[OrderbookEvent],
        action: &PermissionnedOrderbookAction,
    ) -> Result<Vec<u8>, String> {
        if !matches!(self.execution_state.mode(), ExecutionMode::Full) {
            return Err("Can only generate zkvm commitment in FullMode".to_string());
        }

        debug!(
                user_info = ?user_info,
                events = ?events,
                action = ?action,
                "Deriving zkvm commitment metadata from events"
        );

        let (zkvm_state, order_manager) = self.for_zkvm(user_info, events, action)?;

        let zk_orderbook = Orderbook {
            hashed_secret: self.hashed_secret,
            pairs_info: self.pairs_info.clone(),
            lane_id: self.lane_id.clone(),
            balances_merkle_roots: self.balances_merkle_roots.clone(),
            users_info_merkle_root: self.users_info_merkle_root,
            order_manager,
            execution_state: ExecutionState::ZkVm(zkvm_state),
        };

        borsh::to_vec(&zk_orderbook)
            .map_err(|e| format!("Failed to serialize ZkVm orderbook metadata: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orderbook::{OrderSide, OrderType, PairInfo, TokenPair};
    use crate::PermissionnedOrderbookAction;
    use sdk::LaneId;
    use std::collections::HashSet;

    const BASE_TOKEN: &str = "HYLLAR";
    const QUOTE_TOKEN: &str = "ORANJ";

    fn init_full_orderbook() -> Orderbook {
        let mut orderbook =
            Orderbook::init(LaneId::default(), ExecutionMode::Full, b"secret".to_vec())
                .expect("init orderbook");
        let pair: TokenPair = (BASE_TOKEN.to_string(), QUOTE_TOKEN.to_string());
        let info = PairInfo {
            base_scale: 0,
            quote_scale: 0,
        };
        orderbook.create_pair(&pair, &info).expect("create pair");
        orderbook
    }

    fn insert_user(orderbook: &mut Orderbook, username: &str) -> UserInfo {
        let mut user = UserInfo::new(
            username.to_string(),
            format!("{username}-salt").into_bytes(),
        );
        user.nonce = 1;
        orderbook
            .update_user_info_merkle_root(&user)
            .expect("store user info");
        user
    }

    fn set_balance(orderbook: &mut Orderbook, token: &str, user: &UserInfo, amount: u64) {
        orderbook
            .update_balances(token, vec![(user.get_key(), Balance(amount))])
            .expect("update balance");
    }

    #[test]
    fn for_zkvm_collects_users_and_balances() {
        let mut orderbook = init_full_orderbook();
        let alice = insert_user(&mut orderbook, "alice");
        set_balance(&mut orderbook, QUOTE_TOKEN, &alice, 200);

        let events = vec![OrderbookEvent::BalanceUpdated {
            user: alice.user.clone(),
            token: QUOTE_TOKEN.to_string(),
            amount: 200,
        }];

        let action = PermissionnedOrderbookAction::Deposit {
            token: QUOTE_TOKEN.to_string(),
            amount: 200,
        };

        let (zk_state, zk_manager) = orderbook
            .for_zkvm(&alice, &events, &action)
            .expect("for_zkvm success");

        assert_eq!(zk_state.users_info.value.len(), 1);
        assert!(zk_state.users_info.value.contains(&alice));

        let quote_witness = zk_state
            .balances
            .get(QUOTE_TOKEN)
            .expect("quote witness present");
        assert_eq!(quote_witness.value.len(), 1);
        assert_eq!(
            quote_witness
                .value
                .get(&alice.get_key())
                .map(|balance| balance.0),
            Some(200)
        );

        assert!(zk_manager.orders_owner.is_empty());
    }

    #[test]
    fn for_zkvm_errors_for_unknown_user_balances() {
        let mut orderbook = init_full_orderbook();
        let alice = insert_user(&mut orderbook, "alice");

        let events = vec![OrderbookEvent::BalanceUpdated {
            user: "bob".to_string(),
            token: QUOTE_TOKEN.to_string(),
            amount: 50,
        }];

        let action = PermissionnedOrderbookAction::Deposit {
            token: QUOTE_TOKEN.to_string(),
            amount: 50,
        };

        let err = orderbook
            .for_zkvm(&alice, &events, &action)
            .expect_err("for_zkvm should fail for unknown user");

        assert!(err.contains("User info not found for user 'bob'"));
    }

    #[test]
    fn for_zkvm_infers_owner_for_new_order() {
        let mut orderbook = init_full_orderbook();
        let alice = insert_user(&mut orderbook, "alice");
        set_balance(&mut orderbook, QUOTE_TOKEN, &alice, 500);

        let order = Order {
            order_id: "order-123".to_string(),
            order_type: OrderType::Limit,
            order_side: OrderSide::Bid,
            price: Some(42),
            pair: (BASE_TOKEN.to_string(), QUOTE_TOKEN.to_string()),
            quantity: 5,
        };

        let events = vec![
            OrderbookEvent::OrderExecuted {
                order_id: order.order_id.clone(),
                taker_order_id: "market-order".to_string(),
                pair: order.pair.clone(),
            },
            OrderbookEvent::BalanceUpdated {
                user: alice.user.clone(),
                token: QUOTE_TOKEN.to_string(),
                amount: 500,
            },
        ];

        let action = PermissionnedOrderbookAction::CreateOrder(order.clone());

        let (zk_state, zk_manager) = orderbook
            .for_zkvm(&alice, &events, &action)
            .expect("for_zkvm success");

        assert!(
            zk_state.balances.get(QUOTE_TOKEN).is_some(),
            "balance witness not created"
        );

        assert_eq!(orderbook.order_manager.orders_owner.len(), 0);
        assert_eq!(
            zk_manager.orders_owner.get(&order.order_id).copied(),
            Some(alice.get_key())
        );
    }

    #[test]
    fn get_users_info_proof_produces_valid_multiproof() {
        let mut orderbook = init_full_orderbook();
        let alice = insert_user(&mut orderbook, "alice");
        let bob = insert_user(&mut orderbook, "bob");

        let mut users = HashSet::new();
        users.insert(alice.clone());
        users.insert(bob.clone());

        let proof = orderbook
            .get_users_info_proof(&users)
            .expect("proof generation")
            .expect("proof returned");

        let root_bytes: [u8; 32] = orderbook.users_info_merkle_root.as_h256();
        let leaves: Vec<([u8; 32], [u8; 32])> = users
            .iter()
            .map(|user| ((*user.get_key()).into(), user.to_hash_bytes()))
            .collect();

        proof
            .verify(&Sha2::new(), Some(&root_bytes), leaves.clone().into_iter())
            .expect("proof verifies");

        let present_entries = proof
            .entries()
            .filter(|(_, status)| matches!(status, ProofStatus::Present(_)))
            .count();
        assert_eq!(present_entries, leaves.len());
    }

    #[test]
    fn get_users_info_proof_returns_none_for_empty_input() {
        let mut orderbook = init_full_orderbook();
        let users: HashSet<UserInfo> = HashSet::new();

        let proof = orderbook
            .get_users_info_proof(&users)
            .expect("proof generation for empty set");

        assert!(proof.is_none());
    }
}
