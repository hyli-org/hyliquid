use super::*;

use std::collections::BTreeMap;

use crate::orderbook::{
    Order, OrderSide, OrderType, Orderbook, OrderbookEvent, PairInfo, TokenPair,
};
use crate::smt_values::{Balance, UserInfo};
use sdk::merkle_utils::BorshableMerkleProof;
use sdk::LaneId;
use sparse_merkle_tree::MerkleProof;

fn test_user(name: &str) -> UserInfo {
    UserInfo::new(name.to_string(), name.as_bytes().to_vec())
}

fn empty_proof() -> BorshableMerkleProof {
    BorshableMerkleProof(MerkleProof::new(vec![], vec![]))
}

fn sample_pair() -> TokenPair {
    ("ETH".to_string(), "USDC".to_string())
}

fn build_orderbook() -> Orderbook {
    Orderbook::init(LaneId::default(), true, b"secret".to_vec()).unwrap()
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
    let mut user = test_user("alice");
    let key = vec![1, 2, 3, 4];

    let events = orderbook
        .add_session_key(&mut user, &key)
        .expect("should add session key");

    assert_eq!(user.session_keys, vec![key.clone()]);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        OrderbookEvent::SessionKeyAdded { ref user } if user == "alice"
    ));

    let err = orderbook
        .add_session_key(&mut user, &key)
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
        .create_pair(pair.clone(), info.clone())
        .expect("pair creation should succeed");

    assert!(orderbook.pairs_info.contains_key(&pair));
    assert!(orderbook.balances_mt.contains_key(&pair.0));
    assert!(orderbook.balances_mt.contains_key(&pair.1));
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
            pair.clone(),
            PairInfo {
                base_scale: 3,
                quote_scale: 2,
            },
        )
        .unwrap();
    let user = test_user("bob");
    let mut balance_in_proof = Balance(0);

    let events = orderbook
        .deposit(
            pair.1.clone(),
            500,
            &user,
            &mut balance_in_proof,
            &empty_proof(),
        )
        .expect("deposit should succeed");

    assert_eq!(orderbook.get_balance(&user, &pair.1).0, 500);
    assert_eq!(balance_in_proof.0, 0, "input balance remains a snapshot");
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
            pair.clone(),
            PairInfo {
                base_scale: 3,
                quote_scale: 2,
            },
        )
        .unwrap();
    let user = test_user("carol");

    orderbook
        .deposit(
            pair.1.clone(),
            1_000,
            &user,
            &mut Balance(0),
            &empty_proof(),
        )
        .unwrap();

    let mut balances = BTreeMap::new();
    balances.insert(user.clone(), Balance(1_000));

    let events = orderbook
        .withdraw(pair.1.clone(), 400, &user, &balances, &empty_proof())
        .expect("withdraw should succeed");

    assert_eq!(orderbook.get_balance(&user, &pair.1).0, 600);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0],
        OrderbookEvent::BalanceUpdated { ref user, ref token, amount }
            if user == "carol" && token == &pair.1 && amount == 600
    ));

    balances.insert(user.clone(), Balance(600));

    let err = orderbook
        .withdraw(pair.1.clone(), 700, &user, &balances, &empty_proof())
        .expect_err("should reject overdraft");
    assert!(err.contains("Insufficient balance"));
}

