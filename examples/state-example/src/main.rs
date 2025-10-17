mod generated;

use generated::vapp;
use generated::AssetInfo;
use state_core::SMT;
use std::collections::HashMap;

fn main() {
    let mut execute = vapp::ExecuteState::default();
    execute
        .assets
        .insert("ETH".into(), AssetInfo { decimals: 18 });

    let mut full = vapp::FullState {
        execute_state: execute.clone(),
        user_infos: SMT::zero(),
        balances: HashMap::new(),
        assets: execute.assets.clone(),
    };

    let events = execute.compute_events(&vapp::Action::RegisterUser {
        username: "alice".into(),
        name: "Alice".into(),
    });
    println!("events: {:?}", events);

    full.apply_action(&vapp::Action::RegisterUser {
        username: "alice".into(),
        name: "Alice".into(),
    });
    full.apply_action(&vapp::Action::CreditBalance {
        symbol: "ETH".into(),
        username: "alice".into(),
        amount: 100,
    });

    let zk_state = vapp::ZkVmState::default();
    println!("full: {:?}", full);
    println!("zk: {:?}", zk_state);
}
