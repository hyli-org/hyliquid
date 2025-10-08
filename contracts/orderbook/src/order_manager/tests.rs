use super::*;

use std::collections::BTreeMap;

use crate::orderbook::{
    ExecutionMode, ExecutionState, Order, OrderSide, OrderType, Orderbook, OrderbookEvent,
    PairInfo, TokenPair,
};
use crate::smt_values::{Balance, UserInfo};
use sdk::LaneId;

fn test_user(name: &str) -> UserInfo {
    UserInfo::new(name.to_string(), name.as_bytes().to_vec())
}

fn sample_pair() -> TokenPair {
    ("ETH".to_string(), "USDC".to_string())
}

fn build_orderbook() -> Orderbook {
    Orderbook::init(LaneId::default(), ExecutionMode::Full, b"secret".to_vec()).unwrap()
}

fn make_limit_order(id: &str, side: OrderSide, price: u64, quantity: u64) -> Order {
    Order {
        order_id: id.to_string(),
        order_type: OrderType::Limit,
        order_side: side,
        price: Some(price),
        pair: sample_pair(),
        quantity,
    }
}

fn make_market_order(id: &str, side: OrderSide, quantity: u64) -> Order {
    Order {
        order_id: id.to_string(),
        order_type: OrderType::Market,
        order_side: side,
        price: None,
        pair: sample_pair(),
        quantity,
    }
}

#[test]
fn add_session_key_registers_new_key() {
    let mut orderbook = build_orderbook();
    let user = test_user("alice");
    let key = vec![1, 2, 3, 4];

    let events = orderbook
        .add_session_key(user, &key)
        .expect("should add session key");

    let user = orderbook
        .get_user_info("alice")
        .expect("user should exist after adding session key");

    assert_eq!(user.session_keys, vec![key.clone()]);
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0],
        OrderbookEvent::SessionKeyAdded { ref user, .. } if user == "alice"
    ));

    let err = orderbook
        .add_session_key(user, &key)
        .expect_err("duplicate keys must fail");
    assert!(err.contains("already exists"));
}

#[test]
fn create_pair_initializes_balances() {
    let mut orderbook = build_orderbook();
    let pair = sample_pair();
    let info = PairInfo {
        base_scale: 3,
        quote_scale: 2,
    };

    let events = orderbook
        .create_pair(&pair, &info)
        .expect("pair creation should succeed");

    assert!(orderbook.pairs_info.contains_key(&pair));

    match &orderbook.execution_state {
        ExecutionState::Full(state) => {
            assert!(state.balances_mt.contains_key(&pair.0));
            assert!(state.balances_mt.contains_key(&pair.1));
        }
        _ => panic!("Orderbook should be in full execution mode for this test"),
    }
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        OrderbookEvent::PairCreated {
            pair: ref event_pair,
            info: PairInfo {
                base_scale,
                quote_scale,
            },
        } if event_pair == &pair
            && base_scale == info.base_scale
            && quote_scale == info.quote_scale
    ));
}

#[test]
fn deposit_updates_balance_and_event() {
    let mut orderbook = build_orderbook();
    let pair = sample_pair();
    orderbook
        .create_pair(
            &pair,
            &PairInfo {
                base_scale: 3,
                quote_scale: 2,
            },
        )
        .unwrap();
    let user = test_user("bob");

    let events = orderbook
        .deposit(&pair.1, 500, &user)
        .expect("deposit should succeed");

    assert_eq!(orderbook.get_balance(&user, &pair.1).0, 500);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        OrderbookEvent::BalanceUpdated { ref user, ref token, amount }
            if user == "bob" && token == &pair.1 && amount == 500
    ));
}

