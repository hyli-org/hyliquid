use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::BorshableMerkleProof;
use serde::{Deserialize, Serialize};

use crate::{
    model::{
        ExecuteState, Order, OrderType, OrderbookEvent, Pair, PairInfo, UserInfo,
        WithdrawDestination,
    },
    utils,
};

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
    PermissionnedOrderbookAction(PermissionnedOrderbookAction, u32),
    PermissionlessOrderbookAction(PermissionlessOrderbookAction, u32),
}

#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum PermissionnedOrderbookAction {
    Identify, // TODO: This is a temporary solution for withdraws. This should be replaced by a proxy contract
    AddSessionKey,
    CreatePair {
        pair: Pair,
        info: PairInfo,
    },
    Deposit {
        symbol: String,
        amount: u64,
    },
    CreateOrder(Order),
    Cancel {
        order_id: String,
    },
    Withdraw {
        symbol: String,
        amount: u64,
        destination: WithdrawDestination,
    },
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

impl ExecuteState {
    /// Entry point for execution
    pub fn execute_permissionned_action(
        &mut self,
        user_info: UserInfo,
        action: PermissionnedOrderbookAction,
        private_input: &[u8],
    ) -> Result<Vec<OrderbookEvent>, String> {
        match action {
            PermissionnedOrderbookAction::Identify => {
                Ok(vec![]) // Identify action does not change the state
            }
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
            PermissionnedOrderbookAction::Deposit { symbol, amount } => {
                self.deposit(&symbol, amount, &user_info)
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
                    borsh::from_slice::<CreateOrderPrivateInput>(private_input).map_err(|e| {
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
            PermissionnedOrderbookAction::Withdraw { symbol, amount, .. } => {
                // TODO: assert there is a transfer blob for that symbol

                let withdraw_private_data =
                    borsh::from_slice::<WithdrawPrivateInput>(private_input)
                        .map_err(|e| format!("Failed to deserialize WithdrawPrivateInput: {e}"))?;

                // Verify user signature authorization
                utils::verify_user_signature_authorization(
                    &user_info,
                    &withdraw_private_data.public_key,
                    &format!(
                        "{}:{}:withdraw:{symbol}:{amount}",
                        user_info.user, user_info.nonce
                    ),
                    &withdraw_private_data.signature,
                )
                .map_err(|err| format!("Failed to verify user signature authorization: {err}"))?;

                self.withdraw(&symbol, &amount, &user_info)
            }
        }
    }
}
