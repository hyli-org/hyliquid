use alloy::primitives::{Address, TxHash, U256};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::convert::TryInto;

/// Incoming Ethereum transaction (to the bridge)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct EthTransaction {
    pub tx_hash: TxHash,
    pub block_number: u64,
    pub from: Address,
    pub to: Address, // Vault address
    pub amount: U256,
    pub timestamp: u64,
    pub status: TxStatus,
}

/// Transaction status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TxStatus {
    Pending,   // Awaiting confirmation
    Confirmed, // Confirmed on blockchain
}

impl TxStatus {
    fn as_str(&self) -> &'static str {
        match self {
            TxStatus::Pending => "pending",
            TxStatus::Confirmed => "confirmed",
        }
    }
}

impl TryFrom<&str> for TxStatus {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(TxStatus::Pending),
            "confirmed" => Ok(TxStatus::Confirmed),
            other => Err(anyhow!("unknown transaction status: {other}")),
        }
    }
}

#[derive(Clone)]
pub struct BridgeService {
    pool: PgPool,
}

impl BridgeService {
    pub fn new(pool: PgPool) -> Self {
        BridgeService { pool }
    }

    pub async fn ensure_initialized(&self, initial_eth_block: u64) -> Result<()> {
        let block_i64 = i64::try_from(initial_eth_block)
            .context("initial Ethereum block does not fit into i64")?;

        sqlx::query(
            "INSERT INTO bridge_metadata (id, eth_last_block)
             VALUES (0, $1)
             ON CONFLICT (id) DO UPDATE
             SET eth_last_block = GREATEST(bridge_metadata.eth_last_block, EXCLUDED.eth_last_block),
                 updated_at = now()",
        )
        .bind(block_i64)
        .execute(&self.pool)
        .await
        .context("inserting bridge metadata")?;

        Ok(())
    }

    pub async fn eth_last_block(&self) -> Result<u64> {
        let row = sqlx::query("SELECT eth_last_block FROM bridge_metadata WHERE id = 0")
            .fetch_optional(&self.pool)
            .await
            .context("fetching bridge metadata")?;

        let Some(row) = row else {
            return Ok(0);
        };

        let block_number: i64 = row.get("eth_last_block");
        u64::try_from(block_number).context("stored eth_last_block is negative")
    }

    pub async fn record_eth_block(&self, block_number: u64) -> Result<()> {
        let block_i64 =
            i64::try_from(block_number).context("Ethereum block number does not fit in i64")?;

        let updated = sqlx::query(
            "UPDATE bridge_metadata
             SET eth_last_block = GREATEST(eth_last_block, $1),
                 updated_at = now()
             WHERE id = 0",
        )
        .bind(block_i64)
        .execute(&self.pool)
        .await
        .context("updating eth_last_block")?;

        if updated.rows_affected() == 0 {
            // Should not happen if ensure_initialized was called, but handle gracefully.
            self.ensure_initialized(block_number).await?;
        }

        Ok(())
    }

    pub async fn is_eth_pending(&self, tx_hash: &TxHash) -> Result<bool> {
        let exists =
            sqlx::query_scalar::<_, i64>("SELECT 1 FROM bridge_eth_pending_txs WHERE tx_hash = $1")
                .bind(tx_hash_to_vec(tx_hash))
                .fetch_optional(&self.pool)
                .await
                .context("checking pending Ethereum transaction")?;

        Ok(exists.is_some())
    }

    pub async fn is_eth_processed(&self, tx_hash: &TxHash) -> Result<bool> {
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM bridge_eth_processed_txs WHERE tx_hash = $1",
        )
        .bind(tx_hash_to_vec(tx_hash))
        .fetch_optional(&self.pool)
        .await
        .context("checking processed Ethereum transaction")?;

