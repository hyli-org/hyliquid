use borsh::{io::Error, BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::{BorshableMerkleProof, SHA256Hasher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparse_merkle_tree::default_store::DefaultStore;
use sparse_merkle_tree::traits::Value;
use sparse_merkle_tree::{SparseMerkleTree, H256};
use std::collections::{BTreeMap, BTreeSet};

use crate::order_manager::OrderManager;
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

    // Order manager handling all orders
    pub order_manager: OrderManager,

    /// These fields are not committed on-chain
    #[borsh(skip)]
    // TODO: use a new enum ExecutionType: full-mt-server, light-server, zkvm
    pub server_execution: bool,
    // User balances per token: token -> smt(hash(user) -> user_account))
    #[borsh(skip)]
    pub balances_mt:
        BTreeMap<TokenName, SparseMerkleTree<SHA256Hasher, Balance, DefaultStore<Balance>>>,
    #[borsh(skip)]
    // Users info merkle tree
    pub users_info_mt: SparseMerkleTree<SHA256Hasher, UserInfo, DefaultStore<UserInfo>>,
    #[borsh(skip)]
    // Users info salts. user -> salt
    pub users_info_salt: BTreeMap<String, Vec<u8>>,
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
#[derive(Debug, Serialize, Deserialize, Clone, BorshSerialize, BorshDeserialize, PartialEq)]
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

/// Context struct for creating an order, containing all necessary proofs and mappings.
#[derive(Debug, Clone)]
pub struct CreateOrderCtx {
    pub order_user_map: BTreeMap<OrderId, UserInfo>,
    pub users_info: BTreeSet<UserInfo>,
    pub users_info_proof: BorshableMerkleProof,
    pub user_info: UserInfo,
    pub user_info_proof: BorshableMerkleProof,
    pub balances: BTreeMap<TokenName, BTreeMap<UserInfo, Balance>>,
    pub balances_proof: BTreeMap<TokenName, BorshableMerkleProof>,
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
        executed_quantity: u64,
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
        for (token_name, tree) in &self.balances_mt {
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
            order_manager: self.order_manager.clone(),
            server_execution: self.server_execution,
            balances_mt: balances,
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
        for token in &[&pair.0, &pair.1] {
            if !self.balances_merkle_roots.contains_key(*token) {
                if self.server_execution {
                    self.balances_mt.insert(
                        (*token).clone(),
                        SparseMerkleTree::new(H256::zero(), Default::default()),
                    );
                }
                self.balances_merkle_roots
                    .insert((*token).clone(), H256::zero().into());
            }
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
            .order_manager
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

        // Cancel order through order manager
        let mut cancel_events = self.order_manager.cancel_order(&order_id)?;

        if self.server_execution {
            let user_balance = self.get_balance(user_info, &required_token);

            cancel_events.push(OrderbookEvent::BalanceUpdated {
                user: user_info.user.clone(),
                token: required_token.to_string(),
                amount: user_balance.0,
            });
        }
        Ok(cancel_events)
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
        order: Order,
        order_user_map: BTreeMap<OrderId, UserInfo>,
        balances: &BTreeMap<TokenName, BTreeMap<UserInfo, Balance>>,
        balances_proof: &BTreeMap<TokenName, BorshableMerkleProof>,
    ) -> Result<Vec<OrderbookEvent>, String> {
        if self.order_manager.orders.contains_key(&order.order_id) {
            return Err(format!("Order with id {} already exists", order.order_id));
        }

        let mut events = Vec::new();

        // Use OrderManager to handle order logic
        let base_scale = POW10[self
            .pairs_info
            .get(&order.pair)
            .ok_or(format!("Pair {}/{} not found", order.pair.0, order.pair.1))?
            .base_scale as usize];

        // Delegate order execution to the manager
        let order_events = self.order_manager.execute_order(user_info, &order)?;
        events.extend(order_events);

        // Balance change aggregation system based on events
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

        // Process events to calculate balance changes
        for event in &events {
            match event {
                OrderbookEvent::OrderCreated {
                    order: created_order,
                } => {
                    // Deduct liquidity for created order
                    let (quantity, token) = match created_order.order_side {
                        OrderSide::Bid => (
                            -((created_order.quantity * created_order.price.unwrap()) as i128),
                            created_order.pair.1.clone(),
                        ),
                        OrderSide::Ask => {
                            (created_order.quantity as i128, created_order.pair.0.clone())
                        }
                    };

                    record_balance_change(
                        &mut balance_changes,
                        user_info.clone(),
                        &token,
                        quantity,
                    )?;
                }
                OrderbookEvent::OrderExecuted {
                    order_id,
                    taker_order_id,
                    pair,
                } => {
                    let base_token = &pair.0;
                    let quote_token = &pair.1;

                    // Special case: the current order has been fully executed.
                    if taker_order_id == &order.order_id {
                        // We don't process it as it would be counted twice with other matching executed orders
                        continue;
                    };

                    let executed_order_user_info =
                        order_user_map.get(taker_order_id).ok_or_else(|| {
                            format!(
                                "Executed order owner info (order_id: {taker_order_id}) not provided",
                            )
                        })?;

                    // Transfer token logic for executed orders
                    if let Some(executed_order) = self.order_manager.orders.get(order_id) {
                        match executed_order.order_side {
                            OrderSide::Bid => {
                                // Executed order owner receives base token deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    user_info.clone(),
                                    executed_order_user_info.clone(),
                                    base_token,
                                    executed_order.quantity as i128,
                                )?;
                                // User receives quote token
                                record_balance_change(
                                    &mut balance_changes,
                                    user_info.clone(),
                                    quote_token,
                                    (executed_order.price.unwrap() * executed_order.quantity
                                        / base_scale) as i128,
                                )?;
                            }
                            OrderSide::Ask => {
                                // Executed order owner receives quote token deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    user_info.clone(),
                                    executed_order_user_info.clone(),
                                    quote_token,
                                    (executed_order.price.unwrap() * executed_order.quantity
                                        / base_scale) as i128,
                                )?;
                                // User receives base token
                                record_balance_change(
                                    &mut balance_changes,
                                    user_info.clone(),
                                    base_token,
                                    executed_order.quantity as i128,
                                )?;
                            }
                        }
                    } else {
                        return Err(format!("Could not find {order_id}"));
                    }
                }
                OrderbookEvent::OrderUpdate {
                    order_id,
                    pair,
                    executed_quantity,
                    ..
                } => {
                    let executed_order_user_info =
                        order_user_map.get(order_id).ok_or_else(|| {
                            format!("Updated order owner info (order_id: {order_id}) not provided",)
                        })?;

                    let base_token = &pair.0;
                    let quote_token = &pair.1;

                    // Transfer token logic for executed orders
                    if let Some(updated_order) = self.order_manager.orders.get(order_id) {
                        match updated_order.order_side {
                            OrderSide::Bid => {
                                // Executed order owner receives base token deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    user_info.clone(),
                                    executed_order_user_info.clone(),
                                    base_token,
                                    *executed_quantity as i128,
                                )?;
                                // User receives quote token
                                record_balance_change(
                                    &mut balance_changes,
                                    user_info.clone(),
                                    quote_token,
                                    (updated_order.price.unwrap() * executed_quantity / base_scale)
                                        as i128,
                                )?;
                            }
                            OrderSide::Ask => {
                                // Executed order owner receives quote token deducted to user
                                record_transfer(
                                    &mut balance_changes,
                                    user_info.clone(),
                                    executed_order_user_info.clone(),
                                    quote_token,
                                    (updated_order.price.unwrap() * executed_quantity / base_scale)
                                        as i128,
                                )?;
                                // User receives base token
                                record_balance_change(
                                    &mut balance_changes,
                                    user_info.clone(),
                                    base_token,
                                    *executed_quantity as i128,
                                )?;
                            }
                        }
                    } else {
                        return Err(format!("Could not find {order_id}"));
                    }
                }
                _ => {} // Ignore other events for balance changes
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
            balances_mt: BTreeMap::new(),
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
                .balances_mt
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
        self.order_manager.orders.clone()
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
        let user_map = self.order_manager.get_order_user_map(order_side, pair);

        for (order_id, username) in user_map {
            let user_info = self.get_user_info(&username)?;
            map.insert(order_id, user_info);
        }

        Ok(map)
    }

    pub fn get_balance(&self, user: &UserInfo, token: &str) -> Balance {
        self.balances_mt
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
        for (token, balances_mt) in self.balances_mt.iter() {
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
        for (token, balances_mt) in self.balances_mt.iter() {
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
        let tree = self.balances_mt.get(token).unwrap();
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

        let tree = self.balances_mt.get(token).unwrap();
        let proof =
            BorshableMerkleProof(tree.merkle_proof(vec![user_info.get_key()]).map_err(|e| {
                format!(
                    "Failed to create merkle proof for token {token} and user {:?}: {e}",
                    user_info.user
                )
            })?);
        Ok((balance, proof))
    }

    pub fn get_create_order_ctx(
        &self,
        user: &str,
        order: &Order,
    ) -> Result<CreateOrderCtx, String> {
        let order_user_map = self.get_order_user_map(&order.order_side, &order.pair)?;

        let user_info = self.get_user_info(user)?;
        let user_info_proof = self.get_user_info_proofs(&user_info.clone())?;

        let mut users_info: BTreeSet<_> = order_user_map.values().cloned().collect();
        users_info.insert(user_info.clone());

        let users_info_proof = self.get_users_info_proofs(&users_info)?;

        // Determine which token to fetch balances for, based on order_side
        let [token_all_users, token_only_user] = match &order.order_side {
            OrderSide::Bid => [&order.pair.1, &order.pair.0], // For buy, interested in base token
            OrderSide::Ask => [&order.pair.0, &order.pair.1], // For sell, interested in quote token
        };

        // impacted users
        let users_info_vec: Vec<_> = users_info.iter().cloned().collect();
        let (balances_all_users, balances_proof_all_users) =
            self.get_balances_with_proof(&users_info_vec, token_all_users)?;

        // user
        let (balances_only_user, balances_proof_only_user) =
            self.get_balances_with_proof(&[user_info.clone()], token_only_user)?;

        let balances = BTreeMap::from([
            (token_all_users.clone(), balances_all_users),
            (token_only_user.clone(), balances_only_user),
        ]);
        let balances_proof = BTreeMap::from([
            (token_all_users.clone(), balances_proof_all_users),
            (token_only_user.clone(), balances_proof_only_user),
        ]);

        Ok(CreateOrderCtx {
            order_user_map,
            users_info,
            users_info_proof,
            user_info,
            user_info_proof,
            balances,
            balances_proof,
        })
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
