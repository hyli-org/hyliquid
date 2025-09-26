use std::collections::HashMap;

use client_sdk::contract_indexer::AppError;
use sqlx::{PgPool, Row};
use tracing::info;

#[derive(Debug)]
pub struct Asset {
    pub asset_id: i64,
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

    pub async fn get_instrument<'a>(&'a self, symbol: &str) -> Option<&'a Instrument> {
        self.instrument_map.get(symbol)
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

    pub async fn get_asset<'a>(&'a self, symbol: &str) -> Option<&'a Asset> {
        self.asset_map.get(symbol)
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
}
