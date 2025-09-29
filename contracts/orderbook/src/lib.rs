use std::collections::{BTreeMap, BTreeSet};

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use sdk::{merkle_utils::BorshableMerkleProof, RunResult};
use sha2::{Digest, Sha256};

use crate::{
    orderbook::{Order, OrderId, OrderSide, OrderType, Orderbook, PairInfo, TokenName, TokenPair},
    smt_values::{Balance, UserInfo},
};

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod indexer;

pub mod order_manager;
pub mod orderbook;
pub mod smt_values;
mod tests;
pub mod utils;

impl sdk::FullStateRevert for Orderbook {}

impl sdk::ZkContract for Orderbook {
    /// Entry point of the contract's logic
    fn execute(&mut self, calldata: &sdk::Calldata) -> RunResult {
        // Parse contract inputs
        let (action, ctx) = sdk::utils::parse_raw_calldata::<OrderbookAction>(calldata)?;

        let Some(tx_ctx) = &calldata.tx_ctx else {
            return Err("tx_ctx is missing".to_string());
        };

        // The contract must be provided with all blobs
        if calldata.blobs.len() != calldata.tx_blob_count {
            return Err("Calldata is not composed with all tx's blobs".to_string());
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

        let (res, mut user_info, user_info_proof) = match action {
            OrderbookAction::PermissionnedOrderbookAction(action) => {
                if tx_ctx.lane_id != self.lane_id {
                    return Err("Invalid lane id".to_string());
                }

                let mut permissionned_private_input: PermissionnedPrivateInput =
                    borsh::from_slice(&calldata.private_input).map_err(|_| {
                        if self.server_execution {
                            "Failed to deserialize PermissionnedPrivateInput".to_string()
                        } else {
                            panic!("Failed to deserialize PermissionnedPrivateInput")
                        }
                    })?;

                let user_info = &mut permissionned_private_input.user_info;
                let user_info_proof = &permissionned_private_input.user_info_proof;

                // Verify that user info proof is correct
                self.verify_user_info_proof(user_info, user_info_proof)
                    .map_err(|err| {
                        if self.server_execution {
                            format!("Failed to verify user info proof: {err}")
                        } else {
                            panic!("Failed to verify user info proof: {err}")
                        }
                    })?;

                let hashed_secret = Sha256::digest(&permissionned_private_input.secret)
                    .as_slice()
                    .to_vec();
                if hashed_secret != self.hashed_secret {
                    if self.server_execution {
                        return Err("Invalid secret in private input".to_string());
                    } else {
                        panic!("Invalid secret in private input");
                    }
                }

                // Execute the given action
                let events = match action {
                    PermissionnedOrderbookAction::CreatePair { pair, info } => {
                        self.create_pair(pair, info)?
                    }
                    PermissionnedOrderbookAction::AddSessionKey => {
                        // On this step, the public key is provided in private_input and hence is never public.
                        // The orderbook server knows the public key as user informed it offchain.
                        let add_session_key_private_input =
                            borsh::from_slice::<AddSessionKeyPrivateInput>(
                                &permissionned_private_input.private_input,
                            )
                            .map_err(|_| {
                                if self.server_execution {
                                    "Failed to deserialize CreateOrderPrivateInput".to_string()
                                } else {
                                    panic!("Failed to deserialize CreateOrderPrivateInput")
                                }
                            })?;

                        self.add_session_key(
                            user_info,
                            &add_session_key_private_input.new_public_key,
                        )?
                    }
                    PermissionnedOrderbookAction::Deposit { token, amount } => {
                        // This is a permissionned action, the server is responsible for checking that a transfer blob happened
                        let mut deposit_private_input = borsh::from_slice::<DepositPrivateInput>(
                            &permissionned_private_input.private_input,
                        )
                        .map_err(|_| {
                            if self.server_execution {
                                "Failed to deserialize DepositPrivateInput".to_string()
                            } else {
                                panic!("Failed to deserialize DepositPrivateInput")
                            }
                        })?;

                        // Verify the balance is correct
                        self.verify_balance_proof(
                            &token,
                            user_info,
                            &deposit_private_input.balance,
                            &deposit_private_input.balance_proof,
                        )
                        .map_err(|err| {
                            if self.server_execution {
                                format!("Failed to verify balance proof: {err}")
                            } else {
                                panic!("Failed to verify balance proof: {err}")
                            }
                        })?;

                        self.deposit(
                            token,
                            amount,
                            user_info,
                            &mut deposit_private_input.balance,
                            &deposit_private_input.balance_proof,
                        )?
                    }
                    PermissionnedOrderbookAction::CreateOrder {
                        order_id,
                        order_side,
                        order_type,
                        price,
                        pair,
                        quantity,
                    } => {
                        let create_order_private_input =
                            borsh::from_slice::<CreateOrderPrivateInput>(
                                &permissionned_private_input.private_input,
                            )
                            .map_err(|_| {
                                if self.server_execution {
                                    "Failed to deserialize CreateOrderPrivateInput".to_string()
                                } else {
                                    panic!("Failed to deserialize CreateOrderPrivateInput")
                                }
                            })?;

                        // Verify that users info proof are correct for all users
                        self.verify_users_info_proof(
                            &create_order_private_input.users_info,
                            &create_order_private_input.users_info_proof,
                        )
                        .map_err(|err| {
                            if self.server_execution {
                                format!("Failed to verify users info proof: {err}")
                            } else {
                                panic!("Failed to verify users info proof: {err}")
                            }
                        })?;

                        // Verify all balances proofs
                        for (token, balances) in &create_order_private_input.balances {
                            if let Some(balance_proof) =
                                create_order_private_input.balances_proof.get(token)
                            {
                                self.verify_balances_proof(token, balances, balance_proof).map_err(|err| {
                                if self.server_execution {
                                    format!("Failed to verify balance proof for token {token}: {err}")
                                } else {
                                    panic!("Failed to verify balance proof for token {token}: {err}")
                                }
                            })?;
                            } else if self.server_execution {
                                return Err(format!("Missing balance proof for token {token}"));
                            } else {
                                panic!("Missing balance proof for token {token}");
                            };
                        }

                        // Verify that order_user_map is populated with valid users info
                        for (order_id, user_info) in &create_order_private_input.order_user_map {
                            if !self.order_manager.orders.contains_key(order_id) {
                                if self.server_execution {
                                    return Err(format!("Order with id {order_id} does not exist"));
                                } else {
                                    panic!("Order with id {order_id} does not exist");
                                }
                            }
                            // We previously verified create_order_private_input.users_info
                            if !create_order_private_input.users_info.contains(user_info) {
                                if self.server_execution {
                                    return Err(format!(
                                        "Missing user info for user {}",
                                        user_info.user
                                    ));
                                } else {
                                    panic!("Missing user info for user {}", user_info.user);
                                }
                            }
                        }

                        // Verify user signature authorization
                        // On this step, signature is provided in private_input and hence is never public.
                        // The orderbook server knows the signature as user informed it offchain.
                        // As the public key has been registered, only the user can create that signature and hence allow this order creation
                        utils::verify_user_signature_authorization(
                            user_info,
                            &create_order_private_input.public_key,
                            &format!(
                                "{}:{}:create_order:{order_id}",
                                user_info.user, user_info.nonce
                            ),
                            &create_order_private_input.signature,
                        )
                        .map_err(|err| {
                            if self.server_execution {
                                format!("Failed to verify user signature authorization: {err}")
                            } else {
                                panic!("Failed to verify user signature authorization: {err}")
                            }
                        })?;

                        let order = Order {
                            order_id,
                            order_type,
                            order_side,
                            price,
                            pair,
                            quantity,
                        };

                        self.execute_order(
                            user_info,
                            order,
                            create_order_private_input.order_user_map,
                            &create_order_private_input.balances,
                            &create_order_private_input.balances_proof,
                        )?
                    }
                    PermissionnedOrderbookAction::Cancel { order_id } => {
                        let cancel_order_private_data =
                            borsh::from_slice::<CancelOrderPrivateInput>(
                                &permissionned_private_input.private_input,
                            )
                            .map_err(|_| {
                                if self.server_execution {
                                    "Failed to deserialize CancelOrderPrivateInput".to_string()
                                } else {
                                    panic!("Failed to deserialize CancelOrderPrivateInput")
                                }
                            })?;

                        // Verify that balances are correct
                        let order = self
                            .order_manager
                            .orders
                            .get(&order_id)
                            .ok_or(format!("Order {order_id} not found"))?;

                        let token = match &order.order_side {
                            OrderSide::Bid => &order.pair.1,
                            OrderSide::Ask => &order.pair.0,
                        };

                        self.verify_balance_proof(
                            token,
                            user_info,
                            &cancel_order_private_data.balance,
                            &cancel_order_private_data.balance_proof,
                        )
                        .map_err(|err| {
                            if self.server_execution {
                                format!("Failed to verify balance proof: {err}")
                            } else {
                                panic!("Failed to verify balance proof: {err}")
                            }
                        })?;

                        // Verify user signature authorization
                        utils::verify_user_signature_authorization(
                            user_info,
                            &cancel_order_private_data.public_key,
                            &format!("{}:{}:cancel:{order_id}", user_info.user, user_info.nonce),
                            &cancel_order_private_data.signature,
                        )
                        .map_err(|err| {
                            if self.server_execution {
                                format!("Failed to verify user signature authorization: {err}")
                            } else {
                                panic!("Failed to verify user signature authorization: {err}")
                            }
                        })?;

                        self.cancel_order(
                            order_id,
                            user_info,
                            &cancel_order_private_data.balance,
                            &cancel_order_private_data.balance_proof,
                        )?
                    }
                    PermissionnedOrderbookAction::Withdraw { token, amount } => {
                        // TODO: assert there is a transfer blob for that token

                        let withdraw_private_data = borsh::from_slice::<WithdrawPrivateInput>(
                            &permissionned_private_input.private_input,
                        )
                        .map_err(|_| {
                            if self.server_execution {
                                "Failed to deserialize WithdrawPrivateInput".to_string()
                            } else {
                                panic!("Failed to deserialize WithdrawPrivateInput")
                            }
                        })?;

                        // Verify user signature authorization
                        utils::verify_user_signature_authorization(
                            user_info,
                            &withdraw_private_data.public_key,
                            &format!(
                                "{}:{}:withdraw:{token}:{amount}",
                                user_info.user, user_info.nonce
                            ),
                            &withdraw_private_data.signature,
                        )
                        .map_err(|err| {
                            if self.server_execution {
                                format!("Failed to verify user signature authorization: {err}")
                            } else {
                                panic!("Failed to verify user signature authorization: {err}")
                            }
                        })?;

                        // Verify that balances are correct
                        self.verify_balances_proof(
                            &token,
                            &withdraw_private_data.balances,
                            &withdraw_private_data.balances_proof,
                        )
                        .map_err(|err| {
                            if self.server_execution {
                                format!("Failed to verify balance proof: {err}")
                            } else {
                                panic!("Failed to verify balance proof: {err}")
                            }
                        })?;

                        self.withdraw(
                            token,
                            amount,
                            user_info,
                            &withdraw_private_data.balances,
                            &withdraw_private_data.balances_proof,
                        )?
                    }
                };

                let res = borsh::to_vec(&events)
                    .map_err(|_| "Failed to encode OrderbookEvents".to_string())?;

                (res, user_info.clone(), user_info_proof.clone())
            }
            OrderbookAction::PermissionlessOrderbookAction(action) => {
                // Execute the given action
                let (events, user_info, user_info_proof) = match action {
                    PermissionlessOrderbookAction::Escape { user_key } => {
                        let escape_private_input: EscapePrivateInput =
                            borsh::from_slice(&calldata.private_input).map_err(|_| {
                                if self.server_execution {
                                    "Failed to deserialize PermissionnedPrivateInput".to_string()
                                } else {
                                    panic!("Failed to deserialize PermissionnedPrivateInput")
                                }
                            })?;

                        let user_info = escape_private_input.user_info.clone();
                        let user_info_proof = escape_private_input.user_info_proof.clone();

                        // Verify that user info proof is correct
                        self.verify_user_info_proof(&user_info, &user_info_proof)
                            .map_err(|err| {
                                if self.server_execution {
                                    format!("Failed to verify user info proof: {err}")
                                } else {
                                    panic!("Failed to verify user info proof: {err}")
                                }
                            })?;

                        if user_key != std::convert::Into::<[u8; 32]>::into(user_info.get_key()) {
                            if self.server_execution {
                                return Err(
                                    "User info does not correspond with user_key used".to_string()
                                );
                            } else {
                                panic!("User info does not correspond with user_key used")
                            }
                        }
                        let events = self.escape(tx_ctx, &user_info, &user_info_proof)?;

                        (events, user_info, user_info_proof)
                    }
                };

                let res = borsh::to_vec(&events)
                    .map_err(|_| "Failed to encode OrderbookEvents".to_string())?;

                (res, user_info, user_info_proof)
            }
        };

        // Increment user's nonce
        user_info.nonce += 1;
        // Update users merkle tree
        self.update_user_info_merkle_root(&user_info, &user_info_proof)?;

        Ok((res, ctx, vec![]))
    }

    /// In this example, we serialize the full state on-chain.
    fn commit(&self) -> sdk::StateCommitment {
        sdk::StateCommitment(self.as_bytes().expect("Failed to encode Orderbook"))
    }
}

/// Structure to deserialize permissionned private data
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct PermissionnedPrivateInput {
    pub secret: Vec<u8>,

    // Used to assert and increment user's nonce
    pub user_info: UserInfo,
    pub user_info_proof: BorshableMerkleProof,

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
pub struct DepositPrivateInput {
    // Used to assert and increment user's balance
    pub balance: Balance,
    pub balance_proof: BorshableMerkleProof,
}

/// Structure to deserialize private data during order creation
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct CreateOrderPrivateInput {
    // Used to assert user approval of that action
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,

    // Owners of order_ids that the processed order will impact (to be able to fund balances)
    pub order_user_map: BTreeMap<OrderId, UserInfo>,

    // Used to assert all users' info that the processed order will impact
    pub users_info: BTreeSet<UserInfo>,
    // Proof for all user_info used
    pub users_info_proof: BorshableMerkleProof,

    // For each token, for each user, the balance used
    // token -> user: balance
    pub balances: BTreeMap<TokenName, BTreeMap<UserInfo, Balance>>,
    // For each token, the proof for all balances used
    pub balances_proof: BTreeMap<TokenName, BorshableMerkleProof>,
}

/// Structure to deserialize private data during order cancellation
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct CancelOrderPrivateInput {
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
    // Used to assert and increment user's balance
    pub balance: Balance,
    pub balance_proof: BorshableMerkleProof,
}

/// Structure to deserialize private data during withdraw
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct WithdrawPrivateInput {
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
    // Used to assert and increment user's and orderbook's balance
    pub balances: BTreeMap<UserInfo, Balance>,
    pub balances_proof: BorshableMerkleProof,
}

/// Structure to deserialize private data during escape
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct EscapePrivateInput {
    // Used to assert and increment user's nonce
    pub user_info: UserInfo,
    pub user_info_proof: BorshableMerkleProof,
}

/// Enum representing possible calls to the contract functions.
#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Debug, Clone)]
pub enum OrderbookAction {
    PermissionnedOrderbookAction(PermissionnedOrderbookAction),
    PermissionlessOrderbookAction(PermissionlessOrderbookAction),
}

#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Debug, Clone)]
pub enum PermissionnedOrderbookAction {
    AddSessionKey,
    CreatePair {
        pair: TokenPair,
        info: PairInfo,
    },
    Deposit {
        token: String,
        amount: u64,
    },
    CreateOrder {
        order_id: String,
        order_side: OrderSide,
        order_type: OrderType,
        price: Option<u64>,
        pair: TokenPair,
        quantity: u64,
    },
    Cancel {
        order_id: String,
    },
    Withdraw {
        token: String,
        amount: u64,
    },
}

#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Debug, Clone)]
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
            .map_err(|_| "Could not decode Orderbook state".to_string())
            .unwrap()
    }
}
