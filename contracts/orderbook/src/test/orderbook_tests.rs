#![cfg(test)]

use std::collections::{HashMap, HashSet};

use crate::orderbook::{
    ExecutionMode, Order, OrderSide, OrderType, Orderbook, PairInfo, TokenPair,
};
use crate::smt_values::UserInfo;
use crate::{
    AddSessionKeyPrivateInput, CreateOrderPrivateInput, OrderbookAction,
    PermissionnedOrderbookAction, PermissionnedPrivateInput,
};
use k256::ecdsa::signature::DigestSigner;
use k256::ecdsa::{Signature, SigningKey};
use sdk::{guest, LaneId};
use sdk::{tracing, ZkContract};
use sdk::{BlobIndex, Calldata, ContractName, Identity, TxContext, TxHash};
use sha3::{Digest, Sha3_256};

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

fn test_user(name: &str) -> UserInfo {
    UserInfo::new(name.to_string(), name.as_bytes().to_vec())
}

fn get_ctx() -> (ContractName, Identity, TxContext, LaneId, Vec<u8>) {
    let cn: ContractName = ContractName("orderbook".to_owned());
    let id: Identity = Identity::from("orderbook@orderbook");
    let lane_id = LaneId::default();
    let tx_ctx: TxContext = TxContext {
        lane_id: lane_id.clone(),
        ..Default::default()
    };
    let secret: Vec<u8> = b"test-secret".to_vec();
    (cn, id, tx_ctx, lane_id, secret)
}

fn run_action(
    light: &mut Orderbook,
    full: &mut Orderbook,
    user: &str,
    action: PermissionnedOrderbookAction,
    private_payload: Vec<u8>,
) {
    let (cn, id, tx_ctx, _, secret) = get_ctx();

    let user_info = light
        .get_user_info(user)
        .unwrap_or_else(|_| test_user(user));

    let events = light
        .execute_permissionned_action(user_info.clone(), action.clone(), &private_payload)
        .expect("light execution");

    tracing::debug!("light events: {:?}", events);

    let commitment_metadata = full
        .derive_zkvm_commitment_metadata_from_events(&user_info, &events, &action)
        .expect("derive metadata");

    let events_full = full
        .execute_permissionned_action(user_info.clone(), action.clone(), &private_payload)
        .expect("full execution");

    tracing::debug!("full events: {:?}\n\n\n", events_full);
    assert!(
        events.len() == events_full.len(),
        "light and full events should match"
    );

    let permissioned_private_input = PermissionnedPrivateInput {
        secret: secret.to_vec(),
        user_info: user_info.clone(),
        private_input: private_payload,
    };

    let calldata = Calldata {
        identity: id.clone(),
        blobs: vec![OrderbookAction::PermissionnedOrderbookAction(action).as_blob(cn.clone())]
            .into(),
        tx_blob_count: 1,
        index: BlobIndex(0),
        tx_hash: TxHash::from("test-tx-hash"),
        tx_ctx: Some(tx_ctx.clone()),
        private_input: borsh::to_vec(&permissioned_private_input).expect("serialize private input"),
    };

    let res = guest::execute::<Orderbook>(&commitment_metadata, &[calldata]);

    assert!(res.len() == 1, "expected one output");
    let hyli_output = &res[0];
    assert!(hyli_output.success, "execution failed");

    assert!(
        hyli_output.next_state == full.commit(),
        "Full next state mismatch"
    );
}

#[derive(Default, Clone, Copy)]
struct BalanceExpectation {
    base: i128,
    quote: i128,
}

#[derive(Clone, Copy)]
struct BalanceDelta<'a> {
    user: &'a str,
    base: i128,
    quote: i128,
}

#[derive(Clone)]
struct OrderSpec {
    id: &'static str,
    side: OrderSide,
    price: Option<u64>,
    quantity: u64,
}

fn delta<'a>(user: &'a str, base: i128, quote: i128) -> BalanceDelta<'a> {
    BalanceDelta { user, base, quote }
}

fn qty(amount: u64) -> i128 {
    amount as i128
}

fn notional(amount: u64, price: u64) -> i128 {
    i128::from(amount) * i128::from(price)
}
fn apply_balance_deltas<'a>(
    expected: &mut HashMap<&'a str, BalanceExpectation>,
    deltas: &[BalanceDelta<'a>],
) {
    for delta in deltas {
        let entry = expected.get_mut(delta.user).expect("balance entry");
        entry.base += delta.base;
        entry.quote += delta.quote;
    }
}