#[test]
fn withdraw_deducts_balance() {
    let mut orderbook = build_orderbook();
    let pair = sample_pair();
    orderbook
        .create_pair(
            &pair,
            &PairInfo {
                base_scale: 3,
                quote_scale: 2,
            },
        )
        .unwrap();
    let user = test_user("carol");

    orderbook.deposit(&pair.1, 1_000, &user).unwrap();

    let events = orderbook
        .withdraw(&pair.1, &400, &user)
        .expect("withdraw should succeed");

    assert_eq!(orderbook.get_balance(&user, &pair.1).0, 600);
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0],
        OrderbookEvent::BalanceUpdated { ref user, ref token, amount }
            if user == "carol" && token == &pair.1 && amount == 600
    ));

    let err = orderbook
        .withdraw(&pair.1, &700, &user)
        .expect_err("should reject overdraft");
    assert!(err.contains("Insufficient balance"));
}

#[test]
fn cancel_order_refunds_and_removes() {
    let mut orderbook = build_orderbook();
    let pair = sample_pair();
    orderbook
        .create_pair(
            &pair,
            &PairInfo {
                base_scale: 3,
                quote_scale: 2,
            },
        )
        .unwrap();
    let user = test_user("dan");
    let order = make_limit_order("order-1", OrderSide::Bid, 100, 10);

    orderbook
        .order_manager
        .insert_order(&order, &user.get_key())
        .expect("order insertion should succeed");

    let mut balances = BTreeMap::new();
    balances.insert(user.clone(), Balance(0));

    let events = orderbook
        .cancel_order(order.order_id.clone(), &user)
        .expect("cancellation should succeed");

    assert!(orderbook.order_manager.orders.is_empty());
    assert_eq!(orderbook.order_manager.count_buy_orders(&pair), 0);
    assert_eq!(orderbook.order_manager.count_sell_orders(&pair), 0);
    assert_eq!(orderbook.get_balance(&user, &pair.1).0, 10);

    assert!(events.iter().any(|event| matches!(
        event,
        OrderbookEvent::OrderCancelled { order_id, .. } if order_id == "order-1"
    )));
}

#[test]
fn limit_bid_inserts_when_no_liquidity() {
    let mut manager = OrderManager::new();
    let user = test_user("alice");
    let order = make_limit_order("bid-1", OrderSide::Bid, 101, 5);

    let events = manager
        .execute_order(&user.get_key(), &order)
        .expect("order execution should succeed");

    assert_eq!(events.len(), 1);
    assert!(matches!(events[0], OrderbookEvent::OrderCreated { .. }));
    assert_eq!(manager.buy_orders.get(&order.pair).unwrap().len(), 1);
    assert!(manager.orders.contains_key(&order.order_id));
}

#[test]
fn limit_bid_matches_existing_ask() {
    let mut manager = OrderManager::new();
    let maker_user = test_user("maker");
    let taker_user = test_user("taker");

    let resting_order = make_limit_order("ask-1", OrderSide::Ask, 100, 5);
    manager
        .insert_order(&resting_order, &maker_user.get_key())
        .expect("resting ask should be stored");

    let taker_order = make_limit_order("bid-1", OrderSide::Bid, 110, 5);
    let events = manager
        .execute_order(&taker_user.get_key(), &taker_order)
        .expect("matching limit bid should succeed");

    assert!(!manager.orders.contains_key(&taker_order.order_id));
    assert!(manager
        .sell_orders
        .get(&taker_order.pair)
        .unwrap()
        .is_empty());

    assert!(events.iter().any(|event| matches!(
        event,
        OrderbookEvent::OrderExecuted { order_id, taker_order_id, .. }
            if order_id == "ask-1" && taker_order_id == "bid-1"
    )));
}

#[test]
fn limit_bid_inserts_when_price_too_low() {
    let mut manager = OrderManager::new();
    let maker_user = test_user("maker");
    let taker_user = test_user("taker");

    let resting_order = make_limit_order("ask-1", OrderSide::Ask, 120, 5);
    manager
        .insert_order(&resting_order, &maker_user.get_key())
        .expect("resting ask should be stored");

    let taker_order = make_limit_order("bid-1", OrderSide::Bid, 110, 5);
    let events = manager
        .execute_order(&taker_user.get_key(), &taker_order)
        .expect("non crossing bid becomes resting");

    assert!(matches!(
        events.last(),
        Some(OrderbookEvent::OrderCreated { .. })
    ));
    assert!(manager.orders.contains_key(&taker_order.order_id));
    assert_eq!(
        manager
            .buy_orders
            .get(&taker_order.pair)
            .unwrap()
            .first_key_value()
            .map(|(_price, orders)| orders.front().unwrap()),
        Some(&taker_order.order_id)
    );
}

