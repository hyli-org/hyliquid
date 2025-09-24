use borsh::{io::Error, BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use sdk::{ContractName, LaneId, TxContext};

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default, Debug, Clone)]
pub struct Orderbook {
    // Server secret for authentication on permissionned actions
    pub secret: Vec<u8>,
    // Validator public key of the lane this orderbook is running on
    pub lane_id: LaneId,
    // User balances per token: token -> (user -> balance))
    pub balances: BTreeMap<TokenName, BTreeMap<String, u64>>,
    // Users info
    pub users_info: BTreeMap<String, UserInfo>,
    // All orders indexed by order_id
    pub orders: BTreeMap<OrderId, Order>,
    // Buy orders sorted by price (highest first) for each token pair
    pub buy_orders: BTreeMap<TokenPair, VecDeque<OrderId>>,
    // Sell orders sorted by price (lowest first) for each token pair
    pub sell_orders: BTreeMap<TokenPair, VecDeque<OrderId>>,
    // Accepted tokens
    pub accepted_tokens: BTreeSet<ContractName>,
    // Mapping of order IDs to their owners
    pub orders_owner: BTreeMap<OrderId, String>,

    /// These fields are not committed on-chain
    #[borsh(skip)]
    pub server_execution: bool,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default, Debug, Clone)]
pub struct UserInfo {
    pub nonce: u32,
    pub session_keys: Vec<Vec<u8>>,
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

impl Orderbook {
    pub fn add_session_key(
        &mut self,
        user: &str,
        pubkey: &Vec<u8>,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Add the session key to the user's list of session keys
        let keys = &mut self
            .users_info
            .entry(user.to_string())
            .or_default()
            .session_keys;
        if keys.contains(pubkey) {
            return Err("Session key already exists".to_string());
        }
        keys.push(pubkey.clone());

        if self.server_execution {
            Ok(vec![OrderbookEvent::SessionKeyAdded {
                user: user.to_string(),
            }])
        } else {
            Ok(vec![])
        }
    }

    pub fn deposit(
        &mut self,
        token: String,
        amount: u64,
        user: &String,
    ) -> Result<Vec<OrderbookEvent>, String> {
        let server_execution = self.server_execution;
        let user_balance = self.get_balance_mut(user, &token);
        *user_balance += amount;

        if server_execution {
            Ok(vec![OrderbookEvent::BalanceUpdated {
                user: user.to_string(),
                token,
                amount: *user_balance,
            }])
        } else {
            Ok(vec![])
        }
    }

    pub fn withdraw(
        &mut self,
        token: String,
        amount: u64,
        user: &str,
    ) -> Result<Vec<OrderbookEvent>, String> {
        let server_execution = self.server_execution;
        let balance = self.get_balance_mut(user, &token);

        if *balance < amount {
            return Err(format!(
                "Could not withdraw: Insufficient balance: user {user} has {balance} {token} tokens, trying to withdraw {amount}"
            ));
        }

        *balance -= amount;

        if server_execution {
            Ok(vec![OrderbookEvent::BalanceUpdated {
                user: user.to_string(),
                token,
                amount: *balance,
            }])
        } else {
            Ok(vec![])
        }
    }

    pub fn cancel_order(
        &mut self,
        order_id: OrderId,
        user: &str,
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
        self.transfer_tokens("orderbook", user, &required_token, order.quantity)?;

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

        let user_balance = self.get_balance(user, &required_token);

        let mut events = vec![OrderbookEvent::OrderCancelled {
            order_id,
            pair: order.pair,
        }];
        if self.server_execution {
            events.push(OrderbookEvent::BalanceUpdated {
                user: user.to_string(),
                token: required_token.to_string(),
                amount: user_balance,
            });
            let orderbook_balance = self.get_balance("orderbook", &required_token);
            events.push(OrderbookEvent::BalanceUpdated {
                user: "orderbook".into(),
                token: required_token.clone(),
                amount: orderbook_balance + order.quantity * order.price.unwrap(),
            });
        }
        Ok(events)
    }

    pub fn escape(
        &mut self,
        _tx_ctx: &TxContext,
        _user: String,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Logic to allow user to escape with their funds
        // This could involve transferring all their balances to a safe contract or address
        // For now, we just return an empty event list
        Ok(vec![])
    }