#[track_caller]
fn assert_stage<'a>(
    stage: &str,
    light: &Orderbook,
    full: &Orderbook,
    expected: &HashMap<&'a str, BalanceExpectation>,
    users: &[&'a str],
    base_token: &str,
    quote_token: &str,
) {
    for &user in users {
        let expected_entry = expected.get(user).expect("expected balances");
        let expected_base: u64 = expected_entry.base.try_into().expect("base >= 0");
        let expected_quote: u64 = expected_entry.quote.try_into().expect("quote >= 0");

        let light_user = light.get_user_info(user).expect("light user info");
        let full_user = full.get_user_info(user).expect("full user info");

        let light_base = light.get_balance(&light_user, base_token);
        let full_base = full.get_balance(&full_user, base_token);
        let light_quote = light.get_balance(&light_user, quote_token);
        let full_quote = full.get_balance(&full_user, quote_token);

        assert_eq!(
                light_base.0, expected_base,
                "{stage}: user {user} base ({base_token}) balance mismatch for light (expected {expected_base}, got {light_base:?})"
            );
        assert_eq!(
                full_base.0, expected_base,
                "{stage}: user {user} base ({base_token}) balance mismatch for full (expected {expected_base}, got {full_base:?})"
            );
        assert_eq!(
                light_quote.0, expected_quote,
                "{stage}: user {user} quote ({quote_token}) balance mismatch for light (expected {expected_quote}, got {light_quote:?})"
            );
        assert_eq!(
                full_quote.0, expected_quote,
                "{stage}: user {user} quote ({quote_token}) balance mismatch for full (expected {expected_quote}, got {full_quote:?})"
            );
    }
}

fn signer_for<'a>(users: &[&'a str], signers: &'a [TestSigner], user: &str) -> &'a TestSigner {
    let index = users
        .iter()
        .position(|candidate| *candidate == user)
        .expect("signer index");
    &signers[index]
}

fn submit_signed_order<'a>(
    light: &mut Orderbook,
    full: &mut Orderbook,
    users: &[&'a str],
    signers: &'a [TestSigner],
    user: &str,
    order: Order,
) {
    let signer = signer_for(users, signers, user);
    let user_info = full.get_user_info(user).expect("user info for signature");
    let order_id = order.order_id.clone();
    let msg = format!("{}:{}:create_order:{}", user, user_info.nonce, order_id);
    let signature = signer.sign(&msg);
    let private_input = CreateOrderPrivateInput {
        signature,
        public_key: signer.public_key.clone(),
    };
    let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");

    run_action(
        light,
        full,
        user,
        PermissionnedOrderbookAction::CreateOrder(order),
        private_payload,
    );
}

fn add_session_key<'a>(
    light: &mut Orderbook,
    full: &mut Orderbook,
    users: &[&'a str],
    signers: &'a [TestSigner],
    user: &str,
) {
    let signer = signer_for(users, signers, user);
    let payload = borsh::to_vec(&AddSessionKeyPrivateInput {
        new_public_key: signer.public_key.clone(),
    })
    .expect("serialize add session key input");

    run_action(
        light,
        full,
        user,
        PermissionnedOrderbookAction::AddSessionKey,
        payload,
    );
}

fn deposit(light: &mut Orderbook, full: &mut Orderbook, user: &str, token: &str, amount: u64) {
    run_action(
        light,
        full,
        user,
        PermissionnedOrderbookAction::Deposit {
            token: token.to_string(),
            amount,
        },
        Vec::new(),
    );
}

