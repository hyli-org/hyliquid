use sdk::merkle_utils::BorshableMerkleProof;
use sparse_merkle_tree::MerkleProof;
use std::collections::{HashMap, HashSet};

use crate::{
    model::{Balance, Order, OrderbookEvent, Symbol, UserInfo},
    order_manager::OrderManager,
    transaction::PermissionnedOrderbookAction,
    zk::{
        smt::{GetKey, UserBalance},
        FullState, ZkVmState, ZkVmWitness, SMT,
    },
};

type UsersAndBalancesNeeded = (HashSet<UserInfo>, HashMap<Symbol, Vec<UserBalance>>);

type ZkvmComputedInputs = (
    ZkVmWitness<HashSet<UserInfo>>,
    HashMap<Symbol, ZkVmWitness<HashSet<UserBalance>>>,
    OrderManager,
);
/// impl of functions for zkvm state generation and verification
impl FullState {
    fn resolve_user_from_state(&self, fallback: &UserInfo, user: &str) -> Result<UserInfo, String> {
        match self.state.get_user_info(user) {
            Ok(ui) => Ok(ui),
            Err(_) if fallback.user == user => Ok(fallback.clone()),
            Err(e) => Err(e),
        }
    }

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
                    balances_needed
                        .entry(symbol.clone())
                        .or_default()
                        .push(UserBalance {
                            user_key: ui.get_key(),
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

    fn create_users_info_witness(
        &self,
        users: &HashSet<UserInfo>,
    ) -> Result<ZkVmWitness<HashSet<UserInfo>>, String> {
        let proof = self.get_users_info_proofs(users)?;
        let mut set = HashSet::new();
        for user in users {
            set.insert(user.clone());
        }
        Ok(ZkVmWitness { value: set, proof })
    }

    fn create_balances_witness(
        &self,
        symbol: &Symbol,
        users: &[UserInfo],
    ) -> Result<ZkVmWitness<HashSet<UserBalance>>, String> {
        let (balances, proof) = self.get_balances_with_proof(users, symbol)?;
        let mut map = HashSet::new();
        for (user_info, balance) in balances {
            map.insert(UserBalance {
                user_key: user_info.get_key(),
                balance,
            });
        }
        Ok(ZkVmWitness { value: map, proof })
    }

    fn get_users_info_proofs(
        &self,
        users_info: &HashSet<UserInfo>,
    ) -> Result<BorshableMerkleProof, String> {
        if users_info.is_empty() {
            return Ok(BorshableMerkleProof(MerkleProof::new(vec![], vec![])));
        }

        Ok(BorshableMerkleProof(
            self.users_info_mt
                .merkle_proof(
                    users_info
                        .iter()
                        .map(|u| u.get_key().into())
                        .collect::<Vec<_>>(),
                )
                .map_err(|e| {
                    format!("Failed to create merkle proof for users {users_info:?}: {e}")
                })?,
        ))
    }

    fn get_balances_with_proof(
        &self,
        users_info: &[UserInfo],
        symbol: &Symbol,
    ) -> Result<(HashMap<UserInfo, Balance>, BorshableMerkleProof), String> {
        if users_info.is_empty() {
            return Ok((
                HashMap::new(),
                BorshableMerkleProof(MerkleProof::new(vec![], vec![])),
            ));
        }
        let mut balances_map = HashMap::new();
        for user_info in users_info {
            let balance = self.state.get_balance(user_info, symbol);
            balances_map.insert(user_info.clone(), balance);
        }

        let users: Vec<UserInfo> = balances_map.keys().cloned().collect();
        let tree = self
            .balances_mt
            .get(symbol)
            .ok_or_else(|| format!("No balances tree found for {symbol}"))?;
        let proof = BorshableMerkleProof(
            tree.merkle_proof(users.iter().map(|u| u.get_key().into()).collect::<Vec<_>>())
                .map_err(|e| {
                    format!(
                        "Failed to create merkle proof for {symbol} and users {:?}: {e}",
                        users_info
                            .iter()
                            .map(|u| u.user.clone())
                            .collect::<Vec<_>>()
                    )
                })?,
        );

        Ok((balances_map, proof))
    }

    fn for_zkvm(
        &self,
        user_info: &UserInfo,
        events: &[OrderbookEvent],
        action: &PermissionnedOrderbookAction,
    ) -> Result<ZkvmComputedInputs, String> {
        // Atm, we copy everything (will be merklized in a future version)
        let mut zkvm_order_manager = self.state.order_manager.clone();

        // We clear orders_owner and re-populate it based on events with only needed values
        zkvm_order_manager.orders_owner.clear();

        let (users_info_needed, balances_needed) =
            self.collect_user_and_balance_updates(user_info, events)?;

        // Track all users, their balances per symbol, and order-user mapping for executed/updated orders
        for event in events {
            match event {
                OrderbookEvent::OrderExecuted { order_id, .. }
                | OrderbookEvent::OrderUpdate { order_id, .. }
                | OrderbookEvent::OrderCancelled { order_id, .. } => {
                    if let Some(order_owner) = self.state.order_manager.orders_owner.get(order_id) {
                        zkvm_order_manager
                            .orders_owner
                            .insert(order_id.clone(), *order_owner);
                    } else if let PermissionnedOrderbookAction::CreateOrder(Order {
                        order_id: create_order_id,
                        ..
                    }) = action
                    {
                        if create_order_id == order_id {
                            // Special case: the order was created in the same tx, we can use the user_info
                            zkvm_order_manager
                                .orders_owner
                                .insert(order_id.clone(), user_info.get_key());
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

        let mut balances: HashMap<Symbol, ZkVmWitness<HashSet<UserBalance>>> = HashMap::new();
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

        let users_info = self.create_users_info_witness(&users_info_needed)?;

        Ok((users_info, balances, zkvm_order_manager))
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
            assets: self.state.assets_info.clone(),
        };

        borsh::to_vec(&zkvm_state)
            .map_err(|e| format!("Failed to serialize ZkVm orderbook metadata: {e}"))
    }

    pub fn execute_and_update_roots(
        &mut self,
        user_info: &UserInfo,
        action: &PermissionnedOrderbookAction,
        private_input: &[u8],
    ) -> Result<Vec<OrderbookEvent>, String> {
        let events = self.state.execute_permissionned_action(
            user_info.clone(),
            action.clone(),
            private_input,
        )?;

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

        Ok(events)
    }
}
