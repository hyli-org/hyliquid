use anyhow::{Context, Result};
use clap::{command, Parser};
use client_sdk::rest_client::{NodeApiClient, NodeApiHttpClient};
use hyli_modules::utils::logger::setup_tracing;
use orderbook::{
    model::{ExecuteState, OrderbookEvent, UserInfo},
    zk::FullState,
};
use sdk::{info, BlockHeight, LaneId};
use server::setup::setup_database;
use sqlx::{postgres::PgRow, FromRow, Row};
use tracing::warn;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(long, default_value = "config.toml")]
    pub config_file: Vec<String>,

    #[arg(long, default_value = "0")]
    pub commit_id: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_tracing("full", "build_from_events_bisect".to_string()).unwrap();

    let args = Args::parse();
    let config =
        server::conf::Conf::new(args.config_file.clone()).context("reading config file")?;
    let commit_id = args.commit_id as i64;
    let index_database_url = config.indexer_database_url.clone();

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

    let commitments = fetch_commitments(index_database_url).await.unwrap();

    if commitments.len() != events.len() {
        warn!(
            "Commitment count ({}) != event batch count ({}). Bisect may be unreliable.",
            commitments.len(),
            events.len()
        );
    }

    let first_bad = bisect_first_mismatch(
        &events,
        &commitments,
        &secret,
        &validator_lane_id,
        last_block_height,
    )?;
    match first_bad {
        Some(index) => {
            let (_, commit_id, _) = &events[index];
            warn!(
                "First mismatch at index {} (commit_id: {}).",
                index, commit_id
            );
            if let Some(onchain) = commitments.get(index) {
                warn!("Onchain blob_tx_hash: {}", onchain.blob_tx_hash);
            }
        }
        None => {
            info!("No mismatches found in range.");
        }
    }

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
            self.blob_tx_hash,
            self.block_height,
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

fn bisect_first_mismatch(
    events: &[(UserInfo, i64, Vec<OrderbookEvent>)],
    commitments: &[CommitmentRow],
    secret: &[u8],
    validator_lane_id: &LaneId,
    last_block_height: BlockHeight,
) -> Result<Option<usize>> {
    if events.is_empty() || commitments.is_empty() {
        return Ok(None);
    }

    let mut low = 0usize;
    let mut high = events.len().saturating_sub(1);
    let mut first_bad = None;

    while low <= high {
        let mid = low + (high - low) / 2;
        let matches = commitment_matches_at(
            mid,
            events,
            commitments,
            secret,
            validator_lane_id,
            last_block_height,
        )?;

        if matches {
            if mid == events.len() - 1 {
                break;
            }
            low = mid + 1;
        } else {
            first_bad = Some(mid);
            if mid == 0 {
                break;
            }
            high = mid - 1;
        }
    }

    Ok(first_bad)
}

fn commitment_matches_at(
    index: usize,
    events: &[(UserInfo, i64, Vec<OrderbookEvent>)],
    commitments: &[CommitmentRow],
    secret: &[u8],
    validator_lane_id: &LaneId,
    last_block_height: BlockHeight,
) -> Result<bool> {
    let mut light_state = ExecuteState::default();
    let max_index = index.min(events.len().saturating_sub(1));
    for (i, (user_info, _, batch)) in events.iter().enumerate() {
        light_state.apply_events(user_info, batch).unwrap();
        if i == max_index {
            break;
        }
    }

    let full_orderbook_from_light = FullState::from_data(
        &light_state,
        secret.to_vec(),
        validator_lane_id.clone(),
        last_block_height,
    )
    .expect("failed to build full state");

    let commitment = full_orderbook_from_light.commit();
    let onchain_commitment = commitments
        .get(max_index)
        .expect("missing onchain commitment for index");

    Ok(commitment.0 == onchain_commitment.next_state)
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
