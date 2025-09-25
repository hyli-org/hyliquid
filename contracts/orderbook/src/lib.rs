use std::collections::{BTreeMap, BTreeSet};

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use sdk::{merkle_utils::BorshableMerkleProof, RunResult};
use sha2::{Digest, Sha256};

use crate::{
    orderbook::{
        Order, OrderId, OrderSide, OrderType, Orderbook, OrderbookEvent, PairInfo, TokenName,
        TokenPair,
    },
    smt_values::{Balance, UserInfo},
};

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod indexer;

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

        match action {
            OrderbookAction::PermissionnedOrderbookAction(action) => {
                if tx_ctx.lane_id != self.lane_id {
                    return Err("Invalid lane id".to_string());
                }

                let permissionned_private_input: PermissionnedPrivateInput =
                    borsh::from_slice(&calldata.private_input).map_err(|_| {
                        if self.server_execution {
                            "Failed to deserialize PermissionnedPrivateInput".to_string()
                        } else {
                            panic!("Failed to deserialize PermissionnedPrivateInput")
                        }
                    })?;

                // Investigate if still relevant. Investigate if there are security implications to remove it
                let user = &permissionned_private_input.user;

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
                        let mut add_session_key_private_input =
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

                        // Verify that user info proof is correct
                        self.verify_users_info_proof(
                            &[add_session_key_private_input.user_info.clone()],
                            &add_session_key_private_input.user_info_proof,
                        )?;

                        self.add_session_key(
                            &mut add_session_key_private_input.user_info,
                            &add_session_key_private_input.user_info_proof,
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

                        // Verify that user info proof is correct
                        self.verify_users_info_proof(
                            &[deposit_private_input.user_info.clone()],
                            &deposit_private_input.user_info_proof,
                        )?;

                        // Verify the balance is correct
                        self.verify_balances_proof(
                            &token,
                            &BTreeMap::from([(
                                deposit_private_input.user_info.clone(), // FIXME: remove useless clone
                                deposit_private_input.balance.clone(), // FIXME: remove useless clone
                            )]),
                            &deposit_private_input.balance_proof,
                        )?;

                        self.deposit(
                            token,
                            amount,
                            &deposit_private_input.user_info,
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

                        let user_info = create_order_private_input
                            .users_info
                            .iter()
                            .find(|u| u.user == *user)
                            .ok_or_else(|| format!("Missing user info for user {user}"))?;

                        // Verify that users info proof are correct for all users
                        self.verify_users_info_proof(
                            create_order_private_input
                                .users_info
                                .iter()
                                .cloned()
                                .collect::<Vec<_>>()
                                .as_slice(),
                            &create_order_private_input.user_info_proof,
                        )?;

                        // Verify all balances proofs
                        for (token, balances) in &create_order_private_input.balances {
                            let Some(balance_proof) =
                                create_order_private_input.balances_proof.get(token)
                            else {
                                return Err(format!("Missing balance proof for token {token}"));
                            };

                            self.verify_balances_proof(token, balances, balance_proof)?;
                        }

                        // Verify user signature authorization
                        // On this step, signature is provided in private_input and hence is never public.
                        // The orderbook server knows the signature as user informed it offchain.
                        // As the public key has been registered, only the user can create that signature and hence allow this order creation
                        let nonce = user_info.nonce + 1;
                        utils::verify_user_signature_authorization(
                            user,
                            &create_order_private_input.public_key,
                            user_info.session_keys.as_slice(),
                            &format!("{user}:{nonce}:create_order:{order_id}"),
                            &create_order_private_input.signature,
                        )?;

                        // Increment user's nonce
                        let new_user_info = UserInfo {
                            nonce,
                            ..user_info.clone()
                        };

                        // Update the user_info set with the user's info containing the incremented nonce
                        let updated_users_info: BTreeSet<UserInfo> = create_order_private_input
                            .users_info
                            .iter()
                            .filter(|&u| u.user != *user)
                            .cloned()
                            .chain(std::iter::once(new_user_info.clone()))
                            .collect();
                        self.update_users_info_merkle_root(
                            &updated_users_info,
                            &create_order_private_input.user_info_proof,
                        )?;

                        let order = Order {
                            order_id,
                            order_type,
                            order_side,
                            price,
                            pair,
                            quantity,
                        };
                        if self.orders.contains_key(&order.order_id) {
                            return Err(format!("Order with id {} already exists", order.order_id));
                        }

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

                        // Verify user signature authorization
                        let user_info = &cancel_order_private_data.user_info;
                        let nonce = user_info.nonce + 1;
                        utils::verify_user_signature_authorization(
                            user,
                            &cancel_order_private_data.public_key,
                            user_info.session_keys.as_slice(),
                            &format!("{user}:{nonce}:cancel:{order_id}"),
                            &cancel_order_private_data.signature,
                        )?;

                        // Verify that balances are correct
                        let order = self
                            .orders
                            .get(&order_id)
                            .ok_or(format!("Order {order_id} not found"))?
                            .clone();

                        let token = match &order.order_side {
                            OrderSide::Bid => order.pair.1.clone(),
                            OrderSide::Ask => order.pair.0.clone(),
                        };
                        self.verify_balances_proof(
                            &token,
                            &BTreeMap::from([(
                                user_info.clone(),
                                cancel_order_private_data.balance.clone(),
                            )]),
                            &cancel_order_private_data.balance_proof,
                        )?;

                        // Increment user's nonce
                        let new_user_info = UserInfo {
                            nonce,
                            ..user_info.clone()
                        };
                        self.update_users_info_merkle_root(
                            &BTreeSet::from([new_user_info]),
                            &cancel_order_private_data.user_info_proof,
                        )?;

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
                        let user_info = &withdraw_private_data.user_info;
                        let nonce = user_info.nonce + 1;
                        utils::verify_user_signature_authorization(
                            user,
                            &withdraw_private_data.public_key,
                            user_info.session_keys.as_slice(),
                            &format!("{user}:{nonce}:withdraw:{token}:{amount}"),
                            &withdraw_private_data.signature,
                        )?;

                        // Verify that balances are correct
                        self.verify_balances_proof(
                            &token,
                            &withdraw_private_data.balances,
                            &withdraw_private_data.balances_proof,
                        )?;

                        // Increment user's nonce
                        let new_user_info = UserInfo {
                            nonce,
                            ..user_info.clone()
                        };
                        self.update_users_info_merkle_root(
                            &BTreeSet::from([new_user_info]),
                            &withdraw_private_data.user_info_proof,
                        )?;

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

                Ok((res, ctx, vec![]))
            }
            OrderbookAction::PermissionlessOrderbookAction(action) => {
                // Execute the given action
                let events: Vec<OrderbookEvent> = match action {
                    PermissionlessOrderbookAction::Escape { user } => self.escape(tx_ctx, user)?,
                };

                let res = borsh::to_vec(&events)
                    .map_err(|_| "Failed to encode OrderbookEvents".to_string())?;

                Ok((res, ctx, vec![]))
            }
        }
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
    pub user: String,
    pub private_input: Vec<u8>,
}

/// Structure to deserialize private data during order creation
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct AddSessionKeyPrivateInput {
    pub user_info: UserInfo,
    pub user_info_proof: BorshableMerkleProof,
    pub new_public_key: Vec<u8>,
}

/// Structure to deserialize private data during order creation
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct DepositPrivateInput {
    // Used to assert and increment user's nonce
    pub user_info: UserInfo,
    pub user_info_proof: BorshableMerkleProof,
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
    pub user_info_proof: BorshableMerkleProof,

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
    // Used to assert and increment user's nonce
    pub user_info: UserInfo,
    pub user_info_proof: BorshableMerkleProof,
    // Used to assert and increment user's balance
    pub balance: Balance,
    pub balance_proof: BorshableMerkleProof,
}

/// Structure to deserialize private data during withdraw
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct WithdrawPrivateInput {
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
    // Used to assert and increment user's nonce
    pub user_info: UserInfo,
    pub user_info_proof: BorshableMerkleProof,
    // Used to assert and increment user's and orderbook's balance
    pub balances: BTreeMap<UserInfo, Balance>,
    pub balances_proof: BorshableMerkleProof,
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
    Escape { user: String },
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
