use std::collections::HashMap;

use sdk::{ContractName, RunResult, StateCommitment};
use sha2::Sha256;
use sha3::Digest;

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
            OrderbookAction::PermissionnedOrderbookAction(action) => {
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
            OrderbookAction::PermissionlessOrderbookAction(action) => {
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
                balances_roots: &self
                    .balances
                    .iter()
                    .map(|(symbol, user_balance)| {
                        (
                            symbol.clone(),
                            user_balance
                                .compute_root()
                                .expect("compute user balance root"),
                        )
                    })
                    .collect(),
                assets: &self.assets,
                orders: &self.order_manager,
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
                .value
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
                            .value
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
            .value
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
            .value
            .extend(state.users_info.drain().map(|(_name, user)| user));

        for (symbol, witness) in self.balances.iter_mut() {
            if let Some(mut state_balances) = state.balances.remove(symbol) {
                witness
                    .value
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
