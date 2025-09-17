use borsh::{io::Error, BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use sdk::hyli_model_utils::TimestampMs;
use sdk::{ContractName, LaneId};

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default, Debug, Clone)]
pub struct Orderbook {
    // Validator public key of the lane this orderbook is running on
    pub lane_id: LaneId,
    // User balances per token: token -> (user -> (balance, secret))
    pub balances: BTreeMap<TokenName, BTreeMap<String, UserInfo>>,
    // User public keys for session management
    pub session_keys: BTreeMap<String, Vec<Vec<u8>>>,
    // All orders indexed by order_id
    pub orders: BTreeMap<OrderId, Order>,
    // Buy orders sorted by price (highest first) for each token pair
    pub buy_orders: BTreeMap<TokenPair, VecDeque<OrderId>>,
    // Sell orders sorted by price (lowest first) for each token pair
    pub sell_orders: BTreeMap<TokenPair, VecDeque<OrderId>>,
    // History of orders executed, indexed by token pair and timestamp
    pub orders_history: BTreeMap<TokenPair, BTreeMap<TimestampMs, u32>>,
    // Accepted tokens
    pub accepted_tokens: BTreeSet<ContractName>,
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Default, Debug, Clone)]
pub struct UserInfo {
    pub balance: u32,
    pub secret: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize)]
pub struct Order {
    pub order_id: OrderId,
    pub order_type: OrderType,
    pub price: Option<u32>,
    pub pair: TokenPair,
    pub quantity: u32,
    pub filled_quantity: u32,
    pub timestamp: TimestampMs,
}

#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize)]
pub enum OrderType {
    Buy,
    Sell,
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
        pair: TokenPair,
    },
    OrderUpdate {
        order_id: OrderId,
        remaining_quantity: u32,
        pair: TokenPair,
    },
    SessionKeyAdded {
        user: String,
    },
}

impl Orderbook {
    pub fn add_session_key(
        &mut self,
        user: String,
        pubkey: &Vec<u8>,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Add the session key to the user's list of session keys
        let keys = self.session_keys.entry(user.clone()).or_default();
        if keys.contains(pubkey) {
            return Err("Session key already exists".to_string());
        }
        keys.push(pubkey.clone());

        Ok(vec![OrderbookEvent::SessionKeyAdded { user }])
    }

    pub fn deposit(
        &mut self,
        token: String,
        amount: u32,
        user: String,
        secret: &Vec<u8>,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Check if user already exists for this token
        let user_exists = self
            .balances
            .get(&token)
            .and_then(|token_balances| token_balances.get(&user))
            .is_some();

        let user_info = self.get_user_info_mut(&user, &token);
        user_info.balance += amount;

        // Only write the secret if the user doesn't exist yet (empty secret means new user)
        if !user_exists || user_info.secret.is_empty() {
            user_info.secret.clone_from(secret);
        }

        Ok(vec![])
    }

    pub fn withdraw(
        &mut self,
        _token: String,
        _amount: u32,
        _user: String,
    ) -> Result<Vec<OrderbookEvent>, String> {
        todo!("Implement withdraw logic")
    }

    pub fn cancel_order(&mut self, _order_id: OrderId) -> Result<Vec<OrderbookEvent>, String> {
        todo!("Implement order cancellation logic")
    }

