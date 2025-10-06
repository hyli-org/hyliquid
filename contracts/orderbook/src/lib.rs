use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use sdk::{merkle_utils::BorshableMerkleProof, RunResult};
use sha2::{Digest, Sha256};

use crate::{
    orderbook::{ExecutionState, Order, OrderType, Orderbook, OrderbookEvent, PairInfo, TokenPair},
    smt_values::UserInfo,
};

pub mod order_manager;
pub mod orderbook;
pub mod orderbook_state;
pub mod orderbook_witness;
pub mod smt_values;
pub mod utils;

impl sdk::FullStateRevert for Orderbook {}

impl sdk::ZkContract for Orderbook {
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

        // Verify that orderbook_manager.order_owners is populated with valid users info
        self.verify_orders_owners(&action)
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

                let user_info = permissionned_private_input.user_info.clone();

                // Assert that used user_info is correct
                self.has_user_info_key(user_info.get_key())
                    .unwrap_or_else(|e| panic!("User info provided by server is incorrect: {e}"));

                let hashed_secret = Sha256::digest(&permissionned_private_input.secret)
                    .as_slice()
                    .to_vec();
                if hashed_secret != self.hashed_secret {
                    panic!("Invalid secret in private input");
                }

                // Execute the given action
                let events = self.execute_permissionned_action(
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
                        self.has_user_info_key(user_info.get_key())
                            .unwrap_or_else(|e| {
                                panic!("User info provided by server is incorrect: {e}")
                            });

                        if user_key != std::convert::Into::<[u8; 32]>::into(user_info.get_key()) {
                            panic!("User info does not correspond with user_key used")
                        }
                        self.escape(tx_ctx, &user_info, &user_info_proof)?
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
        let mut state_to_commit = Orderbook {
            hashed_secret: self.hashed_secret,
            pairs_info: self.pairs_info.clone(),
            lane_id: self.lane_id.clone(),
            balances_merkle_roots: self.balances_merkle_roots.clone(),
            users_info_merkle_root: self.users_info_merkle_root,
            order_manager: self.order_manager.clone(),
            execution_state: ExecutionState::Light(Default::default()), // Committed state contains nothing
        };

        // cleaning sensitive fields before committing
        state_to_commit.order_manager.orders_owner = BTreeMap::new();

        sdk::StateCommitment(borsh::to_vec(&state_to_commit).expect("Failed to encode Orderbook"))
    }
}

