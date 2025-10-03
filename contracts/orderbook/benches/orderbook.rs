use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use orderbook::orderbook::{
    ExecutionMode, Order, OrderSide, OrderType, Orderbook, OrderbookEvent, PairInfo, TokenName,
};
use orderbook::smt_values::UserInfo;
use sdk::LaneId;

fn sample_user(name: &str) -> UserInfo {
    let mut user = UserInfo::new(name.to_string(), name.as_bytes().to_vec());
    user.nonce = 1; // allow inserting directly into the SMT
    user
}

fn register_user(orderbook: &mut Orderbook, user: &UserInfo) {
    orderbook
        .update_user_info_merkle_root(user)
        .expect("user registration succeeds");
}

fn init_full_orderbook() -> (Orderbook, (TokenName, TokenName)) {
    let pair = ("ETH".to_string(), "USDC".to_string());
    let mut orderbook = Orderbook::init(
        LaneId::default(),
        ExecutionMode::Full,
        b"bench-secret".to_vec(),
    )
    .expect("orderbook init");

    orderbook
        .create_pair(
            &pair,
            &PairInfo {
                base_scale: 0,
                quote_scale: 0,
            },
        )
        .expect("pair creation");

    (orderbook, pair)
}

fn make_order(
    order_id: &str,
    side: OrderSide,
    price: u64,
    quantity: u64,
    pair: &(TokenName, TokenName),
) -> Order {
    Order {
        order_id: order_id.to_string(),
        order_type: OrderType::Limit,
        order_side: side,
        price: Some(price),
        pair: pair.clone(),
        quantity,
    }
}

fn setup_order_execution() -> (Orderbook, UserInfo, Order) {
    let (mut orderbook, pair) = init_full_orderbook();
    let user = sample_user("alice");
    register_user(&mut orderbook, &user);

    orderbook
        .deposit(&pair.1, 10_000, &user)
        .expect("quote deposit");

    let order = make_order("bench-order", OrderSide::Bid, 1_000, 5, &pair);
    (orderbook, user, order)
}

fn setup_for_zkvm() -> (Orderbook, UserInfo, Vec<OrderbookEvent>) {
    let (mut orderbook, pair) = init_full_orderbook();
    let maker = sample_user("bob");
    let taker = sample_user("carol");
    register_user(&mut orderbook, &maker);
    register_user(&mut orderbook, &taker);

    orderbook
        .deposit(&pair.1, 200_000, &maker)
        .expect("maker quote deposit");
    orderbook
        .deposit(&pair.0, 10_000, &taker)
        .expect("taker base deposit");

    let maker_order = make_order("maker-bid", OrderSide::Bid, 900, 100, &pair);
    let mut events = orderbook
        .execute_order(&maker, maker_order)
        .expect("maker order");

    let taker_order = make_order("taker-ask", OrderSide::Ask, 880, 40, &pair);
    events.extend(
        orderbook
            .execute_order(&taker, taker_order)
            .expect("taker order"),
    );

    (orderbook, taker, events)
}

fn setup_bulk_orders(count: usize) -> (Orderbook, UserInfo, Vec<Order>) {
    let (mut orderbook, pair) = init_full_orderbook();
    let user = sample_user("ingest-trader");
    register_user(&mut orderbook, &user);

    let price = 100u64;
    let quantity = 1u64;
    let funds = (count as u64 + 5) * price * quantity;
    orderbook
        .deposit(&pair.1, funds, &user)
        .expect("bulk funding");

    let orders = (0..count)
        .map(|i| {
            make_order(
                &format!("ingest-bid-{i}"),
                OrderSide::Bid,
                price,
                quantity,
                &pair,
            )
        })
        .collect();

    (orderbook, user, orders)
}

fn setup_interleaved_orders(
    count: usize,
) -> (Orderbook, UserInfo, UserInfo, Vec<Order>, Vec<Order>) {
    let (mut orderbook, pair) = init_full_orderbook();
    let maker = sample_user("maker-interleave");
    let taker = sample_user("taker-interleave");
    register_user(&mut orderbook, &maker);
    register_user(&mut orderbook, &taker);

    // Fund sufficiently so maker bids can be booked before the matching loop consumes them.
    orderbook
        .deposit(&pair.1, (count as u64 + 5) * 10_000, &maker)
        .expect("maker funding");
    orderbook
        .deposit(&pair.0, (count as u64 + 5) * 2_000, &taker)
        .expect("taker funding");

    let bids = (0..count)
        .map(|i| make_order(&format!("maker-bid-{i}"), OrderSide::Bid, 1_000, 5, &pair))
        .collect();
    let asks = (0..count)
        .map(|i| make_order(&format!("taker-ask-{i}"), OrderSide::Ask, 990, 5, &pair))
        .collect();

    (orderbook, maker, taker, bids, asks)
}

fn setup_balance_ops(op_count: usize) -> (Orderbook, UserInfo, Vec<u64>) {
    let (mut orderbook, pair) = init_full_orderbook();
    let user = sample_user("balance-user");
    register_user(&mut orderbook, &user);
    orderbook
        .deposit(&pair.1, 1_000_000, &user)
        .expect("starter deposit");
    let amounts: Vec<u64> = (0..op_count).map(|i| 1_000 + (i as u64 * 10)).collect();
    (orderbook, user, amounts)
}