#[test]
fn limit_ask_inserts_when_no_bids() {
    let mut manager = OrderManager::new();
    let user = test_user("frank");
    let order = make_limit_order("ask-1", OrderSide::Ask, 105, 7);

    let events = manager
        .execute_order(&user.get_key(), &order)
        .expect("ask with no bids should rest");

    assert!(matches!(
        events.last(),
        Some(OrderbookEvent::OrderCreated { .. })
    ));
    assert!(manager.orders.contains_key(&order.order_id));
    assert_eq!(
        manager
            .sell_orders
            .get(&order.pair)
            .and_then(|queue| queue.first_key_value())
            .map(|(_price, orders)| orders.front().unwrap()),
        Some(&order.order_id)
    );
}

#[test]
fn limit_ask_matches_existing_bid_partial() {
    let mut manager = OrderManager::new();
    let maker_user = test_user("maker");
    let taker_user = test_user("taker");

    let resting_bid = make_limit_order("bid-1", OrderSide::Bid, 110, 10);
    manager
        .insert_order(&resting_bid, &maker_user.get_key())
        .expect("resting bid should be stored");

    let taker_order = make_limit_order("ask-1", OrderSide::Ask, 100, 6);
    let events = manager
        .execute_order(&taker_user.get_key(), &taker_order)
        .expect("matching ask should succeed");

    let updated_bid = manager.orders.get(&resting_bid.order_id).unwrap();
    assert_eq!(updated_bid.quantity, 4);
    assert!(events.iter().any(|event| matches!(
        event,
        OrderbookEvent::OrderUpdate { order_id, remaining_quantity, .. }
            if order_id == "bid-1" && *remaining_quantity == 4
    )));

    assert!(!manager.orders.contains_key(&taker_order.order_id));
}

#[test]
fn limit_ask_inserts_when_price_above_best_bid() {
    let mut manager = OrderManager::new();
    let maker_user = test_user("maker");
    let taker_user = test_user("taker");

    let resting_bid = make_limit_order("bid-1", OrderSide::Bid, 110, 4);
    manager
        .insert_order(&resting_bid, &maker_user.get_key())
        .expect("resting bid should be stored");

    let taker_order = make_limit_order("ask-1", OrderSide::Ask, 120, 6);
    let events = manager
        .execute_order(&taker_user.get_key(), &taker_order)
        .expect("non crossing ask becomes resting");

    assert!(matches!(
        events.last(),
        Some(OrderbookEvent::OrderCreated { .. })
    ));
    assert!(manager.orders.contains_key(&taker_order.order_id));
    assert_eq!(
        manager
            .sell_orders
            .get(&taker_order.pair)
            .and_then(|queue| queue.first_key_value())
            .map(|(_price, orders)| orders.front().unwrap()),
        Some(&taker_order.order_id)
    );
}

#[test]
fn market_bid_requires_liquidity() {
    let mut manager = OrderManager::new();
    let user = test_user("alice");
    let order = make_market_order("mkt-bid", OrderSide::Bid, 5);

    let err = manager
        .execute_order(&user.get_key(), &order)
        .expect_err("market order without liquidity should fail");
    assert!(err.contains("No matching Bid orders"));
}

