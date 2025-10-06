use borsh::{BorshDeserialize, BorshSerialize};
use std::collections::{HashMap, HashSet};

use crate::{
    monotree_multi_proof::{MonotreeMultiProof, ProofStatus},
    monotree_proof::compute_root_from_proof,
    order_manager::OrderManager,
    orderbook::{ExecutionMode, ExecutionState, Orderbook, OrderbookEvent, TokenName},
    orderbook_state::{MonotreeCommitment, ZkVmState},
    smt_values::{Balance, BorshableH256 as H256, MonotreeValue, UserInfo},
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
        &self,
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
    fn get_users_info_proof(
        &self,
        users_info: &HashSet<UserInfo>,
    ) -> Result<Option<MonotreeMultiProof>, String> {
        if users_info.is_empty() {
            return Ok(None);
        }
        match &self.execution_state {
            ExecutionState::Full(state) => {
                let mut tree = MonotreeCommitment::<UserInfo>::default_from_iter(
                    state
                        .light
                        .users_info
                        .values()
                        .map(|user| (user.get_key(), user.clone())),
                )
                .map_err(|e| format!("Failed to rebuild users info tree: {e}"))?;

                let mut proof_entries = Vec::with_capacity(users_info.len());
                let mut proof_keys = Vec::with_capacity(users_info.len());
                for user in users_info.iter() {
                    let key = user.get_key();
                    proof_keys.push(key.into());
                    proof_entries.push((key, user.clone()));
                }

                tree.upsert_batch(proof_entries.iter().cloned())
                    .map_err(|e| format!("Failed to update users info proof tree: {e}"))?;

                // Aggregate the freshly inserted leaves into a single multiproof so callers can share siblings.
                let multi_proof = MonotreeMultiProof::build(
                    &mut tree.tree,
                    tree.root.as_ref(),
                    proof_keys.iter().cloned(),
                )
                .map_err(|e| format!("Failed to create users info multi-proof: {e}"))?;

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
        &self,
        users_info: &[UserInfo],
        token: &TokenName,
    ) -> Result<(HashMap<UserInfo, Balance>, Option<MonotreeMultiProof>), String> {
        if users_info.is_empty() {
            return Ok((HashMap::new(), None));
        }
        match &self.execution_state {
            ExecutionState::Full(state) => {
                let mut balances_map = HashMap::new();
                for user_info in users_info {
                    let balance = self.get_balance(user_info, token);
                    balances_map.insert(user_info.clone(), balance);
                }

                let token_balances = state
                    .light
                    .balances
                    .get(token)
                    .ok_or_else(|| format!("No balances data found for token {token}"))?;
                let mut tree = MonotreeCommitment::<Balance>::default_from_iter(
                    token_balances
                        .iter()
                        .map(|(key, balance)| (*key, balance.clone())),
                )
                .map_err(|e| format!("Failed to rebuild balances tree for token {token}: {e}"))?;

                let mut entries: Vec<(H256, Balance)> = Vec::with_capacity(balances_map.len());
                let mut proof_keys: Vec<[u8; 32]> = Vec::with_capacity(balances_map.len());
                for (user, balance) in balances_map.iter() {
                    let key = user.get_key();
                    proof_keys.push(key.into());
                    entries.push((key, balance.clone()));
                }
                tree.upsert_batch(entries.iter().cloned()).map_err(|e| {
                    format!("Failed to update balances clone for token {token}: {e}")
                })?;

                // Merge all balance proofs for this token into one multiproof payload.
                let multi_proof = MonotreeMultiProof::build(
                    &mut tree.tree,
                    tree.root.as_ref(),
                    proof_keys.iter().cloned(),
                )
                .map_err(|e| {
                    format!("Failed to create balances multi-proof for token {token}: {e}")
                })?;

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

                let multi_proof = state
                    .users_info
                    .proof
                    .as_ref()
                    .ok_or_else(|| "Missing users_info multiproof".to_string())?;

                let root_bytes: [u8; 32] =
                    self.users_info_merkle_root.as_slice().try_into().unwrap();
                let hasher = Sha2::new();
                let mut verified_keys: HashSet<[u8; 32]> = HashSet::new();

                for user_info in state.users_info.value.iter() {
                    let key = user_info.get_key();
                    let leaf_hash = user_info.to_hash_bytes();
                    let key_bytes: [u8; 32] = (*key).into();
                    match multi_proof.proof_status(&key_bytes) {
                        Some(ProofStatus::Present(path)) => {
                            let is_valid =
                                verify_proof(&hasher, Some(&root_bytes), &leaf_hash, Some(&path));

                            if !is_valid {
                                return Err(format!(
                                    "Invalid users_info proof for user {} with key {}",
                                    user_info.user,
                                    hex::encode(key.as_slice())
                                ));
                            }

                            verified_keys.insert(key_bytes);
                        }
                        Some(ProofStatus::Absent) | None => {
                            return Err(format!(
                                "Missing users_info proof for user {} with key {}",
                                user_info.user,
                                hex::encode(key.as_slice())
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
                    return Err("Unused users_info proofs detected".to_string());
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

    pub fn verify_orders_owners(&self) -> Result<(), String> {
        for (order_id, user_info_key) in &self.order_manager.orders_owner {
            // Verify that the order exists
            if !self.order_manager.orders.contains_key(order_id) {
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
    ) -> Result<Vec<u8>, String> {
        if !matches!(self.execution_state.mode(), ExecutionMode::Full) {
            return Err("Can only generate zkvm commitment in FullMode".to_string());
        }

        let (zkvm_state, order_manager) = self.for_zkvm(user_info, events)?;

        let mut zk_orderbook = Orderbook {
            hashed_secret: self.hashed_secret,
            pairs_info: self.pairs_info.clone(),
            lane_id: self.lane_id.clone(),
            balances_merkle_roots: self.balances_merkle_roots.clone(),
            users_info_merkle_root: self.users_info_merkle_root,
            order_manager,
            execution_state: ExecutionState::ZkVm(zkvm_state),
        };

        if let ExecutionState::ZkVm(state) = &zk_orderbook.execution_state {
            if !state.users_info.value.is_empty() {
                if let Some(multi_proof) = state.users_info.proof.as_ref() {
                    let mut computed_root = None;
                    for user in state.users_info.value.iter() {
                        let key_bytes: [u8; 32] = (*user.get_key()).into();
                        if let Some(ProofStatus::Present(path)) =
                            multi_proof.proof_status(&key_bytes)
                        {
                            let leaf_hash = user.to_hash_bytes();
                            let candidate = compute_root_from_proof(&leaf_hash, &path);
                            match computed_root {
                                Some(existing) if existing != candidate => {
                                    return Err(
                                        "Inconsistent users_info proofs provided".to_string()
                                    )
                                }
                                _ => computed_root = Some(candidate),
                            }
                        }
                    }
                    if let Some(root) = computed_root {
                        zk_orderbook.users_info_merkle_root = root.into();
                    }
                }
            }

            let mut new_balance_roots = zk_orderbook.balances_merkle_roots.clone();
            for (token, witness) in &state.balances {
                if witness.value.is_empty() {
                    continue;
                }

                let mut computed_root = None;
                if let Some(multi_proof) = witness.proof.as_ref() {
                    for (user_key, balance) in witness.value.iter() {
                        let key_bytes: [u8; 32] = (*user_key).into();
                        if let Some(ProofStatus::Present(path)) =
                            multi_proof.proof_status(&key_bytes)
                        {
                            let leaf_hash = balance.to_hash_bytes();
                            let candidate = compute_root_from_proof(&leaf_hash, &path);
                            match computed_root {
                                Some(existing) if existing != candidate => {
                                    return Err(format!(
                                        "Inconsistent balance proofs provided for token {}",
                                        token
                                    ));
                                }
                                _ => computed_root = Some(candidate),
                            }
                        }
                    }
                }

                if let Some(root) = computed_root {
                    new_balance_roots.insert(token.clone(), root.into());
                }
            }

            zk_orderbook.balances_merkle_roots = new_balance_roots;
        }

        borsh::to_vec(&zk_orderbook)
            .map_err(|e| format!("Failed to serialize ZkVm orderbook metadata: {e}"))
    }
}
