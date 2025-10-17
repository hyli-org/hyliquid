use borsh::{BorshDeserialize, BorshSerialize};
use std::collections::{HashMap, HashSet};
use sha2::{Digest, Sha256};
use sparse_merkle_tree::{traits::Value, H256};
use state_core::{BorshableH256, GetHashMapIndex, GetKey, Proof, ZkWitnessSet};
use state_macros::vapp_state;

#[derive(
    Debug,
    Clone,
    Default,
    Eq,
    PartialEq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct UserInfo {
    pub username: String,
    pub name: String,
    pub nonce: u32,
}

impl GetHashMapIndex<String> for UserInfo {
    fn hash_map_index(&self) -> &String {
        &self.username
    }
}

impl GetKey for UserInfo {
    fn get_key(&self) -> BorshableH256 {
        let mut hasher = Sha256::new();
        hasher.update(self.username.as_bytes());
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        BorshableH256::from(bytes)
    }
}

impl Value for UserInfo {
    fn to_h256(&self) -> H256 {
        if self.nonce == 0 {
            return H256::zero();
        }
        let serialized = borsh::to_vec(self).expect("failed to serialize user info");
        let mut hasher = Sha256::new();
        hasher.update(&serialized);
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        H256::from(bytes)
    }

    fn zero() -> Self {
        UserInfo {
            username: String::new(),
            name: String::new(),
            nonce: 0,
        }
    }
}

#[derive(
    Debug,
    Clone,
    Default,
    Eq,
    PartialEq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
)]
pub struct Balance {
    pub username: String,
    pub amount: i64,
}

impl Balance {
    pub fn new(username: String) -> Self {
        Balance {
            username,
            amount: 0,
        }
    }
}

impl GetHashMapIndex<String> for Balance {
    fn hash_map_index(&self) -> &String {
        &self.username
    }
}

impl GetKey for Balance {
    fn get_key(&self) -> BorshableH256 {
        let mut hasher = Sha256::new();
        hasher.update(self.username.as_bytes());
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        BorshableH256::from(bytes)
    }
}

impl Value for Balance {
    fn to_h256(&self) -> H256 {
        if self.amount == 0 {
            return H256::zero();
        }
        let mut hasher = Sha256::new();
        hasher.update(self.username.as_bytes());
        hasher.update(self.amount.to_le_bytes());
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        H256::from(bytes)
    }

    fn zero() -> Self {
        Balance {
            username: String::new(),
            amount: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AssetInfo {
    pub decimals: u8,
}

#[derive(Debug, Clone)]
pub enum Action {
    RegisterUser {
        username: String,
        name: String,
    },
    CreditBalance {
        symbol: String,
        username: String,
        amount: i64,
    },
}

#[derive(Debug, Clone)]
pub enum Event {
    UserRegistered(UserInfo),
    BalanceCredited {
        symbol: String,
        username: String,
        amount: i64,
    },
}

//

#[vapp_state(action = Action, event = Event)]
pub struct Vapp {
    #[commit(SMT)]
    pub user_infos: std::collections::HashMap<String, UserInfo>,

    #[commit(SMT)]
    pub balances: std::collections::HashMap<String, std::collections::HashMap<String, Balance>>,

    #[ident(borsh)]
    pub assets: std::collections::HashMap<String, AssetInfo>,
}

impl vapp::ExecuteState {
    pub fn compute_events_logic(&self, action: &vapp::Action) -> Vec<vapp::Event> {
        match action {
            vapp::Action::RegisterUser { username, name } => {
                if self.user_infos.contains_key(username) {
                    vec![]
                } else {
                    vec![vapp::Event::UserRegistered(UserInfo {
                        username: username.clone(),
                        name: name.clone(),
                        nonce: 0,
                    })]
                }
            }
            vapp::Action::CreditBalance {
                symbol,
                username,
                amount,
            } => {
                if !self.user_infos.contains_key(username) {
                    vec![]
                } else {
                    vec![vapp::Event::BalanceCredited {
                        symbol: symbol.clone(),
                        username: username.clone(),
                        amount: *amount,
                    }]
                }
            }
        }
    }

    pub fn apply_events_logic(&mut self, events: &[vapp::Event]) {
        for event in events {
            match event {
                vapp::Event::UserRegistered(user) => {
                    self.user_infos.insert(
                        user.username.clone(),
                        UserInfo {
                            username: user.username.clone(),
                            name: user.name.clone(),
                            nonce: user.nonce,
                        },
                    );
                }
                vapp::Event::BalanceCredited {
                    symbol,
                    username,
                    amount,
                } => {
                    let balance = self
                        .balances
                        .entry(symbol.clone())
                        .or_default()
                        .entry(username.clone())
                        .or_insert_with(|| Balance::new(username.clone()));
                    balance.amount += amount;
                }
            }
        }
    }
}

impl vapp::FullState {
    pub fn build_witness_state(&self, events: &[vapp::Event]) -> vapp::ZkVmState {
        let mut user_values: HashSet<UserInfo> = HashSet::new();
        let mut balance_values: HashMap<String, HashSet<Balance>> = HashMap::new();

        for event in events {
            match event {
                vapp::Event::UserRegistered(event_user) => {
                    if let Some(user) = self
                        .execute_state
                        .user_infos
                        .get(&event_user.username)
                    {
                        user_values.insert(user.clone());
                    } else {
                        user_values.insert(event_user.clone());
                    }
                }
                vapp::Event::BalanceCredited {
                    symbol,
                    username,
                    ..
                } => {
                    if let Some(balances) = self.execute_state.balances.get(symbol) {
                        if let Some(balance) = balances.get(username) {
                            balance_values
                                .entry(symbol.clone())
                                .or_default()
                                .insert(balance.clone());
                        }
                    }
                }
            }
        }

        let user_infos = ZkWitnessSet {
            values: user_values,
            proof: Proof::CurrentRootHash(self.user_infos.root()),
        };

        let balances = balance_values
            .into_iter()
            .map(|(symbol, values)| {
                let proof = self
                    .balances
                    .get(&symbol)
                    .map(|tree| Proof::CurrentRootHash(tree.root()))
                    .unwrap_or_default();
                (symbol, ZkWitnessSet { values, proof })
            })
            .collect();

        vapp::ZkVmState {
            user_infos,
            balances,
            assets: self.execute_state.assets.clone(),
        }
    }
}
