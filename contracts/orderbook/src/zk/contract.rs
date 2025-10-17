use std::collections::HashMap;

use sdk::{ContractName, RunResult, StateCommitment};
use sha2::Sha256;
use sha3::Digest;
use sparse_merkle_tree::traits::Value;

use crate::{
    model::{Balance, ExecuteState},
    transaction::{
        EscapePrivateInput, OrderbookAction, PermissionlessOrderbookAction,
        PermissionnedOrderbookAction, PermissionnedPrivateInput,
    },
    zk::{
        smt::{BorshableH256 as H256, GetKey, UserBalance},
        ParsedStateCommitment, ZkVmState,
    },
};

impl sdk::FullStateRevert for ZkVmState {}

impl sdk::ZkContract for ZkVmState {
    /// Entry point of the contract's logic
    fn execute(&mut self, calldata: &sdk::Calldata) -> RunResult {
        // Parse contract inputs
        let (action, ctx) = sdk::utils::parse_raw_calldata::<OrderbookAction>(calldata)?;

        let Some(tx_ctx) = &calldata.tx_ctx else {
            panic!("tx_ctx is missing");
        };

        // The contract must be provided with all blobs
        if calldata.blobs.len() != calldata.tx_blob_count {
            panic!("Calldata is not composed with all tx's blobs");
        }

        // Check if blobs in the calldata are all whitelisted
        for (_, blob) in &calldata.blobs {
            if !self.is_blob_whitelisted(&blob.contract_name) {
                return Err(format!(
                    "Blob with contract name {} is not whitelisted",
                    blob.contract_name
                ));
            }
        }

        let mut state = self.into_orderbook_state();

        // Verify that orderbook_manager.order_owners is populated with valid users info
        state
            .verify_orders_owners(&action)
            .unwrap_or_else(|e| panic!("Failed to verify orders owners: {e}"));

        let res = match action {
            OrderbookAction::PermissionnedOrderbookAction(action, _) => {
                if tx_ctx.lane_id != self.lane_id {
                    return Err("Invalid lane id".to_string());
                }

                let permissionned_private_input: PermissionnedPrivateInput =
                    borsh::from_slice(&calldata.private_input).unwrap_or_else(|e| {
                        panic!("Failed to deserialize PermissionnedPrivateInput: {e}")
                    });

                let hashed_secret = Sha256::digest(&permissionned_private_input.secret);
                if hashed_secret.as_slice() != self.hashed_secret.as_slice() {
                    panic!("Invalid secret in private input");
                }

                if let PermissionnedOrderbookAction::Identify = action {
                    // Identify action does not change the state
                    return Ok((vec![], ctx, vec![]));
                }

                let user_info = permissionned_private_input.user_info.clone();

                // Assert that used user_info is correct
                assert!(state
                    .has_user_info_key(user_info.get_key())
                    .unwrap_or_else(|e| panic!("User info provided by server is incorrect: {e}")));

                // Execute the given action
                let events = state.execute_permissionned_action(
                    user_info,
                    action,
                    &permissionned_private_input.private_input,
                )?;

                let res = borsh::to_vec(&events)
                    .map_err(|e| format!("Failed to encode OrderbookEvents: {e}"))?;

                res
            }
            OrderbookAction::PermissionlessOrderbookAction(action, _) => {
                // Execute the given action
                let events = match action {
                    PermissionlessOrderbookAction::Escape { user_key } => {
                        let escape_private_input: EscapePrivateInput =
                            borsh::from_slice(&calldata.private_input).unwrap_or_else(|e| {
                                panic!("Failed to deserialize PermissionnedPrivateInput: {e}")
                            });

                        let user_info = escape_private_input.user_info.clone();

                        // Assert that used user_info is correct
                        state
                            .has_user_info_key(user_info.get_key())
                            .unwrap_or_else(|e| {
                                panic!("User info provided by server is incorrect: {e}")
                            });

                        if user_key != std::convert::Into::<[u8; 32]>::into(user_info.get_key()) {
                            panic!("User info does not correspond with user_key used")
                        }
                        state.escape(&self.last_block_number, calldata, &user_info)?
                    }
                };

                let res = borsh::to_vec(&events)
                    .map_err(|e| format!("Failed to encode OrderbookEvents: {e}"))?;

                res
            }
        };

        self.take_changes_back(&mut state)?;

        Ok((res, ctx, vec![]))
    }

