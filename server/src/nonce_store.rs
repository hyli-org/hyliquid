use alloy::primitives::Address;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::PathBuf};

#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    entries: HashMap<String, u64>,
}

/// Simple on-disk tracker that hands out monotonically increasing nonces per
/// signer and chain id. This lets the local reth harness accept transactions
/// even when the upstream RPC would report wildly different nonces.
pub struct NonceStore {
    path: PathBuf,
    ledger: Ledger,
}

impl NonceStore {
    pub fn load(path: PathBuf) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating nonce store directory {:?}", parent))?;
        }

        let ledger = if path.exists() {
            let bytes = fs::read(&path)
                .with_context(|| format!("reading nonce store {}", path.display()))?;
            if bytes.is_empty() {
                Ledger::default()
            } else {
                serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing nonce store {} as json", path.display()))?
            }
        } else {
            Ledger::default()
        };

        Ok(Self { path, ledger })
    }

    pub fn reset(&mut self, key: &str) {
        self.ledger.entries.remove(key);
    }

    pub fn set(&mut self, key: &str, value: u64) {
        self.ledger.entries.insert(key.to_string(), value);
    }

    pub fn get(&self, key: &str) -> Option<u64> {
        self.ledger.entries.get(key).copied()
    }

    pub fn ensure_default(&mut self, key: &str, default: u64) {
        self.ledger
            .entries
            .entry(key.to_string())
            .or_insert(default);
    }

    pub fn next_nonce(&mut self, key: &str) -> u64 {
        let entry = self.ledger.entries.entry(key.to_string()).or_insert(0);
        let current = *entry;
        *entry = entry.saturating_add(1);
        current
    }

    pub fn persist(&self) -> Result<()> {
        let bytes =
            serde_json::to_vec_pretty(&self.ledger).context("serializing nonce ledger to json")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("writing nonce store {}", self.path.display()))?;
        Ok(())
    }

    pub fn key(address: Address, chain_id: u64) -> String {
        format!("{:#x}-{chain_id}", address)
    }
}
