use borsh::{BorshDeserialize, BorshSerialize};
use hyli_smt_token::SmtTokenAction;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::order_manager::OrderManager;
use crate::orderbook_state::{FullState, LightState, ZkVmState, SMT};
use crate::smt_values::{Balance, BorshableH256 as H256, UserInfo};
use sdk::{BlockHeight, ContractName, LaneId, StructuredBlob};

#[derive(BorshSerialize, BorshDeserialize, Default, Debug, Clone)]
pub struct Orderbook {
    // Server secret for authentication on permissionned actions
    pub hashed_secret: [u8; 32],
    // Registered assets info from their symbol
    pub assets_info: BTreeMap<Symbol, AssetInfo>,
    // Validator public key of the lane this orderbook is running on
    pub lane_id: LaneId,
    // Last block number with an action processed
    pub last_block_number: BlockHeight,

    // Balances merkle tree root for each symbol
    pub balances_merkle_roots: BTreeMap<Symbol, H256>,
    // Users info merkle root
    pub users_info_merkle_root: H256,

    // Order manager handling all orders
    pub order_manager: OrderManager,

    /// These fields are not committed on-chain
    pub execution_state: ExecutionState,
}

pub const ORDERBOOK_ACCOUNT_IDENTITY: &str = "orderbook@orderbook";

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
        balances: HashMap<Symbol, HashMap<H256, Balance>>,
    ) -> Result<Self, String> {
        match mode {
            ExecutionMode::Light => Ok(ExecutionState::Light(LightState {
                users_info,
                balances,
            })),
            ExecutionMode::Full => {
                let mut users_info_mt = SMT::zero();

                let leaves = users_info
                    .values()
                    .map(|user_info| (user_info.get_key(), user_info.clone()))
                    .collect();
                users_info_mt
                    .update_all(leaves)
                    .map_err(|e| format!("Failed to update users info in SMT: {e}"))?;

                let mut balances_mt = HashMap::new();
                for (symbol, symbol_balances) in balances.iter() {
                    let mut tree = SMT::zero();
                    let leaves = symbol_balances
                        .iter()
                        .map(|(user_info_key, balance)| ((*user_info_key), balance.clone()))
                        .collect();
                    tree.update_all(leaves).map_err(|e| {
                        format!("Failed to update balances on symbol {symbol}: {e}")
                    })?;
                    balances_mt.insert(symbol.clone(), tree);
                }

                Ok(ExecutionState::Full(FullState {
                    users_info_mt,
                    balances_mt,
                    light: LightState {
                        balances,
                        users_info: HashMap::new(),
                    },
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
    Default, BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, PartialEq, Eq,
)]
pub struct AssetInfo {
    pub scale: u64,
    pub contract_name: ContractName,
}

impl AssetInfo {
    pub fn new(scale: u64, contract_name: ContractName) -> Self {
        AssetInfo {
            scale,
            contract_name,
        }
    }
}

#[derive(
    BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default, Debug, Clone, PartialEq,
)]
pub struct PairInfo {
    pub base: AssetInfo,
    pub quote: AssetInfo,
}

#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(
    feature = "sqlx",
    sqlx(type_name = "order_side", rename_all = "lowercase")
)]
#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
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
#[serde(rename_all = "snake_case")]
pub enum OrderType {
    Market,
    Limit,
    Stop,
    StopLimit,
    StopMarket,
}

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub struct Order {
    pub order_id: OrderId,
    pub order_type: OrderType,
    pub order_side: OrderSide,
    pub price: Option<u64>,
    pub pair: Pair,
    pub quantity: u64,
}

