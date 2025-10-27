use borsh::{BorshDeserialize, BorshSerialize};
use hyli_smt_token::SmtTokenAction;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    order_manager::OrderManager,
    transaction::{OrderbookAction, PermissionnedOrderbookAction},
    zk::smt::GetKey,
    ORDERBOOK_ACCOUNT_IDENTITY,
};
use sdk::{BlockHeight, ContractName, StructuredBlob};

use crate::zk::H256;

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct ExecuteState {
    pub assets_info: HashMap<Symbol, AssetInfo>, // symbol -> (decimals, precision)
    pub users_info: HashMap<String, UserInfo>,
    pub balances: HashMap<Symbol, HashMap<H256, Balance>>,
    pub order_manager: OrderManager,
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
#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
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

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, BorshDeserialize, BorshSerialize, PartialEq, Eq)]
pub struct WithdrawDestination {
    pub network: String,
    pub address: String,
}

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
impl ExecuteState {
    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn create_pair(&self, pair: &Pair, info: &PairInfo) -> Result<Vec<OrderbookEvent>, String> {
        self.ensure_asset_registration(&pair.0, &info.base)?;
        self.ensure_asset_registration(&pair.1, &info.quote)?;

        Ok(vec![OrderbookEvent::PairCreated {
            pair: pair.clone(),
            info: info.clone(),
        }])
    }

