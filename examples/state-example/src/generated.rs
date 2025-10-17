use state_core::GetHashMapIndex;
use state_macros::vapp_state;

#[derive(Debug, Clone, Default)]
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

#[derive(Debug, Clone, Default)]
pub struct Balance(pub i64);

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

impl vapp::Logic for vapp::ExecuteState {
    fn compute_events(&self, action: &vapp::Action) -> Vec<vapp::Event> {
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

    fn apply_events(&mut self, events: &[vapp::Event]) {
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
                        .or_insert_with(|| Balance(0));
                    balance.0 += amount;
                }
            }
        }
    }
}
