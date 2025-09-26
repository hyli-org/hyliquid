use borsh::{io::Error, BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::{BorshableMerkleProof, SHA256Hasher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparse_merkle_tree::default_store::DefaultStore;
use sparse_merkle_tree::traits::Value;
use sparse_merkle_tree::{SparseMerkleTree, H256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::smt_values::{Balance, UserInfo};
use sdk::{ContractName, LaneId, TxContext};

#[derive(BorshSerialize, BorshDeserialize, Default, Debug)]
pub struct Orderbook {
    // Server secret for authentication on permissionned actions
    pub hashed_secret: [u8; 32],
    // Registered token pairs with asset scales
    pub pairs_info: BTreeMap<TokenPair, PairInfo>,
    // Validator public key of the lane this orderbook is running on
    pub lane_id: LaneId,

    // Balances merkle tree root for each token
    pub balances_merkle_roots: BTreeMap<TokenName, [u8; 32]>,
    // Users info merkle root
    pub users_info_merkle_root: [u8; 32],

    // All orders indexed by order_id
    pub orders: BTreeMap<OrderId, Order>,
    // Buy orders sorted by price (highest first) for each token pair
    pub buy_orders: BTreeMap<TokenPair, VecDeque<OrderId>>,
    // Sell orders sorted by price (lowest first) for each token pair
    pub sell_orders: BTreeMap<TokenPair, VecDeque<OrderId>>,

    /// These fields are not committed on-chain
    #[borsh(skip)]
    pub server_execution: bool,
    // User balances per token: token -> smt(hash(user) -> user_account))
    #[borsh(skip)]
    pub balances:
        BTreeMap<TokenName, SparseMerkleTree<SHA256Hasher, Balance, DefaultStore<Balance>>>,
    #[borsh(skip)]
    // Users info merkle tree
    pub users_info_mt: SparseMerkleTree<SHA256Hasher, UserInfo, DefaultStore<UserInfo>>,
    #[borsh(skip)]
    // Users info salts. user -> salt
    pub users_info_salt: BTreeMap<String, Vec<u8>>,
    #[borsh(skip)]
    // Mapping of order IDs to their owners
    // TODO: Use the mt_key instead of user
    pub orders_owner: BTreeMap<OrderId, String>,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default, Debug, Clone)]
pub struct PairInfo {
    pub base_scale: u64,
    pub quote_scale: u64,
}

#[cfg_attr(feature = "sqlx", derive(sqlx::Type))]
#[cfg_attr(
    feature = "sqlx",
    sqlx(type_name = "order_side", rename_all = "lowercase")
)]
#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize)]
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

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize)]
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

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize)]
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

impl Clone for Orderbook {
    fn clone(&self) -> Self {
        let user_info_root: H256 = *self.users_info_mt.root();
        let user_info_store = self.users_info_mt.store().clone();
        let users_info = SparseMerkleTree::new(user_info_root, user_info_store);

        // Clone the SparseMerkleTree balances using the new function
        let mut balances: BTreeMap<
            TokenName,
            SparseMerkleTree<SHA256Hasher, Balance, DefaultStore<Balance>>,
        > = BTreeMap::new();
        for (token_name, tree) in &self.balances {
            let root = *tree.root();
            let store = tree.store().clone();
            let new_tree = SparseMerkleTree::new(root, store);
            balances.insert(token_name.clone(), new_tree);
        }

        // Clone all the simple fields
        Orderbook {
            hashed_secret: self.hashed_secret,
            pairs_info: self.pairs_info.clone(),
            lane_id: self.lane_id.clone(),
            users_info_merkle_root: self.users_info_merkle_root,
            balances_merkle_roots: self.balances_merkle_roots.clone(),
            users_info_mt: users_info,
            users_info_salt: self.users_info_salt.clone(),
            orders: self.orders.clone(),
            buy_orders: self.buy_orders.clone(),
            sell_orders: self.sell_orders.clone(),
            orders_owner: self.orders_owner.clone(),
            server_execution: self.server_execution,
            balances,
        }
    }
}

// TODO: refactor functions to distinguish business logic from state management.
// FIXME: once the refactor is done; investigate is we can remove the usage if server_execution
impl Orderbook {
    pub fn create_pair(
        &mut self,
        pair: TokenPair,
        info: PairInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        if info.base_scale >= 20 {
            return Err(format!(
                "Base scale too large: {}. Maximum is 19",
                info.base_scale
            ));
        }
        self.pairs_info.insert(pair.clone(), info.clone());

        // Initialize a new SparseMerkleTree for the token pair if not already present
        if !self.balances.contains_key(&pair.0) {
            self.balances.insert(
                pair.0.clone(),
                SparseMerkleTree::new(H256::zero(), Default::default()),
            );
            self.balances_merkle_roots
                .insert(pair.0.clone(), H256::zero().into());
        }
        if !self.balances.contains_key(&pair.1) {
            self.balances.insert(
                pair.1.clone(),
                SparseMerkleTree::new(H256::zero(), Default::default()),
            );
            self.balances_merkle_roots
                .insert(pair.1.clone(), H256::zero().into());
        }

        Ok(vec![OrderbookEvent::PairCreated { pair, info }])
    }

