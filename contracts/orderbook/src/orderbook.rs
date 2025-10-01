use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::{BorshableMerkleProof, SHA256Hasher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparse_merkle_tree::default_store::DefaultStore;
use sparse_merkle_tree::traits::Value;
use sparse_merkle_tree::{MerkleProof, SparseMerkleTree};
use std::collections::{BTreeMap, BTreeSet};

use crate::order_manager::OrderManager;
use crate::smt_values::{Balance, BorshableH256 as H256, UserInfo};
use sdk::{ContractName, LaneId, TxContext};

#[derive(BorshSerialize, BorshDeserialize, Default, Debug, Clone)]
pub struct Orderbook {
    // Server secret for authentication on permissionned actions
    pub hashed_secret: [u8; 32],
    // Registered token pairs with asset scales
    pub pairs_info: BTreeMap<TokenPair, PairInfo>,
    // Validator public key of the lane this orderbook is running on
    pub lane_id: LaneId,

    // Balances merkle tree root for each token
    pub balances_merkle_roots: BTreeMap<TokenName, H256>,
    // Users info merkle root
    pub users_info_merkle_root: H256,

    // Order manager handling all orders
    pub order_manager: OrderManager,

    /// These fields are not committed on-chain
    pub execution_state: ExecutionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Light,
    Full,
    ZkVm,
}

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct LightState {
    pub users_info: BTreeMap<String, UserInfo>,
    pub balances: BTreeMap<TokenName, BTreeMap<H256, Balance>>,
}

#[derive(Debug, Default)]
pub struct FullState {
    pub users_info_mt: SparseMerkleTree<SHA256Hasher, UserInfo, DefaultStore<UserInfo>>,
    pub balances_mt:
        BTreeMap<TokenName, SparseMerkleTree<SHA256Hasher, Balance, DefaultStore<Balance>>>,
    pub users_info: BTreeMap<String, UserInfo>,
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

        let mut balances_mt = BTreeMap::new();
        for (token_name, tree) in &self.balances_mt {
            let root = *tree.root();
            let store = tree.store().clone();
            let new_tree = SparseMerkleTree::new(root, store);
            balances_mt.insert(token_name.clone(), new_tree);
        }

        Self {
            users_info_mt,
            balances_mt,
            users_info: self.users_info.clone(),
        }
    }
}

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

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkVmState {
    pub users_info: ZkVmWitness<BTreeSet<UserInfo>>,
    pub balances: BTreeMap<TokenName, ZkVmWitness<BTreeMap<H256, Balance>>>,
}

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
pub enum ExecutionState {
    Light(LightState),
    Full(FullState),
    ZkVm(ZkVmState),
}

impl Default for ExecutionState {
    fn default() -> Self {
        ExecutionState::ZkVm(ZkVmState::default())
    }
}

impl ExecutionState {
    pub fn new(mode: ExecutionMode) -> Self {
        match mode {
            ExecutionMode::Light => ExecutionState::Light(LightState::default()),
            ExecutionMode::Full => ExecutionState::Full(FullState::default()),
            ExecutionMode::ZkVm => ExecutionState::ZkVm(ZkVmState::default()),
        }
    }

    pub fn mode(&self) -> ExecutionMode {
        match self {
            ExecutionState::Light(_) => ExecutionMode::Light,
            ExecutionState::Full(_) => ExecutionMode::Full,
            ExecutionState::ZkVm(_) => ExecutionMode::ZkVm,
        }
    }
}

#[derive(
    BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default, Debug, Clone, PartialEq,
)]
pub struct PairInfo {
    pub base_scale: u64,
    pub quote_scale: u64,
}

#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(
    feature = "sqlx",
    sqlx(type_name = "order_side", rename_all = "lowercase")
)]
#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub enum OrderSide {
    Bid, // Buy
    Ask, // Sell
}

#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(
    feature = "sqlx",
    sqlx(type_name = "order_type", rename_all = "lowercase")
)]
#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub enum OrderType {
    Market,
    Limit,
    Stop,
    StopLimit,
}

/// Context struct for creating an order, containing all necessary proofs and mappings.
#[derive(Debug, Clone)]
pub struct CreateOrderCtx {
    pub users_info: BTreeSet<UserInfo>,
    pub users_info_proof: BorshableMerkleProof,
    pub user_info: UserInfo,
    pub user_info_proof: BorshableMerkleProof,
    pub balances: BTreeMap<TokenName, BTreeMap<UserInfo, Balance>>,
    pub balances_proof: BTreeMap<TokenName, BorshableMerkleProof>,
}

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub struct Order {
    pub order_id: OrderId,
    pub order_type: OrderType,
    pub order_side: OrderSide,
    pub price: Option<u64>,
    pub pair: TokenPair,
    pub quantity: u64,
}

