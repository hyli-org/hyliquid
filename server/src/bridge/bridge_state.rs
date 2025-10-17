use alloy::primitives::{Address, TxHash, U256};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::str::FromStr;

/// Transaction Ethereum entrante (vers le bridge)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct EthTransaction {
    pub tx_hash: TxHash,
    pub block_number: u64,
    pub from: Address,
    pub to: Address, // Adresse du vault
    pub amount: U256,
    pub timestamp: u64,
    pub status: TxStatus,
}

/// Statut d'une transaction
#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, BorshSerialize, BorshDeserialize,
)]
pub enum TxStatus {
    Pending,   // En attente de confirmation
    Confirmed, // Confirmée sur la blockchain
}

/// État global du bridge (version simplifiée)
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BridgeState {
    // Configuration
    pub eth_contract: Address,
    pub eth_contract_vault_address: Address,

    // État des blockchains
    pub eth_last_block: u64,

    // Transactions en attente
    #[serde(default)]
    pub eth_pending_txs: HashMap<TxHash, EthTransaction>,

    // Transactions déjà observées afin d'éviter de les retraiter.
    #[serde(default)]
    pub processed_eth_txs: HashSet<TxHash>,

    // Métadonnées
    #[serde(default)]
    pub eth_address_bindings: HashMap<Address, String>,
}

#[derive(BorshSerialize, BorshDeserialize)]
struct EthTransactionSer {
    tx_hash: [u8; 32],
    block_number: u64,
    from: [u8; 20],
    to: [u8; 20],
    amount: [u8; 32],
    timestamp: u64,
    status: TxStatus,
}

impl BridgeState {
    /// Crée un nouvel état de bridge
    pub fn from_vault_adress(vault_address: String) -> Self {
        BridgeState {
            eth_contract_vault_address: Address::from_str(vault_address.as_str()).unwrap(),
            ..Default::default()
        }
    }

    pub fn is_eth_pending(&self, tx_hash: &TxHash) -> bool {
        self.eth_pending_txs.contains_key(tx_hash)
    }

    pub fn is_eth_processed(&self, tx_hash: &TxHash) -> bool {
        self.processed_eth_txs.contains(tx_hash)
    }

    pub fn is_eth_tracked(&self, tx_hash: &TxHash) -> bool {
        self.is_eth_pending(tx_hash) || self.is_eth_processed(tx_hash)
    }

    pub fn mark_eth_processed(&mut self, tx_hash: TxHash) {
        self.eth_pending_txs.remove(&tx_hash);
        self.processed_eth_txs.insert(tx_hash);
    }

    pub fn record_eth_identity_binding(&mut self, address: Address, user_identity: String) {
        sdk::info!(
            "Recording ETH address binding: {:?} -> {}",
            address,
            user_identity
        );
        self.eth_address_bindings.insert(address, user_identity);
    }

    pub fn hyli_identity_for_eth(&self, address: &Address) -> Option<&String> {
        self.eth_address_bindings.get(address)
    }

    pub fn eth_address_for_hyli_identity(&self, identity: &str) -> Option<Address> {
        self.eth_address_bindings
            .iter()
            .find_map(|(address, stored_identity)| {
                if stored_identity == identity {
                    Some(*address)
                } else {
                    None
                }
            })
    }

    pub fn record_eth_block(&mut self, block_number: u64) {
        if block_number > self.eth_last_block {
            self.eth_last_block = block_number;
        }
    }

    /// Ajoute une transaction Ethereum en attente. Retourne `true` si elle a été enregistrée.
    pub fn add_eth_pending_transaction(&mut self, tx: EthTransaction) -> bool {
        if self.is_eth_tracked(&tx.tx_hash) {
            return false;
        }

        self.record_eth_block(tx.block_number);
        self.eth_pending_txs.insert(tx.tx_hash, tx);
        true
    }
}

#[derive(BorshSerialize, BorshDeserialize)]
struct BridgeStateSer {
    eth_contract: [u8; 20],
    eth_contract_vault_address: [u8; 20],
    eth_last_block: u64,
    eth_pending_txs: Vec<([u8; 32], EthTransaction)>,
    processed_eth_txs: Vec<[u8; 32]>,
    eth_address_bindings: Vec<([u8; 20], String)>,
}

fn address_to_array(address: &Address) -> [u8; 20] {
    let mut out = [0u8; 20];
    out.copy_from_slice(address.as_ref());
    out
}

fn tx_hash_to_array(hash: &TxHash) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(hash.as_ref());
    out
}

fn u256_to_array(value: &U256) -> [u8; 32] {
    value.to_be_bytes()
}

impl BorshSerialize for EthTransaction {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let ser = EthTransactionSer {
            tx_hash: tx_hash_to_array(&self.tx_hash),
            block_number: self.block_number,
            from: address_to_array(&self.from),
            to: address_to_array(&self.to),
            amount: u256_to_array(&self.amount),
            timestamp: self.timestamp,
            status: self.status.clone(),
        };

        ser.serialize(writer)
    }
}

impl BorshDeserialize for EthTransaction {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let ser = EthTransactionSer::deserialize_reader(reader)?;

        Ok(EthTransaction {
            tx_hash: TxHash::from(ser.tx_hash),
            block_number: ser.block_number,
            from: Address::from(ser.from),
            to: Address::from(ser.to),
            amount: U256::from_be_bytes(ser.amount),
            timestamp: ser.timestamp,
            status: ser.status,
        })
    }
}

impl BorshSerialize for BridgeState {
    fn serialize<W: Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let ser = BridgeStateSer {
            eth_contract: address_to_array(&self.eth_contract),
            eth_contract_vault_address: address_to_array(&self.eth_contract_vault_address),
            eth_last_block: self.eth_last_block,
            eth_pending_txs: self
                .eth_pending_txs
                .iter()
                .map(|(hash, tx)| (tx_hash_to_array(hash), tx.clone()))
                .collect(),
            processed_eth_txs: self
                .processed_eth_txs
                .iter()
                .map(tx_hash_to_array)
                .collect(),
            eth_address_bindings: self
                .eth_address_bindings
                .iter()
                .map(|(address, identity)| (address_to_array(address), identity.clone()))
                .collect(),
        };

        ser.serialize(writer)
    }
}

impl BorshDeserialize for BridgeState {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let ser = BridgeStateSer::deserialize_reader(reader)?;

        let mut eth_pending_txs = HashMap::new();
        for (hash_bytes, tx) in ser.eth_pending_txs {
            eth_pending_txs.insert(TxHash::from(hash_bytes), tx);
        }

        let processed_eth_txs = ser
            .processed_eth_txs
            .into_iter()
            .map(TxHash::from)
            .collect();

        let eth_address_bindings = ser
            .eth_address_bindings
            .into_iter()
            .map(|(addr, identity)| (Address::from(addr), identity))
            .collect();

        Ok(BridgeState {
            eth_contract: Address::from(ser.eth_contract),
            eth_contract_vault_address: Address::from(ser.eth_contract_vault_address),
            eth_last_block: ser.eth_last_block,
            eth_pending_txs,
            processed_eth_txs,
            eth_address_bindings,
        })
    }
}
