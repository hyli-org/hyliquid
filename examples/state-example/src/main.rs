mod generated;

use generated::{vapp, AssetInfo};
use state_core::{SMT, ZkWitnessSet};

fn main() {
    let mut execute = vapp::ExecuteState::default();
    execute
        .assets
        .insert("ETH".into(), AssetInfo { decimals: 18 });

    let full = vapp::FullState {
        execute_state: execute,
        user_infos_smt: SMT::default(),
        balances_smt: SMT::default(),
    };

    let zk_state = vapp::ZkVmState {
        user_infos: ZkWitnessSet::default(),
        balances: ZkWitnessSet::default(),
        assets: full.execute_state.assets.clone(),
    };

    println!("full: {:?}", full);
    println!("zk: {:?}", zk_state);
}
