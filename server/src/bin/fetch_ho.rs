use std::env;

use anyhow::Context;
use hyli_modules::utils::logger::setup_tracing;
use orderbook::model::OrderbookEvent;
use sdk::{info, HyliOutput};
use sqlx::Row;

#[tokio::main]
async fn main() {
    setup_tracing("full", "fetch_ho".to_string()).unwrap();

    let mut args = env::args().skip(1);
    let proof_tx_hash = args.next().expect("usage: fetch_ho <proof_tx_hash>");

    let database_url = std::env::var("HYLI_INDEXER_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/hyli-indexer".to_string());
    info!("Connecting to indexer database at {}", database_url);
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("failed to connect to database");

    let rows = sqlx::query(
        r#"
select * from blob_proof_outputs bpo where proof_tx_hash=$1;
        "#,
    )
    .bind(proof_tx_hash)
    .fetch_all(&pool)
    .await
    .context("running query")
    .unwrap();

    let hyli_output: sqlx::types::Json<HyliOutput> = rows[0].get("hyli_output");
    let hyli_output = hyli_output.0;

    let res: Vec<OrderbookEvent> =
        borsh::from_slice(&hyli_output.program_outputs).expect("invalid program outputs");

    info!("Program outputs: {:?}", res);
    info!("Hyli output: {:?}", hyli_output);
}
