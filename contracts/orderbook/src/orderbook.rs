use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::BorshableMerkleProof;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparse_merkle_tree::SparseMerkleTree;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::order_manager::OrderManager;
use crate::orderbook_state::{FullState, LightState, ZkVmState};
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
    pub fn from_data(
        mode: ExecutionMode,
        users_info: HashMap<String, UserInfo>,
        balances: HashMap<TokenName, HashMap<H256, Balance>>,
    ) -> Result<Self, String> {
        match mode {
            ExecutionMode::Light => Ok(ExecutionState::Light(LightState {
                users_info,
                balances,
            })),
            ExecutionMode::Full => {
                let mut users_info_mt = SparseMerkleTree::default();

                let leaves = users_info
                    .iter()
                    .map(|(_, user_info)| (user_info.get_key().into(), user_info.clone()))
                    .collect();
                users_info_mt
                    .update_all(leaves)
                    .map_err(|e| format!("Failed to update users info in SMT: {e}"))?;

                let mut balances_mt = HashMap::new();
                for (token, token_balances) in balances.iter() {
                    let mut tree = SparseMerkleTree::default();
                    let leaves = token_balances
                        .iter()
                        .map(|(user_info_key, balance)| ((*user_info_key).into(), balance.clone()))
                        .collect();
                    tree.update_all(leaves)
                        .map_err(|e| format!("Failed to update balances on token {token}: {e}"))?;
                    balances_mt.insert(token.clone(), tree);
                }

                Ok(ExecutionState::Full(FullState {
                    users_info_mt,
                    balances_mt,
                    users_info,
                }))
            }
            ExecutionMode::ZkVm => Ok(ExecutionState::ZkVm(ZkVmState::default())),
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
    pub users_info: HashSet<UserInfo>,
    pub users_info_proof: BorshableMerkleProof,
    pub user_info: UserInfo,
    pub user_info_proof: BorshableMerkleProof,
    pub balances: HashMap<TokenName, HashMap<UserInfo, Balance>>,
    pub balances_proof: HashMap<TokenName, BorshableMerkleProof>,
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
        salt: Vec<u8>,
        nonce: u32,
        session_keys: Vec<Vec<u8>>,
    },
    NonceIncremented {
        user: String,
        nonce: u32,
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

        let mut events = match &mut self.execution_state {
            ExecutionState::Full(state) => {
                state
                    .users_info
                    .insert(user_info.user.clone(), user_info.clone());

                vec![OrderbookEvent::SessionKeyAdded {
                    user: user_info.user.to_string(),
                    salt: user_info.salt.clone(),
                    nonce: user_info.nonce,
                    session_keys: user_info.session_keys.clone(),
                }]
            }
            ExecutionState::Light(state) => {
                // Insert or update the user_info entry
                state
                    .users_info
                    .insert(user_info.user.clone(), user_info.clone());

                vec![OrderbookEvent::SessionKeyAdded {
                    user: user_info.user.to_string(),
                    salt: user_info.salt.clone(),
                    nonce: user_info.nonce,
                    session_keys: user_info.session_keys.clone(),
                }]
            }
            ExecutionState::ZkVm(_) => vec![],
        };
        if user_info.nonce == 0 {
            // We incremente nonce to be able to add it to the SMT
            events.push(self.increment_nonce_and_save_user_info(&user_info)?);
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

        let mut events = match self.execution_state.mode() {
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
        events.push(self.increment_nonce_and_save_user_info(user_info)?);

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
        let mut events = self.order_manager.cancel_order(&order_id)?;

        match self.execution_state.mode() {
            ExecutionMode::Light | ExecutionMode::Full => {
                let user_balance = self.get_balance(user_info, &required_token);

                events.push(OrderbookEvent::BalanceUpdated {
                    user: user_info.user.clone(),
                    token: required_token.to_string(),
                    amount: user_balance.0,
                });
            }
            ExecutionMode::ZkVm => {}
        }
        events.push(self.increment_nonce_and_save_user_info(user_info)?);

        Ok(events)
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
        let mut balance_changes: HashMap<TokenName, HashMap<H256, Balance>> = self.get_balances();
        let mut touched_accounts: HashMap<TokenName, HashSet<H256>> = HashMap::new();

        // Helper function to record balance changes
        fn record_balance_change(
            balance_changes: &mut HashMap<TokenName, HashMap<H256, Balance>>,
            touched_accounts: &mut HashMap<TokenName, HashSet<H256>>,
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
            balance_changes: &mut HashMap<TokenName, HashMap<H256, Balance>>,
            touched_accounts: &mut HashMap<TokenName, HashSet<H256>>,
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

        events.push(self.increment_nonce_and_save_user_info(user_info)?);

        Ok(events)
    }
}

impl Orderbook {
    pub fn init(lane_id: LaneId, mode: ExecutionMode, secret: Vec<u8>) -> Result<Self, String> {
        Self::from_data(
            lane_id,
            mode,
            secret,
            BTreeMap::new(),
            OrderManager::default(),
            HashMap::new(),
            HashMap::new(),
        )
    }

    pub fn from_data(
        lane_id: LaneId,
        mode: ExecutionMode,
        secret: Vec<u8>,
        pairs_info: BTreeMap<TokenPair, PairInfo>,
        order_manager: OrderManager,
        users_info: HashMap<String, UserInfo>,
        balances: HashMap<TokenName, HashMap<H256, Balance>>,
    ) -> Result<Self, String> {
        let full_state =
            ExecutionState::from_data(ExecutionMode::Full, users_info.clone(), balances.clone())?;

        let execution_state = if mode == ExecutionMode::Full {
            full_state.clone()
        } else {
            ExecutionState::from_data(mode, users_info, balances)?
        };

        let users_info_merkle_root = match &full_state {
            ExecutionState::Full(state) => (*state.users_info_mt.root()).into(),
            _ => panic!("Business logic error. full_state should be Full"),
        };
        let balances_merkle_roots = match &full_state {
            ExecutionState::Full(state) => state
                .balances_mt
                .iter()
                .map(|(token, balances)| (token.clone(), (*balances.root()).into()))
                .collect(),
            _ => panic!("Business logic error. full_state should be Full"),
        };
        let hashed_secret = Sha256::digest(&secret).into();

        let mut orderbook = Orderbook {
            hashed_secret,
            pairs_info: BTreeMap::new(),
            lane_id,
            balances_merkle_roots,
            users_info_merkle_root,
            order_manager,
            execution_state,
        };

        for (pair, info) in pairs_info {
            orderbook.create_pair(&pair, &info)?;
        }

        Ok(orderbook)
    }

    pub fn get_balances(&self) -> HashMap<TokenName, HashMap<H256, Balance>> {
        match &self.execution_state {
            ExecutionState::Full(state) => {
                let mut balances = HashMap::new();
                for (token, balances_mt) in state.balances_mt.iter() {
                    let token_store = balances_mt.store();
                    let token_balances: HashMap<H256, Balance> = token_store
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
                let mut balances: HashMap<TokenName, HashMap<H256, Balance>> = HashMap::new();
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

impl Orderbook {
    // Detects differences between two orderbooks
    // It is used to detect differences between on-chain and db orderbooks
    pub fn diff(&self, other: &Orderbook) -> BTreeMap<String, String> {
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

        if self.pairs_info != other.pairs_info {
            let mismatching_pairs = self
                .pairs_info
                .iter()
                .filter(|(pair, info)| {
                    other
                        .pairs_info
                        .get(pair)
                        .map_or(true, |o_info| *info != o_info)
                })
                .collect::<BTreeMap<&TokenPair, &PairInfo>>();

            mismatching_pairs.iter().for_each(|(pair, info)| {
                diff.insert("pairs_info".to_string(), format!("{pair:?}: {info:?}"));
            });
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

        if self.balances_merkle_roots != other.balances_merkle_roots {
            diff.insert(
                "balances_merkle_roots".to_string(),
                format!(
                    "{:?} != {:?}",
                    self.balances_merkle_roots, other.balances_merkle_roots
                ),
            );
        }

        if self.users_info_merkle_root != other.users_info_merkle_root {
            diff.insert(
                "users_info_merkle_root".to_string(),
                format!(
                    "{} != {}",
                    hex::encode(self.users_info_merkle_root.as_slice()),
                    hex::encode(other.users_info_merkle_root.as_slice())
                ),
            );
        }

        if self.order_manager != other.order_manager {
            diff.extend(self.order_manager.diff(&other.order_manager));
        }
        if self.execution_state.mode() != other.execution_state.mode() {
            diff.insert(
                "execution_state".to_string(),
                format!(
                    "{:?} != {:?}",
                    self.execution_state.mode(),
                    other.execution_state.mode()
                ),
            );
        }
        diff
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