    pub fn add_session_key(
        &mut self,
        user_info: &mut UserInfo,
        pubkey: &Vec<u8>,
    ) -> Result<Vec<OrderbookEvent>, String> {
        if user_info.session_keys.contains(pubkey) {
            return Err("Session key already exists".to_string());
        }

        // Add the session key to the user's list of session keys
        user_info.session_keys.push(pubkey.clone());

        if self.server_execution {
            // If the user is unknown, add a salt for them
            if !self.users_info_salt.contains_key(&user_info.user) {
                self.users_info_salt
                    .insert(user_info.user.clone(), user_info.salt.clone());
            }

            Ok(vec![OrderbookEvent::SessionKeyAdded {
                user: user_info.user.to_string(),
            }])
        } else {
            Ok(vec![])
        }
    }

    pub fn deposit(
        &mut self,
        token: String,
        amount: u64,
        user_info: &UserInfo,
        balance: &mut Balance,
        balance_proof: &BorshableMerkleProof,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Compute the new balance
        let new_balance = Balance(balance.0.checked_add(amount).ok_or("Balance overflow")?);

        self.update_balances(
            &token,
            vec![(user_info, new_balance.clone())],
            balance_proof,
        )
        .map_err(|e| e.to_string())?;

        if self.server_execution {
            let user_balance = self.get_balance(user_info, &token);
            Ok(vec![OrderbookEvent::BalanceUpdated {
                user: user_info.user.clone(),
                token,
                amount: user_balance.0,
            }])
        } else {
            Ok(vec![])
        }
    }

    pub fn withdraw(
        &mut self,
        token: String,
        amount: u64,
        user_info: &UserInfo,
        balances: &BTreeMap<UserInfo, Balance>,
        balances_proof: &BorshableMerkleProof,
    ) -> Result<Vec<OrderbookEvent>, String> {
        let server_execution = self.server_execution;

        let balance = balances.get(user_info).ok_or_else(|| {
            format!(
                "No balance found for user {} during withdrawal",
                user_info.user
            )
        })?;

        if balance.0 < amount {
            return Err(format!(
                "Could not withdraw: Insufficient balance: user {} has {balance:?} {token} tokens, trying to withdraw {amount}", user_info.user
            ));
        }

        self.deduct_from_account(&token, user_info, amount, balances, balances_proof)
            .map_err(|e| e.to_string())?;

        if server_execution {
            let user_balance = self.get_balance(user_info, &token);
            Ok(vec![OrderbookEvent::BalanceUpdated {
                user: user_info.user.clone(),
                token,
                amount: user_balance.0,
            }])
        } else {
            Ok(vec![])
        }
    }