    fn commit(&self) -> StateCommitment {
        StateCommitment(
            borsh::to_vec(&ParsedStateCommitment {
                users_info_root: self
                    .users_info
                    .compute_root()
                    .expect("compute user info root"),
                balances_roots: self
                    .balances
                    .iter()
                    .filter_map(|(symbol, witness)| {
                        let root = witness.compute_root().expect("compute user balance root");
                        if root == H256::zero() {
                            None
                        } else {
                            Some((symbol.clone(), root))
                        }
                    })
                    .collect(),
                assets: &self.assets,
                orders: self.order_manager.view(),
                hashed_secret: self.hashed_secret,
                lane_id: &self.lane_id,
                last_block_number: &self.last_block_number,
            })
            .expect("Could not encode onchain state into state commitment"),
        )
    }
}

impl ZkVmState {
    pub fn into_orderbook_state(&mut self) -> ExecuteState {
        ExecuteState {
            assets_info: std::mem::take(&mut self.assets), // Assets info is not part of zkvm state
            users_info: self
                .users_info
                .values
                .drain()
                .map(|u| (u.user.clone(), u))
                .collect(),
            balances: self
                .balances
                .iter_mut()
                .map(|(symbol, witness)| {
                    (
                        symbol.clone(),
                        witness
                            .values
                            .drain()
                            .map(|ub| (ub.user_key, ub.balance))
                            .collect::<HashMap<H256, Balance>>(),
                    )
                })
                .collect(),
            order_manager: std::mem::take(&mut self.order_manager), // OrderManager is not part of zkvm state
        }
    }

    pub fn has_user_info_key(&self, user_info_key: H256) -> Result<bool, String> {
        Ok(self
            .users_info
            .values
            .iter()
            .any(|user_info| user_info.get_key() == user_info_key))
    }

    pub fn is_blob_whitelisted(&self, contract_name: &ContractName) -> bool {
        if contract_name.0 == "orderbook" {
            return true;
        }

        self.assets.contains_key(&contract_name.0)
            || self
                .assets
                .values()
                .any(|info| &info.contract_name == contract_name)
    }

    pub fn take_changes_back(&mut self, state: &mut ExecuteState) -> Result<(), String> {
        self.users_info
            .values
            .extend(state.users_info.drain().map(|(_name, user)| user));

        for (symbol, witness) in self.balances.iter_mut() {
            if let Some(mut state_balances) = state.balances.remove(symbol) {
                witness
                    .values
                    .extend(state_balances.drain().map(|sb| UserBalance {
                        user_key: sb.0,
                        balance: sb.1,
                    }));
            }
        }

        std::mem::swap(&mut self.assets, &mut state.assets_info);
        std::mem::swap(&mut self.order_manager, &mut state.order_manager);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AssetInfo, Balance, Order, OrderSide, OrderType, UserInfo};
    use crate::order_manager::OrderManager;
    use crate::zk::{ZkWitnessSet, H256, SMT};
    use borsh::{BorshDeserialize, BorshSerialize};
    use sdk::merkle_utils::BorshableMerkleProof;
    use sdk::{BlockHeight, ContractName, LaneId, ZkContract};
    use std::collections::{HashMap, HashSet};
    use std::mem::discriminant;

    use sparse_merkle_tree::traits::Value;

    use super::super::Proof;

    fn sample_user(name: &str, salt_byte: u8, nonce: u32, extra_key: Option<Vec<u8>>) -> UserInfo {
        let mut user = UserInfo::new(name.to_string(), vec![salt_byte; 4]);
        user.nonce = nonce;
        if let Some(key) = extra_key {
            user.session_keys.push(key);
        }
        user
    }

