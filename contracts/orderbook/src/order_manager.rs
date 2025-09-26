use borsh::{BorshDeserialize, BorshSerialize};
use std::collections::{BTreeMap, VecDeque};

use crate::orderbook::{Order, OrderId, OrderSide, OrderType, OrderbookEvent, TokenPair};
use crate::smt_values::UserInfo;

#[derive(BorshSerialize, BorshDeserialize, Default, Debug, Clone)]
pub struct OrderManager {
    // All orders indexed by order_id
    pub orders: BTreeMap<OrderId, Order>,
    // Buy orders sorted by price (highest first) for each token pair
    pub buy_orders: BTreeMap<TokenPair, VecDeque<OrderId>>,
    // Sell orders sorted by price (lowest first) for each token pair
    pub sell_orders: BTreeMap<TokenPair, VecDeque<OrderId>>,

    /// These fields are not committed on-chain
    #[borsh(skip)]
    // Mapping of order IDs to their owners
    // TODO: Use mt_key instead of user
    pub orders_owner: BTreeMap<OrderId, String>,
}

#[cfg(test)]
mod tests;

impl OrderManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a new order into the appropriate data structures
    pub fn insert_order(
        &mut self,
        order: &Order,
        user_info: &UserInfo,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Function only called for Limit orders
        let price = order.price.ok_or("Price cannot be None for limit orders")?;
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
        // Only useful in server execution
        self.orders_owner
            .insert(order.order_id.clone(), user_info.user.clone());

        Ok(vec![OrderbookEvent::OrderCreated {
            order: order.clone(),
        }])
    }

    /// Cancels an order and removes it from data structures
    pub fn cancel_order(&mut self, order_id: &OrderId) -> Result<Vec<OrderbookEvent>, String> {
        let order = self
            .orders
            .get(order_id)
            .ok_or(format!("Order {order_id} not found"))?
            .clone();

        // Remove the order from storage
        self.orders.remove(order_id);

        // Remove from order lists
        match order.order_side {
            OrderSide::Bid => {
                if let Some(orders) = self.buy_orders.get_mut(&order.pair) {
                    orders.retain(|id| id != order_id);
                }
            }
            OrderSide::Ask => {
                if let Some(orders) = self.sell_orders.get_mut(&order.pair) {
                    orders.retain(|id| id != order_id);
                }
            }
        }

        // Remove owner mapping
        self.orders_owner.remove(order_id);

        Ok(vec![OrderbookEvent::OrderCancelled {
            order_id: order_id.clone(),
            pair: order.pair,
        }])
    }

    /// Executes an order and returns generated events
    pub fn execute_order(
        &mut self,
        user_info: &UserInfo,
        order: &Order,
    ) -> Result<Vec<OrderbookEvent>, String> {
        if self.orders.contains_key(&order.order_id) {
            return Err(format!("Order with id {} already exists", order.order_id));
        }

        let mut events = Vec::new();
        let mut order_to_execute = order.clone();

        // Try to fill existing orders
        match &order.order_side {
            OrderSide::Bid => {
                let sell_orders_option = self.sell_orders.get_mut(&order.pair);

                if sell_orders_option.is_none() && order.order_type == OrderType::Limit {
                    // If there are no sell orders and this is a limit order, add it to the orderbook
                    return self.insert_order(&order_to_execute, user_info);
                } else if sell_orders_option.is_none() {
                    // If there are no sell orders and this is a market order, we cannot proceed
                    return Err(format!(
                        "No matching sell orders for market order {}",
                        order.order_id
                    ));
                }

                let sell_orders = sell_orders_option.unwrap();

                // Get the lowest price sell order
                while let Some(existing_order_id) = sell_orders.pop_front() {
                    let existing_order = self
                        .orders
                        .get_mut(&existing_order_id)
                        .ok_or(format!("Order {existing_order_id} not found"))?;

                    // If the order is a limit order, check if the *selling* price is lower than the limit price
                    if let Some(price) = order_to_execute.price {
                        let existing_order_price = existing_order.price.expect(
                            "An order has been stored without a price limit. This should never happen",
                        );
                        if existing_order_price > price {
                            // Place the order in buy_orders
                            sell_orders.push_front(existing_order_id);
                            return self.insert_order(&order_to_execute, user_info);
                        }
                    }

                    // There is an order that can be filled
                    match existing_order.quantity.cmp(&order_to_execute.quantity) {
                        std::cmp::Ordering::Greater => {
                            // The existing order is not fully covered by this order
                            existing_order.quantity -= order_to_execute.quantity;
                            sell_orders.push_front(existing_order_id.clone());

                            events.push(OrderbookEvent::OrderUpdate {
                                order_id: existing_order_id.clone(),
                                taker_order_id: order_to_execute.order_id.clone(),
                                executed_quantity: order_to_execute.quantity,
                                remaining_quantity: existing_order.quantity,
                                pair: existing_order.pair.clone(),
                            });

                            // Emptying the order to execute
                            order_to_execute.quantity = 0;
                            break;
                        }
                        std::cmp::Ordering::Equal => {
                            // Both orders are executed
                            events.push(OrderbookEvent::OrderExecuted {
                                order_id: existing_order_id.clone(),
                                taker_order_id: order_to_execute.order_id.clone(),
                                pair: existing_order.pair.clone(),
                            });

                            // Emptying the order to execute
                            order_to_execute.quantity = 0;

                            self.orders.remove(&existing_order_id);
                            self.orders_owner.remove(&existing_order_id);
                            break;
                        }
                        std::cmp::Ordering::Less => {
                            // The existing order is fully filled
                            events.push(OrderbookEvent::OrderExecuted {
                                order_id: existing_order_id.clone(),
                                taker_order_id: order_to_execute.order_id.clone(),
                                pair: existing_order.pair.clone(),
                            });

                            order_to_execute.quantity -= existing_order.quantity;
                            self.orders.remove(&existing_order_id);
                            self.orders_owner.remove(&existing_order_id);
                        }
                    }
                }
            }
            OrderSide::Ask => {
                let buy_orders_option = self.buy_orders.get_mut(&order.pair);

                if buy_orders_option.is_none() && order.order_type == OrderType::Limit {
                    // If there are no buy orders and this is a limit order, add it to the orderbook
                    return self.insert_order(&order_to_execute, user_info);
                } else if buy_orders_option.is_none() {
                    // If there are no buy orders and this is a market order, we cannot proceed
                    return Err(format!(
                        "No matching buy orders for market order {}",
                        order.order_id
                    ));
                }

                let buy_orders = buy_orders_option.unwrap();

                while let Some(existing_order_id) = buy_orders.pop_front() {
                    let existing_order = self
                        .orders
                        .get_mut(&existing_order_id)
                        .ok_or(format!("Order {existing_order_id} not found"))?;

                    // If the order is a limit order, check if the *buying* price is higher than the limit price
                    if let Some(price) = order_to_execute.price {
                        let existing_order_price = existing_order.price.expect(
                            "An order has been stored without a price limit. This should never happen",
                        );
                        if existing_order_price < price {
                            // Place the order in sell_orders
                            buy_orders.push_front(existing_order_id);
                            return self.insert_order(&order_to_execute, user_info);
                        }
                    }

                    match existing_order.quantity.cmp(&order_to_execute.quantity) {
                        std::cmp::Ordering::Greater => {
                            // The existing order is not fully covered by this order
                            existing_order.quantity -= order_to_execute.quantity;
                            buy_orders.push_front(existing_order_id);

                            events.push(OrderbookEvent::OrderUpdate {
                                order_id: existing_order.order_id.clone(),
                                taker_order_id: order_to_execute.order_id.clone(),
                                executed_quantity: order_to_execute.quantity,
                                remaining_quantity: existing_order.quantity,
                                pair: existing_order.pair.clone(),
                            });

                            // Emptying the order to execute
                            order_to_execute.quantity = 0;
                            break;
                        }
                        std::cmp::Ordering::Equal => {
                            // The existing order fully covers this order
                            events.push(OrderbookEvent::OrderExecuted {
                                order_id: existing_order_id.clone(),
                                taker_order_id: order_to_execute.order_id.clone(),
                                pair: existing_order.pair.clone(),
                            });

                            // Emptying the order to execute
                            order_to_execute.quantity = 0;

                            self.orders.remove(&existing_order_id);
                            self.orders_owner.remove(&existing_order_id);
                            break;
                        }
                        std::cmp::Ordering::Less => {
                            // The existing order is fully filled
                            events.push(OrderbookEvent::OrderExecuted {
                                order_id: existing_order_id.clone(),
                                taker_order_id: order_to_execute.order_id.clone(),
                                pair: existing_order.pair.clone(),
                            });

                            order_to_execute.quantity -= existing_order.quantity;
                            self.orders.remove(&existing_order_id);
                            self.orders_owner.remove(&existing_order_id);
                        }
                    }
                }
            }
        }

        if order_to_execute.quantity == 0 {
            events.push(OrderbookEvent::OrderExecuted {
                order_id: order_to_execute.order_id.clone(),
                taker_order_id: order_to_execute.order_id.clone(),
                pair: order_to_execute.pair.clone(),
            });
        }

        // If there is still some quantity left and it's a limit order, insert it
        if order_to_execute.quantity > 0 && order_to_execute.order_type == OrderType::Limit {
            let insert_events = self.insert_order(&order_to_execute, user_info)?;
            events.extend(insert_events);
        }

        Ok(events)
    }

    /// Returns a map from order IDs to users
    pub fn get_order_user_map(
        &self,
        order_side: &OrderSide,
        pair: &TokenPair,
    ) -> BTreeMap<OrderId, String> {
        let mut map = BTreeMap::new();
        let pair_key = pair.clone();

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

    /// Returns all orders
    pub fn get_orders(&self) -> &BTreeMap<OrderId, Order> {
        &self.orders
    }

    pub fn get_order(&self, order_id: &OrderId) -> Option<&Order> {
        self.orders.get(order_id)
    }

    /// Returns the owner of an order
    pub fn get_order_owner(&self, order_id: &OrderId) -> Option<&String> {
        self.orders_owner.get(order_id)
    }
}