    pub fn execute_order(
        &mut self,
        user: &String,
        mut order: Order,
        order_user_map: BTreeMap<OrderId, String>,
    ) -> Result<Vec<OrderbookEvent>, String> {
        let mut events = Vec::new();

        let mut transfers_to_process: Vec<(String, String, String, u32)> = vec![];
        let mut order_to_insert: Option<Order> = None;

        let (required_token, required_amount) = match order.order_type {
            OrderType::Buy => (
                order.pair.1.clone(),
                order.price.map(|p| order.quantity * p),
            ),
            OrderType::Sell => (order.pair.0.clone(), Some(order.quantity)),
        };

        let user_balance = self.get_balance(user, &required_token);

        // For limit orders, verify sufficient balance
        if let Some(amount) = required_amount {
            if user_balance < amount {
                return Err(format!(
                    "Insufficient balance for {:?} order: user {} has {} {} tokens, requires {}",
                    order.order_type, user, user_balance, required_token, amount
                ));
            }
        }

        // Try to fill already existing orders
        match &order.order_type {
            OrderType::Buy => {
                let sell_orders_option = self.sell_orders.get_mut(&order.pair);

                if sell_orders_option.is_none() && order.price.is_some() {
                    // If there are no sell orders and this is a limit order, add it to the orderbook

                    self.orders.insert(order.order_id.clone(), order.clone());
                    self.buy_orders
                        .entry(order.pair.clone())
                        .or_default()
                        .push_back(order.order_id.clone());
                    events.push(OrderbookEvent::OrderCreated {
                        order: order.clone(),
                    });

                    // Remove liquitidy from the user balance
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

                    // Update history
                    self.orders_history
                        .entry(order.pair.clone())
                        .or_default()
                        .insert(order.timestamp.clone(), existing_order.price.unwrap());

                    // There is an order that can be filled
                    match existing_order.quantity.cmp(&order.quantity) {
                        std::cmp::Ordering::Greater => {
                            // The existing order do not fully cover this order
                            existing_order.quantity -= order.quantity;

                            sell_orders.push_front(order_id);

                            events.push(OrderbookEvent::OrderUpdate {
                                order_id: existing_order.order_id.clone(),
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
                                pair: order.pair.clone(),
                            });
                            events.push(OrderbookEvent::OrderExecuted {
                                order_id: order.order_id.clone(),
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
            OrderType::Sell => {
                let buy_orders_option = self.buy_orders.get_mut(&order.pair);

                if buy_orders_option.is_none() && order.price.is_some() {
                    // If there are no buy orders and this is a limit order, add it to the orderbook
                    self.orders.insert(order.order_id.clone(), order.clone());
                    self.sell_orders
                        .entry(order.pair.clone())
                        .or_default()
                        .push_back(order.order_id.clone());
                    events.push(OrderbookEvent::OrderCreated {
                        order: order.clone(),
                    });

                    // Remove liquitidy from the user balance
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

                    // Update history
                    self.orders_history
                        .entry(order.pair.clone())
                        .or_default()
                        .insert(order.timestamp.clone(), existing_order.price.unwrap());

                    match existing_order.quantity.cmp(&order.quantity) {
                        std::cmp::Ordering::Greater => {
                            // The existing order do not fully cover this order
                            existing_order.quantity -= order.quantity;

                            buy_orders.push_front(order_id);

                            events.push(OrderbookEvent::OrderUpdate {
                                order_id: existing_order.order_id.clone(),
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
            if order.price.is_some() {
                self.insert_order(order.clone())?;
                // Remove liquitidy from the user balance
                let quantity = match order.order_type {
                    OrderType::Buy => order.quantity * order.price.unwrap(),
                    OrderType::Sell => order.quantity,
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

        Ok(events)
    }
}

impl Orderbook {
    pub fn init(lane_id: LaneId) -> Self {
        let accepted_tokens = BTreeSet::from(["ORANJ".into(), "HYLLAR".into()]);

        Orderbook {
            lane_id,
            orders: BTreeMap::new(),
            balances: BTreeMap::new(),
            session_keys: BTreeMap::new(),
            buy_orders: BTreeMap::new(),
            sell_orders: BTreeMap::new(),
            orders_history: BTreeMap::new(),
            accepted_tokens,
        }
    }

    pub fn partial_commit(&self) -> sdk::StateCommitment {
        let mut partial_state = self.clone();
        partial_state.orders_history = Default::default();

        // Reset all order timestamps to 0
        for (_, order) in partial_state.orders.iter_mut() {
            order.timestamp = TimestampMs(0);
        }

        sdk::StateCommitment(
            borsh::to_vec(&partial_state).expect("Failed to encode Orderbook partial state"),
        )
    }

    fn transfer_tokens(
        &mut self,
        from: &str,
        to: &str,
        token: &str,
        amount: u32,
    ) -> Result<(), String> {
        // Deduct from sender
        let from_user_info = self
            .balances
            .get_mut(token)
            .ok_or(format!("Token {token} not found"))?
            .get_mut(from)
            .ok_or(format!("Token {token} not found for user {from}"))?;

        if from_user_info.balance < amount {
            return Err(format!(
                "Could not transfer: Insufficient balance: user {} has {} {} tokens, trying to transfer {}",
                from, from_user_info.balance, token, amount
            ));
        }
        from_user_info.balance -= amount;

        // Add to receiver
        let to_user_info = self
            .balances
            .get_mut(token)
            .ok_or(format!("Token {token} not found"))?
            .entry(to.to_string())
            .or_default();
        to_user_info.balance += amount;

        Ok(())
    }

    pub fn get_user_info_mut(&mut self, user: &str, token: &str) -> &mut UserInfo {
        self.balances
            .entry(token.to_owned())
            .or_default()
            .entry(user.to_string())
            .or_default()
    }

    pub fn get_balance(&mut self, user: &str, token: &str) -> u32 {
        self.get_user_info_mut(user, token).balance
    }

    fn insert_order(&mut self, order: Order) -> Result<(), String> {
        // Function only called for Limit orders
        let price = order.price.unwrap();
        if price == 0 {
            return Err("Price cannot be zero".to_string());
        }
        let order_list = match order.order_type {
            OrderType::Buy => self.buy_orders.entry(order.pair.clone()).or_default(),
            OrderType::Sell => self.sell_orders.entry(order.pair.clone()).or_default(),
        };

        let insert_pos = order_list
            .iter()
            .position(|id| {
                let other_order = self.orders.get(id).unwrap();
                // To be inserted, the order must be <> than the current one
                match order.order_type {
                    OrderType::Buy => other_order.price.unwrap_or(0) < price,
                    OrderType::Sell => other_order.price.unwrap_or(0) > price,
                }
            })
            .unwrap_or(order_list.len());

        order_list.insert(insert_pos, order.order_id.clone());
        self.orders.insert(order.order_id.clone(), order.clone());
        Ok(())
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
