use crate::smt_values::BorshableH256 as H256;
use borsh::{BorshDeserialize, BorshSerialize};
use std::collections::{BTreeMap, VecDeque};

use crate::orderbook::{Order, OrderId, OrderSide, OrderType, OrderbookEvent, Pair};

trait OrderList {
    fn pop_best(&mut self, side: &OrderSide) -> Option<(u64, VecDeque<OrderId>)>;
}

impl OrderList for BTreeMap<u64, VecDeque<OrderId>> {
    fn pop_best(&mut self, side: &OrderSide) -> Option<(u64, VecDeque<OrderId>)> {
        match side {
            OrderSide::Bid => self.pop_first(),
            OrderSide::Ask => self.pop_last(),
        }
    }
}

#[derive(BorshSerialize, BorshDeserialize, Default, Debug, Clone, PartialEq)]
pub struct OrderManager {
    // All orders indexed by order_id
    pub orders: BTreeMap<OrderId, Order>,
    // Buy orders sorted by price for each token pair
    pub buy_orders: BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,
    // Sell orders sorted by price for each token pair
    pub sell_orders: BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,

    // Mapping of order IDs to their owners
    // This field will not be commited.
    pub orders_owner: BTreeMap<OrderId, H256>,
}

#[cfg(test)]
mod tests;

