use borsh::{io::Error, BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use sdk::RunResult;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub mod indexer;

/// Contract state
#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Debug, Clone, Default)]
pub struct Contract1 {
    pub n: u128,
}

/// In this example, we serialize the full state on-chain
/// Otherwise we should instead implement sdk::TransactionalZkContract
impl sdk::FullStateRevert for Contract1 {}

/// Contract entry point, mapping the actions to the contract functions
impl sdk::ZkContract for Contract1 {
    /// Entry point of the contract's logic
    fn execute(&mut self, calldata: &sdk::Calldata) -> RunResult {
        // Parse contract inputs
        let (action, ctx) = sdk::utils::parse_raw_calldata::<Contract1Action>(calldata)?;

        // Execute the given action
        let res = match action {
            Contract1Action::Increment => self.increment()?,
        };

        Ok((res, ctx, vec![]))
    }

    /// In this example, we serialize the full state on-chain.
    fn commit(&self) -> sdk::StateCommitment {
        sdk::StateCommitment(self.as_bytes().expect("Failed to encsode Balances"))
    }
}

/// Contract functions
impl Contract1 {
    pub fn increment(&mut self) -> Result<Vec<u8>, String> {
        self.n += 1;
        Ok(format!("Successfully incremented to {}", self.n).into_bytes())
    }

    /// Helper to convert the state to bytes, because we serialize the full state on-chain
    pub fn as_bytes(&self) -> Result<Vec<u8>, Error> {
        borsh::to_vec(self)
    }
}

/// Enum representing possible calls to the contract functions.
#[derive(Serialize, Deserialize, BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum Contract1Action {
    Increment,
}

/// Helper to convert the action to a Blob
impl Contract1Action {
    pub fn as_blob(&self, contract_name: sdk::ContractName) -> sdk::Blob {
        sdk::Blob {
            contract_name,
            data: sdk::BlobData(borsh::to_vec(self).expect("Failed to encode Contract1Action")),
        }
    }
}

/// Helper to convert the state commitment back to the contract state, because we serialize the full state on-chain
impl From<sdk::StateCommitment> for Contract1 {
    fn from(state: sdk::StateCommitment) -> Self {
        borsh::from_slice(&state.0)
            .map_err(|_| "Could not decode hyllar state".to_string())
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_increment() {
        let mut contract = Contract1::default();

        assert_eq!(contract.n, 0);

        contract.increment().unwrap();
        assert_eq!(contract.n, 1);

        contract.increment().unwrap();
        assert_eq!(contract.n, 2);
    }
}