    fn ensure_asset_registration(
        &self,
        symbol: &Symbol,
        asset_info: &AssetInfo,
    ) -> Result<(), String> {
        match self.assets_info.get(symbol) {
            Some(existing) => {
                if existing.scale != asset_info.scale
                    || existing.contract_name != asset_info.contract_name
                {
                    Err(format!(
                        "Symbol {symbol} already registered with different parameters"
                    ))
                } else {
                    Ok(())
                }
            }
            None => {
                if asset_info.scale >= 20 {
                    Err(format!(
                        "Scale too large for {symbol}: {} while maximum is 20",
                        asset_info.scale
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }

    fn nonce_increment_event(user_info: &UserInfo) -> Result<OrderbookEvent, String> {
        let next_nonce = user_info.nonce.checked_add(1).ok_or("Nonce overflow")?;

        Ok(OrderbookEvent::NonceIncremented {
            user: user_info.user.clone(),
            nonce: next_nonce,
        })
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn add_session_key(
        &self,
        user_info: UserInfo,
        pubkey: &Vec<u8>,
    ) -> Result<Vec<OrderbookEvent>, String> {
        if user_info.session_keys.contains(pubkey) {
            return Err("Session key already exists".to_string());
        }

        let mut updated_user_info = user_info.clone();
        updated_user_info.session_keys.push(pubkey.clone());

        let mut events = vec![OrderbookEvent::SessionKeyAdded {
            user: updated_user_info.user.to_string(),
            salt: updated_user_info.salt.clone(),
            nonce: updated_user_info.nonce,
            session_keys: updated_user_info.session_keys.clone(),
        }];

        if updated_user_info.nonce == 0 {
            events.push(Self::nonce_increment_event(&updated_user_info)?);
        }

        Ok(events)
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn deposit(
        &self,
        symbol: &str,
        amount: u64,
        user_info: &UserInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Compute the new balance
        let balance = self.get_balance(user_info, symbol);
        let new_balance = Balance(balance.0.checked_add(amount).ok_or("Balance overflow")?);

        Ok(vec![OrderbookEvent::BalanceUpdated {
            user: user_info.user.clone(),
            symbol: symbol.to_string(),
            amount: new_balance.0,
        }])
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn withdraw(
        &self,
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

        let new_total = balance.0 - *amount;

        let mut events = vec![OrderbookEvent::BalanceUpdated {
            user: user_info.user.clone(),
            symbol: symbol.to_string(),
            amount: new_total,
        }];

        events.push(Self::nonce_increment_event(user_info)?);

        Ok(events)
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn cancel_order(
        &self,
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

        let current_balance = self.get_balance(user_info, &required_symbol).0;
        let new_balance = current_balance
            .checked_add(order.quantity)
            .ok_or("Balance overflow")?;

        let events = vec![
            OrderbookEvent::OrderCancelled {
                order_id: order_id.clone(),
                pair: order.pair.clone(),
            },
            OrderbookEvent::BalanceUpdated {
                user: user_info.user.clone(),
                symbol: required_symbol,
                amount: new_balance,
            },
            Self::nonce_increment_event(user_info)?,
        ];

        Ok(events)
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn get_user_info_from_key(&self, key: &H256) -> Result<UserInfo, String> {
        self.users_info
            .iter()
            .find(|(_, info)| info.get_key() == *key)
            .map(|(_, info)| info.clone())
            .ok_or_else(|| {
                format!(
                    "No user info found for key {:?}",
                    hex::encode(key.as_slice())
                )
            })
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn has_user_info_key(&self, user_info_key: H256) -> Result<bool, String> {
        Ok(self
            .users_info
            .values()
            .any(|user_info| user_info.get_key() == user_info_key))
    }

    pub fn from_data(
        pairs_info: BTreeMap<Pair, PairInfo>,
        order_manager: OrderManager,
        users_info: HashMap<String, UserInfo>,
        balances: HashMap<Symbol, HashMap<H256, Balance>>,
    ) -> Result<Self, String> {
        let mut orderbook = ExecuteState {
            assets_info: HashMap::new(),
            users_info,
            balances,
            order_manager,
        };

        for (pair, info) in pairs_info {
            let events = orderbook.create_pair(&pair, &info)?;
            orderbook.apply_events(&UserInfo::default(), &events)?;
        }

        Ok(orderbook)
    }

    pub fn get_balances(&self) -> HashMap<Symbol, HashMap<H256, Balance>> {
        self.balances.clone()
    }

    pub fn get_balance(&self, user: &UserInfo, symbol: &str) -> Balance {
        self.balances
            .get(symbol)
            .and_then(|balances| balances.get(&user.get_key()).cloned())
            .unwrap_or_default()
    }

    pub fn get_orders(&self) -> BTreeMap<String, Order> {
        self.order_manager.orders.clone()
    }

    pub fn apply_events(
        &mut self,
        user_info: &UserInfo,
        events: &[OrderbookEvent],
    ) -> Result<(), String> {
        for event in events {
            match event {
                OrderbookEvent::PairCreated { pair, info } => {
                    self.register_asset(&pair.0, &info.base)?;
                    self.register_asset(&pair.1, &info.quote)?;
                    self.balances.entry(pair.0.clone()).or_default();
                    self.balances.entry(pair.1.clone()).or_default();
                }
                OrderbookEvent::BalanceUpdated {
                    user,
                    symbol,
                    amount,
                } => {
                    let user_info = if user == &user_info.user {
                        user_info.clone()
                    } else {
                        self.get_user_info(user)?
                    };
                    self.update_balances(symbol, vec![(user_info.get_key(), Balance(*amount))])?;
                }
                OrderbookEvent::SessionKeyAdded {
                    user,
                    salt,
                    nonce,
                    session_keys,
                    ..
                } => {
                    let entry = self
                        .users_info
                        .entry(user.clone())
                        .or_insert_with(|| UserInfo {
                            user: user.clone(),
                            salt: salt.clone(),
                            nonce: *nonce,
                            session_keys: session_keys.clone(),
                        });

                    entry.salt = salt.clone();
                    entry.nonce = *nonce;
                    entry.session_keys = session_keys.clone();
                }
                OrderbookEvent::NonceIncremented { user, nonce } => {
                    let entry = self
                        .users_info
                        .entry(user.clone())
                        .or_insert(user_info.clone());
                    entry.nonce = *nonce;
                }
                _ => {}
            }
        }

        self.order_manager.apply_events(user_info.get_key(), events)
    }

    pub fn get_order_owner(&self, order_id: &OrderId) -> Option<&H256> {
        self.order_manager.orders_owner.get(order_id)
    }

    pub fn get_user_info(&self, user: &str) -> Result<UserInfo, String> {
        self.users_info
            .get(user)
            .cloned()
            .ok_or_else(|| format!("User info not found for user '{user}'"))
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn increment_nonce_and_save_user_info(
        &mut self,
        user_info: &UserInfo,
    ) -> Result<OrderbookEvent, String> {
        let mut updated_user_info = user_info.clone();
        updated_user_info.nonce = updated_user_info
            .nonce
            .checked_add(1)
            .ok_or("Nonce overflow")?;

        self.users_info
            .insert(updated_user_info.user.clone(), updated_user_info.clone());

        Ok(OrderbookEvent::NonceIncremented {
            user: user_info.user.clone(),
            nonce: updated_user_info.nonce,
        })
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn update_balances(
        &mut self,
        symbol: &str,
        balances_to_update: Vec<(H256, Balance)>,
    ) -> Result<(), String> {
        self.balances
            .entry(symbol.to_string())
            .or_default()
            .extend(balances_to_update);

        Ok(())
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
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
                        }), _
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

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn execute_order(
        &self,
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
        let order_events = self.order_manager.execute_order_dry_run(&order)?;

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
                    "User with key {} cannot perform {symbol} exchange: balance is {balance:?}, attempted to add {amount}: {e}",
                    hex::encode(user_info_key.as_slice()),
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

        // Updating balances
        for (symbol, user_keys) in touched_accounts {
            let symbol_balances = balance_changes
                .get(&symbol)
                .ok_or_else(|| format!("{symbol} not found in balance_changes"))?;

            for user_key in user_keys {
                let amount = symbol_balances.get(&user_key).ok_or_else(|| {
                    format!(
                        "User with key {} not found in balance_changes for {symbol}",
                        hex::encode(user_key.as_slice())
                    )
                })?;

                let user_name = if user_key == *user_info_key {
                    user_info.user.clone()
                } else {
                    self.get_user_info_from_key(&user_key)?.user.clone()
                };

                events.push(OrderbookEvent::BalanceUpdated {
                    user: user_name,
                    symbol: symbol.clone(),
                    amount: amount.0,
                });
            }
        }

        events.push(Self::nonce_increment_event(user_info)?);

        Ok(events)
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

    pub fn escape(
        &mut self,
        last_block_number: &BlockHeight,
        calldata: &sdk::Calldata,
        user_info: &UserInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Logic to allow user to escape with their funds
        let Some(tx_ctx) = &calldata.tx_ctx else {
            return Err("Escape needs transaction context".to_string());
        };

        // TODO: make this configurable
        if tx_ctx.block_height <= *last_block_number + 5_000 {
            return Err(format!(
                "Escape can't be performed. Please wait {} blocks",
                5_000 - (tx_ctx.block_height.0 - last_block_number.0)
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

            events.push(OrderbookEvent::BalanceUpdated {
                user: user_info.user.clone(),
                symbol: symbol.to_string(),
                amount: 0,
            });

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
}

#[derive(
    Debug,
    Default,
    Clone,
    PartialEq,
    BorshDeserialize,
    BorshSerialize,
    Serialize,
    Deserialize,
    Eq,
    Ord,
    PartialOrd,
    Hash,
)]
pub struct Balance(pub u64);

#[derive(
    BorshSerialize,
    BorshDeserialize,
    Default,
    Debug,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Serialize,
    Deserialize,
)]
pub struct UserInfo {
    pub user: String,
    pub salt: Vec<u8>,
    pub nonce: u32,
    pub session_keys: Vec<Vec<u8>>,
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