impl OrderManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count_buy_orders(&self, pair: &Pair) -> usize {
        self.buy_orders
            .get(pair)
            .map(|v| v.values().map(|v| v.len()).sum())
            .unwrap_or(0)
    }

    pub fn count_sell_orders(&self, pair: &Pair) -> usize {
        self.sell_orders
            .get(pair)
            .map(|v| v.values().map(|v| v.len()).sum())
            .unwrap_or(0)
    }

    pub fn side_map(&self, side: &OrderSide) -> &BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>> {
        match side {
            OrderSide::Bid => &self.buy_orders,
            OrderSide::Ask => &self.sell_orders,
        }
    }

    pub fn side_map_mut(
        &mut self,
        side: &OrderSide,
    ) -> &mut BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>> {
        match side {
            OrderSide::Bid => &mut self.buy_orders,
            OrderSide::Ask => &mut self.sell_orders,
        }
    }

    pub fn get_order_list_mut(
        &mut self,
        side: &OrderSide,
        pair: Pair,
        price: u64,
    ) -> &mut VecDeque<OrderId> {
        self.side_map_mut(side)
            .entry(pair)
            .or_default()
            .entry(price)
            .or_default()
    }

    /// Inserts a new order into the appropriate data structures
    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn insert_order(
        &mut self,
        order: &Order,
        user_info_key: &H256,
    ) -> Result<Vec<OrderbookEvent>, String> {
        // Function only called for Limit orders
        let price = order.price.ok_or("Price cannot be None for limit orders")?;
        if price == 0 {
            return Err("Price cannot be zero".to_string());
        }

        let order_list = self.get_order_list_mut(&order.order_side, order.pair.clone(), price);

        order_list.push_back(order.order_id.clone());

        self.orders.insert(order.order_id.clone(), order.clone());

        // Keep track of the order owner
        // Only useful in server execution
        self.orders_owner
            .insert(order.order_id.clone(), *user_info_key);

        Ok(vec![OrderbookEvent::OrderCreated {
            order: order.clone(),
        }])
    }

    /// Cancels an order and removes it from data structures
    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn cancel_order(&mut self, order_id: &OrderId) -> Result<Vec<OrderbookEvent>, String> {
        let order = self
            .orders
            .get(order_id)
            .ok_or(format!("Order {order_id} not found"))?
            .clone();
        let price = order.price.ok_or("Price cannot be None for limit orders")?;

        // Remove the order from storage
        self.orders.remove(order_id);

        // Remove from order lists
        let order_list = self.get_order_list_mut(&order.order_side, order.pair.clone(), price);
        order_list.retain(|id| id != order_id);

        if order_list.is_empty() {
            self.side_map_mut(&order.order_side)
                .get_mut(&order.pair)
                .map(|v| v.remove(&price));
        }

        // Remove owner mapping
        self.orders_owner.remove(order_id);

        Ok(vec![OrderbookEvent::OrderCancelled {
            order_id: order_id.clone(),
            pair: order.pair,
        }])
    }

    /// Executes an order and returns generated events
    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn execute_order(
        &mut self,
        user_info_key: &H256,
        order: &Order,
    ) -> Result<Vec<OrderbookEvent>, String> {
        if self.orders.contains_key(&order.order_id) {
            return Err(format!("Order with id {} already exists", order.order_id));
        }

        let mut events = Vec::new();
        let mut order_to_execute = order.clone();

        // Try to fill existing orders by looking at the opposite side of the book
        let counter_orders_map = match order.order_side {
            OrderSide::Bid => &mut self.sell_orders,
            OrderSide::Ask => &mut self.buy_orders,
        };

        let counter_orders = match counter_orders_map.get_mut(&order.pair) {
            Some(orders) => orders,
            None => {
                return if order.order_type == OrderType::Limit {
                    self.insert_order(&order_to_execute, user_info_key)
                } else {
                    Err(format!(
                        "No matching {:?} orders for market order {}",
                        order.order_side, order.order_id
                    ))
                };
            }
        };

        while let Some((existing_order_price, mut existing_order_ids)) =
            counter_orders.pop_best(&order.order_side)
        {
            let mut break_outer = false;
            while let Some(existing_order_id) = existing_order_ids.pop_front() {
                let existing_order = self
                    .orders
                    .get_mut(&existing_order_id)
                    .ok_or(format!("Order {existing_order_id} not found"))?;

                // If the order is a limit order, check if the counter price respects the limit
                if let Some(price) = order_to_execute.price {
                    let price_should_defer = match order.order_side {
                        OrderSide::Bid => existing_order_price > price,
                        OrderSide::Ask => existing_order_price < price,
                    };

                    if price_should_defer {
                        existing_order_ids.push_front(existing_order_id);
                        counter_orders.insert(existing_order_price, existing_order_ids);
                        return self.insert_order(&order_to_execute, user_info_key);
                    }
                }

                match existing_order.quantity.cmp(&order_to_execute.quantity) {
                    std::cmp::Ordering::Greater => {
                        // Existing order is partially filled; put the remainder back at the front
                        existing_order.quantity -= order_to_execute.quantity;
                        existing_order_ids.push_front(existing_order_id.clone());
                        counter_orders.insert(existing_order_price, existing_order_ids);

                        events.push(OrderbookEvent::OrderUpdate {
                            order_id: existing_order_id.clone(),
                            taker_order_id: order_to_execute.order_id.clone(),
                            executed_quantity: order_to_execute.quantity,
                            remaining_quantity: existing_order.quantity,
                            pair: existing_order.pair.clone(),
                        });

                        order_to_execute.quantity = 0;
                        break_outer = true;
                        break;
                    }
                    std::cmp::Ordering::Equal => {
                        // Both orders are fully executed
                        events.push(OrderbookEvent::OrderExecuted {
                            order_id: existing_order_id.clone(),
                            taker_order_id: order_to_execute.order_id.clone(),
                            pair: existing_order.pair.clone(),
                        });

                        order_to_execute.quantity = 0;
                        break_outer = true;
                        break;
                    }
                    std::cmp::Ordering::Less => {
                        // Existing order is fully filled; continue to look for liquidity
                        events.push(OrderbookEvent::OrderExecuted {
                            order_id: existing_order_id.clone(),
                            taker_order_id: order_to_execute.order_id.clone(),
                            pair: existing_order.pair.clone(),
                        });

                        order_to_execute.quantity -= existing_order.quantity;
                    }
                }
            }
            if break_outer {
                break;
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
            let insert_events = self.insert_order(&order_to_execute, user_info_key)?;
            events.extend(insert_events);
        }

        Ok(events)
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn clear_executed_orders(&mut self, events: &[OrderbookEvent]) {
        for event in events {
            if let OrderbookEvent::OrderExecuted { order_id, .. } = event {
                self.orders.remove(order_id);
                self.orders_owner.remove(order_id);
            }
        }
    }
}

impl OrderManager {
    /// Helper function to compare order maps and generate diff entries
    fn diff_order_maps(
        &self,
        self_orders: &BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,
        other_orders: &BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,
        field_name: &str,
    ) -> BTreeMap<String, String> {
        let mut diff = BTreeMap::new();

        if self_orders != other_orders {
            diff.insert(
                format!("order_manager.{field_name}"),
                format!("Mismatching {field_name} orders"),
            );

            let other_orders_map = other_orders.iter().collect();
            let self_orders_map = self_orders.iter().collect();

            let mismatching_orders = diff_maps(&other_orders_map, &self_orders_map);
            mismatching_orders.added.iter().for_each(|id| {
                diff.insert(
                    format!("order_manager.{field_name}"),
                    format!(
                        "{}/{} {:?} != None",
                        id.0,
                        id.1,
                        self_orders
                            .get(id)
                            .map_or("None".to_string(), |o| format!("{o:?}"))
                    ),
                );
            });
            mismatching_orders.removed.iter().for_each(|id| {
                diff.insert(
                    format!("order_manager.{field_name}"),
                    format!(
                        "None != {}/{} {:?}",
                        id.0,
                        id.1,
                        other_orders
                            .get(id)
                            .map_or("None".to_string(), |o| format!("{o:?}"))
                    ),
                );
            });
            mismatching_orders.changed.iter().for_each(|(id, _)| {
                diff.insert(
                    format!("order_manager.{field_name}"),
                    format!(
                        "{}/{} {:?} != {}/{} {:?}",
                        id.0,
                        id.1,
                        self_orders
                            .get(id)
                            .map_or("None".to_string(), |o| format!("{o:?}")),
                        id.0,
                        id.1,
                        other_orders
                            .get(id)
                            .map_or("None".to_string(), |o| format!("{o:?}"))
                    ),
                );
            });
        }

        diff
    }

    pub fn diff(&self, other: &OrderManager) -> BTreeMap<String, String> {
        let mut diff = BTreeMap::new();
        if self.orders != other.orders {
            diff.insert(
                "order_manager.orders".to_string(),
                "Mismatching orders".to_string(),
            );

            let other_orders = other.orders.iter().collect();
            let self_orders = self.orders.iter().collect();

            let mismatching_orders = diff_maps(&other_orders, &self_orders);
            mismatching_orders.added.iter().for_each(|id| {
                diff.insert(
                    "order_manager.orders".to_string(),
                    format!(
                        "{id:?}: {:?} != None",
                        self_orders
                            .get(*id)
                            .map_or("None".to_string(), |o| format!("{o:?}"))
                    ),
                );
            });
            mismatching_orders.removed.iter().for_each(|id| {
                diff.insert(
                    "order_manager.orders".to_string(),
                    format!(
                        "None != {id:?}: {:?}",
                        other_orders
                            .get(*id)
                            .map_or("None".to_string(), |o| format!("{o:?}"))
                    ),
                );
            });
            mismatching_orders.changed.iter().for_each(|(id, _)| {
                diff.insert(
                    "order_manager.orders".to_string(),
                    format!(
                        "{id:?}: {:?} != {id:?}: {:?}",
                        self_orders
                            .get(*id)
                            .map_or("None".to_string(), |o| format!("{o:?}")),
                        other_orders
                            .get(*id)
                            .map_or("None".to_string(), |o| format!("{o:?}"))
                    ),
                );
            });
        }

        diff.extend(self.diff_order_maps(&self.buy_orders, &other.buy_orders, "buy_orders"));
        diff.extend(self.diff_order_maps(&self.sell_orders, &other.sell_orders, "sell_orders"));

        // TODO check order_owner

        diff
    }
}

use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
struct MapDiff<'a, K, V> {
    added: HashSet<&'a K>,
    removed: HashSet<&'a K>,
    changed: HashMap<&'a K, (&'a V, &'a V)>, // (old, new)
}

fn diff_maps<'a, K, V>(old: &'a HashMap<K, V>, new: &'a HashMap<K, V>) -> MapDiff<'a, K, V>
where
    K: std::hash::Hash + Eq,
    V: PartialEq,
{
    let mut d = MapDiff {
        added: HashSet::new(),
        removed: HashSet::new(),
        changed: HashMap::new(),
    };

    // supprimées + modifiées
    for (k, old_v) in old.iter() {
        match new.get(k) {
            None => {
                d.removed.insert(k);
            }
            Some(new_v) if new_v != old_v => {
                d.changed.insert(k, (old_v, new_v));
            }
            _ => {}
        }
    }

    // ajoutées
    for k in new.keys() {
        if !old.contains_key(k) {
            d.added.insert(k);
        }
    }

    d
}
