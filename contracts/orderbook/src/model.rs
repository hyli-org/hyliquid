use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    order_manager::OrderManager,
    transaction::{OrderbookAction, PermissionnedOrderbookAction},
};
use sdk::{merkle_utils::BorshableMerkleProof, ContractName, TxContext};

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
impl ExecuteState {
    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn create_pair(
        &mut self,
        pair: &Pair,
        info: &PairInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        self.register_asset(&pair.0, &info.base)?;
        self.register_asset(&pair.1, &info.quote)?;

        for symbol in &[&pair.0, &pair.1] {
            self.balances.entry((*symbol).clone()).or_default();
        }

        Ok(vec![OrderbookEvent::PairCreated {
            pair: pair.clone(),
            info: info.clone(),
        }])
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

        self.users_info
            .insert(user_info.user.clone(), user_info.clone());

        let mut events = vec![OrderbookEvent::SessionKeyAdded {
            user: user_info.user.to_string(),
            salt: user_info.salt.clone(),
            nonce: user_info.nonce,
            session_keys: user_info.session_keys.clone(),
        }];

        if user_info.nonce == 0 {
            // We incremente nonce
            events.push(self.increment_nonce_and_save_user_info(&user_info)?);
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

        Ok(vec![OrderbookEvent::BalanceUpdated {
            user: user_info.user.clone(),
            symbol: symbol.to_string(),
            amount: new_balance.0,
        }])
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

        let user_balance = self.get_balance(user_info, symbol);

        Ok(vec![
            OrderbookEvent::BalanceUpdated {
                user: user_info.user.clone(),
                symbol: symbol.to_string(),
                amount: user_balance.0,
            },
            self.increment_nonce_and_save_user_info(user_info)?,
        ])
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

        let user_balance = self.get_balance(user_info, &required_symbol);

        events.push(OrderbookEvent::BalanceUpdated {
            user: user_info.user.clone(),
            symbol: required_symbol.to_string(),
            amount: user_balance.0,
        });

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

    // pub fn init(lane_id: LaneId, secret: Vec<u8>) -> Result<Self, String> {
    //     Self::from_data(
    //         lane_id,
    //         secret,
    //         BTreeMap::new(),
    //         OrderManager::default(),
    //         HashMap::new(),
    //         HashMap::new(),
    //     )
    // }

    // pub fn from_data(
    //     lane_id: LaneId,
    //     mode: ExecutionMode,
    //     secret: Vec<u8>,
    //     pairs_info: BTreeMap<Pair, PairInfo>,
    //     order_manager: OrderManager,
    //     users_info: HashMap<String, UserInfo>,
    //     balances: HashMap<Symbol, HashMap<H256, Balance>>,
    // ) -> Result<Self, String> {
    //     let full_state =
    //         ExecutionState::from_data(ExecutionMode::Full, users_info.clone(), balances.clone())?;

    //     let execution_state = if mode == ExecutionMode::Full {
    //         full_state.clone()
    //     } else {
    //         ExecutionState::from_data(mode, users_info, balances)?
    //     };

    //     let users_info_merkle_root = match &full_state {
    //         ExecutionState::Full(state) => (*state.users_info_mt.root()).into(),
    //         _ => panic!("Business logic error. full_state should be Full"),
    //     };
    //     let balances_merkle_roots = match &full_state {
    //         ExecutionState::Full(state) => state
    //             .balances_mt
    //             .iter()
    //             .map(|(symbol, balances)| (symbol.clone(), (*balances.root()).into()))
    //             .collect(),
    //         _ => panic!("Business logic error. full_state should be Full"),
    //     };
    //     let hashed_secret = Sha256::digest(&secret).into();

    //     let mut orderbook = Orderbook {
    //         hashed_secret,
    //         assets_info: BTreeMap::new(),
    //         lane_id,
    //         balances_merkle_roots,
    //         users_info_merkle_root,
    //         order_manager,
    //         execution_state,
    //     };

    //     for (pair, info) in pairs_info {
    //         orderbook.create_pair(&pair, &info)?;
    //     }

    //     Ok(orderbook)
    // }

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

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
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

    pub(crate) fn execute_order(
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

                let user_info = self.get_user_info_from_key(&user_key).unwrap();
                events.push(OrderbookEvent::BalanceUpdated {
                    user: user_info.user.clone(),
                    symbol: symbol.clone(),
                    amount: amount.0,
                });

                balances_to_update.push((user_key, amount.clone()));
            }

            self.update_balances(&symbol, balances_to_update)?;
        }

        events.push(self.increment_nonce_and_save_user_info(user_info)?);

        Ok(events)
    }