    pub fn cancel_order(
        &mut self,
        order_id: OrderId,
        user_info: &UserInfo,
        balance: &Balance,
        balance_proof: &BorshableMerkleProof,
    ) -> Result<Vec<OrderbookEvent>, String> {
        let order = self
            .orders
            .get(&order_id)
            .ok_or(format!("Order {order_id} not found"))?
            .clone();

        let required_token = match &order.order_side {
            OrderSide::Bid => order.pair.1.clone(),
            OrderSide::Ask => order.pair.0.clone(),
        };

        // Refund the reserved amount to the user
        self.fund_account(
            &required_token,
            user_info,
            &Balance(order.quantity),
            &BTreeMap::from([(user_info.clone(), balance.clone())]),
            balance_proof,
        )
        .map_err(|e| e.to_string())?;

        // Now that all operations have succeeded, remove the order from storage
        self.orders.remove(&order_id);

        // Remove from orders list
        match order.order_side {
            OrderSide::Bid => {
                if let Some(orders) = self.buy_orders.get_mut(&order.pair) {
                    orders.retain(|id| id != &order_id);
                }
            }
            OrderSide::Ask => {
                if let Some(orders) = self.sell_orders.get_mut(&order.pair) {
                    orders.retain(|id| id != &order_id);
                }
            }
        }

        let mut events = vec![OrderbookEvent::OrderCancelled {
            order_id,
            pair: order.pair,
        }];

        if self.server_execution {
            let user_balance = self.get_balance(user_info, &required_token);

            events.push(OrderbookEvent::BalanceUpdated {
                user: user_info.user.clone(),
                token: required_token.to_string(),
                amount: user_balance.0,
            });
        }
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
        mut order: Order,
        order_user_map: BTreeMap<OrderId, UserInfo>,
        balances: &BTreeMap<TokenName, BTreeMap<UserInfo, Balance>>,
        balances_proof: &BTreeMap<TokenName, BorshableMerkleProof>,
    ) -> Result<Vec<OrderbookEvent>, String> {
        let mut events = Vec::new();

        // New balance aggregation system: tracks net balance changes per user per token
        let mut balance_changes: BTreeMap<TokenName, BTreeMap<UserInfo, Balance>> =
            balances.clone();

        // Helper function to record balance changes
        fn record_balance_change(
            balance_changes: &mut BTreeMap<TokenName, BTreeMap<UserInfo, Balance>>,
            user_info: UserInfo,
            token: &TokenName,
            amount: i128,
        ) -> Result<(), String> {
            let user = user_info.user.clone();
            let token_balances = balance_changes.entry(token.clone()).or_default();
            let balance = token_balances.entry(user_info).or_default();

            let new_value: u64 = ((balance.0 as i128) + amount).try_into().map_err(|e| {
                format!(
                    "User {user} cannot perform token {token} exchange: balance is {}, attempted to add {amount}: {e}", balance.0
                )
            })?;
            *balance = Balance(new_value);
            Ok(())
        }

        // Helper function to record transfers between users
        fn record_transfer(
            balance_changes: &mut BTreeMap<TokenName, BTreeMap<UserInfo, Balance>>,
            from: UserInfo,
            to: UserInfo,
            token: &TokenName,
            amount: i128,
        ) -> Result<(), String> {
            record_balance_change(balance_changes, from, token, -amount)?;
            record_balance_change(balance_changes, to, token, amount)?;
            Ok(())
        }

        let mut order_to_insert: Option<Order> = None;

        let base_scale = POW10[self
            .pairs_info
            .get(&order.pair)
            .ok_or(format!("Pair {}/{} not found", order.pair.0, order.pair.1))?
            .base_scale as usize];

        // Try to fill already existing orders
        match &order.order_side {
            OrderSide::Bid => {
                let required_token = order.pair.1.clone();

                let sell_orders_option = self.sell_orders.get_mut(&order.pair);

                if sell_orders_option.is_none() && order.order_type == OrderType::Limit {
                    // If there are no sell orders and this is a limit order, add it to the orderbook
                    self.insert_order(&order, user_info)?;
                    events.push(OrderbookEvent::OrderCreated {
                        order: order.clone(),
                    });

                    // Remove liquitidy from the user balance
                    record_balance_change(
                        &mut balance_changes,
                        user_info.clone(),
                        &required_token,
                        -((order.quantity * order.price.unwrap()) as i128),
                    )?;

                    return Ok(events);
                } else if sell_orders_option.is_none() {
                    // If there are no sell orders and this is a market order, we cannot proceed
                    return Err(format!(
                        "No matching sell orders for market order {}",
                        order.order_id
                    ));
                }

                let sell_orders = sell_orders_option.unwrap();

                // Get the lowest price sell order
                while let Some(order_id) = sell_orders.pop_front() {
                    let existing_order = self
                        .orders
                        .get_mut(&order_id)
                        .ok_or(format!("Order {order_id} not found"))?;

                    let existing_order_user = order_user_map
                        .get(&order_id)
                        .ok_or(format!("Order user not found for order {order_id}"))?;

                    // If the order is a limit order, check if the *selling* price is lower than the limit price
                    if let Some(price) = order.price {
                        let existing_order_price = existing_order.price.expect(
                        "An order has been stored without a price limit. This should never happen",
                        );
                        if existing_order_price > price {
                            // Place the order in buy_orders
                            order_to_insert = Some(order);

                            // Put back the sell order we popped
                            sell_orders.push_front(order_id);
                            break;
                        }
                    }

                    // There is an order that can be filled
                    match existing_order.quantity.cmp(&order.quantity) {
                        std::cmp::Ordering::Greater => {
                            // The existing order do not fully cover this order
                            existing_order.quantity -= order.quantity;

                            sell_orders.push_front(order_id);

                            events.push(OrderbookEvent::OrderUpdate {
                                order_id: existing_order.order_id.clone(),
                                taker_order_id: order.order_id.clone(),
                                remaining_quantity: existing_order.quantity,
                                pair: order.pair.clone(),
                            });

                            // Send token from user to the order owner
                            record_transfer(
                                &mut balance_changes,
                                user_info.clone(),
                                existing_order_user.clone(),
                                &order.pair.1,
                                (existing_order.price.unwrap() * order.quantity / base_scale)
                                    as i128,
                            )?;
                            // Send token to the user
                            record_balance_change(
                                &mut balance_changes,
                                user_info.clone(),
                                &order.pair.0,
                                order.quantity as i128,
                            )?;

                            break;
                        }
                        std::cmp::Ordering::Equal => {
                            // The two orders are executed
                            events.push(OrderbookEvent::OrderExecuted {
                                order_id: order_id.clone(),
                                taker_order_id: order.order_id.clone(),
                                pair: order.pair.clone(),
                            });
                            events.push(OrderbookEvent::OrderExecuted {
                                order_id: order.order_id.clone(),
                                taker_order_id: order_id.clone(),
                                pair: order.pair.clone(),
                            });

                            // Send token to the order owner
                            record_transfer(
                                &mut balance_changes,
                                user_info.clone(),
                                existing_order_user.clone(),
                                &order.pair.1,
                                (existing_order.price.unwrap() * existing_order.quantity
                                    / base_scale) as i128,
                            )?;

                            // Send token to the user
                            record_balance_change(
                                &mut balance_changes,
                                user_info.clone(),
                                &order.pair.0,
                                existing_order.quantity as i128,
                            )?;

                            self.orders.remove(&order_id);
                            break;
                        }
                        std::cmp::Ordering::Less => {
                            // The existing order is fully filled
                            events.push(OrderbookEvent::OrderExecuted {
                                order_id: existing_order.order_id.clone(),
                                taker_order_id: order.order_id.clone(),
                                pair: order.pair.clone(),
                            });
                            record_transfer(
                                &mut balance_changes,
                                user_info.clone(),
                                existing_order_user.clone(),
                                &order.pair.1,
                                (existing_order.price.unwrap() * existing_order.quantity
                                    / base_scale) as i128,
                            )?;
                            record_balance_change(
                                &mut balance_changes,
                                user_info.clone(),
                                &order.pair.0,
                                existing_order.quantity as i128,
                            )?;

                            order.quantity -= existing_order.quantity;

                            // We DO NOT push bash the order_id back to the sell orders
                            self.orders.remove(&order_id);

                            // Update the order to insert
                            order_to_insert = Some(order.clone());
                        }
                    }
                }
            }
            OrderSide::Ask => {
                let required_token = order.pair.0.clone();
                let buy_orders_option = self.buy_orders.get_mut(&order.pair);

                if buy_orders_option.is_none() && order.order_type == OrderType::Limit {
                    // If there are no buy orders and this is a limit order, add it to the orderbook
                    self.insert_order(&order, user_info)?;
                    events.push(OrderbookEvent::OrderCreated {
                        order: order.clone(),
                    });

                    // Remove liquitidy from the user balance
                    record_balance_change(
                        &mut balance_changes,
                        user_info.clone(),
                        &required_token,
                        -(order.quantity as i128),
                    )?;

                    return Ok(events);
                } else if buy_orders_option.is_none() {
                    // If there are no buy orders and this is a market order, we cannot proceed
                    return Err(format!(
                        "No matching buy orders for market order {}",
                        order.order_id
                    ));
                }

                let buy_orders = buy_orders_option.unwrap();

                while let Some(order_id) = buy_orders.pop_front() {
                    let existing_order = self
                        .orders
                        .get_mut(&order_id)
                        .ok_or(format!("Order {order_id} not found"))?;

                    let existing_order_user = order_user_map
                        .get(&order_id)
                        .ok_or(format!("Order user not found for order {order_id}"))?;

                    // If the ordrer is a limit order, check if the *buying* price is higher than the limit price
                    if let Some(price) = order.price {
                        let existing_order_price = existing_order.price.expect(
                        "An order has been stored without a price limit. This should never happen",
                        );
                        if existing_order_price < price {
                            // Place the order in sell_orders
                            order_to_insert = Some(order.clone());

                            // Put back the buy order we popped
                            buy_orders.push_front(order_id);
                            break;
                        }
                    }

                    match existing_order.quantity.cmp(&order.quantity) {
                        std::cmp::Ordering::Greater => {
                            // The existing order do not fully cover this order
                            existing_order.quantity -= order.quantity;

                            buy_orders.push_front(order_id);

                            events.push(OrderbookEvent::OrderUpdate {
                                order_id: existing_order.order_id.clone(),
                                taker_order_id: order.order_id.clone(),
                                remaining_quantity: existing_order.quantity,
                                pair: order.pair.clone(),
                            });

                            // Send token to the order owner
                            record_transfer(
                                &mut balance_changes,
                                user_info.clone(),
                                existing_order_user.clone(),
                                &order.pair.0,
                                order.quantity as i128,
                            )?;
                            // Send token to the user
                            record_balance_change(
                                &mut balance_changes,
                                user_info.clone(),
                                &order.pair.1,
                                (existing_order.price.unwrap() * order.quantity / base_scale)
                                    as i128,
                            )?;
                            break;
                        }
                        std::cmp::Ordering::Equal => {
                            // The existing order fully covers this order
                            events.push(OrderbookEvent::OrderExecuted {
                                order_id: existing_order.order_id.clone(),
                                taker_order_id: order.order_id.clone(),
                                pair: order.pair.clone(),
                            });

                            // Send token to the order owner
                            record_transfer(
                                &mut balance_changes,
                                user_info.clone(),
                                existing_order_user.clone(),
                                &order.pair.0,
                                existing_order.quantity as i128,
                            )?;
                            // Send token to the user
                            record_balance_change(
                                &mut balance_changes,
                                user_info.clone(),
                                &order.pair.1,
                                (existing_order.price.unwrap() * existing_order.quantity
                                    / base_scale) as i128,
                            )?;

                            self.orders.remove(&order_id);
                            break;
                        }
                        std::cmp::Ordering::Less => {
                            // The existing order is fully filled
                            events.push(OrderbookEvent::OrderExecuted {
                                order_id: existing_order.order_id.clone(),
                                taker_order_id: order.order_id.clone(),
                                pair: order.pair.clone(),
                            });
                            record_transfer(
                                &mut balance_changes,
                                user_info.clone(),
                                existing_order_user.clone(),
                                &order.pair.0,
                                existing_order.quantity as i128,
                            )?;
                            record_balance_change(
                                &mut balance_changes,
                                user_info.clone(),
                                &order.pair.1,
                                (existing_order.price.unwrap() * existing_order.quantity
                                    / base_scale) as i128,
                            )?;
                            order.quantity -= existing_order.quantity;

                            // We DO NOT push bash the order_id back to the buy orders
                            self.orders.remove(&order_id);

                            // Update the order to insert
                            order_to_insert = Some(order.clone());
                        }
                    }
                }
            }
        }

        // If there is still some quantity left, we need to insert the order in the orderbook
        if let Some(order) = order_to_insert {
            if order.order_type == OrderType::Limit {
                // Insert order
                self.insert_order(&order, user_info)?;

                // Remove liquidity from the user balance
                let (quantity, required_token) = match order.order_side {
                    OrderSide::Bid => (
                        order.quantity * order.price.unwrap() / base_scale,
                        order.pair.1.clone(),
                    ),
                    OrderSide::Ask => (order.quantity, order.pair.0.clone()),
                };
                events.push(OrderbookEvent::OrderCreated { order });

                record_balance_change(
                    &mut balance_changes,
                    user_info.clone(),
                    &required_token,
                    -(quantity as i128),
                )?;
            }
        }

        // Updating balances
        for (token, user_balances) in balance_changes {
            let balances_to_update: Vec<(&UserInfo, Balance)> = user_balances
                .iter()
                .map(|(user_info, amount)| {
                    if self.server_execution {
                        events.push(OrderbookEvent::BalanceUpdated {
                            user: user_info.user.clone(),
                            token: token.clone(),
                            amount: amount.0,
                        });
                    }
                    (user_info, amount.clone())
                })
                .collect();

            let balances_proof = balances_proof.get(&token).ok_or_else(|| {
                format!("No balance proof provided for token {token} during update")
            })?;

            self.update_balances(&token, balances_to_update, balances_proof)?;
        }

        Ok(events)
    }
}

