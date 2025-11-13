use sdk::merkle_utils::BorshableMerkleProof;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    model::{
        Balance, Order, OrderCollectionMode, OrderSide, OrderType, OrderbookEvent, Symbol, UserInfo,
    },
    transaction::PermissionnedOrderbookAction,
    zk::{
        order_merkle::OrderPriceLevel,
        smt::{GetKey, UserBalance},
        FullState, OrderManagerWitnesses, Proof, ZkVmState, ZkWitnessSet, SMT,
    },
};

type UsersAndBalancesNeeded = (HashSet<UserInfo>, HashMap<Symbol, Vec<UserBalance>>);
type OrdersNeeded = (
    HashSet<Order>,
    HashSet<OrderPriceLevel>,
    HashSet<OrderPriceLevel>,
);

type ZkvmComputedInputs = (
    ZkWitnessSet<UserInfo>,
    HashMap<Symbol, ZkWitnessSet<UserBalance>>,
    OrderManagerWitnesses,
);

/// impl of functions for zkvm state generation and verification
impl FullState {
    pub fn collect_user_and_balance_updates(
        &self,
        base_user: &UserInfo,
        events: &[OrderbookEvent],
    ) -> Result<UsersAndBalancesNeeded, String> {
        let mut users_info_needed: HashSet<UserInfo> = HashSet::new();
        let base = self.resolve_user_from_state(base_user, &base_user.user)?;
        users_info_needed.insert(base);
        let mut balances_needed: HashMap<Symbol, Vec<UserBalance>> = HashMap::new();

        for event in events {
            match event {
                OrderbookEvent::BalanceUpdated {
                    user,
                    symbol,
                    amount,
                } => {
                    let ui = self.resolve_user_from_state(base_user, user)?;
                    users_info_needed.insert(ui.clone());
                    let user_key = ui.get_key();
                    users_info_needed.insert(ui);
                    balances_needed
                        .entry(symbol.clone())
                        .or_default()
                        .push(UserBalance {
                            user_key,
                            balance: Balance(*amount),
                        });
                }
                OrderbookEvent::SessionKeyAdded { user, .. }
                | OrderbookEvent::NonceIncremented { user, .. } => {
                    let ui = self.resolve_user_from_state(base_user, user)?;
                    users_info_needed.insert(ui);
                }
                OrderbookEvent::PairCreated { pair, .. } => {
                    balances_needed.entry(pair.0.clone()).or_default();
                    balances_needed.entry(pair.1.clone()).or_default();
                }
                _ => {}
            }
        }

        Ok((users_info_needed, balances_needed))
    }