#[test]
fn cancel_order_refunds_and_removes() {
    let mut orderbook = build_orderbook();
    let pair = sample_pair();
    orderbook
        .create_pair(
            pair.clone(),
            PairInfo {
                base_scale: 3,
                quote_scale: 2,
            },
        )
        .unwrap();
    let user = test_user("dan");
    let order = make_limit_order("order-1", OrderSide::Bid, 100, 10);

    orderbook
        .order_manager
        .insert_order(&order, &user)
        .expect("order insertion should succeed");

    let mut balances = BTreeMap::new();
    balances.insert(user.clone(), Balance(0));

    let events = orderbook
        .cancel_order(order.order_id.clone(), &user, &Balance(0), &empty_proof())
        .expect("cancellation should succeed");

    assert!(orderbook.order_manager.orders.is_empty());
    assert!(orderbook
        .order_manager
        .buy_orders
        .get(&pair)
        .map(|queue| queue.is_empty())
        .unwrap_or(true));
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
        .execute_order(&user, &order)
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
        .insert_order(&resting_order, &maker_user)
        .expect("resting ask should be stored");

    let taker_order = make_limit_order("bid-1", OrderSide::Bid, 110, 5);
    let events = manager
        .execute_order(&taker_user, &taker_order)
        .expect("matching limit bid should succeed");

    assert!(!manager.orders.contains_key(&resting_order.order_id));
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
        .insert_order(&resting_order, &maker_user)
        .expect("resting ask should be stored");

    let taker_order = make_limit_order("bid-1", OrderSide::Bid, 110, 5);
    let events = manager
        .execute_order(&taker_user, &taker_order)
        .expect("non crossing bid becomes resting");

    assert!(matches!(
        events.last(),
        Some(OrderbookEvent::OrderCreated { .. })
    ));
    assert!(manager.orders.contains_key(&taker_order.order_id));
    assert_eq!(
        manager.buy_orders.get(&taker_order.pair).unwrap().front(),
        Some(&taker_order.order_id)
    );
}

#[test]
fn limit_ask_inserts_when_no_bids() {
    let mut manager = OrderManager::new();
    let user = test_user("frank");
    let order = make_limit_order("ask-1", OrderSide::Ask, 105, 7);

    let events = manager
        .execute_order(&user, &order)
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
            .and_then(|queue| queue.front()),
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
        .insert_order(&resting_bid, &maker_user)
        .expect("resting bid should be stored");

    let taker_order = make_limit_order("ask-1", OrderSide::Ask, 100, 6);
    let events = manager
        .execute_order(&taker_user, &taker_order)
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
        .insert_order(&resting_bid, &maker_user)
        .expect("resting bid should be stored");

    let taker_order = make_limit_order("ask-1", OrderSide::Ask, 120, 6);
    let events = manager
        .execute_order(&taker_user, &taker_order)
        .expect("non crossing ask becomes resting");

    assert!(matches!(
        events.last(),
        Some(OrderbookEvent::OrderCreated { .. })
    ));
    assert!(manager.orders.contains_key(&taker_order.order_id));
    assert_eq!(
        manager.sell_orders.get(&taker_order.pair).unwrap().front(),
        Some(&taker_order.order_id)
    );
}

#[test]
fn market_bid_requires_liquidity() {
    let mut manager = OrderManager::new();
    let user = test_user("alice");
    let order = make_market_order("mkt-bid", OrderSide::Bid, 5);

    let err = manager
        .execute_order(&user, &order)
        .expect_err("market order without liquidity should fail");
    assert!(err.contains("No matching sell orders"));
}

#[test]
fn market_bid_consumes_multiple_asks() {
    let mut manager = OrderManager::new();
    let maker1 = test_user("maker1");
    let maker2 = test_user("maker2");
    let taker = test_user("taker");

    manager
        .insert_order(&make_limit_order("ask-1", OrderSide::Ask, 90, 3), &maker1)
        .unwrap();
    manager
        .insert_order(&make_limit_order("ask-2", OrderSide::Ask, 95, 4), &maker2)
        .unwrap();

    let events = manager
        .execute_order(&taker, &make_market_order("bid-1", OrderSide::Bid, 5))
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
        .insert_order(&make_limit_order("bid-1", OrderSide::Bid, 110, 2), &maker1)
        .unwrap();
    manager
        .insert_order(&make_limit_order("bid-2", OrderSide::Bid, 105, 3), &maker2)
        .unwrap();

    let events = manager
        .execute_order(&taker, &make_market_order("ask-1", OrderSide::Ask, 4))
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
        .execute_order(&user, &order)
        .expect_err("market ask without bids should fail");
    assert!(err.contains("No matching buy orders"));
}