// TODO: make clear which function are used in the contract and which are used only by the server
impl Orderbook {
    pub fn init(lane_id: LaneId, server_execution: bool, secret: Vec<u8>) -> Result<Self, String> {
        let users_info_mt = SparseMerkleTree::default();
        let users_info_merkle_root = (*users_info_mt.root()).into();
        let hashed_secret = Sha256::digest(&secret).into();

        Ok(Orderbook {
            hashed_secret,
            pairs_info: BTreeMap::new(),
            lane_id,
            server_execution,
            users_info_mt,
            users_info_merkle_root,
            balances: BTreeMap::new(),
            balances_merkle_roots: BTreeMap::new(),
            ..Default::default()
        })
    }

    pub fn update_user_info_merkle_root(
        &mut self,
        user_info: &UserInfo,
        user_info_proof: &BorshableMerkleProof,
    ) -> Result<(), String> {
        let new_users_info_merkle_root = if self.server_execution {
            // Update the users_info the root *and the merkle tree* only for server execution
            (*self
                .users_info_mt
                .update(user_info.get_key(), user_info.clone())
                .map_err(|e| format!("Failed to update user info in SMT: {e}"))?)
            .into()
        } else {
            // Update the only users_info_merkle_root
            user_info_proof
                .0
                .clone()
                .compute_root::<SHA256Hasher>(vec![(user_info.get_key(), user_info.to_h256())])
                .unwrap_or_else(|e| {
                    panic!("Failed to compute new root on user_info merkle tree: {e}")
                })
                .into()
        };

        self.users_info_merkle_root = new_users_info_merkle_root;
        Ok(())
    }