    fn sample_zk_state() -> ZkVmState {
        let alice = sample_user("alice", 0xAA, 7, Some(vec![1, 2, 3, 4]));
        let bob = sample_user("bob", 0xBB, 11, Some(vec![9, 9, 9]));

        let mut users_values: HashSet<UserInfo> = HashSet::new();
        users_values.insert(alice.clone());
        users_values.insert(bob.clone());

        let users_info = ZkWitnessSet {
            values: users_values,
            proof: Proof::CurrentRootHash(H256::default()),
        };

        let alice_key = alice.get_key();
        let bob_key = bob.get_key();

        let mut eth_balances: HashSet<UserBalance> = HashSet::new();
        eth_balances.insert(UserBalance {
            user_key: alice_key,
            balance: Balance(1_000),
        });
        eth_balances.insert(UserBalance {
            user_key: bob_key,
            balance: Balance(2_000),
        });

        let mut usdc_balances: HashSet<UserBalance> = HashSet::new();
        usdc_balances.insert(UserBalance {
            user_key: alice_key,
            balance: Balance(5_000),
        });

        let mut balances: HashMap<String, ZkWitnessSet<UserBalance>> = HashMap::new();
        balances.insert(
            "ETH".to_string(),
            ZkWitnessSet {
                values: eth_balances,
                proof: Proof::CurrentRootHash(H256::default()),
            },
        );
        balances.insert(
            "USDC".to_string(),
            ZkWitnessSet {
                values: usdc_balances,
                proof: Proof::CurrentRootHash(H256::default()),
            },
        );

        let mut assets = HashMap::new();
        assets.insert(
            "ETH".to_string(),
            AssetInfo::new(18, ContractName("eth".to_string())),
        );
        assets.insert(
            "USDC".to_string(),
            AssetInfo::new(6, ContractName("usdc".to_string())),
        );

        let order_id = "order-1".to_string();
        let pair = ("ETH".to_string(), "USDC".to_string());
        let price = 1_500;
        let order = Order {
            order_id: order_id.clone(),
            order_type: OrderType::Limit,
            order_side: OrderSide::Bid,
            price: Some(price),
            pair: pair.clone(),
            quantity: 3,
        };

        let mut order_manager = OrderManager::default();
        order_manager.orders.insert(order_id.clone(), order.clone());
        order_manager
            .buy_orders
            .entry(pair.clone())
            .or_default()
            .entry(price)
            .or_default()
            .push_back(order_id.clone());
        order_manager
            .orders_owner
            .insert(order_id.clone(), alice_key);

        ZkVmState {
            users_info,
            balances,
            lane_id: LaneId::default(),
            hashed_secret: [42; 32],
            last_block_number: BlockHeight::default(),
            order_manager,
            assets,
        }
    }

    fn assert_users_match(
        execution_users: &HashMap<String, UserInfo>,
        expected_users: &ZkWitnessSet<UserInfo>,
    ) {
        assert_eq!(
            execution_users.len(),
            expected_users.values.len(),
            "user map size mismatch"
        );
        for expected in expected_users.values.iter() {
            let actual = execution_users
                .get(&expected.user)
                .unwrap_or_else(|| panic!("missing user {}", expected.user));
            assert_eq!(
                actual, expected,
                "user {} mismatch between witnesses and execution state",
                expected.user
            );
        }
    }

    fn assert_balances_match(
        execution_balances: &HashMap<String, HashMap<H256, Balance>>,
        expected_balances: &HashMap<String, ZkWitnessSet<UserBalance>>,
    ) {
        assert_eq!(
            execution_balances.len(),
            expected_balances.len(),
            "symbol count mismatch"
        );
        for (symbol, witness) in expected_balances {
            let actual = execution_balances
                .get(symbol)
                .unwrap_or_else(|| panic!("missing balances for symbol {symbol}"));
            assert_eq!(
                actual.len(),
                witness.values.len(),
                "balance entry count mismatch for symbol {symbol}"
            );
            for expected_balance in witness.values.iter() {
                let actual_balance = actual
                    .get(&expected_balance.user_key)
                    .unwrap_or_else(|| panic!("missing user balance for symbol {symbol}"));
                assert_eq!(
                    actual_balance, &expected_balance.balance,
                    "balance mismatch for symbol {symbol}"
                );
            }
        }
    }