pub type OrderId = String;
pub type TokenName = String;
pub type TokenPair = (TokenName, TokenName);

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub enum OrderbookEvent {
    PairCreated {
        pair: TokenPair,
        info: PairInfo,
    },
    OrderCreated {
        order: Order,
    },
    OrderCancelled {
        order_id: OrderId,
        pair: TokenPair,
    },
    OrderExecuted {
        order_id: OrderId,
        taker_order_id: OrderId,
        pair: TokenPair,
    },
    OrderUpdate {
        order_id: OrderId,
        taker_order_id: OrderId,
        executed_quantity: u64,
        remaining_quantity: u64,
        pair: TokenPair,
    },
    BalanceUpdated {
        user: String,
        token: String,
        amount: u64,
    },
    SessionKeyAdded {
        user: String,
    },
}

/// impl of functions for actions execution
impl Orderbook {
    pub fn create_pair(
        &mut self,
        pair: &TokenPair,
        info: &PairInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        if info.base_scale >= 20 {
            return Err(format!(
                "Base scale too large: {}. Maximum is 19",
                info.base_scale
            ));
        }
        self.pairs_info.insert(pair.clone(), info.clone());

        // Initialize a new SparseMerkleTree for the token pair if not already present
        for token in &[&pair.0, &pair.1] {
            if !self.balances_merkle_roots.contains_key(*token) {
                match &mut self.execution_state {
                    ExecutionState::Full(state) => {
                        state
                            .balances_mt
                            .entry((*token).clone())
                            .or_insert_with(|| {
                                SparseMerkleTree::new(
                                    sparse_merkle_tree::H256::zero(),
                                    Default::default(),
                                )
                            });
                    }
                    ExecutionState::Light(state) => {
                        state.balances.entry((*token).clone()).or_default();
                    }
                    ExecutionState::ZkVm(_) => {}
                }
                self.balances_merkle_roots
                    .insert((*token).clone(), sparse_merkle_tree::H256::zero().into());
            }
        }

        Ok(vec![OrderbookEvent::PairCreated {
            pair: pair.clone(),
            info: info.clone(),
        }])
    }

    pub fn add_session_key(
        &mut self,
        mut user_info: UserInfo,
        pubkey: &Vec<u8>,
    ) -> Result<Vec<OrderbookEvent>, String> {
        if user_info.session_keys.contains(pubkey) {
            return Err("Session key already exists".to_string());
        }

        // Add the session key to the user's list of session keys
        user_info.session_keys.push(pubkey.clone());

        let events = match &mut self.execution_state {
            ExecutionState::Full(state) => {
                state
                    .users_info
                    .insert(user_info.user.clone(), user_info.clone());

                vec![OrderbookEvent::SessionKeyAdded {
                    user: user_info.user.to_string(),
                }]
            }
            ExecutionState::Light(state) => {
                // Insert or update the user_info entry
                state
                    .users_info
                    .insert(user_info.user.clone(), user_info.clone());

                vec![OrderbookEvent::SessionKeyAdded {
                    user: user_info.user.to_string(),
                }]
            }
            ExecutionState::ZkVm(_) => vec![],
        };
        if user_info.nonce == 0 {
            // We incremente nonce to be able to add it to the SMT
            self.increment_nonce_and_save_user_info(&user_info)?;
        } else {
            self.update_user_info_merkle_root(&user_info)?;
        }

        Ok(events)
    }

    pub fn deposit(
        &mut self,
        token: &str,
        amount: u64,
        user_info: &UserInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Compute the new balance
        let balance = self.get_balance(user_info, token);
        let new_balance = Balance(balance.0.checked_add(amount).ok_or("Balance overflow")?);

        self.update_balances(token, vec![(user_info.get_key(), new_balance.clone())])
            .map_err(|e| e.to_string())?;

        let events = match self.execution_state.mode() {
            ExecutionMode::Full | ExecutionMode::Light => {
                vec![OrderbookEvent::BalanceUpdated {
                    user: user_info.user.clone(),
                    token: token.to_string(),
                    amount: new_balance.0,
                }]
            }
            ExecutionMode::ZkVm => vec![],
        };

        Ok(events)
    }

