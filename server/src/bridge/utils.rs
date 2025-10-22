use std::time::SystemTime;

use alloy::primitives::{TxHash, U256};

use crate::{
    bridge::eth::EthListener,
    services::bridge_service::{EthTransaction, TxStatus},
};

pub fn log_to_eth_transaction(log: alloy::rpc::types::Log) -> EthTransaction {
    let (from, to, amount) = EthListener::parse_log_data(&log);
    let res = EthTransaction {
        tx_hash: log.transaction_hash.unwrap_or(TxHash::from([0u8; 32])),
        block_number: log.block_number.unwrap_or_default(),
        from,
        to,
        amount: U256::from(amount),
        timestamp: SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        status: TxStatus::Confirmed,
    };

    tracing::debug!("Parsed EthTransaction: {:?}", res);

    res
}
