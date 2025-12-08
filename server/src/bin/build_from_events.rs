use anyhow::{Context, Result};
use clap::{command, Parser};
use client_sdk::rest_client::{NodeApiClient, NodeApiHttpClient};
use hyli_modules::utils::logger::setup_tracing;
use orderbook::{
    model::{ExecuteState, OrderbookEvent, UserInfo},
    zk::FullState,
};
use sdk::{info, BlockHeight, LaneId, StateCommitment};
use server::{init::DebugStateCommitment, setup::setup_database};
use sqlx::{postgres::PgRow, FromRow, Row};
use tracing::{error, warn};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "0")]
    pub commit_id: u32,

    #[arg(long, default_value = "false")]
    pub fast_mode: bool,

    #[arg(
        long,
        default_value = "postgres://postgres:postgres@localhost:5433/hyli-indexer"
    )]
    pub indexer_db_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_tracing("full", "build_from_events".to_string()).unwrap();

    let args = Args::parse();
    let config =
        server::conf::Conf::new(args.config_file.clone()).context("reading config file")?;
    let commit_id = args.commit_id as i64;
    let fast_mode = args.fast_mode;
    let index_database_url = args.indexer_db_url;

    let pool = setup_database(&config, false)
        .await
        .expect("failed to setup database");

    let node_url = config.node_url;
    let node_client = NodeApiHttpClient::new(node_url).unwrap();

    let secret = config.secret.clone();
    let validator_lane_id = node_client
        .get_node_info()
        .await
        .unwrap()
        .pubkey
        .map(LaneId)
        .unwrap();
    let last_block_height = BlockHeight::default();

    let rows =
        sqlx::query("SELECT * FROM contract_events WHERE commit_id <= $1 order by commit_id asc")
            .bind(commit_id)
            .fetch_all(&pool)
            .await
            .expect("failed to fetch events");

    let mut events: Vec<(UserInfo, i64, Vec<OrderbookEvent>)> = Vec::new();
    for row in rows {
        let r: Vec<u8> = row.get("events");
        let orderbook_events: Vec<OrderbookEvent> = borsh::from_slice(&r).expect("invalid events");
        let r: Vec<u8> = row.get("user_info");
        let user_info: UserInfo = borsh::from_slice(&r).expect("invalid user info");
        let r: i64 = row.get("commit_id");
        events.push((user_info, r, orderbook_events));
    }

    let mut commitments = fetch_commitments(index_database_url).await.unwrap();

    info!("Executing {} events", events.len());
    let mut light_state = ExecuteState::default();

    let last_commit_id = commit_id;
    for (user_info, commit_id, events) in events {
        info!("Executing events of commit id: {}", commit_id);
        for event in &events {
            info!("\tEvent: {}", event);
        }

        light_state
            .apply_events(&user_info, &events.clone())
            .unwrap();

        if fast_mode && commit_id <= last_commit_id - 5 {
            commitments.remove(0);
            continue;
        }
        info!("events: {:?}", events);

        let full_orderbook_from_light = FullState::from_data(
            &light_state,
            secret.clone(),
            validator_lane_id.clone(),
            last_block_height,
        )
        .expect("failed to build full state");

        let commitment = full_orderbook_from_light.commit();
        let onchain_commitment = commitments.remove(0);

        let onchain =
            DebugStateCommitment::from(StateCommitment(onchain_commitment.next_state.clone()));
        // Log existing & new orderbook and spot diff
        let rebuilt_from_light_debug =
            DebugStateCommitment::from(full_orderbook_from_light.commit());

        info!("blob_tx_hash: {:?}", onchain_commitment.blob_tx_hash);

        info!(
            "balances_roots.BTC onchain: {:?}",
            onchain.balances_roots.get("BTC")
        );

        info!(
            "balances_roots.BTC rebuilt: {:?}",
            rebuilt_from_light_debug.balances_roots.get("BTC")
        );

        let diff = onchain.diff(&rebuilt_from_light_debug);
        let mut has_diff = false;
        if !diff.is_empty() {
            warn!("⚠️  Differences (onchain vs rebuilt):");
            for (key, value) in diff.iter() {
                warn!("  {}: {}", key, value);
            }

            // info!("onchain state: {:#?}", onchain);
            // info!("db state: {:#?}", db_state);
            // info!("Light state: {:#?}", light_state);

            has_diff = true;
        }

        if commitment.0 != onchain_commitment.next_state {
            error!("Built commitment: {:?}", commitment);
            error!(
                "Onchain commitment: {:?}",
                StateCommitment(onchain_commitment.next_state)
            );
            error!(
                "Initial state: {:?}",
                StateCommitment(onchain_commitment.initial_state)
            );
            panic!("Commitment mismatch");
        }

        if has_diff {
            panic!("Differences found in states, but commitment matches");
        }
        info!("✅ No differences found between onchain and rebuilt");
    }

    info!("Executed all events");

    let full_orderbook =
        FullState::from_data(&light_state, secret, validator_lane_id, last_block_height)
            .expect("failed to build full state");

    server::init::check(&node_client, light_state, full_orderbook)
        .await
        .map_err(|e| anyhow::anyhow!(e.1))
        .unwrap();
    Ok(())
}