    pub fn withdraw(
        &mut self,
        token: &str,
        amount: &u64,
        user_info: &UserInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        let balance = self.get_balance(user_info, token);

        if balance.0 < *amount {
            return Err(format!(
                "Could not withdraw: Insufficient balance: user {} has {balance:?} {token} tokens, trying to withdraw {amount}", user_info.user
            ));
        }

        self.deduct_from_account(token, user_info, *amount)
            .map_err(|e| e.to_string())?;

        let events = match self.execution_state.mode() {
            ExecutionMode::Light | ExecutionMode::Full => {
                let user_balance = self.get_balance(user_info, token);
                vec![OrderbookEvent::BalanceUpdated {
                    user: user_info.user.clone(),
                    token: token.to_string(),
                    amount: user_balance.0,
                }]
            }
            ExecutionMode::ZkVm => vec![],
        };

        // Increment user's nonce
        self.increment_nonce_and_save_user_info(user_info)?;

        Ok(events)
    }

    pub fn cancel_order(
        &mut self,
        order_id: OrderId,
        user_info: &UserInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        let order = self
            .order_manager
            .orders
            .get(&order_id)
            .ok_or(format!("Order {order_id} not found"))?
            .clone();

        let required_token = match &order.order_side {
            OrderSide::Bid => order.pair.1.clone(),
            OrderSide::Ask => order.pair.0.clone(),
        };

        // Refund the reserved amount to the user
        self.fund_account(&required_token, user_info, &Balance(order.quantity))
            .map_err(|e| e.to_string())?;

        // Cancel order through order manager
        let mut cancel_events = self.order_manager.cancel_order(&order_id)?;

        match self.execution_state.mode() {
            ExecutionMode::Light | ExecutionMode::Full => {
                let user_balance = self.get_balance(user_info, &required_token);

                cancel_events.push(OrderbookEvent::BalanceUpdated {
                    user: user_info.user.clone(),
                    token: required_token.to_string(),
                    amount: user_balance.0,
                });
            }
            ExecutionMode::ZkVm => {}
        }
        self.increment_nonce_and_save_user_info(user_info)?;

        Ok(cancel_events)
    }

    pub fn escape(
        &mut self,
        _tx_ctx: &TxContext,
        _user_info: &UserInfo,
        _user_info_proof: &BorshableMerkleProof,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Logic to allow user to escape with their funds
        // This could involve transferring all their balances to a safe contract or address
        // For now, we just return an empty event list
        Ok(vec![])
    }