    pub fn execute_order(
        &mut self,
        user: &str,
        mut order: Order,
        order_user_map: BTreeMap<OrderId, String>,
    ) -> Result<Vec<OrderbookEvent>, String> {
        let mut events = Vec::new();

        let mut transfers_to_process: Vec<(String, String, String, u64)> = vec![];
        let mut order_to_insert: Option<Order> = None;

        let (required_token, required_amount) = match order.order_side {
            OrderSide::Bid => (
                order.pair.1.clone(),
                order.price.map(|p| order.quantity * p),
            ),
            OrderSide::Ask => (order.pair.0.clone(), Some(order.quantity)),
        };

        let user_balance = self.get_balance(user, &required_token);

        // For limit orders, verify sufficient balance
        if let Some(amount) = required_amount {
            if user_balance < amount {
                return Err(format!(
                    "Insufficient balance for {:?} order: user {} has {} {} tokens, requires {}",
                    order.order_side, user, user_balance, required_token, amount
                ));
            }
        }

        // Try to fill already existing orders
        match &order.order_side {
            OrderSide::Bid => {
                let sell_orders_option = self.sell_orders.get_mut(&order.pair);

                if sell_orders_option.is_none() && order.order_type == OrderType::Limit {
                    // If there are no sell orders and this is a limit order, add it to the orderbook
                    self.insert_order(order.clone(), user.to_string())?;
                    events.push(OrderbookEvent::OrderCreated {
                        order: order.clone(),
                    });

                    // Remove liquitidy from the user balance
                    if self.server_execution {
                        events.push(OrderbookEvent::BalanceUpdated {
                            user: user.to_string(),
                            token: required_token.clone(),
                            amount: user_balance - order.quantity * order.price.unwrap(),
                        });
                        let orderbook_balance = self.get_balance("orderbook", &required_token);
                        events.push(OrderbookEvent::BalanceUpdated {
                            user: "orderbook".into(),
                            token: required_token.clone(),
                            amount: orderbook_balance + order.quantity * order.price.unwrap(),
                        });
                    }
                    self.transfer_tokens(
                        user,
                        "orderbook",
                        &required_token,
                        order.quantity * order.price.unwrap(),
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

                            // Send token to the order owner
                            transfers_to_process.push((
                                user.to_string(),
                                existing_order_user.clone(),
                                order.pair.1.clone(),
                                existing_order.price.unwrap() * order.quantity,
                            ));
                            // Send token to the user
                            transfers_to_process.push((
                                "orderbook".to_string(),
                                user.to_string(),
                                order.pair.0.clone(),
                                order.quantity,
                            ));
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
                            transfers_to_process.push((
                                user.to_string(),
                                existing_order_user.clone(),
                                order.pair.1.clone(),
                                existing_order.price.unwrap() * existing_order.quantity,
                            ));

                            // Send token to the user
                            transfers_to_process.push((
                                "orderbook".to_string(),
                                user.to_string(),
                                order.pair.0.clone(),
                                existing_order.quantity,
                            ));

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
                            transfers_to_process.push((
                                user.to_string(),
                                existing_order_user.clone(),
                                order.pair.1.clone(),
                                existing_order.price.unwrap() * existing_order.quantity,
                            ));
                            transfers_to_process.push((
                                "orderbook".to_string(),
                                user.to_string(),
                                order.pair.0.clone(),
                                existing_order.quantity,
                            ));
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
                let buy_orders_option = self.buy_orders.get_mut(&order.pair);

                if buy_orders_option.is_none() && order.order_type == OrderType::Limit {
                    // If there are no buy orders and this is a limit order, add it to the orderbook
                    self.insert_order(order.clone(), user.to_string())?;
                    events.push(OrderbookEvent::OrderCreated {
                        order: order.clone(),
                    });

                    // Remove liquitidy from the user balance
                    if self.server_execution {
                        events.push(OrderbookEvent::BalanceUpdated {
                            user: user.to_string(),
                            token: required_token.clone(),
                            amount: user_balance - order.quantity,
                        });
                        let orderbook_balance = self.get_balance("orderbook", &required_token);
                        events.push(OrderbookEvent::BalanceUpdated {
                            user: "orderbook".into(),
                            token: required_token.clone(),
                            amount: orderbook_balance + order.quantity,
                        });
                    }
                    self.transfer_tokens(user, "orderbook", &required_token, order.quantity)?;

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
                            transfers_to_process.push((
                                user.to_string(),
                                existing_order_user.clone(),
                                order.pair.0.clone(),
                                order.quantity,
                            ));
                            // Send token to the user
                            transfers_to_process.push((
                                "orderbook".to_string(),
                                user.to_string(),
                                order.pair.1.clone(),
                                existing_order.price.unwrap() * order.quantity,
                            ));
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
                            transfers_to_process.push((
                                user.to_string(),
                                existing_order_user.clone(),
                                order.pair.0.clone(),
                                existing_order.quantity,
                            ));
                            transfers_to_process.push((
                                "orderbook".to_string(),
                                user.to_string(),
                                order.pair.1.clone(),
                                existing_order.price.unwrap() * existing_order.quantity,
                            ));

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
                            transfers_to_process.push((
                                user.to_string(),
                                existing_order_user.clone(),
                                order.pair.0.clone(),
                                existing_order.quantity,
                            ));
                            transfers_to_process.push((
                                "orderbook".to_string(),
                                user.to_string(),
                                order.pair.1.clone(),
                                existing_order.price.unwrap() * existing_order.quantity,
                            ));
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
                self.insert_order(order.clone(), user.to_string())?;
                // Remove liquidity from the user balance
                let quantity = match order.order_side {
                    OrderSide::Bid => order.quantity * order.price.unwrap(),
                    OrderSide::Ask => order.quantity,
                };

                transfers_to_process.push((
                    user.to_string(),
                    "orderbook".to_string(),
                    required_token,
                    quantity,
                ));
                events.push(OrderbookEvent::OrderCreated { order });
            }
        }

        // Updating balances
        // If not limit order: assert that total balance in user_to_fund is equal to the order quantity
        let mut ids = BTreeMap::<String, BTreeSet<String>>::new();
        for (from, to, token, amout) in transfers_to_process {
            self.transfer_tokens(&from, &to, &token, amout)?;
            let t = ids.entry(token.clone()).or_default();
            t.insert(from.clone());
            t.insert(to.clone());
        }
        if self.server_execution {
            for (token, users) in ids {
                for user in users {
                    let user_balance = self.get_balance(&user, &token);
                    events.push(OrderbookEvent::BalanceUpdated {
                        user: user.to_string(),
                        token: token.clone(),
                        amount: user_balance,
                    });
                }
            }
        }

        Ok(events)
    }
}

impl Orderbook {
    pub fn init(lane_id: LaneId, server_execution: bool, secret: Vec<u8>) -> Self {
        let accepted_tokens = BTreeSet::from(["ORANJ".into(), "HYLLAR".into()]);

        Orderbook {
            secret,
            lane_id,
            orders: BTreeMap::new(),
            balances: BTreeMap::new(),
            users_info: BTreeMap::new(),
            buy_orders: BTreeMap::new(),
            sell_orders: BTreeMap::new(),
            orders_owner: BTreeMap::new(),
            accepted_tokens,
            server_execution,
        }
    }

    fn transfer_tokens(
        &mut self,
        from: &str,
        to: &str,
        token: &str,
        amount: u64,
    ) -> Result<(), String> {
        // Deduct from sender
        let from_user_balance = self.get_balance_mut(from, token);

        if *from_user_balance < amount {
            return Err(format!(
                "Could not transfer: Insufficient balance: user {from} has {from_user_balance} {token} tokens, trying to transfer {amount}"
            ));
        }
        *from_user_balance -= amount;

        // Add to receiver
        let to_user_balance = self.get_balance_mut(to, token);
        *to_user_balance += amount;

        Ok(())
    }

    pub fn get_user_info_mut(&mut self, user: &str) -> &mut UserInfo {
        self.users_info.entry(user.to_string()).or_default()
    }

    pub fn get_balance(&mut self, user: &str, token: &str) -> u64 {
        *self.get_balance_mut(user, token)
    }

    pub fn get_balance_mut(&mut self, user: &str, token: &str) -> &mut u64 {
        self.balances
            .entry(token.to_string())
            .or_default()
            .entry(user.to_string())
            .or_default()
    }

    pub fn get_nonce(&self, user: &str) -> u32 {
        self.users_info
            .get(user)
            .map(|info| info.nonce)
            .unwrap_or(0)
    }

    pub fn get_session_keys(&self, user: &str) -> Vec<Vec<u8>> {
        self.users_info
            .get(user)
            .map(|info| info.session_keys.clone())
            .unwrap_or_default()
    }

    pub fn increment_nonce(&mut self, user: &str) {
        self.get_user_info_mut(user).nonce += 1;
    }

    fn insert_order(&mut self, order: Order, user: String) -> Result<(), String> {
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
            .insert(order.order_id.clone(), user.clone());
        Ok(())
    }

    /// Returns a mapping from order IDs to user names
    pub fn get_order_user_map(
        &self,
        order_side: &OrderSide,
        pair: &TokenPair,
    ) -> BTreeMap<OrderId, String> {
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
                    map.insert(order_id.clone(), user.clone());
                }
            }
        }
        map
    }

    pub fn is_blob_whitelisted(&self, contract_name: &ContractName) -> bool {
        self.accepted_tokens.contains(contract_name)
            || contract_name.0 == "orderbook"
            || contract_name.0 == "wallet"
            || contract_name.0 == "secp256k1"
    }

    pub fn as_bytes(&self) -> Result<Vec<u8>, Error> {
        borsh::to_vec(self)
    }
}