impl Orderbook {
    pub fn execute_permissionned_action(
        &mut self,
        user_info: UserInfo,
        action: PermissionnedOrderbookAction,
        private_input: &[u8],
    ) -> Result<Vec<OrderbookEvent>, String> {
        match action {
            PermissionnedOrderbookAction::CreatePair { pair, info } => {
                self.create_pair(&pair, &info)
            }
            PermissionnedOrderbookAction::AddSessionKey => {
                // On this step, the public key is provided in private_input and hence is never public.
                // The orderbook server knows the public key as user informed it offchain.
                let add_session_key_private_input =
                    borsh::from_slice::<AddSessionKeyPrivateInput>(private_input).map_err(|e| {
                        format!("Failed to deserialize CreateOrderPrivateInput: {e}")
                    })?;

                self.add_session_key(user_info, &add_session_key_private_input.new_public_key)
            }
            PermissionnedOrderbookAction::Deposit { token, amount } => {
                self.deposit(&token, amount, &user_info)
            }
            PermissionnedOrderbookAction::CreateOrder(Order {
                order_id,
                order_side,
                order_type,
                price,
                pair,
                quantity,
            }) => {
                // Assert that the order is correctly created
                if order_type == OrderType::Limit && price.is_none() {
                    return Err("Limit orders must have a price".to_string());
                }
                if order_type == OrderType::Market && price.is_some() {
                    return Err("Market orders cannot have a price".to_string());
                }

                let create_order_private_input =
                    borsh::from_slice::<CreateOrderPrivateInput>(private_input).map_err(|e| {
                        format!("Failed to deserialize CreateOrderPrivateInput: {e}")
                    })?;

                // Verify user signature authorization
                // On this step, signature is provided in private_input and hence is never public.
                // The orderbook server knows the signature as user informed it offchain.
                // As the public key has been registered, only the user can create that signature and hence allow this order creation
                utils::verify_user_signature_authorization(
                    &user_info,
                    &create_order_private_input.public_key,
                    &format!(
                        "{}:{}:create_order:{order_id}",
                        user_info.user, user_info.nonce
                    ),
                    &create_order_private_input.signature,
                )
                .map_err(|err| format!("Failed to verify user signature authorization: {err}"))?;

                let order = Order {
                    order_id,
                    order_type,
                    order_side,
                    price,
                    pair,
                    quantity,
                };

                self.execute_order(&user_info, order)
            }
            PermissionnedOrderbookAction::Cancel { order_id } => {
                let cancel_order_private_data =
                    borsh::from_slice::<CancelOrderPrivateInput>(private_input).map_err(|e| {
                        format!("Failed to deserialize CancelOrderPrivateInput: {e}")
                    })?;
                // Verify user signature authorization
                utils::verify_user_signature_authorization(
                    &user_info,
                    &cancel_order_private_data.public_key,
                    &format!("{}:{}:cancel:{order_id}", user_info.user, user_info.nonce),
                    &cancel_order_private_data.signature,
                )
                .map_err(|err| format!("Failed to verify user signature authorization: {err}"))?;

                self.cancel_order(order_id, &user_info)
            }
            PermissionnedOrderbookAction::Withdraw { token, amount } => {
                // TODO: assert there is a transfer blob for that token

                let withdraw_private_data =
                    borsh::from_slice::<WithdrawPrivateInput>(private_input)
                        .map_err(|e| format!("Failed to deserialize WithdrawPrivateInput: {e}"))?;

                // Verify user signature authorization
                utils::verify_user_signature_authorization(
                    &user_info,
                    &withdraw_private_data.public_key,
                    &format!(
                        "{}:{}:withdraw:{token}:{amount}",
                        user_info.user, user_info.nonce
                    ),
                    &withdraw_private_data.signature,
                )
                .map_err(|err| format!("Failed to verify user signature authorization: {err}"))?;

                self.withdraw(&token, &amount, &user_info)
            }
        }
    }
}

/// Structure to deserialize permissionned private data
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct PermissionnedPrivateInput {
    pub secret: Vec<u8>,

    // Used to assert and increment user's nonce
    pub user_info: UserInfo,

    // Used to execute the specific action for the user
    pub private_input: Vec<u8>,
}

/// Structure to deserialize private data during order creation
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct AddSessionKeyPrivateInput {
    pub new_public_key: Vec<u8>,
}

/// Structure to deserialize private data during order creation
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct CreateOrderPrivateInput {
    // Used to assert user approval of that action
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
}

/// Structure to deserialize private data during order cancellation
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct CancelOrderPrivateInput {
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
}

/// Structure to deserialize private data during withdraw
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct WithdrawPrivateInput {
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
}

/// Structure to deserialize private data during escape
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct EscapePrivateInput {
    // Used to assert and increment user's nonce
    pub user_info: UserInfo,
    pub user_info_proof: BorshableMerkleProof,
}

/// Enum representing possible calls to the contract functions.
#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum OrderbookAction {
    PermissionnedOrderbookAction(PermissionnedOrderbookAction),
    PermissionlessOrderbookAction(PermissionlessOrderbookAction),
}

#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum PermissionnedOrderbookAction {
    AddSessionKey,
    CreatePair { pair: TokenPair, info: PairInfo },
    Deposit { token: String, amount: u64 },
    CreateOrder(Order),
    Cancel { order_id: String },
    Withdraw { token: String, amount: u64 },
}

#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum PermissionlessOrderbookAction {
    Escape { user_key: [u8; 32] },
}

impl OrderbookAction {
    pub fn as_blob(&self, contract_name: sdk::ContractName) -> sdk::Blob {
        sdk::Blob {
            contract_name,
            data: sdk::BlobData(borsh::to_vec(self).expect("Failed to encode OrderbookAction")),
        }
    }
}

impl From<sdk::StateCommitment> for Orderbook {
    fn from(state: sdk::StateCommitment) -> Self {
        borsh::from_slice(&state.0)
            .map_err(|e| format!("Could not decode Orderbook state: {e}"))
            .unwrap()
    }
}

pub mod test {
    mod orderbook_tests;
}
