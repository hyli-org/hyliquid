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

    let mut events = Vec::new();

    let register_events = full.apply_action(&vapp::Action::RegisterUser {
        username: "alice".into(),
        name: "Alice".into(),
    });

    let commit = full.commit();
    println!("commit: {:?}", commit);

    println!("register events: {:?}", register_events);
    events.extend(register_events);

    let credit_events = full.apply_action(&vapp::Action::CreditBalance {
        symbol: "ETH".into(),
        username: "alice".into(),
        amount: 100,
    });
    println!("credit events: {:?}", credit_events);
    events.extend(credit_events);

    let zk_state = full.build_witness_state(&events);

    let commit = zk_state.commit();
    println!("commit: {:?}", commit);

    println!("full: {:?}", full);
    println!("zk: {:?}", zk_state);
}
