use std::env;

use client_sdk::rest_client::{NodeApiClient, NodeApiHttpClient};
use hyli_modules::utils::logger::setup_tracing;
use orderbook::{
    model::{ExecuteState, OrderbookEvent, UserInfo},
    zk::FullState,
};
use sdk::{info, BlockHeight, LaneId};
use sqlx::Row;
use tracing::error;

#[tokio::main]
async fn main() {
    setup_tracing("full", "build_from_events".to_string()).unwrap();

    let mut args = env::args().skip(1);
    let commit_id = args.next().expect("usage: build_from_events <commit_id>");
    let database_url = std::env::var("HYLI_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/orderbook".to_string());

    info!("Connecting to database at {}", database_url);

    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .expect("failed to connect to database");

    let commit_id = commit_id.parse::<i64>().expect("invalid commit id");

    let rows =
        sqlx::query("SELECT * FROM contract_events WHERE commit_id <= $1 order by commit_id asc")
            .bind(commit_id)
            .fetch_all(&pool)
            .await
            .expect("failed to fetch events");

    let mut events: Vec<(UserInfo, Vec<OrderbookEvent>)> = Vec::new();
    for row in rows {
        let r: Vec<u8> = row.get("events");
        let orderbook_events: Vec<OrderbookEvent> = borsh::from_slice(&r).expect("invalid events");
        let r: Vec<u8> = row.get("user_info");
        let user_info: UserInfo = borsh::from_slice(&r).expect("invalid user info");
        events.push((user_info, orderbook_events));
    }

    info!("Executing {} events", events.len());
    let mut light_state = ExecuteState::default();
    let mut i = 0;
    for (user_info, events) in events {
        light_state.apply_events(&user_info, &events).unwrap();
        i += 1;
        if i % 1000 == 0 {
            info!("Executed {} events", i);
        }
    }
    info!("Executed all events");

    let node_url =
        std::env::var("HYLI_NODE_URL").unwrap_or_else(|_| "http://localhost:4321".to_string());
    let node_client = NodeApiHttpClient::new(node_url).unwrap();

    let secret = vec![1, 2, 3];
    let validator_lane_id = node_client
        .get_node_info()
        .await
        .unwrap()
        .pubkey
        .map(LaneId)
        .unwrap();
    let last_block_height = BlockHeight::default();

    let full_orderbook =
        FullState::from_data(&light_state, secret, validator_lane_id, last_block_height)
            .expect("failed to build full state");

    server::init::check(&node_client, light_state, full_orderbook)
        .await
        .map_err(|e| anyhow::anyhow!(e.1))
        .unwrap();
}