fn setup_user_registrations(count: usize) -> (Orderbook, Vec<UserInfo>) {
    let (orderbook, _) = init_full_orderbook();
    let users = (0..count)
        .map(|i| sample_user(&format!("user-{i}")))
        .collect();
    (orderbook, users)
}

fn setup_for_zkvm_scaled(scale: usize) -> (Orderbook, UserInfo, Vec<OrderbookEvent>) {
    let (mut orderbook, pair) = init_full_orderbook();
    let mut events = Vec::new();
    let mut taker_ref = None;

    for i in 0..scale {
        let maker = sample_user(&format!("maker-{i}"));
        let taker = sample_user(&format!("taker-{i}"));
        if taker_ref.is_none() {
            taker_ref = Some(taker.clone());
        }

        register_user(&mut orderbook, &maker);
        register_user(&mut orderbook, &taker);

        orderbook
            .deposit(&pair.1, 150_000, &maker)
            .expect("maker deposit");
        orderbook
            .deposit(&pair.0, 12_000, &taker)
            .expect("taker deposit");

        let maker_order = make_order(&format!("maker-{i}-bid"), OrderSide::Bid, 1_000, 10, &pair);
        events.extend(
            orderbook
                .execute_order(&maker, maker_order)
                .expect("maker order"),
        );

        let taker_order = make_order(&format!("taker-{i}-ask"), OrderSide::Ask, 990, 5, &pair);
        events.extend(
            orderbook
                .execute_order(&taker, taker_order)
                .expect("taker order"),
        );
    }

    (orderbook, taker_ref.expect("at least one taker"), events)
}

fn bench_execute_order(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_execute");
    group.bench_function("limit_order", |b| {
        b.iter_batched(
            setup_order_execution,
            |(mut orderbook, user, order)| {
                orderbook
                    .execute_order(&user, order)
                    .expect("execute order during bench");
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn bench_bulk_ingest(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_ingest");
    for &orders in &[10usize, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("sequential_bids", orders),
            &orders,
            |b, &n| {
                b.iter_batched(
                    || setup_bulk_orders(n),
                    |(mut orderbook, user, orders)| {
                        for order in orders {
                            orderbook
                                .execute_order(&user, order)
                                .expect("bulk order execution");
                        }
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_interleaved_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_matching");
    for &pairs in &[5usize, 20, 50] {
        group.bench_with_input(BenchmarkId::new("bid_vs_ask", pairs), &pairs, |b, &n| {
            b.iter_batched(
                || setup_interleaved_orders(n),
                |(mut orderbook, maker, taker, bids, asks)| {
                    for (bid, ask) in bids.into_iter().zip(asks.into_iter()) {
                        orderbook
                            .execute_order(&maker, bid)
                            .expect("maker order during bench");
                        orderbook
                            .execute_order(&taker, ask)
                            .expect("taker order during bench");
                    }
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_balance_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_balance_ops");
    group.bench_function("deposit_withdraw_cycle", |b| {
        b.iter_batched(
            || setup_balance_ops(50),
            |(mut orderbook, user, amounts)| {
                for amount in &amounts {
                    orderbook.deposit("USDC", *amount, &user).expect("deposit");
                }
                for amount in &amounts {
                    let half = *amount / 2;
                    orderbook.withdraw("USDC", &half, &user).expect("withdraw");
                }
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn bench_user_registration(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_user_reg");
    group.bench_function("register_50_users", |b| {
        b.iter_batched(
            || setup_user_registrations(50),
            |(mut orderbook, users)| {
                for user in users {
                    orderbook
                        .update_user_info_merkle_root(&user)
                        .expect("user registration bench");
                }
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn bench_for_zkvm(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_for_zkvm");
    group.bench_function("witness_generation", |b| {
        b.iter_batched(
            setup_for_zkvm,
            |(orderbook, taker, events)| {
                orderbook
                    .for_zkvm(&taker, &events)
                    .expect("for_zkvm during bench");
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn bench_zkvm_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_for_zkvm_scaled");
    group.measurement_time(std::time::Duration::from_secs(10));
    group.sample_size(50);
    for &scale in &[1usize, 8, 32] {
        group.bench_with_input(BenchmarkId::new("witness_users", scale), &scale, |b, &n| {
            b.iter_batched(
                || setup_for_zkvm_scaled(n),
                |(orderbook, taker, events)| {
                    orderbook
                        .for_zkvm(&taker, &events)
                        .expect("scaled zkvm bench");
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_zkvm_commitment(c: &mut Criterion) {
    let mut group = c.benchmark_group("orderbook_commitment");
    group.bench_function("derive_metadata", |b| {
        b.iter_batched(
            setup_for_zkvm,
            |(orderbook, taker, events)| {
                orderbook
                    .derive_zkvm_commitment_metadata_from_events(&taker, &events)
                    .expect("commitment generation");
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

criterion_group!(
    orderbook_benches,
    bench_execute_order,
    bench_bulk_ingest,
    bench_interleaved_matching,
    bench_balance_operations,
    bench_user_registration,
    bench_for_zkvm,
    bench_zkvm_scaling,
    bench_zkvm_commitment,
);
criterion_main!(orderbook_benches);