#[derive(Debug, Clone)]
struct CommitmentRow {
    initial_state: Vec<u8>,
    next_state: Vec<u8>,
    blob_tx_hash: String,
    block_height: i64,
}

impl std::fmt::Display for CommitmentRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CommitmentRow {{ blob_tx_hash: {}, block_height: {}, initial_state: {}, next_state: {} }}",
            self.blob_tx_hash, self.block_height,
            hex::encode(self.initial_state.as_slice()),
            hex::encode(self.next_state.as_slice())
        )
    }
}

impl FromRow<'_, PgRow> for CommitmentRow {
    fn from_row(row: &PgRow) -> sqlx::Result<Self> {
        let initial_state: String = row.get("initial_state");
        let next_state: String = row.get("next_state");

        let initial_state = serde_json::from_str(&initial_state).expect("invalid initial state");
        let next_state = serde_json::from_str(&next_state).expect("invalid next state");

        let block_height: i64 = row.get("block_height");
        let blob_tx_hash: String = row.get("blob_tx_hash");
        Ok(CommitmentRow {
            initial_state,
            next_state,
            blob_tx_hash,
            block_height,
        })
    }
}

async fn fetch_commitments(index_database_url: String) -> Result<Vec<CommitmentRow>> {
    info!("Connecting to indexer database at {}", index_database_url);
    let pool = sqlx::PgPool::connect(&index_database_url)
        .await
        .expect("failed to connect to database");

    let rows: Vec<CommitmentRow> = sqlx::query_as::<_, CommitmentRow>(
        r#"
        select 
        tx.tx_hash as blob_tx_hash, tx.block_height, 
            bpo.hyli_output->>'initial_state' as initial_state, 
            bpo.hyli_output->>'next_state' as next_state
        from transactions tx
        left join blob_proof_outputs bpo on bpo.blob_tx_hash = tx.tx_hash  
        where bpo.contract_name = 'orderbook'
        order by tx.block_height, tx.index asc;
        "#,
    )
    .fetch_all(&pool)
    .await
    .context("running query")?;

    info!("Fetched {} settlement commitments", rows.len());

    let broken = check_chain_breaks(&rows);
    if !broken.is_empty() {
        warn!("Chain breaks at indices: {:?}", broken);
    }

    Ok(rows)
}

fn check_chain_breaks(rows: &[CommitmentRow]) -> Vec<usize> {
    let mut bad = Vec::new();
    for i in 0..rows.len().saturating_sub(1) {
        if rows[i].next_state != rows[i + 1].initial_state {
            bad.push(i);
        }
    }
    bad
}
