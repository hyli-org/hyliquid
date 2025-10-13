use sdk::merkle_utils::BorshableMerkleProof;
use sparse_merkle_tree::MerkleProof;
use std::collections::{HashMap, HashSet};

use crate::{
    model::{Balance, Order, OrderbookEvent, Symbol, UserInfo},
    order_manager::OrderManager,
    transaction::PermissionnedOrderbookAction,
    zk::{smt::BorshableH256 as H256, FullState, OnChainState, ZkVmState, ZkVmWitness},
};

/// impl of functions for zkvm state generation and verification
impl FullState {
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
    ) -> Result<ZkVmWitness<HashMap<H256, Balance>>, String> {
        let (balances, proof) = self.get_balances_with_proof(users, symbol)?;
        let mut map = HashMap::new();
        for (user_info, balance) in balances {
            map.insert(user_info.get_key(), balance);
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
    ) -> Result<
        (
            ZkVmWitness<HashSet<UserInfo>>,
            HashMap<Symbol, ZkVmWitness<HashMap<H256, Balance>>>,
            OrderManager,
        ),
        String,
    > {
        // Atm, we copy everything (will be merklized in a future version)
        let mut zkvm_order_manager = self.state.order_manager.clone();

        // We clear orders_owner and re-populate it based on events with only needed values
        zkvm_order_manager.orders_owner.clear();

        let mut users_info_needed: HashSet<UserInfo> = HashSet::new();
        let mut balances_needed: HashMap<String, HashMap<H256, Balance>> = HashMap::new();

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
                OrderbookEvent::BalanceUpdated {
                    user,
                    symbol,
                    amount,
                } => {
                    // Get user_info (if available)
                    let ui = match self.state.get_user_info(user) {
                        Ok(ui) => ui,
                        Err(_) => {
                            if user_info.user == *user {
                                user_info.clone()
                            } else {
                                return Err(format!("User info not found for user '{user}'"));
                            }
                        }
                    };
                    users_info_needed.insert(ui.clone());

                    balances_needed
                        .entry(symbol.clone())
                        .or_default()
                        .insert(ui.get_key(), Balance(*amount));
                }
                OrderbookEvent::SessionKeyAdded { user, .. } => {
                    // Get user_info (if available)
                    let ui = match self.state.get_user_info(user) {
                        Ok(ui) => ui,
                        Err(_) => {
                            if user_info.user == *user {
                                user_info.clone()
                            } else {
                                return Err(format!("User info not found for user '{user}'"));
                            }
                        }
                    };
                    users_info_needed.insert(ui);
                }
                _ => {}
            }
        }

        let mut balances: HashMap<Symbol, ZkVmWitness<HashMap<H256, Balance>>> = HashMap::new();
        for (symbol, symbol_balances) in balances_needed.iter() {
            let users: Vec<UserInfo> = symbol_balances
                .keys()
                .filter_map(|key| self.state.get_user_info_from_key(key).ok())
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

        let onchain_state = OnChainState {
            users_info_root: self.users_info_mt.root(),
            balances_roots: self
                .balances_mt
                .iter()
                .map(|(symbol, tree)| (symbol.clone(), tree.root()))
                .collect(),
            assets: self.state.assets_info.clone(),
            hashed_secret: self.hashed_secret.clone(),
            orders: order_manager,
            lane_id: self.lane_id.clone(),
        };

        let zkvm_state = ZkVmState {
            onchain_state,
            users_info,
            balances,
        };

        borsh::to_vec(&zkvm_state)
            .map_err(|e| format!("Failed to serialize ZkVm orderbook metadata: {e}"))
    }
}