pub type OrderId = String;
pub type Symbol = String;
pub type Pair = (Symbol, Symbol);

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
pub enum OrderbookEvent {
    PairCreated {
        pair: Pair,
        info: PairInfo,
    },
    OrderCreated {
        order: Order,
    },
    OrderCancelled {
        order_id: OrderId,
        pair: Pair,
    },
    OrderExecuted {
        order_id: OrderId,
        taker_order_id: OrderId,
        pair: Pair,
    },
    OrderUpdate {
        order_id: OrderId,
        taker_order_id: OrderId,
        executed_quantity: u64,
        remaining_quantity: u64,
        pair: Pair,
    },
    BalanceUpdated {
        user: String,
        symbol: String,
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
    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn create_pair(
        &mut self,
        pair: &Pair,
        info: &PairInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        self.register_asset(&pair.0, &info.base)?;
        self.register_asset(&pair.1, &info.quote)?;

        // Initialize a new SparseMerkleTree for the symbol pair if not already present
        for symbol in &[&pair.0, &pair.1] {
            if !self.balances_merkle_roots.contains_key(*symbol) {
                match &mut self.execution_state {
                    ExecutionState::Full(state) => {
                        state.balances_mt.entry((*symbol).clone()).or_default();
                    }
                    ExecutionState::Light(state) => {
                        state.balances.entry((*symbol).clone()).or_default();
                    }
                    ExecutionState::ZkVm(_) => {}
                }
                self.balances_merkle_roots
                    .insert((*symbol).clone(), sparse_merkle_tree::H256::zero().into());
            }
        }

        Ok(vec![OrderbookEvent::PairCreated {
            pair: pair.clone(),
            info: info.clone(),
        }])
    }

    fn register_asset(&mut self, symbol: &Symbol, asset_info: &AssetInfo) -> Result<(), String> {
        match self.assets_info.get_mut(symbol) {
            Some(existing) => {
                if existing.scale != asset_info.scale
                    || existing.contract_name != asset_info.contract_name
                {
                    return Err(format!(
                        "Symbol {symbol} already registered with different parameters"
                    ));
                }
            }
            None => {
                if asset_info.scale >= 20 {
                    return Err(format!(
                        "Scale too large for {symbol}: {} while maximum is 20",
                        asset_info.scale
                    ));
                }
                self.assets_info.insert(symbol.clone(), asset_info.clone());
            }
        }

        Ok(())
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
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
                    .light
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

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn deposit(
        &mut self,
        symbol: &str,
        amount: u64,
        user_info: &UserInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Compute the new balance
        let balance = self.get_balance(user_info, symbol);
        let new_balance = Balance(balance.0.checked_add(amount).ok_or("Balance overflow")?);

        self.update_balances(symbol, vec![(user_info.get_key(), new_balance.clone())])
            .map_err(|e| e.to_string())?;

        let events = match self.execution_state.mode() {
            ExecutionMode::Full | ExecutionMode::Light => {
                vec![OrderbookEvent::BalanceUpdated {
                    user: user_info.user.clone(),
                    symbol: symbol.to_string(),
                    amount: new_balance.0,
                }]
            }
            ExecutionMode::ZkVm => vec![],
        };

        Ok(events)
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn withdraw(
        &mut self,
        symbol: &str,
        amount: &u64,
        user_info: &UserInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        let balance = self.get_balance(user_info, symbol);

        if balance.0 < *amount {
            return Err(format!(
                "Could not withdraw: Insufficient balance: user {} has {balance:?} {symbol} symbols, trying to withdraw {amount}", user_info.user
            ));
        }

        self.deduct_from_account(symbol, user_info, *amount)
            .map_err(|e| e.to_string())?;

        let mut events = match self.execution_state.mode() {
            ExecutionMode::Light | ExecutionMode::Full => {
                let user_balance = self.get_balance(user_info, symbol);
                vec![OrderbookEvent::BalanceUpdated {
                    user: user_info.user.clone(),
                    symbol: symbol.to_string(),
                    amount: user_balance.0,
                }]
            }
            ExecutionMode::ZkVm => vec![],
        };

        // Increment user's nonce
        events.push(self.increment_nonce_and_save_user_info(user_info)?);

        Ok(events)
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
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

        let required_symbol = match &order.order_side {
            OrderSide::Bid => order.pair.1.clone(),
            OrderSide::Ask => order.pair.0.clone(),
        };

        // Refund the reserved amount to the user
        self.fund_account(&required_symbol, user_info, &Balance(order.quantity))
            .map_err(|e| e.to_string())?;

        // Cancel order through order manager
        let mut events = self.order_manager.cancel_order(&order_id)?;

        match self.execution_state.mode() {
            ExecutionMode::Light | ExecutionMode::Full => {
                let user_balance = self.get_balance(user_info, &required_symbol);

                events.push(OrderbookEvent::BalanceUpdated {
                    user: user_info.user.clone(),
                    symbol: required_symbol.to_string(),
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
        calldata: &sdk::Calldata,
        user_info: &UserInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Logic to allow user to escape with their funds
        let Some(tx_ctx) = &calldata.tx_ctx else {
            return Err("Escape needs transaction context".to_string());
        };

        // TODO: make this configurable
        if tx_ctx.block_height <= self.last_block_number + 5_000 {
            return Err(format!(
                "Escape can't be performed. Please wait {} blocks",
                5_000 - (tx_ctx.block_height.0 - self.last_block_number.0)
            ));
        }

        let mut events = Vec::new();

        // Keep track of user's balance for each token
        let mut user_balances = self.get_user_balances(&user_info.get_key());

        // Find and cancel all orders that belong to this user and cancel them
        let user_orders = self
            .order_manager
            .orders_owner
            .iter()
            .filter_map(|(order_id, owner_key)| {
                if owner_key == &user_info.get_key() {
                    self.order_manager.orders.get(order_id)
                } else {
                    None
                }
            })
            .cloned()
            .collect::<Vec<_>>();

        for order in user_orders {
            // Cancel order
            events.extend(self.order_manager.cancel_order(&order.order_id)?);
            let required_symbol = match &order.order_side {
                OrderSide::Bid => order.pair.1.clone(),
                OrderSide::Ask => order.pair.0.clone(),
            };
            // Virtually refund user
            let user_balance = user_balances.entry(required_symbol).or_default();
            *user_balance = Balance(user_balance.0 + order.quantity);
        }

        // Update all balances in the SMT
        for (symbol, balance) in user_balances {
            // Remove all balance from user
            self.update_balances(&symbol, vec![(user_info.get_key(), Balance(0))])
                .map_err(|e| e.to_string())?;

            match self.execution_state.mode() {
                ExecutionMode::Light | ExecutionMode::Full => {
                    events.push(OrderbookEvent::BalanceUpdated {
                        user: user_info.user.clone(),
                        symbol: symbol.to_string(),
                        amount: 0,
                    });
                }
                ExecutionMode::ZkVm => {}
            }

            // Skip verification for zero balances
            if balance.0 == 0 {
                continue;
            }

            // Ensure there is a transfer blob for this token with the correct amount
            let mut found_valid_transfer = false;

            let Some(asset_info) = self.assets_info.get(&symbol) else {
                return Err(format!("Asset info for symbol {symbol} not found"));
            };

            for (_, blob) in calldata.blobs.iter() {
                if blob.contract_name == asset_info.contract_name {
                    let Ok(structured) = StructuredBlob::<SmtTokenAction>::try_from(blob.clone())
                    else {
                        continue;
                    };

                    if let SmtTokenAction::Transfer {
                        sender,
                        recipient,
                        amount,
                    } = structured.data.parameters
                    {
                        if sender.0 == ORDERBOOK_ACCOUNT_IDENTITY
                            && recipient.0 == user_info.user
                            && amount == balance.0 as u128
                        {
                            found_valid_transfer = true;
                            break;
                        }
                    }
                }
            }

            if !found_valid_transfer {
                return Err(format!(
                    "No valid escape transfer blob found for symbol {symbol} with amount {} for user {}",
                    balance.0,
                    user_info.user
                ));
            }
        }
        Ok(events)
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
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
        let base_asset_info = self
            .assets_info
            .get(&order.pair.0)
            .ok_or(format!("Asset info for {} not found", order.pair.0))?;
        let base_scale = POW10[base_asset_info.scale as usize];

        // Delegate order execution to the manager
        let order_events = self.order_manager.execute_order(user_info_key, &order)?;

        events.extend(order_events);

        // Balance change aggregation system based on events
        let mut balance_changes: HashMap<Symbol, HashMap<H256, Balance>> = self.get_balances();
        let mut touched_accounts: HashMap<Symbol, HashSet<H256>> = HashMap::new();

        // Helper function to record balance changes
        fn record_balance_change(
            balance_changes: &mut HashMap<Symbol, HashMap<H256, Balance>>,
            touched_accounts: &mut HashMap<Symbol, HashSet<H256>>,
            user_info_key: &H256,
            symbol: &Symbol,
            amount: i128,
        ) -> Result<(), String> {
            let symbol_balances = balance_changes.get_mut(symbol);
            let symbol_balances = match symbol_balances {
                Some(sb) => sb,
                None => return Err(format!("Symbol {symbol} not found in balance_changes")),
            };

            let balance = symbol_balances.entry(*user_info_key).or_default();

            let new_value: u64 = ((balance.0 as i128) + amount).try_into().map_err(|e| {
                format!(
                    "User with key {} cannot perform {symbol} exchange: balance is {}, attempted to add {amount}: {e}", hex::encode(user_info_key.as_slice()), balance.0
                )
            })?;

            *balance = Balance(new_value);
            touched_accounts
                .entry(symbol.clone())
                .or_default()
                .insert(*user_info_key);
            Ok(())
        }

        // Helper function to record transfers between users
        fn record_transfer(
            balance_changes: &mut HashMap<Symbol, HashMap<H256, Balance>>,
            touched_accounts: &mut HashMap<Symbol, HashSet<H256>>,
            from: &H256,
            to: &H256,
            symbol: &Symbol,
            amount: i128,
        ) -> Result<(), String> {
            record_balance_change(balance_changes, touched_accounts, from, symbol, -amount)?;
            record_balance_change(balance_changes, touched_accounts, to, symbol, amount)?;
            Ok(())
        }

        // Process events to calculate balance changes
        for event in &events {
            match event {
                OrderbookEvent::OrderCreated {
                    order: created_order,
                } => {
                    // Deduct liquidity for created order
                    let (quantity, symbol) = match created_order.order_side {
                        OrderSide::Bid => (
                            -((created_order.quantity * created_order.price.unwrap() / base_scale)
                                as i128),
                            created_order.pair.1.clone(),
                        ),
                        OrderSide::Ask => (
                            -(created_order.quantity as i128),
                            created_order.pair.0.clone(),
                        ),
                    };
                    record_balance_change(
                        &mut balance_changes,
                        &mut touched_accounts,
                        user_info_key,
                        &symbol,
                        quantity,
                    )?;
                }
                OrderbookEvent::OrderExecuted { order_id, pair, .. } => {
                    let base_symbol = &pair.0;
                    let quote_symbol = &pair.1;

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

                    // Transfer logic for executed orders
                    if let Some(executed_order) = self.order_manager.orders.get(order_id) {
                        match executed_order.order_side {
                            OrderSide::Bid => {
                                // Executed order owner receives base symbol deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    executed_order_user_info,
                                    base_symbol,
                                    executed_order.quantity as i128,
                                )?;
                                // User receives quote symbol
                                record_balance_change(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    quote_symbol,
                                    (executed_order.price.unwrap() * executed_order.quantity
                                        / base_scale) as i128,
                                )?;
                                touched_accounts
                                    .entry(quote_symbol.clone())
                                    .or_default()
                                    .insert(*executed_order_user_info);
                            }
                            OrderSide::Ask => {
                                // Executed order owner receives quote symbol deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    executed_order_user_info,
                                    quote_symbol,
                                    (executed_order.price.unwrap() * executed_order.quantity
                                        / base_scale) as i128,
                                )?;
                                // User receives base symbol
                                record_balance_change(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    base_symbol,
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
                    let updated_order_user_info = self.order_manager.orders_owner.get(order_id).ok_or_else(|| {
                            format!(
                                "Executed order owner info (order_id: {order_id}) not found in order manager",
                            )
                        })?;

                    let base_symbol = &pair.0;
                    let quote_symbol = &pair.1;

                    // Transfer logic for executed orders
                    if let Some(updated_order) = self.order_manager.orders.get(order_id) {
                        match updated_order.order_side {
                            OrderSide::Bid => {
                                // Executed order owner receives base symbol deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    updated_order_user_info,
                                    base_symbol,
                                    *executed_quantity as i128,
                                )?;
                                // User receives quote symbol
                                record_balance_change(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    quote_symbol,
                                    (updated_order.price.unwrap() * executed_quantity / base_scale)
                                        as i128,
                                )?;
                                touched_accounts
                                    .entry(quote_symbol.clone())
                                    .or_default()
                                    .insert(*updated_order_user_info);
                            }
                            OrderSide::Ask => {
                                // Executed order owner receives quote symbol deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    updated_order_user_info,
                                    quote_symbol,
                                    (updated_order.price.unwrap() * executed_quantity / base_scale)
                                        as i128,
                                )?;
                                // User receives base symbol
                                record_balance_change(
                                    &mut balance_changes,
                                    &mut touched_accounts,
                                    user_info_key,
                                    base_symbol,
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
        for (symbol, user_keys) in touched_accounts {
            let symbol_balances = balance_changes
                .get(&symbol)
                .ok_or_else(|| format!("{symbol} not found in balance_changes"))?;

            let mut balances_to_update: Vec<(H256, Balance)> = Vec::new();
            for user_key in user_keys {
                let amount = symbol_balances.get(&user_key).ok_or_else(|| {
                    format!(
                        "User with key {} not found in balance_changes for {symbol}",
                        hex::encode(user_key.as_slice())
                    )
                })?;

                if !matches!(self.execution_state.mode(), ExecutionMode::ZkVm) {
                    let user_info = self.get_user_info_from_key(&user_key).unwrap();
                    events.push(OrderbookEvent::BalanceUpdated {
                        user: user_info.user.clone(),
                        symbol: symbol.clone(),
                        amount: amount.0,
                    });
                }

                balances_to_update.push((user_key, amount.clone()));
            }

            self.update_balances(&symbol, balances_to_update)?;
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
            BlockHeight(0),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_data(
        lane_id: LaneId,
        mode: ExecutionMode,
        secret: Vec<u8>,
        pairs_info: BTreeMap<Pair, PairInfo>,
        order_manager: OrderManager,
        users_info: HashMap<String, UserInfo>,
        balances: HashMap<Symbol, HashMap<H256, Balance>>,
        last_block_number: BlockHeight,
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
                .map(|(symbol, balances)| (symbol.clone(), (*balances.root()).into()))
                .collect(),
            _ => panic!("Business logic error. full_state should be Full"),
        };
        let hashed_secret = Sha256::digest(&secret).into();

        let mut orderbook = Orderbook {
            hashed_secret,
            assets_info: BTreeMap::new(),
            lane_id,
            balances_merkle_roots,
            users_info_merkle_root,
            order_manager,
            execution_state,
            last_block_number,
        };

        for (pair, info) in pairs_info {
            orderbook.create_pair(&pair, &info)?;
        }

        Ok(orderbook)
    }

    pub fn get_user_balances(&self, user_key: &H256) -> HashMap<Symbol, Balance> {
        let mut user_balances = HashMap::new();
        for (symbol, balances) in self.get_balances() {
            if let Some(balance) = balances.get(user_key) {
                user_balances.insert(symbol, balance.clone());
            }
        }
        user_balances
    }

    pub fn get_balances(&self) -> HashMap<Symbol, HashMap<H256, Balance>> {
        match &self.execution_state {
            ExecutionState::Full(state) => state.light.balances.clone(),
            ExecutionState::Light(state) => state.balances.clone(),
            ExecutionState::ZkVm(state) => {
                let mut balances: HashMap<Symbol, HashMap<H256, Balance>> = HashMap::new();
                for (symbol, witness) in state.balances.iter() {
                    balances.insert(symbol.clone(), witness.value.clone());
                }
                balances
            }
        }
    }

    pub fn get_balance(&self, user: &UserInfo, symbol: &str) -> Balance {
        match &self.execution_state {
            ExecutionState::Full(state) => state
                .light
                .balances
                .get(symbol)
                .and_then(|tree| tree.get(&user.get_key()).cloned())
                .unwrap_or_default(),
            ExecutionState::Light(state) => state
                .balances
                .get(symbol)
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
                    .get(symbol)
                    .and_then(|user_balances| user_balances.value.get(&user_key).cloned())
                    .unwrap_or_default()
            }
        }
    }

    pub fn is_blob_whitelisted(&self, contract_name: &ContractName) -> bool {
        if contract_name.0 == "orderbook" {
            return true;
        }

        self.assets_info.contains_key(&contract_name.0)
            || self
                .assets_info
                .values()
                .any(|info| &info.contract_name == contract_name)
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
            ExecutionState::Full(state) => state
                .light
                .users_info
                .get(user)
                .cloned()
                .ok_or_else(|| format!("User info for '{user}' not found")),
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

        if self.assets_info != other.assets_info {
            let mut mismatches = Vec::new();

            for (symbol, info) in &self.assets_info {
                match other.assets_info.get(symbol) {
                    Some(other_info) if other_info == info => {}
                    Some(other_info) => {
                        mismatches.push(format!("{symbol}: {info:?} != {other_info:?}"))
                    }
                    None => mismatches.push(format!("{symbol}: present only on self: {info:?}")),
                }
            }

            for (symbol, info) in &other.assets_info {
                if !self.assets_info.contains_key(symbol) {
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