    // TODO: code factorization
    pub fn collect_orders_updates(
        &self,
        events: &[OrderbookEvent],
        collection_mode: OrderCollectionMode,
    ) -> Result<OrdersNeeded, String> {
        let mut orders_to_update: HashSet<Order> = HashSet::new();
        let mut bid_order_price_levels: HashSet<OrderPriceLevel> = HashSet::new();
        let mut ask_order_price_levels: HashSet<OrderPriceLevel> = HashSet::new();

        for event in events.iter() {
            match event {
                OrderbookEvent::OrderCancelled { order_id, pair } => {
                    let order = self
                        .state
                        .order_manager
                        .orders
                        .get(order_id)
                        .ok_or_else(|| format!("Order cancelled {order_id} not found"))?;

                    orders_to_update.insert(order.clone());

                    let price = order
                        .price
                        .ok_or_else(|| format!("Order {order_id} has no price"))?;

                    // Get the actual order queue for this price level
                    let side_map = match order.order_side {
                        OrderSide::Bid => &self.state.order_manager.bid_orders,
                        OrderSide::Ask => &self.state.order_manager.ask_orders,
                    };

                    let price_map = side_map
                        .get(pair)
                        .ok_or_else(|| format!("Price map not found for pair {pair:?}"))?;

                    let order_queue = price_map
                        .get(&price)
                        .ok_or_else(|| format!("Order queue not found for price {price}"))?;

                    let price_level = OrderPriceLevel {
                        pair: pair.clone(),
                        price,
                        order_ids: order_queue.iter().cloned().collect(),
                    };

                    match order.order_side {
                        OrderSide::Bid => {
                            bid_order_price_levels.insert(price_level);
                        }
                        OrderSide::Ask => {
                            ask_order_price_levels.insert(price_level);
                        }
                    }
                }
                OrderbookEvent::OrderUpdate { order_id, pair, .. } => {
                    // Get the order from the current state
                    let order = self
                        .state
                        .order_manager
                        .orders
                        .get(order_id)
                        .ok_or_else(|| format!("Order updated {order_id} not found"))?;

                    orders_to_update.insert(order.clone());

                    let price = order
                        .price
                        .ok_or_else(|| format!("Order {order_id} has no price"))?;

                    // Get the actual order queue for this price level
                    let side_map = match order.order_side {
                        OrderSide::Bid => &self.state.order_manager.bid_orders,
                        OrderSide::Ask => &self.state.order_manager.ask_orders,
                    };

                    let price_map = side_map
                        .get(pair)
                        .ok_or_else(|| format!("Price map not found for pair {pair:?}"))?;

                    let order_queue = price_map
                        .get(&price)
                        .ok_or_else(|| format!("Order queue not found for price {price}"))?;

                    let price_level = OrderPriceLevel {
                        pair: pair.clone(),
                        price,
                        // Keeping this order for initial state witness
                        order_ids: order_queue.iter().cloned().collect(),
                    };

                    match order.order_side {
                        OrderSide::Bid => {
                            bid_order_price_levels.insert(price_level);
                        }
                        OrderSide::Ask => {
                            ask_order_price_levels.insert(price_level);
                        }
                    }
                }
                OrderbookEvent::OrderExecuted {
                    order_id,
                    taker_order_id,
                    pair,
                    ..
                } => {
                    if order_id == taker_order_id {
                        // Market orders are not stored in the SMT
                        continue;
                    }

                    // Get the order from the current state
                    let order = self
                        .state
                        .order_manager
                        .orders
                        .get(order_id)
                        .ok_or_else(|| format!("Order executed {order_id} not found"))?;

                    orders_to_update.insert(order.clone());

                    let price = order
                        .price
                        .ok_or_else(|| format!("Order {order_id} has no price"))?;

                    // Get the actual order queue for this price level
                    let side_map = match order.order_side {
                        OrderSide::Bid => &self.state.order_manager.bid_orders,
                        OrderSide::Ask => &self.state.order_manager.ask_orders,
                    };

                    let price_map = side_map
                        .get(pair)
                        .ok_or_else(|| format!("Price map not found for pair {pair:?}"))?;

                    let order_queue = price_map
                        .get(&price)
                        .ok_or_else(|| format!("Order queue not found for price {price}"))?;

                    let price_level = OrderPriceLevel {
                        pair: pair.clone(),
                        price,
                        order_ids: order_queue.iter().cloned().collect(),
                    };

                    match order.order_side {
                        OrderSide::Bid => {
                            bid_order_price_levels.insert(price_level);
                        }
                        OrderSide::Ask => {
                            ask_order_price_levels.insert(price_level);
                        }
                    }
                }
                OrderbookEvent::OrderCreated { order } => {
                    if order.order_type == OrderType::Market {
                        // Market orders are not stored in the SMT
                        continue;
                    }

                    let mut created_order = order.clone();
                    if matches!(collection_mode, OrderCollectionMode::ForInitialStateWitness) {
                        // Quantity at 0 will make the SMT value be zeroed out. This is necessary to prove the order's non-existence in the tree
                        created_order.quantity = 0;
                    }
                    orders_to_update.insert(created_order);

                    let price = order
                        .price
                        .ok_or_else(|| format!("Order {} has no price", order.order_id))?;

                    // Get the actual order queue for this price level
                    let side_map = match order.order_side {
                        OrderSide::Bid => &self.state.order_manager.bid_orders,
                        OrderSide::Ask => &self.state.order_manager.ask_orders,
                    };

                    let price_map = side_map.get(&order.pair).cloned().unwrap_or_default();

                    let order_queue = price_map.get(&price).cloned().unwrap_or_default();

                    let price_level = OrderPriceLevel {
                        pair: order.pair.clone(),
                        price,
                        order_ids: order_queue.iter().cloned().collect(),
                    };

                    match order.order_side {
                        OrderSide::Bid => {
                            bid_order_price_levels.insert(price_level);
                        }
                        OrderSide::Ask => {
                            ask_order_price_levels.insert(price_level);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok((
            orders_to_update,
            bid_order_price_levels,
            ask_order_price_levels,
        ))
    }

    fn create_users_info_witness(
        &self,
        users: &HashSet<UserInfo>,
    ) -> Result<ZkWitnessSet<UserInfo>, String> {
        let proof = self.get_users_info_proofs(users)?;
        let mut values = HashSet::new();
        for user in users {
            values.insert(user.clone());
        }
        Ok(ZkWitnessSet { values, proof })
    }

    fn create_balances_witness(
        &self,
        symbol: &Symbol,
        users: &[UserInfo],
    ) -> Result<ZkWitnessSet<UserBalance>, String> {
        let (balances, proof) = self.get_balances_with_proof(users, symbol)?;
        let mut values = HashSet::new();
        for (user_info, balance) in balances {
            values.insert(UserBalance {
                user_key: user_info.get_key(),
                balance,
            });
        }
        Ok(ZkWitnessSet { values, proof })
    }

    fn get_users_info_proofs(&self, users_info: &HashSet<UserInfo>) -> Result<Proof, String> {
        if users_info.is_empty() {
            return Ok(Proof::CurrentRootHash(self.users_info_mt.root()));
        }

        Ok(Proof::Some(BorshableMerkleProof(
            self.users_info_mt
                .merkle_proof(users_info.iter())
                .map_err(|e| {
                    format!("Failed to create merkle proof for users {users_info:?}: {e}")
                })?,
        )))
    }

    fn get_balances_with_proof(
        &self,
        users_info: &[UserInfo],
        symbol: &Symbol,
    ) -> Result<(HashMap<UserInfo, Balance>, Proof), String> {
        let zero_tree = SMT::<UserBalance>::zero();
        let tree = self.balances_mt.get(symbol).unwrap_or(&zero_tree);

        if users_info.is_empty() {
            let root = &tree.root();
            return Ok((HashMap::new(), Proof::CurrentRootHash(*root)));
        }

        let mut balances_map = HashMap::new();
        for user_info in users_info {
            let balance = self.state.get_balance(user_info, symbol);
            balances_map.insert(user_info.clone(), balance);
        }

        let users: Vec<UserInfo> = balances_map.keys().cloned().collect();
        let proof = BorshableMerkleProof(tree.merkle_proof(users.iter()).map_err(|e| {
            format!(
                "Failed to create merkle proof for {symbol} and users {:?}: {e}",
                users_info
                    .iter()
                    .map(|u| u.user.clone())
                    .collect::<Vec<_>>()
            )
        })?);

        Ok((balances_map, Proof::Some(proof)))
    }

    fn for_zkvm(
        &self,
        user_info: &UserInfo,
        events: &[OrderbookEvent],
        action: &PermissionnedOrderbookAction,
    ) -> Result<ZkvmComputedInputs, String> {
        // We populate orders owners based on events with only needed values
        let mut orders_owner = BTreeMap::new();

        // Track all users, their balances per symbol, and order-user mapping for executed/updated orders
        for event in events {
            match event {
                OrderbookEvent::OrderExecuted { order_id, .. }
                | OrderbookEvent::OrderUpdate { order_id, .. }
                | OrderbookEvent::OrderCancelled { order_id, .. } => {
                    if let Some(order_owner) = self.state.order_manager.orders_owner.get(order_id) {
                        orders_owner.insert(order_id.clone(), *order_owner);
                    } else if let PermissionnedOrderbookAction::CreateOrder(Order {
                        order_id: create_order_id,
                        ..
                    }) = action
                    {
                        if create_order_id == order_id {
                            // Special case: the order was created in the same tx, we can use the user_info
                            orders_owner.insert(order_id.clone(), user_info.get_key());
                        }
                    } else {
                        return Err(format!(
                            "Order with id {order_id} does not have an owner in orders_owner mapping"
                        ));
                    }
                }
                OrderbookEvent::BalanceUpdated { .. } => {}
                OrderbookEvent::SessionKeyAdded { .. } => {}
                _ => {}
            }
        }

        // We collect user and balance updates and compute their witnesses
        let (users_info_needed, balances_needed) =
            self.collect_user_and_balance_updates(user_info, events)?;

        let mut balances: HashMap<Symbol, ZkWitnessSet<UserBalance>> = HashMap::new();
        for (symbol, user_keys) in balances_needed.iter() {
            let users: Vec<UserInfo> = user_keys
                .iter()
                .filter_map(|user_balance| {
                    self.state
                        .get_user_info_from_key(&user_balance.user_key)
                        .ok()
                })
                .collect();

            let witness = self.create_balances_witness(symbol, &users)?;
            balances.insert(symbol.clone(), witness);
        }

        let empty_users: Vec<UserInfo> = Vec::new();
        for symbol in self.state.balances.keys() {
            if !balances.contains_key(symbol) {
                let witness = self.create_balances_witness(symbol, &empty_users)?;
                balances.insert(symbol.clone(), witness);
            }
        }

        let users_info = self.create_users_info_witness(&users_info_needed)?;
        // We collect order updates...
        // NB: We MUST include created order with quantity set to 0. This will prove their non-existence in the SMT
        // This is handled by retaining zeroed orders for proof generation.
        let (
            orders_initial_state,
            bid_order_price_levels_initial_state,
            ask_order_price_levels_initial_state,
        ) = self.collect_orders_updates(events, OrderCollectionMode::ForInitialStateWitness)?;

        // ... and compute their witnesses
        let order_manager = self
            .order_manager_mt
            .create_orders_witnesses(
                orders_initial_state,
                bid_order_price_levels_initial_state,
                ask_order_price_levels_initial_state,
                orders_owner,
            )
            .map_err(|e| format!("Failed to build order manager witness: {e}"))?;

        Ok((users_info, balances, order_manager))
    }

    pub fn derive_zkvm_commitment_metadata_from_events(
        &self,
        user_info: &UserInfo,
        events: &[OrderbookEvent],
        action: &PermissionnedOrderbookAction,
    ) -> Result<Vec<u8>, String> {
        let (users_info, balances, order_manager) = self.for_zkvm(user_info, events, action)?;

        let zkvm_state = ZkVmState {
            users_info,
            balances,
            order_manager,
            lane_id: self.lane_id.clone(),
            hashed_secret: self.hashed_secret,
            last_block_number: self.last_block_number,
            assets: self.state.assets_info.clone().into_iter().collect(),
        };

        borsh::to_vec(&zkvm_state)
            .map_err(|e| format!("Failed to serialize ZkVm orderbook metadata: {e}"))
    }

    pub fn apply_events_and_update_roots(
        &mut self,
        user_info: &UserInfo,
        events: Vec<OrderbookEvent>,
    ) -> Result<(), String> {
        self.state
            .apply_events_preserving_zeroed_orders(user_info, &events)
            .map_err(|e| format!("Could not apply events to state: {e}"))?;

        let (users_to_update, mut balances_to_update) =
            self.collect_user_and_balance_updates(user_info, &events)?;

        // Update users_info SMT
        self.users_info_mt
            .update_all(users_to_update.into_iter())
            .map_err(|_| "Updating users info mt".to_string())?;

        // Update balances SMTs
        for (symbol, user_keys) in balances_to_update.drain() {
            self.balances_mt
                .entry(symbol.clone())
                .or_insert_with(SMT::zero)
                .update_all(user_keys.into_iter())
                .map_err(|_| "Updating balances info mt".to_string())?;
        }

        // Ensure every tracked asset has a corresponding balances SMT even if
        // the action did not emit explicit balance updates (e.g. pair creation).
        for symbol in self.state.balances.keys() {
            self.balances_mt
                .entry(symbol.clone())
                .or_insert_with(SMT::zero);
        }

        let (orders_to_update, bid_order_price_levels, ask_order_price_levels) =
            self.collect_orders_updates(&events, OrderCollectionMode::ForExecuting)?;

        // Update order manager SMT
        self.order_manager_mt
            .orders
            .update_all(orders_to_update.into_iter())
            .map_err(|_| "Updating order manager mt".to_string())?;
        self.order_manager_mt
            .bid_orders
            .update_all(bid_order_price_levels.into_iter())
            .map_err(|_| "Updating bid orders mt".to_string())?;
        self.order_manager_mt
            .ask_orders
            .update_all(ask_order_price_levels.into_iter())
            .map_err(|_| "Updating ask orders mt".to_string())?;

        // Clean orderbook_manager from used orders
        self.state.order_manager.clean(&events);

        Ok(())
    }
}
