use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use sdk::RunResult;

use crate::orderbook::{Order, OrderType, Orderbook, TokenPair};

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

        let user = calldata.identity.0.clone();

        let Some(tx_ctx) = &calldata.tx_ctx else {
            return Err("tx_ctx is missing".to_string());
        };

        if tx_ctx.lane_id != self.lane_id {
            return Err("Invalid lane id".to_string());
        }

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

        // Execute the given action
        let events = match action {
            OrderbookAction::AddSessionKey {} => {
                // On this step, the public key is provided in private_input and hence is never public.
                // The orderbook server knows the public key as user informed it offchain.
                // TODO: For this transaction to be valid, we need to check that there is a wallet blob.
                let pubkey = &calldata.private_input;
                self.add_session_key(user, pubkey)?
            }
            OrderbookAction::Deposit { token, amount } => {
                // TODO: assert there is a transfer blob for that token
                self.deposit(token, amount, user, &calldata.private_input)?
            }
            OrderbookAction::CreateOrder {
                order_id,
                order_type,
                price,
                pair,
                quantity,
            } => {
                let private_data =
                    borsh::from_slice::<CreateOrderPrivateInput>(&calldata.private_input)
                        .unwrap_or_else(|_| {
                            // We need to panic here to avoid generating a proof
                            panic!("Failed to deserialize CreateOrderPrivateInput")
                        });

                // Verify user signature authorization
                // On this step, signature is provided in private_input and hence is never public.
                // The orderbook server knows the signature as user informed it offchain.
                // As the public key has been registered, only the user can create that signature and hence allow this order creation
                utils::verify_user_signature_authorization(
                    &private_data.user,
                    &private_data.public_key,
                    &private_data.signature,
                    &order_id,
                    &self.session_keys,
                )?;

                let order = Order {
                    order_id,
                    order_type,
                    price,
                    pair,
                    quantity,
                    timestamp: tx_ctx.timestamp.clone(),
                };
                if self.orders.contains_key(&order.order_id) {
                    return Err(format!("Order with id {} already exists", order.order_id));
                }
                self.execute_order(&user, order, private_data.order_user_map)?
            }
            OrderbookAction::Cancel { order_id } => {
                // TODO: assert user is allowed to cancel order
                // &calldata.private_input

                self.cancel_order(order_id)?
            }
            OrderbookAction::Withdraw { token, amount } => {
                // TODO: assert user is allowed to withdraw
                // &calldata.private_input

                // TODO: assert there is a transfer blob for that token
                self.withdraw(token, amount, user)?
            }
        };

        let res =
            borsh::to_vec(&events).map_err(|_| "Failed to encode OrderbookEvents".to_string())?;

        Ok((res, ctx, vec![]))
    }

    /// In this example, we serialize the full state on-chain.
    fn commit(&self) -> sdk::StateCommitment {
        sdk::StateCommitment(self.as_bytes().expect("Failed to encode Orderbook"))
    }
}

/// Structure to deserialize private data during order creation
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct CreateOrderPrivateInput {
    pub signature: Vec<u8>,
    pub public_key: Vec<u8>,
    pub user: String,
    // Owners of order_ids that the processed order will impact (to be able to fund balances)
    pub order_user_map: BTreeMap<orderbook::OrderId, String>,
}

/// Enum representing possible calls to the contract functions.
#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Debug, Clone)]
pub enum OrderbookAction {
    AddSessionKey {},
    Deposit {
        token: String,
        amount: u32,
    },
    CreateOrder {
        order_id: String,
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
