mod generated;

use generated::vapp::{self, WitnessBridge};
use generated::AssetInfo;
use state_core::{ZkWitnessSet, SMT};

fn main() {
    let mut execute = vapp::ExecuteState::default();
    execute
        .assets
        .insert("ETH".into(), AssetInfo { decimals: 18 });

    let mut full = vapp::FullState {
        execute_state: execute.clone(),
        user_infos_smt: SMT::default(),
        balances_smt: SMT::default(),
    };

    let mut zk_state = vapp::ZkVmState {
        user_infos: ZkWitnessSet::default(),
        balances: ZkWitnessSet::default(),
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

    let drained = zk_state.drain_to_execute_state();
    println!("drained execute: {:?}", drained);
    zk_state.populate_from_execute_state(drained);

    println!("full: {:?}", full);
    println!("zk: {:?}", zk_state);
}