#[test]
fn market_bid_consumes_multiple_asks() {
    let mut manager = OrderManager::new();
    let maker1 = test_user("maker1");
    let maker2 = test_user("maker2");
    let taker = test_user("taker");

    manager
        .insert_order(
            &make_limit_order("ask-1", OrderSide::Ask, 90, 3),
            &maker1.get_key(),
        )
        .unwrap();
    manager
        .insert_order(
            &make_limit_order("ask-2", OrderSide::Ask, 95, 4),
            &maker2.get_key(),
        )
        .unwrap();

    let events = manager
        .execute_order(
            &taker.get_key(),
            &make_market_order("bid-1", OrderSide::Bid, 5),
        )
        .expect("market bid should execute against asks");

    assert!(manager.orders.contains_key("ask-2"));
    assert_eq!(manager.orders.get("ask-2").unwrap().quantity, 2);
    assert!(events.iter().any(|event| matches!(
        event,
        OrderbookEvent::OrderExecuted { order_id, taker_order_id, .. }
            if order_id == "ask-1" && taker_order_id == "bid-1"
    )));
}

#[test]
fn market_ask_consumes_bids() {
    let mut manager = OrderManager::new();
    let maker1 = test_user("maker1");
    let maker2 = test_user("maker2");
    let taker = test_user("taker");

    manager
        .insert_order(
            &make_limit_order("bid-1", OrderSide::Bid, 110, 2),
            &maker1.get_key(),
        )
        .unwrap();
    manager
        .insert_order(
            &make_limit_order("bid-2", OrderSide::Bid, 105, 3),
            &maker2.get_key(),
        )
        .unwrap();

    let events = manager
        .execute_order(
            &taker.get_key(),
            &make_market_order("ask-1", OrderSide::Ask, 4),
        )
        .expect("market ask should execute against bids");

    assert!(manager.orders.contains_key("bid-2"));
    assert_eq!(manager.orders.get("bid-2").unwrap().quantity, 1);
    assert!(events.iter().any(|event| matches!(
        event,
        OrderbookEvent::OrderExecuted { order_id, taker_order_id, .. }
            if order_id == "bid-1" && taker_order_id == "ask-1"
    )));
}

#[test]
fn market_ask_without_bids_fails() {
    let mut manager = OrderManager::new();
    let user = test_user("eve");
    let order = make_market_order("ask-1", OrderSide::Ask, 3);

    let err = manager
        .execute_order(&user.get_key(), &order)
        .expect_err("market ask without bids should fail");
    assert!(err.contains("No matching Ask orders"), "{err}");
}

#[test]
fn perf_insert_order_sequential() {
    use std::time::Instant;

    let mut manager = OrderManager::new();
    let user = test_user("perf_user");
    let num_orders = 10_000;

    // Test d'insertion séquentielle (meilleur cas - ordres déjà triés)
    println!("\n=== Performance Test: insert_order (Sequential) ===");
    println!(
        "Inserting {} bid orders in ascending price order",
        num_orders
    );

    let start = Instant::now();
    for i in 0..num_orders {
        let order = make_limit_order(
            &format!("bid-{}", i),
            OrderSide::Bid,
            100 + i as u64, // Prix croissant
            10,
        );
        manager
            .insert_order(&order, &user.get_key())
            .expect("insertion should succeed");
    }
    let duration = start.elapsed();

    println!("Total time: {:?}", duration);
    println!("Average time per insertion: {:?}", duration / num_orders);
    println!(
        "Orders per second: {:.0}",
        num_orders as f64 / duration.as_secs_f64()
    );
    println!("Total orders in book: {}", manager.orders.len());

    assert_eq!(manager.orders.len(), num_orders as usize);
    assert_eq!(
        manager.count_buy_orders(&sample_pair()),
        num_orders as usize
    );
}

#[test]
fn perf_insert_order_reverse() {
    use std::time::Instant;

    let mut manager = OrderManager::new();
    let user = test_user("perf_user");
    let num_orders = 10_000;

    // Test d'insertion en ordre inverse (pire cas - insertion toujours en tête)
    println!("\n=== Performance Test: insert_order (Reverse) ===");
    println!(
        "Inserting {} bid orders in descending price order",
        num_orders
    );

    let start = Instant::now();
    for i in 0..num_orders {
        let order = make_limit_order(
            &format!("bid-{}", i),
            OrderSide::Bid,
            100_000 - i as u64, // Prix décroissant
            10,
        );
        manager
            .insert_order(&order, &user.get_key())
            .expect("insertion should succeed");
    }
    let duration = start.elapsed();

    println!("Total time: {:?}", duration);
    println!("Average time per insertion: {:?}", duration / num_orders);
    println!(
        "Orders per second: {:.0}",
        num_orders as f64 / duration.as_secs_f64()
    );
    println!("Total orders in book: {}", manager.orders.len());

    assert_eq!(manager.orders.len(), num_orders as usize);
    assert_eq!(
        manager.count_buy_orders(&sample_pair()),
        num_orders as usize
    );
}