    fn assert_witness_equal<T>(actual: &ZkWitnessSet<T>, expected: &ZkWitnessSet<T>, label: &str)
    where
        T: BorshDeserialize
            + BorshSerialize
            + Default
            + Value
            + crate::zk::smt::GetKey
            + Ord
            + Clone
            + Eq
            + std::hash::Hash
            + std::fmt::Debug,
    {
        assert_eq!(
            actual.values, expected.values,
            "{} witness values differ",
            label
        );
        assert_eq!(
            discriminant(&actual.proof),
            discriminant(&expected.proof),
            "{} proof discriminant differs",
            label
        );
        if let (Proof::CurrentRootHash(actual_root), Proof::CurrentRootHash(expected_root)) =
            (&actual.proof, &expected.proof)
        {
            assert_eq!(actual_root, expected_root, "{} root hash differs", label);
        }
    }

    #[test]
    fn zkvm_state_roundtrip_is_lossless() {
        let mut zk_state = sample_zk_state();
        let expected_state = zk_state.clone();

        let mut execution_state = zk_state.into_orderbook_state();

        assert_eq!(
            execution_state.assets_info, expected_state.assets,
            "asset info mismatch after into_orderbook_state"
        );
        assert_eq!(
            execution_state.order_manager, expected_state.order_manager,
            "order manager mismatch after into_orderbook_state"
        );
        assert_users_match(&execution_state.users_info, &expected_state.users_info);
        assert_balances_match(&execution_state.balances, &expected_state.balances);

        zk_state
            .take_changes_back(&mut execution_state)
            .expect("take_changes_back should succeed");

        assert_eq!(zk_state.assets, expected_state.assets, "assets mismatch");
        assert_eq!(
            zk_state.order_manager, expected_state.order_manager,
            "order manager mismatch"
        );
        assert_eq!(zk_state.lane_id, expected_state.lane_id, "lane id mismatch");
        assert_eq!(
            zk_state.hashed_secret, expected_state.hashed_secret,
            "hashed secret mismatch"
        );
        assert_eq!(
            zk_state.last_block_number, expected_state.last_block_number,
            "last block number mismatch"
        );
        assert_witness_equal(&zk_state.users_info, &expected_state.users_info, "users");
        assert_eq!(
            zk_state.balances.len(),
            expected_state.balances.len(),
            "balance witness map size mismatch"
        );
        for (symbol, expected_witness) in expected_state.balances.iter() {
            let actual_witness = zk_state
                .balances
                .get(symbol)
                .unwrap_or_else(|| panic!("missing witness for symbol {symbol}"));
            assert_witness_equal(
                actual_witness,
                expected_witness,
                &format!("balances {symbol}"),
            );
        }

        assert!(
            execution_state.users_info.is_empty(),
            "execution users map should be empty after take back"
        );
        assert!(
            execution_state.balances.is_empty(),
            "execution balances should be drained after take back"
        );
        assert!(
            execution_state.assets_info.is_empty(),
            "execution assets should be empty after take back"
        );
        assert!(
            execution_state.order_manager.orders.is_empty()
                && execution_state.order_manager.buy_orders.is_empty()
                && execution_state.order_manager.sell_orders.is_empty()
                && execution_state.order_manager.orders_owner.is_empty(),
            "execution order manager should be empty after take back"
        );
    }