    pub fn fund_account(
        &mut self,
        token: &str,
        user_info: &UserInfo,
        amount: &Balance,
        balances: &BTreeMap<UserInfo, Balance>,
        balances_proof: &BorshableMerkleProof,
    ) -> Result<(), String> {
        let current_balance = balances.get(user_info).ok_or_else(|| {
            format!(
                "No balance found for user {} during update of token {token}",
                user_info.user
            )
        })?;

        self.update_balances(
            token,
            vec![(user_info, Balance(current_balance.0 + amount.0))],
            balances_proof,
        )
        .map_err(|e| e.to_string())
    }

    pub fn deduct_from_account(
        &mut self,
        token: &str,
        user_info: &UserInfo,
        amount: u64,
        balances: &BTreeMap<UserInfo, Balance>,
        balances_proof: &BorshableMerkleProof,
    ) -> Result<(), String> {
        let current_balance = balances.get(user_info).ok_or_else(|| {
            format!(
                "No balance found for user {} during update of token {token}",
                user_info.user
            )
        })?;

        if current_balance.0 < amount {
            return Err(format!(
                "Insufficient balance: user {} has {} {} tokens, trying to remove {}",
                user_info.user, current_balance.0, token, amount
            ));
        }

        self.update_balances(
            token,
            vec![(user_info, Balance(current_balance.0 - amount))],
            balances_proof,
        )
        .map_err(|e| e.to_string())
    }

