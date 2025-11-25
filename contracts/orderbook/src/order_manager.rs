use crate::model::{Order, OrderId, OrderSide, OrderType, OrderbookEvent, Pair};
use crate::zk::H256;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, VecDeque};

#[derive(Serialize, BorshSerialize, BorshDeserialize, Default, Debug, Clone, PartialEq, Eq)]
pub struct OrderManager {
    // All orders indexed by order_id
    pub orders: HashMap<OrderId, Order>,
    // Buy orders sorted by price for each token pair
    pub bid_orders: HashMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,
    // Ask orders sorted by price for each token pair
    pub ask_orders: HashMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,

    // Mapping of order IDs to their owners
    pub orders_owner: HashMap<OrderId, H256>,
}

#[cfg(test)]
mod tests;

impl OrderManager {
    pub fn new() -> Self {
        Self::default()
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

    pub fn side_map(&self, side: &OrderSide) -> &HashMap<Pair, BTreeMap<u64, VecDeque<OrderId>>> {
        match side {
            OrderSide::Bid => &self.bid_orders,
            OrderSide::Ask => &self.ask_orders,
        }
    }

    pub fn side_map_mut(
        &mut self,
        side: &OrderSide,
    ) -> &mut HashMap<Pair, BTreeMap<u64, VecDeque<OrderId>>> {
        match side {
            OrderSide::Bid => &mut self.bid_orders,
            OrderSide::Ask => &mut self.ask_orders,
        }
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
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

    /// Cancels an order and removes it from data structures
    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn cancel_order_dry_run(&self, order_id: &OrderId) -> Result<Vec<OrderbookEvent>, String> {
        let order = self
            .orders
            .get(order_id)
            .ok_or_else(|| format!("Order {order_id} not found"))?;

        Ok(vec![OrderbookEvent::OrderCancelled {
            order_id: order_id.clone(),
            pair: order.pair.clone(),
        }])
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn execute_order_dry_run(&self, order: &Order) -> Result<Vec<OrderbookEvent>, String> {
        #[cfg(feature = "instrumentation")]
        let span = sdk::tracing::span!(sdk::tracing::Level::INFO, "get_existing_order").entered();
        if let Some(existing_order) = self.orders.get(&order.order_id) {
            // When loaded in the SMT, an existing order with zero quantity means it is not part of the SMT
            if existing_order.quantity != 0 {
                return Err(format!(
                    "Order with id {} already exists with non-zero quantity",
                    order.order_id
                ));
            }
        }
        #[cfg(feature = "instrumentation")]
        span.exit();

        let mut events = Vec::new();
        let mut order_to_execute = order.clone();

        #[cfg(feature = "instrumentation")]
        let span =
            sdk::tracing::span!(sdk::tracing::Level::INFO, "get_counter_orders_map").entered();
        let counter_orders_map = match order.order_side {
            OrderSide::Bid => self.ask_orders.get(&order.pair),
            OrderSide::Ask => self.bid_orders.get(&order.pair),
        };
        #[cfg(feature = "instrumentation")]
        span.exit();

        #[cfg(feature = "instrumentation")]
        let span = sdk::tracing::span!(sdk::tracing::Level::INFO, "get_counter_orders").entered();
        let counter_orders: Box<dyn Iterator<Item = (&u64, &VecDeque<OrderId>)>> =
            match counter_orders_map {
                Some(orders) => match order.order_side {
                    OrderSide::Bid => Box::new(orders.iter()),
                    OrderSide::Ask => Box::new(orders.iter().rev()),
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
        #[cfg(feature = "instrumentation")]
        span.exit();

        #[cfg(feature = "instrumentation")]
        sdk::tracing::event!(sdk::tracing::Level::INFO, "Got counter orders");

        for (existing_order_price, existing_order_ids) in counter_orders {
            #[cfg(feature = "instrumentation")]
            sdk::tracing::event!(
                sdk::tracing::Level::INFO,
                "Got existing order price {existing_order_price}"
            );
            let mut break_outer = false;

            for existing_order_id in existing_order_ids.iter() {
                #[cfg(feature = "instrumentation")]
                let span = sdk::tracing::span!(
                    sdk::tracing::Level::INFO,
                    "get_existing_order",
                    existing_order_id = existing_order_id.to_string()
                )
                .entered();
                let existing_order = self
                    .orders
                    .get(existing_order_id)
                    .ok_or(format!("Order {existing_order_id} not found"))?;
                #[cfg(feature = "instrumentation")]
                span.exit();

                if let Some(price) = order_to_execute.price {
                    let price_should_defer = match order.order_side {
                        OrderSide::Bid => *existing_order_price > price,
                        OrderSide::Ask => *existing_order_price < price,
                    };

                    if price_should_defer {
                        #[cfg(feature = "instrumentation")]
                        sdk::tracing::event!(
                            sdk::tracing::Level::INFO,
                            "Price should defer, pushing existing order id {existing_order_id} to front"
                        );
                        // existing_order_ids.push_front(existing_order_id);
                        // if !existing_order_ids.is_empty() {
                        //     // counter_orders.push_front((existing_order_price, existing_order_ids));
                        // }
                        events.extend(Self::simulate_insert_order(&order_to_execute)?);
                        return Ok(events);
                    }
                }

                #[cfg(feature = "instrumentation")]
                let span = sdk::tracing::span!(
                    sdk::tracing::Level::INFO,
                    "match_existing_order_quantity",
                    existing_order_quantity = existing_order.quantity,
                    order_to_execute_quantity = order_to_execute.quantity
                )
                .entered();
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
                #[cfg(feature = "instrumentation")]
                span.exit();
            }

            if break_outer {
                #[cfg(feature = "instrumentation")]
                sdk::tracing::event!(sdk::tracing::Level::INFO, "Break outer");
                if !existing_order_ids.is_empty() {
                    // counter_orders.push_front((existing_order_price, existing_order_ids));
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

        #[cfg(feature = "instrumentation")]
        let span =
            sdk::tracing::span!(sdk::tracing::Level::INFO, "execute_order_dry_run_final").entered();
        if order_to_execute.quantity > 0 && order_to_execute.order_type == OrderType::Limit {
            let insert_events = Self::simulate_insert_order(&order_to_execute)?;
            events.extend(insert_events);
        }
        #[cfg(feature = "instrumentation")]
        span.exit();

        Ok(events)
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn apply_event(
        &mut self,
        user_info_key: H256,
        event: &OrderbookEvent,
    ) -> Result<(), String> {
        match event {
            OrderbookEvent::OrderCreated { order } => {
                #[cfg(feature = "instrumentation")]
                let span =
                    sdk::tracing::span!(sdk::tracing::Level::INFO, "apply_events_order_created")
                        .entered();
                let price = order.price.ok_or_else(|| {
                    "OrderCreated event missing price for limit order".to_string()
                })?;

                let level = self
                    .side_map_mut(&order.order_side)
                    .entry(order.pair.clone())
                    .or_default()
                    .entry(price)
                    .or_default();

                level.push_back(order.order_id.clone());
                self.orders.insert(order.order_id.clone(), order.clone());
                self.orders_owner
                    .entry(order.order_id.clone())
                    .or_insert(user_info_key);
                #[cfg(feature = "instrumentation")]
                span.exit();
            }
            OrderbookEvent::OrderCancelled { order_id, .. } => {
                #[cfg(feature = "instrumentation")]
                let span =
                    sdk::tracing::span!(sdk::tracing::Level::INFO, "apply_events_order_cancelled")
                        .entered();
                let order = self
                    .orders
                    .get(order_id)
                    .ok_or_else(|| {
                        format!("OrderCancelled event missing order {order_id}").to_string()
                    })?
                    .clone();

                // Remove order from price level
                if let Some(price) = order.price {
                    let order_list =
                        self.get_order_list_mut(&order.order_side, order.pair.clone(), price);
                    // We shall not remove empty price levels from the orderbook here, as it will be needed for computing SMT root later
                    order_list.retain(|id| id != order_id);
                }

                // We shall not remove order from the orderbook here, as it will be needed for computing SMT root later
                let order_mut = self.orders.get_mut(order_id).unwrap();
                order_mut.quantity = 0;

                self.orders_owner.remove(order_id);
                #[cfg(feature = "instrumentation")]
                span.exit();
            }
            OrderbookEvent::OrderExecuted {
                order_id,
                taker_order_id,
                ..
            } => {
                if order_id == taker_order_id {
                    return Ok(());
                }

                #[cfg(feature = "instrumentation")]
                let span =
                    sdk::tracing::span!(sdk::tracing::Level::INFO, "apply_events_order_executed")
                        .entered();
                let order = self
                    .orders
                    .get(order_id)
                    .ok_or_else(|| {
                        format!("OrderExecuted event missing order {order_id}").to_string()
                    })?
                    .clone();

                // Remove order from price level
                if let Some(price) = order.price {
                    let order_list =
                        self.get_order_list_mut(&order.order_side, order.pair.clone(), price);

                    // We shall not remove empty price levels from the orderbook here, as it will be needed for computing SMT root later
                    order_list.retain(|id| id != order_id);
                }

                // We shall not remove order from the orderbook here, as it will be needed for computing SMT root later
                let order_mut = self.orders.get_mut(order_id).unwrap();
                order_mut.quantity = 0;

                self.orders_owner.remove(order_id);
                #[cfg(feature = "instrumentation")]
                span.exit();
            }
            OrderbookEvent::OrderUpdate {
                order_id,
                remaining_quantity,
                ..
            } => {
                #[cfg(feature = "instrumentation")]
                let span =
                    sdk::tracing::span!(sdk::tracing::Level::INFO, "apply_events_order_update")
                        .entered();
                let order = self.orders.get_mut(order_id).ok_or_else(|| {
                    format!("OrderUpdate event missing order {order_id}").to_string()
                })?;
                order.quantity = *remaining_quantity;
                #[cfg(feature = "instrumentation")]
                span.exit();
            }
            _ => {}
        }

        Ok(())
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(self)))]
    pub fn clean(&mut self, events: &[OrderbookEvent]) {
        for event in events {
            match event {
                OrderbookEvent::OrderExecuted {
                    order_id,
                    taker_order_id,
                    ..
                } => {
                    if order_id == taker_order_id {
                        continue;
                    }

                    if let Some(stored_order) = self.orders.get(order_id).cloned() {
                        self.clean_empty_price_levels(&stored_order.order_side, &stored_order.pair);
                        self.orders.remove(order_id);
                    }

                    self.orders_owner.remove(order_id);
                }
                OrderbookEvent::OrderCancelled { order_id, .. } => {
                    if let Some(stored_order) = self.orders.get(order_id).cloned() {
                        self.clean_empty_price_levels(&stored_order.order_side, &stored_order.pair);
                        self.orders.remove(order_id);
                    }

                    self.orders_owner.remove(order_id);
                }
                _ => {}
            }
        }
    }
}

impl OrderManager {
    fn clean_empty_price_levels(&mut self, side: &OrderSide, pair: &Pair) {
        let side_book = self.side_map_mut(side);
        let should_remove_pair = if let Some(price_levels) = side_book.get_mut(pair) {
            // Remove empty price levels
            price_levels.retain(|_, order_ids| !order_ids.is_empty());
            price_levels.is_empty()
        } else {
            false
        };

        if should_remove_pair {
            side_book.remove(pair);
        }
    }

    #[cfg_attr(feature = "instrumentation", tracing::instrument(skip(order)))]
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
        self_orders: &HashMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,
        other_orders: &HashMap<Pair, BTreeMap<u64, VecDeque<OrderId>>>,
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

            diff_maps(
                &mut diff,
                &format!("order_manager.{field_name}"),
                &other_orders_map,
                &self_orders_map,
            );
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

            diff_maps(
                &mut diff,
                "order_manager.orders",
                &other_orders,
                &self_orders,
            );
        }

        diff.extend(self.diff_order_maps(&self.bid_orders, &other.bid_orders, "bid_orders"));
        diff.extend(self.diff_order_maps(&self.ask_orders, &other.ask_orders, "ask_orders"));

        // TODO check order_owner

        diff
    }
}

/// Impl of functions for testing purposes
impl OrderManager {
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
}
use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct MapDiff<'a, K, V> {
    pub added: HashSet<&'a K>,
    pub removed: HashSet<&'a K>,
    pub changed: BTreeMap<&'a K, (&'a V, &'a V)>, // (old, new)
}

pub fn diff_maps<'a, K, V>(
    diff: &mut BTreeMap<String, String>,
    key: &str,
    old: &'a BTreeMap<K, V>,
    new: &'a BTreeMap<K, V>,
) where
    K: std::hash::Hash + Eq + std::fmt::Debug + Ord,
    V: PartialEq + std::fmt::Debug,
{
    let mut d = MapDiff {
        added: HashSet::new(),
        removed: HashSet::new(),
        changed: BTreeMap::new(),
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

    d.update_diff(key, diff, old, new);
}

impl<'a, K, V> MapDiff<'a, K, V>
where
    K: std::hash::Hash + Eq + std::fmt::Debug + Ord,
    V: PartialEq + std::fmt::Debug,
{
    pub fn update_diff(
        &self,
        key: &str,
        diff: &mut BTreeMap<String, String>,
        old: &BTreeMap<K, V>,
        new: &BTreeMap<K, V>,
    ) {
        self.added.iter().for_each(|k| {
            diff.insert(
                format!("{key}.{:?} added", k),
                format!(
                    "{:?}",
                    old.get(k).map_or("None".to_string(), |o| format!("{o:?}"))
                ),
            );
        });
        self.removed.iter().for_each(|k| {
            diff.insert(
                format!("{key}.{:?} removed", k),
                format!(
                    "{:?}",
                    new.get(k).map_or("None".to_string(), |o| format!("{o:?}"))
                ),
            );
        });
        self.changed.iter().for_each(|(k, (old, new))| {
            diff.insert(
                format!("{key}.{:?} changed", k),
                format!("{:?} -> {:?}", old, new),
            );
        });
    }
}
