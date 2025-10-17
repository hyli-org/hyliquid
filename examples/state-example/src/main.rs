mod generated;

use generated::{vapp, AssetInfo};

fn main() {
    let mut execute = vapp::ExecuteState::default();
    execute
        .assets
        .insert("ETH".into(), AssetInfo { decimals: 18 });

    let full = vapp::FullState {
        execute_state: execute,
    };

    let zk_state = vapp::ZkVmState;

    println!("full: {:?}", full);
    println!("zk: {:?}", zk_state);
}