    pub fn update_balances(
        &mut self,
        token: &str,
        balances_to_update: Vec<(&UserInfo, Balance)>,
        balances_proof: &BorshableMerkleProof,
    ) -> Result<(), String> {
        let new_balance_merkle_root = if self.server_execution {
            // Update the balances root *and the merkle tree* for server execution
            let tree = self
                .balances
                .entry(token.to_string())
                .or_insert_with(|| SparseMerkleTree::new(H256::zero(), Default::default()));
            let leaves = balances_to_update
                .iter()
                .map(|(user_info, balance)| (user_info.get_key(), balance.clone()))
                .collect();
            (*tree
                .update_all(leaves)
                .map_err(|e| format!("Failed to update balances on token {token}: {e}"))?)
            .into()
        } else {
            // Only update the merkle root using the proof and new leaves
            let leaves = balances_to_update
                .iter()
                .map(|(user_info, balance)| (user_info.get_key(), balance.to_h256()))
                .collect();
            balances_proof
                .0
                .clone()
                .compute_root::<SHA256Hasher>(leaves)
                .unwrap_or_else(|e| panic!("Failed to compute new root on token {token}: {e}"))
                .into()
        };

        self.balances_merkle_roots
            .insert(token.to_string(), new_balance_merkle_root);
        Ok(())
    }

    pub fn verify_user_info_proof(
        &self,
        user_info: &UserInfo,
        user_info_proof: &BorshableMerkleProof,
    ) -> Result<(), String> {
        // Verify that users info are correct
        user_info_proof
            .0
            .clone()
            .verify::<SHA256Hasher>(
                &TryInto::<[u8; 32]>::try_into(self.users_info_merkle_root.as_slice())
                    .map_err(|e| format!("Failed to cast proof root to H256: {e}"))?
                    .into(),
                vec![(user_info.get_key(), user_info.to_h256())],
            )
            .expect("Failed to verify proof");
        Ok(())
    }

    pub fn verify_users_info_proof(
        &self,
        users_info: &BTreeSet<UserInfo>,
        user_info_proof: &BorshableMerkleProof,
    ) -> Result<(), String> {
        // Verify that users info are correct
        let leaves: Vec<_> = users_info
            .iter()
            .map(|user_info| (user_info.get_key(), user_info.to_h256()))
            .collect();

        user_info_proof
            .0
            .clone()
            .verify::<SHA256Hasher>(
                &TryInto::<[u8; 32]>::try_into(self.users_info_merkle_root.as_slice())
                    .map_err(|e| format!("Failed to cast proof root to H256: {e}"))?
                    .into(),
                leaves,
            )
            .expect("Failed to verify proof");
        Ok(())
    }

