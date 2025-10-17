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