#[test]
fn perf_insert_order_random() {
    use std::time::Instant;

    let mut manager = OrderManager::new();
    let user = test_user("perf_user");
    let num_orders = 10_000;

    // Test d'insertion aléatoire (cas moyen)
    println!("\n=== Performance Test: insert_order (Random) ===");
    println!("Inserting {} bid orders with random prices", num_orders);

    // Génération de prix pseudo-aléatoires (déterministe pour la reproductibilité)
    let mut prices = Vec::new();
    let mut seed = 12345u64;
    for _ in 0..num_orders {
        seed = (seed.wrapping_mul(1103515245).wrapping_add(12345)) % (1u64 << 31);
        prices.push(1000 + (seed % 90_000));
    }

    let start = Instant::now();
    for (i, price) in prices.iter().enumerate() {
        let order = make_limit_order(&format!("bid-{}", i), OrderSide::Bid, *price, 10);
        manager
            .insert_order(&order, &user.get_key())
            .expect("insertion should succeed");
    }
    let duration = start.elapsed();

    println!("Total time: {:?}", duration);
    println!("Average time per insertion: {:?}", duration / num_orders);
    println!(
        "Orders per second: {:.0}",
        num_orders as f64 / duration.as_secs_f64()
    );
    println!("Total orders in book: {}", manager.orders.len());

    assert_eq!(manager.orders.len(), num_orders as usize);
    assert_eq!(
        manager.count_buy_orders(&sample_pair()),
        num_orders as usize
    );
}

#[test]
fn perf_insert_order_mixed_sides() {
    use std::time::Instant;

    let mut manager = OrderManager::new();
    let user = test_user("perf_user");
    let num_orders_per_side = 5_000;

    // Test d'insertion avec les deux côtés du carnet
    println!("\n=== Performance Test: insert_order (Mixed Sides) ===");
    println!(
        "Inserting {} bid orders and {} ask orders",
        num_orders_per_side, num_orders_per_side
    );

    let start = Instant::now();

    // Insertion de bids
    for i in 0..num_orders_per_side {
        let order = make_limit_order(&format!("bid-{}", i), OrderSide::Bid, 50_000 - i as u64, 10);
        manager
            .insert_order(&order, &user.get_key())
            .expect("insertion should succeed");
    }

    // Insertion d'asks
    for i in 0..num_orders_per_side {
        let order = make_limit_order(&format!("ask-{}", i), OrderSide::Ask, 60_000 + i as u64, 10);
        manager
            .insert_order(&order, &user.get_key())
            .expect("insertion should succeed");
    }

    let duration = start.elapsed();
    let total_orders = num_orders_per_side * 2;

    println!("Total time: {:?}", duration);
    println!("Average time per insertion: {:?}", duration / total_orders);
    println!(
        "Orders per second: {:.0}",
        total_orders as f64 / duration.as_secs_f64()
    );
    println!("Total orders in book: {}", manager.orders.len());
    println!("Bid orders: {}", manager.count_buy_orders(&sample_pair()));
    println!("Ask orders: {}", manager.count_sell_orders(&sample_pair()));

    assert_eq!(manager.orders.len(), total_orders as usize);
    assert_eq!(
        manager.count_buy_orders(&sample_pair()),
        num_orders_per_side as usize
    );
    assert_eq!(
        manager.count_sell_orders(&sample_pair()),
        num_orders_per_side as usize
    );
}