    pub fn verify_balance_proof(
        &self,
        token: &TokenName,
        user_info: &UserInfo,
        balance: &Balance,
        balance_proof: &BorshableMerkleProof,
    ) -> Result<(), String> {
        // Verify that users balance are correct
        let token_root = self
            .balances_merkle_roots
            .get(token.as_str())
            .ok_or(format!("Token {token} not found in balances merkle roots"))?;

        balance_proof
            .0
            .clone()
            .verify::<SHA256Hasher>(
                &TryInto::<[u8; 32]>::try_into(token_root.as_slice())
                    .map_err(|e| format!("Failed to cast proof root to H256: {e}"))?
                    .into(),
                vec![(user_info.get_key(), balance.to_h256())],
            )
            .expect("Failed to verify proof");
        Ok(())
    }

    pub fn verify_balances_proof(
        &self,
        token: &TokenName,
        balances: &BTreeMap<UserInfo, Balance>,
        balance_proof: &BorshableMerkleProof,
    ) -> Result<(), String> {
        // Verify that users balance are correct
        let token_root = self
            .balances_merkle_roots
            .get(token.as_str())
            .ok_or(format!("Token {token} not found in balances merkle roots"))?;

        let leaves = balances
            .iter()
            .map(|(user, balance)| (user.get_key(), balance.to_h256()))
            .collect::<Vec<_>>();

        balance_proof
            .0
            .clone()
            .verify::<SHA256Hasher>(
                &TryInto::<[u8; 32]>::try_into(token_root.as_slice())
                    .map_err(|e| format!("Failed to cast proof root to H256: {e}"))?
                    .into(),
                leaves,
            )
            .expect("Failed to verify proof");
        Ok(())
    }

    fn insert_order(&mut self, order: &Order, user_info: &UserInfo) -> Result<(), String> {
        // Function only called for Limit orders
        let price = order.price.unwrap();
        if price == 0 {
            return Err("Price cannot be zero".to_string());
        }
        let order_list = match order.order_side {
            OrderSide::Bid => self.buy_orders.entry(order.pair.clone()).or_default(),
            OrderSide::Ask => self.sell_orders.entry(order.pair.clone()).or_default(),
        };

        let insert_pos = order_list
            .iter()
            .position(|id| {
                let other_order = self.orders.get(id).unwrap();
                // To be inserted, the order must be <> than the current one
                match order.order_side {
                    OrderSide::Bid => other_order.price.unwrap_or(0) < price,
                    OrderSide::Ask => other_order.price.unwrap_or(0) > price,
                }
            })
            .unwrap_or(order_list.len());

        order_list.insert(insert_pos, order.order_id.clone());
        self.orders.insert(order.order_id.clone(), order.clone());
        // Keep track of the order owner
        self.orders_owner
            .insert(order.order_id.clone(), user_info.user.clone());
        Ok(())
    }

    pub fn is_blob_whitelisted(&self, contract_name: &ContractName) -> bool {
        if contract_name.0 == "orderbook" {
            return true;
        }

        self.pairs_info
            .keys()
            .any(|pair| pair.0 == contract_name.0 || pair.1 == contract_name.0)
    }

    pub fn as_bytes(&self) -> Result<Vec<u8>, Error> {
        borsh::to_vec(self)
    }
}

/// Implementation of functions that are only used by the server.
impl Orderbook {
    pub fn get_orders(&self) -> BTreeMap<String, Order> {
        self.orders.clone()
    }

    pub fn get_user_info(&self, user: &str) -> Result<UserInfo, String> {
        let salt = self
            .users_info_salt
            .get(user)
            .ok_or_else(|| format!("No salt found for user '{user}'"))?;
        let key = UserInfo::compute_key(user, salt.as_slice());
        self.users_info_mt.get(&key).map_err(|e| {
            format!("Failed to get user info for user '{user}' with key {key:?}: {e}",)
        })
    }

    pub fn get_user_info_proofs(
        &self,
        user_info: &UserInfo,
    ) -> Result<BorshableMerkleProof, String> {
        Ok(BorshableMerkleProof(
            self.users_info_mt
                .merkle_proof(vec![user_info.get_key()])
                .map_err(|e| {
                    format!(
                        "Failed to create merkle proof for user {:?}: {e}",
                        user_info.user
                    )
                })?,
        ))
    }

    pub fn get_users_info_proofs(
        &self,
        users_info: &BTreeSet<UserInfo>,
    ) -> Result<BorshableMerkleProof, String> {
        Ok(BorshableMerkleProof(
            self.users_info_mt
                .merkle_proof(users_info.iter().map(|u| u.get_key()).collect::<Vec<_>>())
                .map_err(|e| {
                    format!("Failed to create merkle proof for users {users_info:?}: {e}")
                })?,
        ))
    }