#[allow(clippy::too_many_arguments)]
#[track_caller]
fn execute_market_order<'a>(
    stage: &str,
    order: Order,
    user: &'a str,
    light: &mut Orderbook,
    full: &mut Orderbook,
    users: &[&'a str],
    signers: &'a [TestSigner],
    expected: &mut HashMap<&'a str, BalanceExpectation>,
    base_token: &str,
    quote_token: &str,
    deltas: &[BalanceDelta<'a>],
) {
    submit_signed_order(light, full, users, signers, user, order);
    apply_balance_deltas(expected, deltas);
    assert_stage(stage, light, full, expected, users, base_token, quote_token);
}

#[test_log::test]
fn test_complex_multi_user_orderbook() {
    let (_, _, _, lane_id, secret) = get_ctx();

    let mut light = Orderbook::init(lane_id.clone(), ExecutionMode::Light, secret.clone()).unwrap();
    let mut full = Orderbook::init(lane_id.clone(), ExecutionMode::Full, secret.clone()).unwrap();

    let pair: TokenPair = ("HYLLAR".to_string(), "ORANJ".to_string());
    let base_token = pair.0.clone();
    let quote_token = pair.1.clone();
    let pair_info = PairInfo {
        base_scale: 0,
        quote_scale: 0,
    };

    let users = ["alice", "bob", "charlie"];
    let (alice, bob, charlie) = (users[0], users[1], users[2]);
    let signers: Vec<TestSigner> = (0..users.len())
        .map(|idx| TestSigner::new((idx + 1) as u8))
        .collect();

    let mut expected_balances: HashMap<&str, BalanceExpectation> = users
        .iter()
        .map(|&user| (user, BalanceExpectation::default()))
        .collect();

    for &user in &users {
        add_session_key(&mut light, &mut full, &users, &signers, user);
    }

    run_action(
        &mut light,
        &mut full,
        alice,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info.clone(),
        },
        Vec::new(),
    );

    let funded_amount = 10_000_u64;
    for &user in &users {
        deposit(&mut light, &mut full, user, &base_token, funded_amount);
        deposit(&mut light, &mut full, user, &quote_token, funded_amount);
        apply_balance_deltas(
            &mut expected_balances,
            &[
                delta(user, qty(funded_amount), 0),
                delta(user, 0, qty(funded_amount)),
            ],
        );
    }

    assert_stage(
        "after deposits",
        &light,
        &full,
        &expected_balances,
        &users,
        &base_token,
        &quote_token,
    );

    let limit_orders = vec![
        OrderSpec {
            id: "ask-lim1",
            side: OrderSide::Ask,
            price: Some(14),
            quantity: 30,
        },
        OrderSpec {
            id: "ask-lim2",
            side: OrderSide::Ask,
            price: Some(13),
            quantity: 35,
        },
        OrderSpec {
            id: "ask-lim3",
            side: OrderSide::Ask,
            price: Some(12),
            quantity: 25,
        },
        OrderSpec {
            id: "ask-lim4",
            side: OrderSide::Ask,
            price: Some(11),
            quantity: 20,
        },
        OrderSpec {
            id: "ask-lim5",
            side: OrderSide::Ask,
            price: Some(10),
            quantity: 30,
        },
        OrderSpec {
            id: "ask-lim6",
            side: OrderSide::Ask,
            price: Some(9),
            quantity: 40,
        },
        OrderSpec {
            id: "bid-lim1",
            side: OrderSide::Bid,
            price: Some(8),
            quantity: 20,
        },
        OrderSpec {
            id: "bid-lim2",
            side: OrderSide::Bid,
            price: Some(7),
            quantity: 15,
        },
        OrderSpec {
            id: "bid-lim3",
            side: OrderSide::Bid,
            price: Some(6),
            quantity: 25,
        },
        OrderSpec {
            id: "bid-lim4",
            side: OrderSide::Bid,
            price: Some(5),
            quantity: 20,
        },
        OrderSpec {
            id: "bid-lim5",
            side: OrderSide::Bid,
            price: Some(4),
            quantity: 15,
        },
        OrderSpec {
            id: "bid-lim6",
            side: OrderSide::Bid,
            price: Some(3),
            quantity: 10,
        },
    ];

    for (index, spec) in limit_orders.iter().enumerate() {
        let user = users[index % users.len()];
        let order = Order {
            order_id: spec.id.to_string(),
            order_side: spec.side.clone(),
            order_type: OrderType::Limit,
            price: spec.price,
            pair: pair.clone(),
            quantity: spec.quantity,
        };

        submit_signed_order(&mut light, &mut full, &users, &signers, user, order);

        match spec.side {
            OrderSide::Ask => apply_balance_deltas(
                &mut expected_balances,
                &[delta(user, -qty(spec.quantity), 0)],
            ),
            OrderSide::Bid => {
                let price = spec.price.expect("bid limit order should have price");
                apply_balance_deltas(
                    &mut expected_balances,
                    &[delta(user, 0, -notional(spec.quantity, price))],
                );
            }
        }
    }

    let buy_orders = light.order_manager.buy_orders.get(&pair).unwrap().clone();
    let sell_orders = light.order_manager.sell_orders.get(&pair).unwrap().clone();

    let all_order_ids: Vec<String> = buy_orders
        .iter()
        .chain(sell_orders.iter())
        .flat_map(|(_price, orders)| orders.iter().cloned())
        .collect();

    let limit_order_ids: HashSet<String> = limit_orders
        .iter()
        .map(|spec| spec.id.to_string())
        .collect();

    for order_id in &all_order_ids {
        assert!(
            limit_order_ids.contains(order_id),
            "Order id {order_id} from buy/sell orders not found in limit_orders"
        );
    }
    assert_eq!(
        all_order_ids.len(),
        limit_orders.len(),
        "Mismatch in number of orders in order book"
    );

    assert_stage(
        "after limit orders",
        &light,
        &full,
        &expected_balances,
        &users,
        &base_token,
        &quote_token,
    );

    execute_market_order(
        "after partially filling ask-lim6",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 20,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_token,
        &quote_token,
        &[
            delta(alice, qty(20), -notional(20, 9)),
            delta(charlie, 0, notional(20, 9)),
        ],
    );

    execute_market_order(
        "after clearing ask-lim6 and half of ask-lim5",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 35,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_token,
        &quote_token,
        &[
            delta(alice, qty(20), -notional(20, 9)),
            delta(alice, qty(15), -notional(15, 10)),
            delta(charlie, 0, notional(20, 9)),
            delta(bob, 0, notional(15, 10)),
        ],
    );

    execute_market_order(
        "after clearing ask-lim5",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 15,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_token,
        &quote_token,
        &[
            delta(alice, qty(15), -notional(15, 10)),
            delta(bob, 0, notional(15, 10)),
        ],
    );

    execute_market_order(
        "after self match on ask-lim4",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 10,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_token,
        &quote_token,
        &[
            delta(alice, qty(10), -notional(10, 11)),
            delta(alice, 0, notional(10, 11)),
        ],
    );

    execute_market_order(
        "after clearing remaining ask orders",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 100,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_token,
        &quote_token,
        &[
            delta(alice, qty(10), -notional(10, 11)),
            delta(alice, qty(25), -notional(25, 12)),
            delta(alice, qty(35), -notional(35, 13)),
            delta(alice, qty(30), -notional(30, 14)),
            delta(alice, 0, notional(10, 11)),
            delta(alice, 0, notional(30, 14)),
            delta(bob, 0, notional(35, 13)),
            delta(charlie, 0, notional(25, 12)),
        ],
    );

    let sell_orders = light.order_manager.sell_orders.get(&pair).unwrap().clone();
    assert!(sell_orders.is_empty(), "all sell orders should be filled");

    execute_market_order(
        "after partially filling bid-lim1",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 10,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_token,
        &quote_token,
        &[
            delta(alice, -qty(10), notional(10, 8)),
            delta(alice, qty(10), 0),
        ],
    );

    execute_market_order(
        "after clearing bid-lim1 and half of bid-lim2",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 20,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_token,
        &quote_token,
        &[
            delta(alice, -qty(10), notional(10, 8)),
            delta(alice, -qty(10), notional(10, 7)),
            delta(alice, qty(10), 0),
            delta(bob, qty(10), 0),
        ],
    );

    execute_market_order(
        "after clearing bid-lim2",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 5,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_token,
        &quote_token,
        &[delta(alice, -qty(5), notional(5, 7)), delta(bob, qty(5), 0)],
    );

    execute_market_order(
        "after clearing remaining bid orders",
        Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 70,
        },
        alice,
        &mut light,
        &mut full,
        &users,
        &signers,
        &mut expected_balances,
        &base_token,
        &quote_token,
        &[
            delta(alice, -qty(25), notional(25, 6)),
            delta(alice, -qty(20), notional(20, 5)),
            delta(alice, -qty(15), notional(15, 4)),
            delta(alice, -qty(10), notional(10, 3)),
            delta(charlie, qty(25), 0),
            delta(alice, qty(20), 0),
            delta(bob, qty(15), 0),
            delta(charlie, qty(10), 0),
        ],
    );

    let sell_orders = light.order_manager.sell_orders.get(&pair).unwrap().clone();
    assert!(sell_orders.is_empty(), "all sell orders should be filled");
    let buy_orders = light.order_manager.buy_orders.get(&pair).unwrap().clone();
    assert!(buy_orders.is_empty(), "all buy orders should be filled");
}
