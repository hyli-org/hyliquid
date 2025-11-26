use std::collections::BTreeMap;
use std::env;

use anyhow::{anyhow, Result};
use borsh::BorshDeserialize;
use orderbook::zk::{OrderManagerRoots, H256};
use orderbook::{model::AssetInfo, zk::FullState};
use sdk::{BlobIndex, ContractName, LaneId, StateCommitment};

#[derive(Debug, BorshDeserialize, Eq, PartialEq)]
struct DebugStateCommitment {
    pub users_info_root: H256,
    pub balances_roots: BTreeMap<String, H256>,
    pub assets: BTreeMap<String, AssetInfo>,
    pub order_manager_roots: OrderManagerRoots,
    pub hashed_secret: [u8; 32],
    pub lane_id: LaneId,
    pub last_block_number: sdk::BlockHeight,
}

fn decode(hexstr: &str) -> Result<DebugStateCommitment> {
    let bytes = hex::decode(hexstr.trim_start_matches("0x"))?;
    Ok(DebugStateCommitment::from(StateCommitment(bytes)))
}

impl From<StateCommitment> for DebugStateCommitment {
    fn from(value: StateCommitment) -> Self {
        borsh::from_slice(&value.0).expect("Failed to deserialize DebugStateCommitment")
    }
}

fn print_commit(label: &str, c: &DebugStateCommitment) {
    println!("=== {label} ===");
    println!("users_root: 0x{}", hex::encode(c.users_info_root));
    println!("lane_id: 0x{}", hex::encode(&(c.lane_id.0).0));
    println!("last_block: {}", (c.last_block_number).0);
    println!(
        "assets: {:?}",
        c.assets
            .iter()
            .map(|(k, v)| format!("{k}:{{scale:{}, cn:{}}}", v.scale, v.contract_name.0))
            .collect::<Vec<_>>()
    );
    println!(
        "balances symbols: {:?}",
        c.balances_roots
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    );
    println!(
        "order_manager roots: bids={:?} asks={:?}",
        c.order_manager_roots.bid_orders_root, c.order_manager_roots.ask_orders_root
    );
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!(
            "usage: cargo run -p server --bin diff_commitments -- <onchain_hex> <prover_hex>"
        );
        std::process::exit(1);
    }
    let onchain = decode(&args[1]).map_err(|e| anyhow!("decode onchain: {e}"))?;
    let prover = decode(&args[2]).map_err(|e| anyhow!("decode prover: {e}"))?;

    print_commit("onchain", &onchain);
    print_commit("prover", &prover);

    if onchain.assets != prover.assets {
        println!("Assets differ:");
        for (k, v) in onchain.assets.iter() {
            if let Some(other) = prover.assets.get(k) {
                if other != v {
                    println!("  {k}: onchain {:?} != prover {:?}", v, other);
                }
            } else {
                println!("  {k}: present only on onchain");
            }
        }
        for (k, v) in prover.assets.iter() {
            if !onchain.assets.contains_key(k) {
                println!("  {k}: present only on prover {:?}", v);
            }
        }
    }

    if onchain.balances_roots != prover.balances_roots {
        println!("Balance roots differ:");
        println!("  onchain: {:?}", onchain.balances_roots);
        println!("  prover: {:?}", prover.balances_roots);
    }

    if onchain.order_manager_roots != prover.order_manager_roots {
        println!(
            "Order manager roots differ: onchain {:?} vs prover {:?}",
            onchain.order_manager_roots, prover.order_manager_roots
        );
    }

    if onchain.users_info_root != prover.users_info_root {
        println!(
            "Users root differ: onchain {:?} vs prover {:?}",
            onchain.users_info_root, prover.users_info_root
        );
    }

    Ok(())
}
