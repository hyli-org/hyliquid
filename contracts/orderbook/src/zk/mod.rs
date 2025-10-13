use std::collections::{HashMap, HashSet};

use borsh::{BorshDeserialize, BorshSerialize};
use sdk::merkle_utils::BorshableMerkleProof;
use sdk::LaneId;
use sparse_merkle_tree::MerkleProof;

use crate::model::{AssetInfo, Balance, ExecuteState, Symbol, UserInfo};
use crate::order_manager::OrderManager;

pub use smt::BorshableH256 as H256;
pub use smt::SMT;

mod commitment_metadata;
mod contract;
mod smt;

#[derive(Debug, Clone, BorshDeserialize, BorshSerialize)]
struct ZkVmWitness<T: BorshDeserialize + BorshSerialize + Default> {
    pub value: T,
    pub proof: BorshableMerkleProof,
}

impl<T: BorshDeserialize + BorshSerialize + Default> Default for ZkVmWitness<T> {
    fn default() -> Self {
        ZkVmWitness {
            value: T::default(),
            proof: BorshableMerkleProof(MerkleProof::new(vec![], vec![])),
        }
    }
}

// Full state with commitment structures
#[derive(Default, Debug)]
pub struct FullState {
    pub users_info_mt: SMT<UserInfo>,
    pub balances_mt: HashMap<String, SMT<Balance>>,
    pub state: ExecuteState,
    pub hashed_secret: H256,
    pub lane_id: LaneId,
}

// Committed state
#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct OnChainState {
    pub users_info_root: H256,
    pub balances_roots: HashMap<Symbol, H256>,
    pub assets: HashMap<Symbol, AssetInfo>,
    pub orders: OrderManager,
    pub hashed_secret: H256,
    pub lane_id: LaneId,
}

#[derive(Debug, Default, Clone, BorshDeserialize, BorshSerialize)]
pub struct ZkVmState {
    pub onchain_state: OnChainState,
    pub users_info: ZkVmWitness<HashSet<UserInfo>>,
    pub balances: HashMap<Symbol, ZkVmWitness<HashMap<H256, Balance>>>,
}

/// impl of functions for state management
impl borsh::BorshSerialize for FullState {
    fn serialize<W: std::io::Write>(
        &self,
        _writer: &mut W,
    ) -> std::result::Result<(), std::io::Error> {
        panic!("FullState::serialize: todo!()")
    }
}

impl borsh::BorshDeserialize for FullState {
    fn deserialize_reader<R: std::io::Read>(
        _reader: &mut R,
    ) -> std::result::Result<Self, std::io::Error> {
        panic!("FullState::deserialize: todo!()")
    }
}

// impl ExtractableZkWitnesses for ExecuteState {
//     type Witnesses = ZkVmState;

//     fn extract_state(witnesses: Self::Witnesses) -> Self {
//         todo!()
//     }
// }

// impl ExecuteState {
//     fn test() {
//         let test = borsh::from_slice::<
//             <crate::model::ExecuteState as ExtractableZkWitnesses>::Witnesses,
//         >(&[0u8; 32])
//         .unwrap();
//     }
// }

// trait ExtractableZkWitnesses {
//     type Witnesses: BorshDeserialize;
//     fn extract_state(witnesses: Self::Witnesses) -> Self;
// }
