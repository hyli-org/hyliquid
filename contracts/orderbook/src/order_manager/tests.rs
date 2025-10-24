use super::*;

use std::collections::BTreeMap;

use borsh::BorshSerialize;
use k256::ecdsa::signature::DigestSigner;
use k256::ecdsa::{Signature, SigningKey};
use sha3::{Digest, Sha3_256};

use crate::model::WithdrawDestination;
use crate::zk::smt::GetKey;
use crate::{
    model::{
        AssetInfo, Balance, ExecuteState, Order, OrderSide, OrderType, OrderbookEvent, Pair,
        PairInfo, UserInfo,
    },
    transaction::{
        AddSessionKeyPrivateInput, CreateOrderPrivateInput, PermissionnedOrderbookAction,
        WithdrawPrivateInput,
    },
    zk::FullState,
};
use sdk::{BlockHeight, ContractName, LaneId};

fn test_user(name: &str) -> UserInfo {
    UserInfo::new(name.to_string(), name.as_bytes().to_vec())
}

fn sample_pair() -> Pair {
    ("ETH".to_string(), "USDC".to_string())
}

fn make_pair_info(pair: &Pair, base_scale: u64, quote_scale: u64) -> PairInfo {
    PairInfo {
        base: AssetInfo::new(base_scale, ContractName(pair.0.clone())),
        quote: AssetInfo::new(quote_scale, ContractName(pair.1.clone())),
    }
}

fn build_orderbook() -> FullState {
    let light = ExecuteState::default();
    FullState::from_data(
        &light,
        b"secret".to_vec(),
        LaneId::default(),
        BlockHeight(0),
    )
    .expect("Failed to create FullState in test")
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

struct TestSigner {
    signing_key: SigningKey,
    public_key: Vec<u8>,
}

impl TestSigner {
    fn new(seed: u8) -> Self {
        let field_bytes = k256::FieldBytes::from([seed; 32]);
        let signing_key = SigningKey::from_bytes(&field_bytes).expect("signing key");
        let public_key = signing_key
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        Self {
            signing_key,
            public_key,
        }
    }

    fn sign(&self, msg: &str) -> Vec<u8> {
        let mut hasher = Sha3_256::new();
        hasher.update(msg.as_bytes());
        let signature: Signature = self.signing_key.sign_digest(hasher);
        signature.to_vec()
    }
}

fn serialize<T: BorshSerialize>(value: &T) -> Vec<u8> {
    borsh::to_vec(value).expect("serialize private input")
}

fn apply_user_updates(user: &mut UserInfo, events: &[OrderbookEvent]) {
    for event in events {
        match event {
            OrderbookEvent::SessionKeyAdded { session_keys, .. } => {
                user.session_keys = session_keys.clone();
            }
            OrderbookEvent::NonceIncremented { nonce, .. } => {
                user.nonce = *nonce;
            }
            _ => {}
        }
    }
}

fn execute_action_ok(
    orderbook: &mut FullState,
    user: &mut UserInfo,
    action: PermissionnedOrderbookAction,
    private_input: Vec<u8>,
) -> Vec<OrderbookEvent> {
    let events = orderbook
        .state
        .generate_permissionned_execution_events(user, action, &private_input)
        .expect("failed to generate execution events");

    orderbook
        .apply_events_and_update_roots(user, events.clone())
        .expect("action should succeed");
    apply_user_updates(user, &events);
    events
}

fn execute_action_err(
    orderbook: &mut FullState,
    user: &UserInfo,
    action: PermissionnedOrderbookAction,
    private_input: Vec<u8>,
) -> String {
    orderbook
        .state
        .generate_permissionned_execution_events(user, action, &private_input)
        .expect_err("action should fail")
}

#[test]
fn add_session_key_registers_new_key() {
    let mut orderbook = build_orderbook();
    let mut user = test_user("alice");
    let signer = TestSigner::new(1);
    let key = signer.public_key.clone();

    let private_input = serialize(&AddSessionKeyPrivateInput {
        new_public_key: key.clone(),
    });
    let events = execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::AddSessionKey,
        private_input,
    );

    let user = orderbook
        .state
        .get_user_info("alice")
        .expect("user should exist after adding session key");

    assert_eq!(user.session_keys, vec![key.clone()]);
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0],
        OrderbookEvent::SessionKeyAdded { ref user, .. } if user == "alice"
    ));
    let err = orderbook
        .state
        .generate_permissionned_execution_events(
            &user,
            PermissionnedOrderbookAction::AddSessionKey,
            &serialize(&AddSessionKeyPrivateInput {
                new_public_key: key,
            }),
        )
        .expect_err("duplicate keys must fail");
    assert!(err.contains("already exists"));
}

