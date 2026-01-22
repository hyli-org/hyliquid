use std::collections::HashMap;

use client_sdk::contract_indexer::AppError;
use sdk::{ContractName, TxHash};
use sqlx::{PgPool, Row};
use tracing::info;

#[derive(Debug)]
pub struct Asset {
    pub asset_id: i64,
    pub contract_name: String,
    pub symbol: String,
    pub scale: i16,
    pub step: i64,
}

#[derive(Debug, Clone, sqlx::Type)]
#[sqlx(type_name = "market_status", rename_all = "lowercase")]
pub enum MarketStatus {
    Active,
    Halted,
    Closed,
}

#[derive(Debug)]
pub struct Instrument {
    pub instrument_id: i64,
    pub symbol: String,
    pub tick_size: i64,
    pub qty_step: i64,
    pub base_asset_id: i64,
    pub quote_asset_id: i64,
    pub status: MarketStatus,
}

pub struct AssetService {
    pool: PgPool,
    asset_map: HashMap<String, Asset>,
    instrument_map: HashMap<String, Instrument>,
}

impl AssetService {
    pub async fn new(pool: PgPool) -> Self {
        let rows = sqlx::query("SELECT * FROM assets")
            .fetch_all(&pool)
            .await
            .unwrap();

        let asset_map: HashMap<String, Asset> = rows
            .iter()
            .map(|row| {
                (
                    row.get("symbol"),
                    Asset {
                        asset_id: row.get("asset_id"),
                        contract_name: row.get("contract_name"),
                        symbol: row.get("symbol"),
                        scale: row.get("scale"),
                        step: row.get("step"),
                    },
                )
            })
            .collect();

        let rows = sqlx::query("SELECT * FROM instruments")
            .fetch_all(&pool)
            .await
            .unwrap();

        let instrument_map: HashMap<String, Instrument> = rows
            .iter()
            .map(|row| {
                (
                    row.get("symbol"),
                    Instrument {
                        instrument_id: row.get("instrument_id"),
                        symbol: row.get("symbol"),
                        tick_size: row.get("tick_size"),
                        qty_step: row.get("qty_step"),
                        base_asset_id: row.get("base_asset_id"),
                        quote_asset_id: row.get("quote_asset_id"),
                        status: row.get("status"),
                    },
                )
            })
            .collect();

        info!(
            "Loaded {} assets and {} instruments into memory",
            asset_map.len(),
            instrument_map.len()
        );

        AssetService {
            pool,
            asset_map,
            instrument_map,
        }
    }

    pub async fn reload_instrument_map(&mut self) -> Result<(), AppError> {
        self.instrument_map = sqlx::query("SELECT * FROM instruments")
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| {
                (
                    row.get("symbol"),
                    Instrument {
                        instrument_id: row.get("instrument_id"),
                        symbol: row.get("symbol"),
                        tick_size: row.get("tick_size"),
                        qty_step: row.get("qty_step"),
                        base_asset_id: row.get("base_asset_id"),
                        quote_asset_id: row.get("quote_asset_id"),
                        status: row.get("status"),
                    },
                )
            })
            .collect();

        Ok(())
    }

    pub fn get_instrument<'a>(&'a self, symbol: &str) -> Option<&'a Instrument> {
        self.instrument_map.get(symbol)
    }

    pub async fn get_all_instruments(
        &self,
        commit_id: i64,
    ) -> Result<HashMap<String, Instrument>, AppError> {
        let rows = sqlx::query("SELECT * FROM instruments where commit_id <= $1")
            .bind(commit_id)
            .fetch_all(&self.pool)
            .await?;

        Ok(rows
            .iter()
            .map(|row| {
                (
                    row.get("symbol"),
                    Instrument {
                        instrument_id: row.get("instrument_id"),
                        symbol: row.get("symbol"),
                        tick_size: row.get("tick_size"),
                        qty_step: row.get("qty_step"),
                        base_asset_id: row.get("base_asset_id"),
                        quote_asset_id: row.get("quote_asset_id"),
                        status: row.get("status"),
                    },
                )
            })
            .collect())
    }

    pub async fn get_all_assets(&self) -> &HashMap<String, Asset> {
        &self.asset_map
    }

    pub async fn add_instrument(&mut self, instrument: Instrument) -> Result<(), AppError> {
        sqlx::query("INSERT INTO instruments (symbol, tick_size, qty_step, base_asset_id, quote_asset_id, status) VALUES ($1, $2, $3, $4, $5, $6, $7)")
            .bind(instrument.symbol.clone())
            .bind(instrument.tick_size)
            .bind(instrument.qty_step)
            .bind(instrument.base_asset_id)
            .bind(instrument.quote_asset_id)
            .bind(instrument.status.clone())
            .execute(&self.pool)
            .await?;

        self.instrument_map
            .insert(instrument.symbol.clone(), instrument);

        Ok(())
    }

    pub async fn get_asset_from_contract_name(&self, contract_name: &str) -> Option<&Asset> {
        self.asset_map
            .values()
            .find(|asset| asset.contract_name == contract_name)
    }

    pub fn get_asset<'a>(&'a self, symbol: &str) -> Option<&'a Asset> {
        self.asset_map.get(symbol)
    }

    pub async fn get_symbol_from_contract_name(&self, contract_name: &str) -> Option<String> {
        self.asset_map
            .values()
            .find(|asset| asset.contract_name == contract_name)
            .map(|asset| asset.symbol.clone())
    }

    pub async fn get_contract_name_from_symbol(&self, symbol: &str) -> Option<ContractName> {
        self.asset_map
            .values()
            .find(|asset| asset.symbol == symbol)
            .map(|asset| asset.contract_name.clone().into())
    }

    pub async fn add_asset(&mut self, asset: Asset) -> Result<(), AppError> {
        sqlx::query("INSERT INTO assets (symbol, scale, step) VALUES ($1, $2, $3)")
            .bind(asset.symbol.clone())
            .bind(asset.scale)
            .bind(asset.step)
            .execute(&self.pool)
            .await?;

        self.asset_map.insert(asset.symbol.clone(), asset);
        Ok(())
    }

    /// Get commit_id from a given tx_hash
    pub async fn get_commit_id_from_tx_hash(&self, tx_hash: &TxHash) -> Option<i64> {
        let row = sqlx::query("SELECT commit_id FROM commits WHERE tx_hash = $1")
            .bind(&tx_hash.0)
            .fetch_one(&self.pool)
            .await
            .ok()?;
        Some(row.get::<i64, _>("commit_id"))
    }

    /// Get last tx_hash in commits table
    /// Used for offline mode to get the last tx_hash from the commit table
    pub async fn get_last_tx_hash_in_commit_table(&self) -> Option<TxHash> {
        let row = sqlx::query("SELECT tx_hash FROM commits ORDER BY commit_id DESC LIMIT 1")
            .fetch_one(&self.pool)
            .await
            .ok()?;
        Some(TxHash(row.get::<Vec<u8>, _>("tx_hash")))
    }
}
