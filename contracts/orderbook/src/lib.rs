use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use sdk::RunResult;

use crate::orderbook::{Order, OrderSide, OrderType, Orderbook, OrderbookEvent, TokenPair};

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod indexer;

pub mod orderbook;
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

                let user = &permissionned_private_input.user;

                // TODO: Make a proper authentication mechanism
                if permissionned_private_input.secret != self.secret {
                    if self.server_execution {
                        return Err("Invalid secret in private input".to_string());
                    } else {
                        // We need to panic here to avoid generating a proof
                        panic!("Invalid secret in private input");
                    }
                }

                // Execute the given action
                let events = match action {
                    PermissionnedOrderbookAction::AddSessionKey => {
                        // On this step, the public key is provided in private_input and hence is never public.
                        // The orderbook server knows the public key as user informed it offchain.
                        let private_input = borsh::from_slice::<AddSessionKeyPrivateInput>(
                            &permissionned_private_input.private_input,
                        )
                        .map_err(|_| {
                            if self.server_execution {
                                "Failed to deserialize CreateOrderPrivateInput".to_string()
                            } else {
                                panic!("Failed to deserialize CreateOrderPrivateInput")
                            }
                        })?;
                        self.add_session_key(user, &private_input.public_key)?
                    }
                    PermissionnedOrderbookAction::Deposit { token, amount } => {
                        // TODO: assert there is a transfer blob for that token
                        self.deposit(
                            token,
                            amount,
                            user,
                            &permissionned_private_input.private_input,
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
                        let create_order_private_data =
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

                        // Verify user signature authorization
                        // On this step, signature is provided in private_input and hence is never public.
                        // The orderbook server knows the signature as user informed it offchain.
                        // As the public key has been registered, only the user can create that signature and hence allow this order creation
                        utils::verify_user_signature_authorization(
                            user,
                            &create_order_private_data.public_key,
                            &create_order_private_data.signature,
                            &order_id,
                            &self.session_keys,
                        )?;

                        let order = Order {
                            order_id,
                            order_type,
                            order_side,
                            price,
                            pair,
                            quantity,
                            timestamp: tx_ctx.timestamp.clone(),
                        };
                        if self.orders.contains_key(&order.order_id) {
                            return Err(format!("Order with id {} already exists", order.order_id));
                        }
                        self.execute_order(user, order, create_order_private_data.order_user_map)?
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
                        utils::verify_user_signature_authorization(
                            user,
                            &cancel_order_private_data.public_key,
                            &cancel_order_private_data.signature,
                            &format!("cancel:{order_id}"),
                            &self.session_keys,
                        )?;

                        self.cancel_order(order_id, user)?
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
                            user,
                            &withdraw_private_data.public_key,
                            &withdraw_private_data.signature,
                            &format!("{}:{}:{token}:{amount}", user, withdraw_private_data.nonce),
                            &self.session_keys,
                        )?;
                        self.withdraw(token, amount, user)?
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
    pub public_key: Vec<u8>,
}

/// Structure to deserialize private data during order creation
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct CreateOrderPrivateInput {
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
    // Owners of order_ids that the processed order will impact (to be able to fund balances)
    pub order_user_map: BTreeMap<orderbook::OrderId, String>,
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
    pub nonce: u32,
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
    Deposit {
        token: String,
        amount: u32,
    },
    CreateOrder {
        order_id: String,
        order_side: OrderSide,
        order_type: OrderType,
        price: Option<u32>,
        pair: TokenPair,
        quantity: u32,
    },
    Cancel {
        order_id: String,
    },
    Withdraw {
        token: String,
        amount: u32,
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