#[test]
fn create_pair_initializes_balances() {
    let mut orderbook = build_orderbook();
    let mut user = test_user("alice");
    let pair = sample_pair();
    let info = make_pair_info(&pair, 3, 2);

    let events = execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: info.clone(),
        },
        Vec::new(),
    );

    let base_symbol_info = orderbook
        .state
        .assets_info
        .get(&pair.0)
        .expect("base symbol must be registered");
    assert_eq!(base_symbol_info.scale, info.base.scale);
    assert_eq!(base_symbol_info.contract_name, info.base.contract_name);

    let quote_symbol_info = orderbook
        .state
        .assets_info
        .get(&pair.1)
        .expect("quote symbol must be registered");
    assert_eq!(quote_symbol_info.scale, info.quote.scale);
    assert_eq!(quote_symbol_info.contract_name, info.quote.contract_name);

    assert!(orderbook.balances_mt.contains_key(&pair.0));
    assert!(orderbook.balances_mt.contains_key(&pair.1));

    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        OrderbookEvent::PairCreated {
            pair: ref event_pair,
            info: ref created_info,
        } if event_pair == &pair && created_info == &info
    ));
}

#[test]
fn create_pair_rejects_conflicting_symbol_registration() {
    let mut orderbook = build_orderbook();
    let mut user = test_user("alice");
    let pair = sample_pair();
    let info = make_pair_info(&pair, 3, 2);

    execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: info.clone(),
        },
        Vec::new(),
    );

    let mut conflicting_info = make_pair_info(&pair, 3, 2);
    conflicting_info.base.contract_name = ContractName("alt-base".to_string());

    let err = execute_action_err(
        &mut orderbook,
        &user,
        PermissionnedOrderbookAction::CreatePair {
            pair,
            info: conflicting_info,
        },
        Vec::new(),
    );
    assert!(err.contains("already registered"));
}

#[test]
fn create_pair_merges_metadata_without_overrides() {
    let mut orderbook = build_orderbook();
    let mut user = test_user("alice");
    let pair = sample_pair();

    let first_info = make_pair_info(&pair, 3, 2);

    execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: first_info.clone(),
        },
        Vec::new(),
    );

    let second_info = make_pair_info(&pair, 3, 2);

    execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::CreatePair {
            pair,
            info: second_info,
        },
        Vec::new(),
    );
}

#[test_log::test]
fn deposit_updates_balance_and_event() {
    let mut orderbook = build_orderbook();
    let pair = sample_pair();
    let mut user = test_user("bob");

    execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: make_pair_info(&pair, 3, 2),
        },
        Vec::new(),
    );

    let events = execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::Deposit {
            symbol: pair.1.clone(),
            amount: 500,
        },
        Vec::new(),
    );

    assert_eq!(orderbook.state.get_balance(&user, &pair.1).0, 500);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        OrderbookEvent::BalanceUpdated { ref user, ref symbol, amount }
            if user == "bob" && symbol == &pair.1 && amount == 500
    ));
}

#[test]
fn withdraw_deducts_balance() {
    let mut orderbook = build_orderbook();
    let pair = sample_pair();
    let mut user = test_user("carol");
    let signer = TestSigner::new(2);
    let session_key = signer.public_key.clone();

    execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::AddSessionKey,
        serialize(&AddSessionKeyPrivateInput {
            new_public_key: session_key.clone(),
        }),
    );

    execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: make_pair_info(&pair, 3, 2),
        },
        Vec::new(),
    );

    execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::Deposit {
            symbol: pair.1.clone(),
            amount: 1_000,
        },
        Vec::new(),
    );

    let destination = WithdrawDestination {
        network: "hyli".to_string(),
        address: "dest-address".to_string(),
    };
    let withdraw_message = format!("{}:{}:withdraw:{}:{}", user.user, user.nonce, pair.1, 400);
    let withdraw_events = execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::Withdraw {
            symbol: pair.1.clone(),
            amount: 400,
            destination: destination.clone(),
        },
        serialize(&WithdrawPrivateInput {
            signature: signer.sign(&withdraw_message),
            public_key: session_key.clone(),
        }),
    );

    assert_eq!(orderbook.state.get_balance(&user, &pair.1).0, 600);
    assert_eq!(withdraw_events.len(), 2);
    assert!(matches!(
        withdraw_events[0],
        OrderbookEvent::BalanceUpdated { ref user, ref symbol, amount }
            if user == "carol" && symbol == &pair.1 && amount == 600
    ));

    let overdraft_message = format!("{}:{}:withdraw:{}:{}", user.user, user.nonce, pair.1, 700);
    let err = execute_action_err(
        &mut orderbook,
        &user,
        PermissionnedOrderbookAction::Withdraw {
            symbol: pair.1.clone(),
            amount: 700,
            destination,
        },
        serialize(&WithdrawPrivateInput {
            signature: signer.sign(&overdraft_message),
            public_key: session_key,
        }),
    );
    assert!(err.contains("Insufficient balance"));
}