    pub fn execute_order(
        &mut self,
        user_info: &UserInfo,
        order: Order,
    ) -> Result<Vec<OrderbookEvent>, String> {
        if self.order_manager.orders.contains_key(&order.order_id) {
            return Err(format!("Order with id {} already exists", order.order_id));
        }

        let user_info_key = &user_info.get_key();
        let mut events = Vec::new();

        // Use OrderManager to handle order logic
        let base_scale = POW10[self
            .pairs_info
            .get(&order.pair)
            .ok_or(format!("Pair {}/{} not found", order.pair.0, order.pair.1))?
            .base_scale as usize];

        // Delegate order execution to the manager
        let order_events = self.order_manager.execute_order(user_info_key, &order)?;

        events.extend(order_events);

        // Balance change aggregation system based on events
        let mut balance_changes: BTreeMap<TokenName, BTreeMap<H256, Balance>> = self.get_balances();
        let mut touched_accounts: BTreeMap<TokenName, BTreeSet<H256>> = BTreeMap::new();

        // Helper function to record balance changes
        fn record_balance_change(
            balance_changes: &mut BTreeMap<TokenName, BTreeMap<H256, Balance>>,
            touched_accounts: &mut BTreeMap<TokenName, BTreeSet<H256>>,
            user_info_key: &H256,
            token: &TokenName,
            amount: i128,
        ) -> Result<(), String> {
            let token_balances = balance_changes.get_mut(token);
            let token_balances = match token_balances {
                Some(tb) => tb,
                None => return Err(format!("Token {token} not found in balance_changes")),
            };

            let balance = token_balances.entry(*user_info_key).or_default();

            let new_value: u64 = ((balance.0 as i128) + amount).try_into().map_err(|e| {
                format!(
                    "User with key {} cannot perform token {token} exchange: balance is {}, attempted to add {amount}: {e}", hex::encode(user_info_key.as_slice()), balance.0
                )
            })?;

            *balance = Balance(new_value);
            touched_accounts
                .entry(token.clone())
                .or_default()
                .insert(*user_info_key);
            Ok(())
        }

        // Helper function to record transfers between users
        fn record_transfer(
            balance_changes: &mut BTreeMap<TokenName, BTreeMap<H256, Balance>>,
            touched_accounts: &mut BTreeMap<TokenName, BTreeSet<H256>>,
            from: &H256,
            to: &H256,
            token: &TokenName,
            amount: i128,
        ) -> Result<(), String> {
            record_balance_change(balance_changes, touched_accounts, from, token, -amount)?;
            record_balance_change(balance_changes, touched_accounts, to, token, amount)?;
            Ok(())
        }

        // Process events to calculate balance changes
        for event in &events {
            match event {
                OrderbookEvent::OrderCreated {
                    order: created_order,
                } => {
                    // Deduct liquidity for created order
                    let (quantity, token) = match created_order.order_side {
                        OrderSide::Bid => (
                            -((created_order.quantity * created_order.price.unwrap() / base_scale)
                                as i128),
                            created_order.pair.1.clone(),
                        ),
                        OrderSide::Ask => {
                            (created_order.quantity as i128, created_order.pair.0.clone())
                        }
                    };
                    record_balance_change(
                        &mut balance_changes,
                        &mut touched_accounts,
                        user_info_key,
                        &token,
                        quantity,
                    )?;
                }
                OrderbookEvent::OrderExecuted { order_id, pair, .. } => {
                    let base_token = &pair.0;
                    let quote_token = &pair.1;

                    // Special case: the current order has been fully executed.
                    if order_id == &order.order_id {
                        // We don't process it as it would be counted twice with other matching executed orders
                        continue;
                    };

                    let executed_order_user_info = self.order_manager.orders_owner.get(order_id).ok_or_else(|| {
                        format!(
                            "Executed order owner info (order_id: {order_id}) not found in order manager",
                        )
                    })?;

                    // Transfer token logic for executed orders
                    if let Some(executed_order) = self.order_manager.orders.get(order_id) {
                        match executed_order.order_side {
                            OrderSide::Bid => {
                                // Executed order owner receives base token deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    executed_order_user_info,
                                    base_token,
                                    executed_order.quantity as i128,
                                )?;
                                // User receives quote token
                                record_balance_change(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    quote_token,
                                    (executed_order.price.unwrap() * executed_order.quantity
                                        / base_scale) as i128,
                                )?;
                                touched_accounts
                                    .entry(quote_token.clone())
                                    .or_default()
                                    .insert(*executed_order_user_info);
                            }
                            OrderSide::Ask => {
                                // Executed order owner receives quote token deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    executed_order_user_info,
                                    quote_token,
                                    (executed_order.price.unwrap() * executed_order.quantity
                                        / base_scale) as i128,
                                )?;
                                // User receives base token
                                record_balance_change(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    base_token,
                                    executed_order.quantity as i128,
                                )?;
                            }
                        }
                    } else {
                        return Err(format!("Could not find {order_id}"));
                    }
                }
                OrderbookEvent::OrderUpdate {
                    order_id,
                    pair,
                    executed_quantity,
                    ..
                } => {
                    let executed_order_user_info = self.order_manager.orders_owner.get(order_id).ok_or_else(|| {
                            format!(
                                "Executed order owner info (order_id: {order_id}) not found in order manager",
                            )
                        })?;

                    let base_token = &pair.0;
                    let quote_token = &pair.1;

                    // Transfer token logic for executed orders
                    if let Some(updated_order) = self.order_manager.orders.get(order_id) {
                        match updated_order.order_side {
                            OrderSide::Bid => {
                                // Executed order owner receives base token deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    executed_order_user_info,
                                    base_token,
                                    *executed_quantity as i128,
                                )?;
                                // User receives quote token
                                record_balance_change(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    quote_token,
                                    (updated_order.price.unwrap() * executed_quantity / base_scale)
                                        as i128,
                                )?;
                                touched_accounts
                                    .entry(quote_token.clone())
                                    .or_default()
                                    .insert(*executed_order_user_info);
                            }
                            OrderSide::Ask => {
                                // Executed order owner receives quote token deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    executed_order_user_info,
                                    quote_token,
                                    (updated_order.price.unwrap() * executed_quantity / base_scale)
                                        as i128,
                                )?;
                                // User receives base token
                                record_balance_change(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    base_token,
                                    *executed_quantity as i128,
                                )?;
                            }
                        }
                    } else {
                        return Err(format!("Could not find {order_id}"));
                    }
                }
                _ => {} // Ignore other events for balance changes
            }
        }

        // Clear executed orders from the order manager
        self.order_manager.clear_executed_orders(&events);

        // Updating balances
        for (token, user_keys) in touched_accounts {
            let token_balances = balance_changes
                .get(&token)
                .ok_or_else(|| format!("Token {token} not found in balance_changes"))?;

            let mut balances_to_update: Vec<(H256, Balance)> = Vec::new();
            for user_key in user_keys {
                let amount = token_balances.get(&user_key).ok_or_else(|| {
                    format!(
                        "User with key {} not found in balance_changes for token {token}",
                        hex::encode(user_key.as_slice())
                    )
                })?;

                if !matches!(self.execution_state.mode(), ExecutionMode::ZkVm) {
                    let user_info = self.get_user_info_from_key(&user_key).unwrap();
                    events.push(OrderbookEvent::BalanceUpdated {
                        user: user_info.user.clone(),
                        token: token.clone(),
                        amount: amount.0,
                    });
                }

                balances_to_update.push((user_key, amount.clone()));
            }

            self.update_balances(&token, balances_to_update)?;
        }

        self.increment_nonce_and_save_user_info(user_info)?;

        Ok(events)
    }
}