    #[test]
    fn commit_skips_zero_root_balance_witnesses() {
        let users_witness = ZkWitnessSet {
            values: HashSet::new(),
            proof: Proof::CurrentRootHash(H256::default()),
        };

        let zero_balance_witness = ZkWitnessSet {
            values: HashSet::new(),
            proof: Proof::CurrentRootHash(H256::default()),
        };

        let mut non_zero_bytes = [0u8; 32];
        non_zero_bytes[31] = 1;
        let non_zero_root = H256::from(non_zero_bytes);
        let non_zero_witness = ZkWitnessSet {
            values: HashSet::new(),
            proof: Proof::CurrentRootHash(non_zero_root),
        };

        let mut balances = HashMap::new();
        balances.insert("ZERO".to_string(), zero_balance_witness);
        balances.insert("NONZERO".to_string(), non_zero_witness);

        let lane_id = LaneId::default();
        let last_block_number = BlockHeight::default();
        let hashed_secret = [7u8; 32];
        let order_manager = OrderManager::default();
        let assets: HashMap<String, AssetInfo> = HashMap::new();

        let zk_state = ZkVmState {
            users_info: users_witness.clone(),
            balances,
            lane_id: lane_id.clone(),
            hashed_secret,
            last_block_number,
            order_manager: order_manager.clone(),
            assets: assets.clone(),
        };

        let commit = zk_state.commit();

        let mut expected_balances = HashMap::new();
        expected_balances.insert("NONZERO".to_string(), non_zero_root);

        let expected_commitment = StateCommitment(
            borsh::to_vec(&ParsedStateCommitment {
                users_info_root: users_witness.clone().compute_root().expect("users root"),
                balances_roots: expected_balances,
                assets: &assets,
                orders: order_manager.view(),
                hashed_secret,
                lane_id: &lane_id,
                last_block_number: &last_block_number,
            })
            .expect("encode expected commitment"),
        );

        assert_eq!(
            commit.0, expected_commitment.0,
            "commit should drop zero-root balance witnesses"
        );
    }

    #[test]
    fn commit_uses_proof_derived_balance_roots() {
        let alice = sample_user("alice", 0xAB, 3, None);
        let user_balance = UserBalance {
            user_key: alice.get_key(),
            balance: Balance(50),
        };

        let mut balance_tree = SMT::zero();
        balance_tree
            .update_all(std::iter::once(user_balance.clone()))
            .expect("update balance tree");
        let balance_root = balance_tree.root();
        let balance_proof = balance_tree
            .merkle_proof([user_balance.clone()].iter())
            .expect("balance proof");

        let balance_witness = ZkWitnessSet {
            values: HashSet::from([user_balance.clone()]),
            proof: Proof::Some(BorshableMerkleProof(balance_proof)),
        };

        let users_witness = ZkWitnessSet {
            values: HashSet::from([alice]),
            proof: Proof::CurrentRootHash(H256::default()),
        };

        let mut balances = HashMap::new();
        balances.insert("TOKEN".to_string(), balance_witness.clone());

        let lane_id = LaneId::default();
        let last_block_number = BlockHeight::default();
        let hashed_secret = [11u8; 32];
        let order_manager = OrderManager::default();
        let assets: HashMap<String, AssetInfo> = HashMap::new();

        let zk_state = ZkVmState {
            users_info: users_witness.clone(),
            balances,
            lane_id: lane_id.clone(),
            hashed_secret,
            last_block_number,
            order_manager: order_manager.clone(),
            assets: assets.clone(),
        };

        let commit = zk_state.commit();

        let expected_commitment = StateCommitment(
            borsh::to_vec(&ParsedStateCommitment {
                users_info_root: users_witness.compute_root().expect("users root"),
                balances_roots: HashMap::from([("TOKEN".to_string(), balance_root)]),
                assets: &assets,
                orders: order_manager.view(),
                hashed_secret,
                lane_id: &lane_id,
                last_block_number: &last_block_number,
            })
            .expect("encode expected commitment"),
        );

        assert_eq!(
            commit.0, expected_commitment.0,
            "commit should honor roots derived from balance proofs"
        );
    }
}