#[test]
fn cancel_order_refunds_and_removes() {
    let mut orderbook = build_orderbook();
    let pair = sample_pair();
    let mut user = test_user("dan");
    let signer = TestSigner::new(3);
    let session_key = signer.public_key.clone();

    execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::AddSessionKey,
        serialize(&AddSessionKeyPrivateInput {
            new_public_key: session_key.clone(),
        }),
    );

    execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: make_pair_info(&pair, 3, 2),
        },
        Vec::new(),
    );

    orderbook
        .state
        .users_info
        .insert(user.user.clone(), user.clone());
    let order = make_limit_order("order-1", OrderSide::Bid, 100, 10);

    orderbook
        .state
        .order_manager
        .insert_order(&order, &user.get_key())
        .expect("order insertion should succeed");

    let mut balances = BTreeMap::new();
    balances.insert(user.clone(), Balance(0));

    let cancel_message = format!("{}:{}:cancel:{}", user.user, user.nonce, order.order_id);
    let events = execute_action_ok(
        &mut orderbook,
        &mut user,
        PermissionnedOrderbookAction::Cancel {
            order_id: order.order_id.clone(),
        },
        serialize(&CreateOrderPrivateInput {
            signature: signer.sign(&cancel_message),
            public_key: session_key,
        }),
    );

    assert!(orderbook.state.order_manager.orders.is_empty());
    assert_eq!(orderbook.state.order_manager.count_buy_orders(&pair), 0);
    assert_eq!(orderbook.state.order_manager.count_sell_orders(&pair), 0);
    assert_eq!(orderbook.state.get_balance(&user, &pair.1).0, 10);

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
    assert_eq!(manager.bid_orders.get(&order.pair).unwrap().len(), 1);
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
    assert!(!manager.ask_orders.contains_key(&taker_order.pair));

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
            .bid_orders
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
            .ask_orders
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
            .ask_orders
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
    println!("Inserting {num_orders} bid orders in ascending price order");

    let start = Instant::now();
    for i in 0..num_orders {
        let order = make_limit_order(
            &format!("bid-{i}"),
            OrderSide::Bid,
            100 + i as u64, // Prix croissant
            10,
        );
        manager
            .insert_order(&order, &user.get_key())
            .expect("insertion should succeed");
    }
    let duration = start.elapsed();

    println!("Total time: {duration:?}");
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
    println!("Inserting {num_orders} bid orders in descending price order");

    let start = Instant::now();
    for i in 0..num_orders {
        let order = make_limit_order(
            &format!("bid-{i}"),
            OrderSide::Bid,
            100_000 - i as u64, // Prix décroissant
            10,
        );
        manager
            .insert_order(&order, &user.get_key())
            .expect("insertion should succeed");
    }
    let duration = start.elapsed();

    println!("Total time: {duration:?}");
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
    println!("Inserting {num_orders} bid orders with random prices");

    // Génération de prix pseudo-aléatoires (déterministe pour la reproductibilité)
    let mut prices = Vec::new();
    let mut seed = 12345u64;
    for _ in 0..num_orders {
        seed = (seed.wrapping_mul(1103515245).wrapping_add(12345)) % (1u64 << 31);
        prices.push(1000 + (seed % 90_000));
    }

    let start = Instant::now();
    for (i, price) in prices.iter().enumerate() {
        let order = make_limit_order(&format!("bid-{i}"), OrderSide::Bid, *price, 10);
        manager
            .insert_order(&order, &user.get_key())
            .expect("insertion should succeed");
    }
    let duration = start.elapsed();

    println!("Total time: {duration:?}");
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
    println!("Inserting {num_orders_per_side} bid orders and {num_orders_per_side} ask orders");

    let start = Instant::now();

    // Insertion de bids
    for i in 0..num_orders_per_side {
        let order = make_limit_order(&format!("bid-{i}"), OrderSide::Bid, 50_000 - i as u64, 10);
        manager
            .insert_order(&order, &user.get_key())
            .expect("insertion should succeed");
    }

    // Insertion d'asks
    for i in 0..num_orders_per_side {
        let order = make_limit_order(&format!("ask-{i}"), OrderSide::Ask, 60_000 + i as u64, 10);
        manager
            .insert_order(&order, &user.get_key())
            .expect("insertion should succeed");
    }

    let duration = start.elapsed();
    let total_orders = num_orders_per_side * 2;

    println!("Total time: {duration:?}");
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
