use std::env;

use hyli_modules::utils::logger::setup_tracing;
use orderbook::model::{ExecuteState, OrderbookEvent, UserInfo};
use sdk::info;
use sqlx::Row;

#[tokio::main]
async fn main() {
    setup_tracing("full", "build_from_events".to_string()).unwrap();

    let mut args = env::args().skip(1);
    let commit_id = args.next().expect("usage: build_from_events <commit_id>");
    let database_url = std::env::var("DATABASE_URL")
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

    let mut light_state = ExecuteState::default();
    for (user_info, events) in events {
        light_state.apply_events(&user_info, &events).unwrap();
    }

    println!("{light_state:#?}");
}