    pub fn get_user_info_with_proof(
        &self,
        user: &str,
    ) -> Result<(UserInfo, BorshableMerkleProof), String> {
        let user_info = self
            .users_info_salt
            .get(user)
            .and_then(|salt| {
                let key = UserInfo::compute_key(user, salt.as_slice());
                self.users_info_mt.get(&key).ok()
            })
            .ok_or_else(|| format!("User info not found for user '{user}'"))?;

        let proof = BorshableMerkleProof(
            self.users_info_mt
                .merkle_proof(vec![user_info.get_key()])
                .map_err(|e| {
                    format!(
                        "Failed to create merkle proof for user {:?}: {e}",
                        user_info.user
                    )
                })?,
        );

        Ok((user_info, proof))
    }

    /// Returns a mapping from order IDs to user names
    pub fn get_order_user_map(
        &self,
        order_side: &OrderSide,
        pair: &TokenPair,
    ) -> Result<BTreeMap<OrderId, UserInfo>, String> {
        let mut map = BTreeMap::new();
        let (base_token, quote_token) = pair.clone();
        let pair_key = (base_token.clone(), quote_token.clone());

        let relevant_orders = match order_side {
            OrderSide::Bid => self.sell_orders.get(&pair_key),
            OrderSide::Ask => self.buy_orders.get(&pair_key),
        };

        if let Some(order_ids) = relevant_orders {
            for order_id in order_ids {
                if let Some(user) = self.orders_owner.get(order_id) {
                    let user_info = self.get_user_info(user)?;
                    map.insert(order_id.clone(), user_info);
                }
            }
        }
        Ok(map)
    }

    pub fn get_balance(&self, user: &UserInfo, token: &str) -> Balance {
        self.balances
            .get(token)
            .and_then(|tree| tree.get(&user.get_key()).ok())
            .unwrap_or_default()
    }

    pub fn get_balances(&self) -> BTreeMap<TokenName, BTreeMap<String, u64>> {
        // Create an inverse hashmap: key -> username
        let mut key_to_username = BTreeMap::new();
        for (username, salt) in self.users_info_salt.iter() {
            let user_key = UserInfo::compute_key(username, salt);
            key_to_username.insert(user_key, username.clone());
        }

        let mut balances = BTreeMap::new();
        for (token, balances_mt) in self.balances.iter() {
            let token_store = balances_mt.store();
            let token_balances = balances.entry(token.clone()).or_insert_with(BTreeMap::new);
            for (user_info_key, balance) in token_store.leaves_map().iter() {
                let user_identifier = key_to_username
                    .get(user_info_key)
                    .cloned()
                    .unwrap_or_else(|| hex::encode(user_info_key.as_slice()));
                token_balances.insert(user_identifier, balance.0);
            }
        }
        balances
    }

    pub fn get_balances_for_account(
        &self,
        user: &str,
    ) -> Result<BTreeMap<TokenName, Balance>, String> {
        // First compute the users key
        let user_salt = self
            .users_info_salt
            .get(user)
            .ok_or_else(|| format!("No salt found for user '{user}'"))?;

        let user_key = UserInfo::compute_key(user, user_salt);

        let mut balances = BTreeMap::new();
        for (token, balances_mt) in self.balances.iter() {
            let token_store = balances_mt.store();
            let user_balance = token_store
                .leaves_map()
                .get(&user_key)
                .cloned()
                .unwrap_or_default();
            balances.insert(token.clone(), user_balance);
        }
        Ok(balances)
    }

    pub fn get_balances_with_proof(
        &self,
        users_info: &[UserInfo],
        token: &TokenName,
    ) -> Result<(BTreeMap<UserInfo, Balance>, BorshableMerkleProof), String> {
        let mut balances_map = BTreeMap::new();
        for user_info in users_info {
            let balance = self.get_balance(user_info, token);
            balances_map.insert(user_info.clone(), balance);
        }

        let users: Vec<UserInfo> = balances_map.keys().cloned().collect();
        let tree = self.balances.get(token).unwrap();
        let proof = BorshableMerkleProof(
            tree.merkle_proof(users.iter().map(|u| u.get_key()).collect::<Vec<_>>())
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

    pub fn get_balance_with_proof(
        &self,
        user_info: &UserInfo,
        token: &TokenName,
    ) -> Result<(Balance, BorshableMerkleProof), String> {
        let balance = self.get_balance(user_info, token);

        let tree = self.balances.get(token).unwrap();
        let proof =
            BorshableMerkleProof(tree.merkle_proof(vec![user_info.get_key()]).map_err(|e| {
                format!(
                    "Failed to create merkle proof for token {token} and user {:?}: {e}",
                    user_info.user
                )
            })?);
        Ok((balance, proof))
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
