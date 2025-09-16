use anyhow::Context;
use client_sdk::transaction_builder::TxExecutorHandler;
use sdk::{utils::as_hyli_output, Blob, Calldata, Contract, ContractName, ZkContract};

use crate::Contract1;

pub mod metadata {
    pub const CONTRACT1_ELF: &[u8] = include_bytes!("../../contract1.img");
    pub const PROGRAM_ID: [u8; 32] = sdk::str_to_u8(include_str!("../../contract1.txt"));
}

impl TxExecutorHandler for Contract1 {
    type Contract = Self;

    fn build_commitment_metadata(&self, _blob: &Blob) -> anyhow::Result<Vec<u8>> {
        borsh::to_vec(self).context("Failed to encode Contract1")
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
        _contract: &Contract,
        _metadata: &Option<Vec<u8>>,
    ) -> anyhow::Result<Self> {
        Ok(Self::default())
    }

    fn get_state_commitment(&self) -> sdk::StateCommitment {
        self.commit()
    }
}
