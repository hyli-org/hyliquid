use std::sync::Arc;

use alloy_genesis::{ChainConfig, Genesis};
use alloy_primitives::Address;
use reth_ethereum::chainspec::ChainSpec;
use serde_json::json;

const DEFAULT_BALANCE: &str = "0xD3C21BCECCEDA1000000";

pub fn custom_chain_config(chain_id: u64) -> ChainConfig {
    let value = json!({
        "chainId": chain_id,
        "homesteadBlock": 0,
        "daoForkBlock": 0,
        "daoForkSupport": true,
        "eip150Block": 0,
        "eip155Block": 0,
        "eip158Block": 0,
        "byzantiumBlock": 0,
        "constantinopleBlock": 0,
        "petersburgBlock": 0,
        "istanbulBlock": 0,
        "muirGlacierBlock": 0,
        "berlinBlock": 0,
        "londonBlock": 0,
        "arrowGlacierBlock": 0,
        "grayGlacierBlock": 0,
        "bedrockBlock": 0,
        "mergeNetsplitBlock": 0,
        "terminalTotalDifficulty": "0",
        "regolithTime": 0,
        "shanghaiTime": 1704992401u64,
        "canyonTime": 1704992401u64
    });
    serde_json::from_value(value).unwrap()
}

pub fn custom_chain(chain_id: u64, extra_alloc: &[Address]) -> Arc<ChainSpec> {
    let mut alloc = json!({
        "0x6Be02d1d3665660d22FF9624b7BE0551ee1Ac91b": { "balance": "0x4a47e3c12448f4ad000000" },
        "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266": { "balance": DEFAULT_BALANCE },
        "0x70997970C51812dc3A010C7d01b50e0d17dc79C8": { "balance": DEFAULT_BALANCE },
        "0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC": { "balance": DEFAULT_BALANCE },
    });

    for address in extra_alloc {
        let key = format!("{:#x}", address);
        alloc[key] = json!({ "balance": DEFAULT_BALANCE });
    }

    let value = json!({
        "nonce": "0x42",
        "timestamp": "1704992401",
        "extraData": "0x5343",
        "gasLimit": "0x5208000",
        "difficulty": "0x400000000",
        "mixHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "coinbase": "0x0000000000000000000000000000000000000000",
        "alloc": alloc,
        "number": "0x0",
        "gasUsed": "0x0",
        "parentHash": "0x0000000000000000000000000000000000000000000000000000000000000000",
        "config": serde_json::to_value(custom_chain_config(chain_id)).unwrap()
    });

    let genesis: Genesis = serde_json::from_value(value).unwrap();
    let chain_spec: ChainSpec = ChainSpec::from_genesis(genesis);
    Arc::new(chain_spec)
}
