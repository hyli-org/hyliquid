use anyhow::Context;
use client_sdk::transaction_builder::TxExecutorHandler;
use sdk::{utils::as_hyli_output, Blob, Calldata, Contract, ContractName, ZkContract};

use crate::Orderbook;

impl TxExecutorHandler for Orderbook {
    type Contract = Orderbook;

    fn build_commitment_metadata(&self, _blob: &Blob) -> anyhow::Result<Vec<u8>> {
        borsh::to_vec(self).context("Failed to encode Orderbook")
    }

    fn handle(&mut self, calldata: &Calldata) -> anyhow::Result<sdk::HyliOutput> {
        let initial_state_commitment = <Self as ZkContract>::commit(self);
        let mut res = <Self as ZkContract>::execute(self, calldata);
        let next_state_commitment = <Self as ZkContract>::commit(self);
        Ok(as_hyli_output(
            initial_state_commitment,
            next_state_commitment,
            calldata,
            &mut res,
        ))
    }

    fn construct_state(
        _contract_name: &ContractName,
        contract: &Contract,
        _metadata: &Option<Vec<u8>>,
    ) -> anyhow::Result<Self> {
        borsh::from_slice(&contract.state.0).context("Failed to decode Orderbook state")
    }

    fn get_state_commitment(&self) -> sdk::StateCommitment {
        self.commit()
    }
}