        Ok(exists.is_some())
    }

    pub async fn is_eth_tracked(&self, tx_hash: &TxHash) -> Result<bool> {
        Ok(self.is_eth_pending(tx_hash).await? || self.is_eth_processed(tx_hash).await?)
    }

    pub async fn mark_eth_processed(&self, tx_hash: TxHash) -> Result<()> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .context("starting transaction to mark ETH tx processed")?;

        let hash_vec = tx_hash_to_vec(&tx_hash);

        sqlx::query("DELETE FROM bridge_eth_pending_txs WHERE tx_hash = $1")
            .bind(&hash_vec)
            .execute(&mut *transaction)
            .await
            .context("removing pending Ethereum transaction")?;

        sqlx::query(
            "INSERT INTO bridge_eth_processed_txs (tx_hash)
             VALUES ($1)
             ON CONFLICT (tx_hash) DO NOTHING",
        )
        .bind(&hash_vec)
        .execute(&mut *transaction)
        .await
        .context("recording processed Ethereum transaction")?;

        transaction
            .commit()
            .await
            .context("committing Ethereum transaction processing")?;
        Ok(())
    }

    pub async fn record_eth_identity_binding(
        &self,
        address: Address,
        user_identity: String,
    ) -> Result<()> {
        sdk::info!(
            "Recording ETH address binding: {:?} -> {}",
            address,
            user_identity
        );

        sqlx::query(
            "INSERT INTO bridge_eth_address_bindings (eth_address, user_identity, updated_at)
             VALUES ($1, $2, now())
             ON CONFLICT (eth_address) DO UPDATE
             SET user_identity = EXCLUDED.user_identity,
                 updated_at = now()",
        )
        .bind(address_to_vec(&address))
        .bind(user_identity)
        .execute(&self.pool)
        .await
        .context("recording ETH identity binding")?;

        Ok(())
    }

    pub async fn hyli_identity_for_eth(&self, address: &Address) -> Result<Option<String>> {
        let row = sqlx::query(
            "SELECT user_identity
             FROM bridge_eth_address_bindings
             WHERE eth_address = $1",
        )
        .bind(address_to_vec(address))
        .fetch_optional(&self.pool)
        .await
        .context("fetching identity for ETH address")?;

        Ok(row.map(|row| row.get::<String, _>("user_identity")))
    }

    pub async fn eth_address_for_hyli_identity(&self, identity: &str) -> Result<Option<Address>> {
        let row = sqlx::query(
            "SELECT eth_address
             FROM bridge_eth_address_bindings
             WHERE user_identity = $1
             ORDER BY created_at ASC
             LIMIT 1",
        )
        .bind(identity)
        .fetch_optional(&self.pool)
        .await
        .context("fetching ETH address for identity")?;

        let Some(row) = row else {
            return Ok(None);
        };

        let bytes: Vec<u8> = row.get("eth_address");
        Ok(Some(bytes_to_address(&bytes)?))
    }

    pub async fn add_eth_pending_transaction(&self, tx: EthTransaction) -> Result<bool> {
        if self.is_eth_tracked(&tx.tx_hash).await? {
            return Ok(false);
        }

        self.record_eth_block(tx.block_number).await?;

        sqlx::query(
            "INSERT INTO bridge_eth_pending_txs
                (tx_hash, block_number, from_address, to_address, amount, timestamp, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(tx_hash_to_vec(&tx.tx_hash))
        .bind(i64::try_from(tx.block_number).context("block number does not fit in i64")?)
        .bind(address_to_vec(&tx.from))
        .bind(address_to_vec(&tx.to))
        .bind(u256_to_vec(&tx.amount))
        .bind(i64::try_from(tx.timestamp).context("timestamp does not fit in i64")?)
        .bind(tx.status.as_str())
        .execute(&self.pool)
        .await
        .context("inserting pending Ethereum transaction")?;

        Ok(true)
    }

    pub async fn pending_eth_transactions_for_address(
        &self,
        address: &Address,
    ) -> Result<Vec<EthTransaction>> {
        let rows = sqlx::query(
            "SELECT tx_hash, block_number, from_address, to_address,
                    amount, timestamp, status
             FROM bridge_eth_pending_txs
             WHERE from_address = $1",
        )
        .bind(address_to_vec(address))
        .fetch_all(&self.pool)
        .await
        .context("fetching pending Ethereum transactions for address")?;

        let mut transactions = Vec::with_capacity(rows.len());
        for row in rows {
            let tx_hash_bytes: Vec<u8> = row.get("tx_hash");
            let block_number: i64 = row.get("block_number");
            let from_bytes: Vec<u8> = row.get("from_address");
            let to_bytes: Vec<u8> = row.get("to_address");
            let amount_bytes: Vec<u8> = row.get("amount");
            let timestamp: i64 = row.get("timestamp");
            let status: String = row.get("status");

            transactions.push(EthTransaction {
                tx_hash: bytes_to_tx_hash(&tx_hash_bytes)?,
                block_number: u64::try_from(block_number)
                    .context("stored block number is negative")?,
                from: bytes_to_address(&from_bytes)?,
                to: bytes_to_address(&to_bytes)?,
                amount: bytes_to_u256(&amount_bytes)?,
                timestamp: u64::try_from(timestamp).context("stored timestamp is negative")?,
                status: TxStatus::try_from(status.as_str())?,
            });
        }

        Ok(transactions)
    }

    pub async fn pending_eth_tx_count(&self) -> Result<usize> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bridge_eth_pending_txs")
            .fetch_one(&self.pool)
            .await
            .context("counting pending Ethereum transactions")?;

        usize::try_from(count).context("pending transaction count is negative")
    }
}

fn address_to_vec(address: &Address) -> Vec<u8> {
    address.as_slice().to_vec()
}

fn tx_hash_to_vec(hash: &TxHash) -> Vec<u8> {
    hash.as_slice().to_vec()
}

fn u256_to_vec(value: &U256) -> Vec<u8> {
    value.to_be_bytes::<32>().to_vec()
}

fn bytes_to_address(bytes: &[u8]) -> Result<Address> {
    let array: [u8; 20] = bytes
        .try_into()
        .map_err(|_| anyhow!("address has invalid length {}", bytes.len()))?;
    Ok(Address::from(array))
}

fn bytes_to_tx_hash(bytes: &[u8]) -> Result<TxHash> {
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("tx hash has invalid length {}", bytes.len()))?;
    Ok(TxHash::from(array))
}

fn bytes_to_u256(bytes: &[u8]) -> Result<U256> {
    let array: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("amount has invalid length {}", bytes.len()))?;
    Ok(U256::from_be_bytes(array))
}
