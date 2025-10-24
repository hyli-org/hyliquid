use crate::model::{Order, OrderId, OrderSide, OrderType, OrderbookEvent, Pair};
use crate::zk::H256;
use borsh::{BorshDeserialize, BorshSerialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(BorshSerialize, BorshDeserialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct OrderManager {
    // All orders indexed by order_id
    pub orders: BTreeMap<OrderId, Order>,
    // Buy orders sorted by price for each token pair
    pub bid_orders: BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,
    // Ask orders sorted by price for each token pair
    pub ask_orders: BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,

    // Mapping of order IDs to their owners
    pub orders_owner: BTreeMap<OrderId, H256>,
}

#[derive(BorshSerialize, Debug, Clone, PartialEq, Eq)]
pub struct OrderManagerView<'a> {
    pub orders: &'a BTreeMap<OrderId, Order>,
    pub bid_orders: &'a BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,
    pub ask_orders: &'a BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,
    pub orders_owner: BTreeMap<OrderId, H256>,
}

#[cfg(test)]
mod tests;

impl OrderManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn view<'a>(&'a self) -> OrderManagerView<'a> {
        OrderManagerView {
            orders: &self.orders,
            bid_orders: &self.bid_orders,
            ask_orders: &self.ask_orders,
            orders_owner: Default::default(),
        }
    }

    pub fn count_buy_orders(&self, pair: &Pair) -> usize {
        self.bid_orders
            .get(pair)
            .map(|v| v.values().map(|v| v.len()).sum())
            .unwrap_or(0)
    }

    pub fn count_sell_orders(&self, pair: &Pair) -> usize {
        self.ask_orders
            .get(pair)
            .map(|v| v.values().map(|v| v.len()).sum())
            .unwrap_or(0)
    }

    pub fn side_map(&self, side: &OrderSide) -> &BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>> {
        match side {
            OrderSide::Bid => &self.bid_orders,
            OrderSide::Ask => &self.ask_orders,
        }
    }

    pub fn side_map_mut(
        &mut self,
        side: &OrderSide,
    ) -> &mut BTreeMap<Pair, BTreeMap<u64, VecDeque<OrderId>>> {
        match side {
            OrderSide::Bid => &mut self.bid_orders,
            OrderSide::Ask => &mut self.ask_orders,
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
        let events = self.execute_order_dry_run(order)?;
        self.apply_events(*user_info_key, &events)?;

        Ok(events)
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn execute_order_dry_run(&self, order: &Order) -> Result<Vec<OrderbookEvent>, String> {
        if self.orders.contains_key(&order.order_id) {
            return Err(format!("Order with id {} already exists", order.order_id));
        }

        let mut events = Vec::new();
        let mut order_to_execute = order.clone();

        let counter_orders_map = match order.order_side {
            OrderSide::Bid => self.ask_orders.get(&order.pair),
            OrderSide::Ask => self.bid_orders.get(&order.pair),
        };

        let mut counter_orders: VecDeque<(u64, VecDeque<OrderId>)> = match counter_orders_map {
            Some(orders) => match order.order_side {
                OrderSide::Bid => orders
                    .iter()
                    .map(|(price, ids)| (*price, ids.clone()))
                    .collect(),
                OrderSide::Ask => orders
                    .iter()
                    .rev()
                    .map(|(price, ids)| (*price, ids.clone()))
                    .collect(),
            },
            None => {
                return if order.order_type == OrderType::Limit {
                    Self::simulate_insert_order(order)
                } else {
                    Err(format!(
                        "No matching {:?} orders for market order {}",
                        order.order_side, order.order_id
                    ))
                };
            }
        };

        while let Some((existing_order_price, mut existing_order_ids)) = counter_orders.pop_front()
        {
            let mut break_outer = false;

            while let Some(existing_order_id) = existing_order_ids.pop_front() {
                let existing_order = self
                    .orders
                    .get(&existing_order_id)
                    .ok_or(format!("Order {existing_order_id} not found"))?;

                if let Some(price) = order_to_execute.price {
                    let price_should_defer = match order.order_side {
                        OrderSide::Bid => existing_order_price > price,
                        OrderSide::Ask => existing_order_price < price,
                    };

                    if price_should_defer {
                        existing_order_ids.push_front(existing_order_id);
                        if !existing_order_ids.is_empty() {
                            counter_orders.push_front((existing_order_price, existing_order_ids));
                        }
                        return Self::simulate_insert_order(&order_to_execute);
                    }
                }

                match existing_order.quantity.cmp(&order_to_execute.quantity) {
                    std::cmp::Ordering::Greater => {
                        let remaining_quantity =
                            existing_order.quantity - order_to_execute.quantity;
                        events.push(OrderbookEvent::OrderUpdate {
                            order_id: existing_order_id.clone(),
                            taker_order_id: order_to_execute.order_id.clone(),
                            executed_quantity: order_to_execute.quantity,
                            remaining_quantity,
                            pair: existing_order.pair.clone(),
                        });

                        order_to_execute.quantity = 0;
                        break_outer = true;
                        break;
                    }
                    std::cmp::Ordering::Equal => {
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
                if !existing_order_ids.is_empty() {
                    counter_orders.push_front((existing_order_price, existing_order_ids));
                }
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

        if order_to_execute.quantity > 0 && order_to_execute.order_type == OrderType::Limit {
            let insert_events = Self::simulate_insert_order(&order_to_execute)?;
            events.extend(insert_events);
        }

        Ok(events)
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn apply_events(
        &mut self,
        user_info_key: H256,
        events: &[OrderbookEvent],
    ) -> Result<(), String> {
        for event in events {
            match event {
                OrderbookEvent::OrderCreated { order } => {
                    let price = order.price.ok_or_else(|| {
                        "OrderCreated event missing price for limit order".to_string()
                    })?;

                    let level = self
                        .side_map_mut(&order.order_side)
                        .entry(order.pair.clone())
                        .or_default()
                        .entry(price)
                        .or_default();

                    if !level.contains(&order.order_id) {
                        level.push_back(order.order_id.clone());
                    }

                    self.orders.insert(order.order_id.clone(), order.clone());

                    self.orders_owner
                        .entry(order.order_id.clone())
                        .or_insert(user_info_key);
                }
                OrderbookEvent::OrderCancelled { order_id, .. } => {
                    let stored_order =
                        self.orders.get(order_id).cloned().ok_or_else(|| {
                            format!("Order {order_id} not found for cancellation")
                        })?;

                    self.remove_order_from_orderbook(
                        &stored_order.order_side,
                        &stored_order.pair,
                        stored_order.price,
                        order_id,
                    );
                    self.orders.remove(order_id);
                    self.orders_owner.remove(order_id);
                }
                OrderbookEvent::OrderExecuted {
                    order_id,
                    taker_order_id,
                    ..
                } => {
                    if order_id == taker_order_id {
                        continue;
                    }

                    if let Some(stored_order) = self.orders.get(order_id).cloned() {
                        self.remove_order_from_orderbook(
                            &stored_order.order_side,
                            &stored_order.pair,
                            stored_order.price,
                            order_id,
                        );
                        self.orders.remove(order_id);
                    }

                    self.orders_owner.remove(order_id);
                }
                OrderbookEvent::OrderUpdate {
                    order_id,
                    remaining_quantity,
                    ..
                } => {
                    if let Some(order) = self.orders.get_mut(order_id) {
                        order.quantity = *remaining_quantity;
                    }
                }
                _ => {}
            }
        }

        Ok(())
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
    fn remove_order_from_orderbook(
        &mut self,
        side: &OrderSide,
        pair: &Pair,
        price: Option<u64>,
        order_id: &OrderId,
    ) {
        if let Some(price) = price {
            let side_book = self.side_map_mut(side);
            let should_remove_pair = if let Some(price_levels) = side_book.get_mut(pair) {
                if let Some(order_ids) = price_levels.get_mut(&price) {
                    order_ids.retain(|existing_id| existing_id != order_id);
                    if order_ids.is_empty() {
                        price_levels.remove(&price);
                    }
                }

                price_levels.is_empty()
            } else {
                false
            };

            if should_remove_pair {
                side_book.remove(pair);
            }
        }
    }

    fn simulate_insert_order(order: &Order) -> Result<Vec<OrderbookEvent>, String> {
        let price = order
            .price
            .ok_or_else(|| "Price cannot be None for limit orders".to_string())?;

        if price == 0 {
            return Err("Price cannot be zero".to_string());
        }

        Ok(vec![OrderbookEvent::OrderCreated {
            order: order.clone(),
        }])
    }

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

        diff.extend(self.diff_order_maps(&self.bid_orders, &other.bid_orders, "bid_orders"));
        diff.extend(self.diff_order_maps(&self.ask_orders, &other.ask_orders, "ask_orders"));

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
