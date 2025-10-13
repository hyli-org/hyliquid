use std::collections::BTreeMap;

use sdk::{merkle_utils::SHA256Hasher, ContractName, RunResult};
use sha2::Sha256;
use sha3::Digest;
use sparse_merkle_tree::{traits::Value, MerkleProof};

use crate::{
    model::ExecuteState,
    transaction::{
        EscapePrivateInput, OrderbookAction, PermissionlessOrderbookAction,
        PermissionnedOrderbookAction, PermissionnedPrivateInput,
    },
    zk::{smt::BorshableH256 as H256, OnChainState, ZkVmState},
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

        // Verify that balances are correct
        self.verify_balances_proof()
            .unwrap_or_else(|e| panic!("Failed to verify balances proof: {e}"));

        // Verify that users info proof are correct
        self.verify_users_info_proof()
            .unwrap_or_else(|e| panic!("Failed to verify users info proof: {e}"));

        let (mut state, onchain_state) = self.derive_orderbook_state();

        // Verify that orderbook_manager.order_owners is populated with valid users info
        state
            .verify_orders_owners(&action)
            .unwrap_or_else(|e| panic!("Failed to verify orders owners: {e}"));

        let res = match action {
            OrderbookAction::PermissionnedOrderbookAction(action) => {
                if tx_ctx.lane_id != onchain_state.lane_id {
                    return Err("Invalid lane id".to_string());
                }

                let permissionned_private_input: PermissionnedPrivateInput =
                    borsh::from_slice(&calldata.private_input).unwrap_or_else(|e| {
                        panic!("Failed to deserialize PermissionnedPrivateInput: {e}")
                    });

                let hashed_secret = Sha256::digest(&permissionned_private_input.secret);
                if hashed_secret.as_slice() != onchain_state.hashed_secret.0.as_slice() {
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
            OrderbookAction::PermissionlessOrderbookAction(action) => {
                // Execute the given action
                let events = match action {
                    PermissionlessOrderbookAction::Escape { user_key } => {
                        let escape_private_input: EscapePrivateInput =
                            borsh::from_slice(&calldata.private_input).unwrap_or_else(|e| {
                                panic!("Failed to deserialize PermissionnedPrivateInput: {e}")
                            });

                        let user_info = escape_private_input.user_info.clone();
                        let user_info_proof = escape_private_input.user_info_proof.clone();

                        // Assert that used user_info is correct
                        state
                            .has_user_info_key(user_info.get_key())
                            .unwrap_or_else(|e| {
                                panic!("User info provided by server is incorrect: {e}")
                            });

                        if user_key != std::convert::Into::<[u8; 32]>::into(user_info.get_key()) {
                            panic!("User info does not correspond with user_key used")
                        }
                        state.escape(tx_ctx, &user_info, &user_info_proof)?
                    }
                };

                let res = borsh::to_vec(&events)
                    .map_err(|e| format!("Failed to encode OrderbookEvents: {e}"))?;

                res
            }
        };

        Ok((res, ctx, vec![]))
    }

    /// We serialize a curated version of the state on-chain
    fn commit(&self) -> sdk::StateCommitment {
        let mut state_to_commit = self.onchain_state.clone();

        // cleaning sensitive fields before committing
        state_to_commit.orders.orders_owner = BTreeMap::new();

        sdk::StateCommitment(borsh::to_vec(&state_to_commit).expect("Failed to encode Orderbook"))
    }
}

impl ZkVmState {
    pub fn verify_users_info_proof(&self) -> Result<(), String> {
        if self.onchain_state.users_info_root == sparse_merkle_tree::H256::zero().into() {
            return Ok(());
        }

        let leaves = self
            .users_info
            .value
            .iter()
            .map(|user_info| (user_info.get_key().into(), user_info.to_h256()))
            .collect::<Vec<_>>();

        if leaves.is_empty() {
            if self.users_info.proof.0 == MerkleProof::new(vec![], vec![]) {
                return Ok(());
            }
            return Err("No leaves in users_info proof, proof should be empty".to_string());
        }

        let is_valid = self
            .users_info
            .proof
            .0
            .clone()
            .verify::<SHA256Hasher>(
                &TryInto::<[u8; 32]>::try_into(self.onchain_state.users_info_root.as_slice())
                    .map_err(|e| format!("Failed to cast proof root to H256: {e}"))?
                    .into(),
                leaves.clone(),
            )
            .map_err(|e| format!("Failed to verify users_info proof: {e}"))?;

        if !is_valid {
            let derived_root = self
                .users_info
                .proof
                .0
                .clone()
                .compute_root::<SHA256Hasher>(leaves)
                .map_err(|e| format!("Failed to compute users_info proof root: {e}"))?;
            return Err(format!(
                "Invalid users_info proof; root is {} instead of {}, value: {:?}",
                hex::encode(self.onchain_state.users_info_root.as_slice()),
                hex::encode(derived_root.as_slice()),
                self.users_info.value
            ));
        }

        Ok(())
    }

    pub fn verify_balances_proof(&self) -> Result<(), String> {
        for (symbol, witness) in &self.balances {
            // Verify that users balance are correct
            let symbol_root = self
                .onchain_state
                .balances_roots
                .get(symbol.as_str())
                .ok_or(format!("{symbol} not found in balances merkle roots"))?;

            let leaves = witness
                .value
                .iter()
                .map(|(user_info_key, balance)| ((*user_info_key).into(), balance.to_h256()))
                .collect::<Vec<_>>();

            if leaves.is_empty() {
                if witness.proof.0 == MerkleProof::new(vec![], vec![]) {
                    return Ok(());
                }
                return Err("No leaves in users_info proof, proof should be empty".to_string());
            }

            let is_valid = &witness
                .proof
                .0
                .clone()
                .verify::<SHA256Hasher>(
                    &TryInto::<[u8; 32]>::try_into(symbol_root.as_slice())
                        .map_err(|e| format!("Failed to cast proof root to H256: {e}"))?
                        .into(),
                    leaves,
                )
                .map_err(|e| format!("Failed to verify balances proof for {symbol}: {e}"))?;

            if !is_valid {
                return Err(format!("Invalid balances proof for {symbol}"));
            }
        }
        Ok(())
    }

    pub fn derive_orderbook_state(&self) -> (ExecuteState, &OnChainState) {
        (
            ExecuteState {
                assets_info: self.onchain_state.assets.clone(), // Assets info is not part of zkvm state
                users_info: self
                    .users_info
                    .value
                    .clone()
                    .into_iter()
                    .map(|u| (u.user.clone(), u))
                    .collect(),
                balances: self
                    .balances
                    .clone()
                    .into_iter()
                    .map(|(symbol, witness)| (symbol.clone(), witness.value))
                    .collect(),
                order_manager: self.onchain_state.orders.clone(), // OrderManager is not part of zkvm state
            },
            &self.onchain_state,
        )
    }

    pub fn has_user_info_key(&self, user_info_key: H256) -> Result<bool, String> {
        Ok(self
            .users_info
            .value
            .iter()
            .any(|user_info| user_info.get_key() == user_info_key))
    }

    pub fn is_blob_whitelisted(&self, contract_name: &ContractName) -> bool {
        if contract_name.0 == "orderbook" {
            return true;
        }

        self.onchain_state.assets.contains_key(&contract_name.0)
            || self
                .onchain_state
                .assets
                .values()
                .any(|info| &info.contract_name == contract_name)
    }
}