    // Detects differences between two orderbooks
    // It is used to detect differences between on-chain and db orderbooks
    // pub fn diff(&self, other: &LightState) -> BTreeMap<String, String> {
    //     let mut diff = BTreeMap::new();
    //     if self.hashed_secret != other.hashed_secret {
    //         diff.insert(
    //             "hashed_secret".to_string(),
    //             format!(
    //                 "{} != {}",
    //                 hex::encode(self.hashed_secret.as_slice()),
    //                 hex::encode(other.hashed_secret.as_slice())
    //             ),
    //         );
    //     }

    //     if self.assets_info != other.assets_info {
    //         let mut mismatches = Vec::new();

    //         for (symbol, info) in &self.assets_info {
    //             match other.assets_info.get(symbol) {
    //                 Some(other_info) if other_info == info => {}
    //                 Some(other_info) => {
    //                     mismatches.push(format!("{symbol}: {info:?} != {other_info:?}"))
    //                 }
    //                 None => mismatches.push(format!("{symbol}: present only on self: {info:?}")),
    //             }
    //         }

    //         for (symbol, info) in &other.assets_info {
    //             if !self.assets_info.contains_key(symbol) {
    //                 mismatches.push(format!("{symbol}: present only on other: {info:?}"));
    //             }
    //         }

    //         diff.insert("symbols_info".to_string(), mismatches.join("; "));
    //     }

    //     if self.lane_id != other.lane_id {
    //         diff.insert(
    //             "lane_id".to_string(),
    //             format!(
    //                 "{} != {}",
    //                 hex::encode(&self.lane_id.0 .0),
    //                 hex::encode(&other.lane_id.0 .0)
    //             ),
    //         );
    //     }

    //     if self.balances_merkle_roots != other.balances_merkle_roots {
    //         diff.insert(
    //             "balances_merkle_roots".to_string(),
    //             format!(
    //                 "{:?} != {:?}",
    //                 self.balances_merkle_roots, other.balances_merkle_roots
    //             ),
    //         );
    //     }

    //     if self.users_info_merkle_root != other.users_info_merkle_root {
    //         diff.insert(
    //             "users_info_merkle_root".to_string(),
    //             format!(
    //                 "{} != {}",
    //                 hex::encode(self.users_info_merkle_root.as_slice()),
    //                 hex::encode(other.users_info_merkle_root.as_slice())
    //             ),
    //         );
    //     }

    //     if self.order_manager != other.order_manager {
    //         diff.extend(self.order_manager.diff(&other.order_manager));
    //     }
    //     if self.execution_state.mode() != other.execution_state.mode() {
    //         diff.insert(
    //             "execution_state".to_string(),
    //             format!(
    //                 "{:?} != {:?}",
    //                 self.execution_state.mode(),
    //                 other.execution_state.mode()
    //             ),
    //         );
    //     }
    //     diff
    // }
}

#[derive(
    Debug, Default, Clone, PartialEq, BorshDeserialize, BorshSerialize, Serialize, Deserialize,
)]
pub struct Balance(pub u64);

#[derive(
    BorshSerialize, BorshDeserialize, Default, Debug, Clone, Eq, PartialEq, Ord, PartialOrd,
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