/// impl of functions for zkvm state generation and verification
impl Orderbook {
    fn create_users_info_witness(
        &self,
        users: &BTreeSet<UserInfo>,
    ) -> Result<ZkVmWitness<BTreeSet<UserInfo>>, String> {
        let proof = self.get_users_info_proofs(users)?;
        let mut set = BTreeSet::new();
        for user in users {
            set.insert(user.clone());
        }
        Ok(ZkVmWitness { value: set, proof })
    }

    fn create_balances_witness(
        &self,
        token: &TokenName,
        users: &[UserInfo],
    ) -> Result<ZkVmWitness<BTreeMap<H256, Balance>>, String> {
        let (balances, proof) = self.get_balances_with_proof(users, token)?;
        let mut map = BTreeMap::new();
        for (user_info, balance) in balances {
            map.insert(user_info.get_key(), balance);
        }
        Ok(ZkVmWitness { value: map, proof })
    }

    fn get_users_info_proofs(
        &self,
        users_info: &BTreeSet<UserInfo>,
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
    ) -> Result<(BTreeMap<UserInfo, Balance>, BorshableMerkleProof), String> {
        if users_info.is_empty() {
            return Ok((
                BTreeMap::new(),
                BorshableMerkleProof(MerkleProof::new(vec![], vec![])),
            ));
        }
        match &self.execution_state {
            ExecutionState::Full(state) => {
                let mut balances_map = BTreeMap::new();
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

    fn get_user_info_from_key(&self, key: &H256) -> Result<UserInfo, String> {
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
        let mut order_user_map = BTreeMap::new();
        let mut users_info_needed: BTreeSet<UserInfo> = BTreeSet::new();
        let mut balances_needed: BTreeMap<String, BTreeMap<H256, Balance>> = BTreeMap::new();

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

        let mut balances: BTreeMap<TokenName, ZkVmWitness<BTreeMap<H256, Balance>>> =
            BTreeMap::new();
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

impl Orderbook {
    pub fn init(lane_id: LaneId, mode: ExecutionMode, secret: Vec<u8>) -> Result<Self, String> {
        let execution_state = ExecutionState::new(mode);
        let users_info_merkle_root = match &execution_state {
            ExecutionState::Full(state) => (*state.users_info_mt.root()).into(),
            _ => sparse_merkle_tree::H256::zero().into(),
        };
        let hashed_secret = Sha256::digest(&secret).into();

        Ok(Orderbook {
            hashed_secret,
            pairs_info: BTreeMap::new(),
            lane_id,
            balances_merkle_roots: BTreeMap::new(),
            users_info_merkle_root,
            order_manager: OrderManager::default(),
            execution_state,
        })
    }

    pub fn get_balances(&self) -> BTreeMap<TokenName, BTreeMap<H256, Balance>> {
        match &self.execution_state {
            ExecutionState::Full(state) => {
                let mut balances = BTreeMap::new();
                for (token, balances_mt) in state.balances_mt.iter() {
                    let token_store = balances_mt.store();
                    let token_balances: BTreeMap<H256, Balance> = token_store
                        .leaves_map()
                        .iter()
                        .map(|(k, v)| ((*k).into(), v.clone()))
                        .collect();
                    balances.insert(token.clone(), token_balances.clone());
                }
                balances
            }
            ExecutionState::Light(state) => state.balances.clone(),
            ExecutionState::ZkVm(state) => {
                let mut balances: BTreeMap<TokenName, BTreeMap<H256, Balance>> = BTreeMap::new();
                for (token, witness) in state.balances.iter() {
                    balances.insert(token.clone(), witness.value.clone());
                }
                balances
            }
        }
    }

    pub fn get_balance(&self, user: &UserInfo, token: &str) -> Balance {
        match &self.execution_state {
            ExecutionState::Full(state) => state
                .balances_mt
                .get(token)
                .and_then(|tree| tree.get(&user.get_key()).ok())
                .unwrap_or_default(),
            ExecutionState::Light(state) => state
                .balances
                .get(token)
                .and_then(|balances| balances.get(&user.get_key()).cloned())
                .unwrap_or_default(),
            ExecutionState::ZkVm(state) => {
                let user_key = match state
                    .users_info
                    .value
                    .iter()
                    .find(|user_info| user_info.user == user.user)
                    .map(|ui| ui.get_key())
                {
                    Some(key) => key,
                    None => return Balance(0),
                };

                state
                    .balances
                    .get(token)
                    .and_then(|user_balances| user_balances.value.get(&user_key).cloned())
                    .unwrap_or_default()
            }
        }
    }

    pub fn is_blob_whitelisted(&self, contract_name: &ContractName) -> bool {
        if contract_name.0 == "orderbook" {
            return true;
        }

        self.pairs_info
            .keys()
            .any(|pair| pair.0 == contract_name.0 || pair.1 == contract_name.0)
    }
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
    ) -> Result<(), String> {
        let mut updated_user_info = user_info.clone();
        updated_user_info.nonce = updated_user_info
            .nonce
            .checked_add(1)
            .ok_or("Nonce overflow")?;
        self.update_user_info_merkle_root(&updated_user_info)
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
        token: &str,
        balances_to_update: Vec<(H256, Balance)>,
    ) -> Result<(), String> {
        match &mut self.execution_state {
            ExecutionState::Full(state) => {
                let tree = state
                    .balances_mt
                    .entry(token.to_string())
                    .or_insert_with(|| {
                        SparseMerkleTree::new(sparse_merkle_tree::H256::zero(), Default::default())
                    });
                let leaves = balances_to_update
                    .iter()
                    .map(|(user_info_key, balance)| ((*user_info_key).into(), balance.clone()))
                    .collect();
                let new_root = tree
                    .update_all(leaves)
                    .map_err(|e| format!("Failed to update balances on token {token}: {e}"))?;
                self.balances_merkle_roots
                    .insert(token.to_string(), (*new_root).into());
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

/// Implementation of functions that are only used by the server.
impl Orderbook {
    pub fn get_orders(&self) -> BTreeMap<String, Order> {
        self.order_manager.orders.clone()
    }

    pub fn get_order_owner(&self, order_id: &OrderId) -> Option<&H256> {
        self.order_manager.orders_owner.get(order_id)
    }

    pub fn get_user_info(&self, user: &str) -> Result<UserInfo, String> {
        match &self.execution_state {
            ExecutionState::Full(state) => {
                let user_info = state
                    .users_info
                    .get(user)
                    .ok_or_else(|| format!("No salt found for user '{user}'"))?;
                let key = user_info.get_key();
                state.users_info_mt.get(&key).map_err(|e| {
                    format!(
                        "Failed to get user info for user '{user}' with key {:?}: {e}",
                        hex::encode(key.as_slice())
                    )
                })
            }
            ExecutionState::Light(state) => state
                .users_info
                .get(user)
                .cloned()
                .ok_or_else(|| format!("User info not found for user '{user}'")),
            ExecutionState::ZkVm(_) => {
                Err("User info lookup is not available in ZkVm execution mode".to_string())
            }
        }
    }
}

// To avoid recomputing powers of 10
const POW10: [u64; 20] = [
    1,
    10,
    100,
    1_000,
    10_000,
    100_000,
    1_000_000,
    10_000_000,
    100_000_000,
    1_000_000_000,
    10_000_000_000,
    100_000_000_000,
    1_000_000_000_000,
    10_000_000_000_000,
    100_000_000_000_000,
    1_000_000_000_000_000,
    10_000_000_000_000_000,
    100_000_000_000_000_000,
    1_000_000_000_000_000_000,
    10_000_000_000_000_000_000,
];
