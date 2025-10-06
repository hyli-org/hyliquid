#![cfg(test)]

use std::collections::HashMap;

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
use sdk::ZkContract;
use sdk::{guest, LaneId};
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

    sdk::info!("light events: {:?}", events);

    let commitment_metadata = full
        .derive_zkvm_commitment_metadata_from_events(&user_info, &events, &action)
        .expect("derive metadata");

    let events_full = full
        .execute_permissionned_action(user_info.clone(), action.clone(), &private_payload)
        .expect("full execution");

    sdk::info!("full events: {:?}\n\n\n", events_full);
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

#[test_log::test]
fn test_light_full_zkvm_pipeline_execution() {
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

    let user1_name = "user1";
    let user2_name = "user2";

    let signer1 = TestSigner::new(1);
    let signer2 = TestSigner::new(2);

    // 1. Register user 1
    // 2. Register user 2
    // 3. Create pair via light, replicate on full
    // 4. Deposit ORANJ for user1
    // 5. Deposit HYLLAR for user2
    // 6. Create bid order for user1
    // 7. Create ask order for user2 that should match bid order

    let add_session_key_user1 = borsh::to_vec(&AddSessionKeyPrivateInput {
        new_public_key: signer1.public_key.clone(),
    })
    .expect("serialize add session key input for user1");

    sdk::info!("Step 1: adding session key for {}", user1_name);
    run_action(
        &mut light,
        &mut full,
        user1_name,
        PermissionnedOrderbookAction::AddSessionKey,
        add_session_key_user1,
    );

    sdk::info!("Step 2: registering {}", user2_name);
    let add_session_key_user2 = borsh::to_vec(&AddSessionKeyPrivateInput {
        new_public_key: signer2.public_key.clone(),
    })
    .expect("serialize add session key input for user2");

    sdk::info!("Step 2: adding session key for {}", user2_name);
    run_action(
        &mut light,
        &mut full,
        user2_name,
        PermissionnedOrderbookAction::AddSessionKey,
        add_session_key_user2,
    );

    sdk::info!(
        "Step 3: creating trading pair {}/{}",
        base_token,
        quote_token
    );
    run_action(
        &mut light,
        &mut full,
        user1_name,
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info.clone(),
        },
        Vec::new(),
    );

    sdk::info!(
        "Step 4: depositing 1_000 for {} for {}",
        quote_token,
        user1_name
    );
    run_action(
        &mut light,
        &mut full,
        user1_name,
        PermissionnedOrderbookAction::Deposit {
            token: quote_token.clone(),
            amount: 1_000_u64,
        },
        Vec::new(),
    );

    sdk::info!("Step 5: depositing 1_000 {} for {}", base_token, user2_name);
    run_action(
        &mut light,
        &mut full,
        user2_name,
        PermissionnedOrderbookAction::Deposit {
            token: base_token.clone(),
            amount: 1_000_u64,
        },
        Vec::new(),
    );

    let bid_order_id = "bid-user1".to_string();
    let bid_quantity = 50_u64;
    let bid_price = Some(10_u64);
    let user1_info_for_bid = full
        .get_user_info(user1_name)
        .expect("user1 info before bid order");
    let bid_msg = format!(
        "{}:{}:create_order:{}",
        user1_name, user1_info_for_bid.nonce, bid_order_id
    );
    let bid_signature = signer1.sign(&bid_msg);
    let bid_private_input = CreateOrderPrivateInput {
        signature: bid_signature,
        public_key: signer1.public_key.clone(),
    };
    let bid_private_payload =
        borsh::to_vec(&bid_private_input).expect("serialize create order input for bid order");

    sdk::info!(
        "Step 6: creating bid order {} for {}",
        bid_order_id,
        user1_name
    );
    run_action(
        &mut light,
        &mut full,
        user1_name,
        PermissionnedOrderbookAction::CreateOrder(Order {
            order_id: bid_order_id.clone(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: bid_price,
            pair: pair.clone(),
            quantity: bid_quantity,
        }),
        bid_private_payload,
    );

    let ask_order_id = "ask-user2".to_string();
    let ask_quantity = bid_quantity;
    let ask_price = bid_price;
    let user2_info_for_ask = full
        .get_user_info(user2_name)
        .expect("user2 info before ask order");
    let ask_msg = format!(
        "{}:{}:create_order:{}",
        user2_name, user2_info_for_ask.nonce, ask_order_id
    );
    let ask_signature = signer2.sign(&ask_msg);
    let ask_private_input = CreateOrderPrivateInput {
        signature: ask_signature,
        public_key: signer2.public_key.clone(),
    };
    let ask_private_payload =
        borsh::to_vec(&ask_private_input).expect("serialize create order input for ask order");

    sdk::info!(
        "Step 7: creating ask order {} for {}",
        ask_order_id,
        user2_name
    );
    run_action(
        &mut light,
        &mut full,
        user2_name,
        PermissionnedOrderbookAction::CreateOrder(Order {
            order_id: ask_order_id.clone(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: ask_price,
            pair: pair.clone(),
            quantity: ask_quantity,
        }),
        ask_private_payload,
    );

    sdk::info!("Verifying matching orders were cleared");
    assert!(
        full.order_manager.orders.is_empty(),
        "order book should be empty after matching orders"
    );

    sdk::info!("Ensuring light and full states remain in sync");
    assert_eq!(
        light.order_manager, full.order_manager,
        "light and full commitments diverged: {light:?} != {full:?}"
    );
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

    // Create 3 users
    let users = ["alice", "bob", "charlie"];
    let mut signers = vec![];

    // Initialize signers for each user
    for i in 0..users.len() {
        signers.push(TestSigner::new((i + 1) as u8));
    }

    #[derive(Clone, Copy)]
    struct BalanceExpectation {
        base: i128,
        quote: i128,
    }

    let assert_balances = |stage: &str,
                           light: &Orderbook,
                           full: &Orderbook,
                           expected: &HashMap<&str, BalanceExpectation>,
                           users: &[&str],
                           base_token: &str,
                           quote_token: &str| {
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
    };

    let mut expected_balances: std::collections::HashMap<&str, BalanceExpectation> = users
        .iter()
        .map(|&user| (user, BalanceExpectation { base: 0, quote: 0 }))
        .collect();

    // Step 1: Add session keys for all users
    for (i, &user_name) in users.iter().enumerate() {
        let add_session_key = borsh::to_vec(&AddSessionKeyPrivateInput {
            new_public_key: signers[i].public_key.clone(),
        })
        .expect("serialize add session key input");

        run_action(
            &mut light,
            &mut full,
            user_name,
            PermissionnedOrderbookAction::AddSessionKey,
            add_session_key,
        );
    }

    // Step 2: Create trading pair
    run_action(
        &mut light,
        &mut full,
        users[0],
        PermissionnedOrderbookAction::CreatePair {
            pair: pair.clone(),
            info: pair_info.clone(),
        },
        Vec::new(),
    );

    // Step 3: Fund all users with both tokens and record expected balances
    let funded_amount = 10_000_u64;
    for &user_name in &users {
        run_action(
            &mut light,
            &mut full,
            user_name,
            PermissionnedOrderbookAction::Deposit {
                token: base_token.clone(),
                amount: funded_amount,
            },
            Vec::new(),
        );
        run_action(
            &mut light,
            &mut full,
            user_name,
            PermissionnedOrderbookAction::Deposit {
                token: quote_token.clone(),
                amount: funded_amount,
            },
            Vec::new(),
        );
    }

    for &user_name in &users {
        let light_user = light.get_user_info(user_name).expect("light user info");
        let base_balance = light.get_balance(&light_user, &base_token).0 as i128;
        let quote_balance = light.get_balance(&light_user, &quote_token).0 as i128;
        let entry = expected_balances
            .get_mut(user_name)
            .expect("user balance entry");
        entry.base = base_balance;
        entry.quote = quote_balance;
    }

    assert_balances(
        "after deposits",
        &light,
        &full,
        &expected_balances,
        &users,
        &base_token,
        &quote_token,
    );

    // Step 4: We create a set of limit orders
    // Each of the three users will place 3 Ask limit orders and 3 Bid limit orders
    let limit_orders = [
        ("ask-lim1", OrderSide::Ask, OrderType::Limit, Some(14), 30),
        ("ask-lim2", OrderSide::Ask, OrderType::Limit, Some(13), 35),
        ("ask-lim3", OrderSide::Ask, OrderType::Limit, Some(12), 25),
        ("ask-lim4", OrderSide::Ask, OrderType::Limit, Some(11), 20),
        ("ask-lim5", OrderSide::Ask, OrderType::Limit, Some(10), 30),
        ("ask-lim6", OrderSide::Ask, OrderType::Limit, Some(9), 40),
        /////////////
        ("bid-lim1", OrderSide::Bid, OrderType::Limit, Some(8), 20),
        ("bid-lim2", OrderSide::Bid, OrderType::Limit, Some(7), 15),
        ("bid-lim3", OrderSide::Bid, OrderType::Limit, Some(6), 25),
        ("bid-lim4", OrderSide::Bid, OrderType::Limit, Some(5), 20),
        ("bid-lim5", OrderSide::Bid, OrderType::Limit, Some(4), 15),
        ("bid-lim6", OrderSide::Bid, OrderType::Limit, Some(3), 10),
    ];

    for (i, (order_id, side, order_type, price, quantity)) in limit_orders.iter().enumerate() {
        let user = users[i % users.len()];

        let user_index = users
            .iter()
            .position(|candidate| *candidate == user)
            .expect("user index");
        let signer = &signers[user_index];
        let user_info = full.get_user_info(user).expect("user info for signature");
        let msg = format!("{}:{}:create_order:{}", user, user_info.nonce, order_id);
        let signature = signer.sign(&msg);
        let private_input = CreateOrderPrivateInput {
            signature,
            public_key: signer.public_key.clone(),
        };
        let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");

        run_action(
            &mut light,
            &mut full,
            user,
            PermissionnedOrderbookAction::CreateOrder(Order {
                order_id: order_id.to_string(),
                order_side: side.clone(),
                order_type: order_type.clone(),
                price: *price,
                pair: pair.clone(),
                quantity: *quantity,
            }),
            private_payload,
        );

        // Update expected balances
        expected_balances
            .entry(user)
            .and_modify(|entry| match side {
                OrderSide::Ask => {
                    entry.base -= *quantity as i128;
                }
                OrderSide::Bid => {
                    let price = price.expect("bid limit order should have price");
                    entry.quote -= (*quantity * price) as i128;
                }
            });
    }

    let buy_orders = light.order_manager.buy_orders.get(&(pair)).unwrap().clone();
    let sell_orders = light.order_manager.sell_orders.get(&pair).unwrap().clone();

    let all_order_ids: Vec<String> = buy_orders
        .iter()
        .chain(sell_orders.iter())
        .cloned()
        .collect();

    let limit_order_ids: std::collections::HashSet<String> = limit_orders
        .iter()
        .map(|(order_id, _, _, _, _)| order_id.to_string())
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

    assert_balances(
        "after limit orders",
        &light,
        &full,
        &expected_balances,
        &users,
        &base_token,
        &quote_token,
    );

    // Step 5: Create a market order that will partially match ask-lim6
    {
        let market_order = Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 20, // this consumes half of "ask-lim6"
        };
        let alice = users[0];
        let signer = &signers[0];
        let user_info = full.get_user_info(alice).expect("user info for signature");

        let msg = format!(
            "{}:{}:create_order:{}",
            alice, user_info.nonce, market_order.order_id
        );
        let signature = signer.sign(&msg);
        let private_input = CreateOrderPrivateInput {
            signature,
            public_key: signer.public_key.clone(),
        };
        let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");
        run_action(
            &mut light,
            &mut full,
            alice,
            PermissionnedOrderbookAction::CreateOrder(market_order),
            private_payload,
        );

        // Update expected balances
        expected_balances.entry(alice).and_modify(|entry| {
            // Market Ask of 20 consumes hald of "ask-lim6" (20@9)
            entry.base += 20;
            entry.quote -= (20 * 9) as i128;
        });

        expected_balances.entry("charlie").and_modify(|entry| {
            entry.quote += (20 * 9) as i128;
        });

        assert_balances(
            "after market order 1",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );
    }

    // Step 6: Create a market order that will fully match ask-lim6 and partially match ask-lim5
    {
        let market_order = Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 20 + 15, // this consumes last half of "ask-lim6" and half of "ask-lim5"
        };
        let alice = users[0];
        let signer = &signers[0];
        let user_info = full.get_user_info(alice).expect("user info for signature");

        let msg = format!(
            "{}:{}:create_order:{}",
            alice, user_info.nonce, market_order.order_id
        );
        let signature = signer.sign(&msg);
        let private_input = CreateOrderPrivateInput {
            signature,
            public_key: signer.public_key.clone(),
        };
        let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");
        run_action(
            &mut light,
            &mut full,
            alice,
            PermissionnedOrderbookAction::CreateOrder(market_order),
            private_payload,
        );

        // Update expected balances
        expected_balances.entry(alice).and_modify(|entry| {
            // Market Ask of 20 consumes half of "ask-lim6" (20@9)
            entry.base += 20;
            entry.quote -= (20 * 9) as i128;
            // Market Ask of 15 consumes half of "ask-lim5" (15@10)
            entry.base += 15;
            entry.quote -= (15 * 10) as i128;
        });

        expected_balances.entry("charlie").and_modify(|entry| {
            entry.quote += (20 * 9) as i128;
        });
        expected_balances.entry("bob").and_modify(|entry| {
            entry.quote += (15 * 10) as i128;
        });

        assert_balances(
            "after market order 2",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );
    }

    // Step 7: Create a market order that will fully match ask-lim5
    {
        let market_order = Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 15, // this consumes last half of "ask-lim5"
        };
        let alice = users[0];
        let signer = &signers[0];
        let user_info = full.get_user_info(alice).expect("user info for signature");

        let msg = format!(
            "{}:{}:create_order:{}",
            alice, user_info.nonce, market_order.order_id
        );
        let signature = signer.sign(&msg);
        let private_input = CreateOrderPrivateInput {
            signature,
            public_key: signer.public_key.clone(),
        };
        let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");
        run_action(
            &mut light,
            &mut full,
            alice,
            PermissionnedOrderbookAction::CreateOrder(market_order),
            private_payload,
        );

        // Update expected balances
        expected_balances.entry(alice).and_modify(|entry| {
            // Market Ask of 15 consumes half of "ask-lim5" (15@10)
            entry.base += 15;
            entry.quote -= (15 * 10) as i128;
        });

        expected_balances.entry("bob").and_modify(|entry| {
            entry.quote += (15 * 10) as i128;
        });

        assert_balances(
            "after market order 3",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );
    }

    // Step 8: Create a market order that will partially match ask-lim4 (own user with own order)
    {
        let market_order = Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 10, // this consumes half of "ask-lim4"
        };
        let alice = users[0];
        let signer = &signers[0];
        let user_info = full.get_user_info(alice).expect("user info for signature");

        let msg = format!(
            "{}:{}:create_order:{}",
            alice, user_info.nonce, market_order.order_id
        );
        let signature = signer.sign(&msg);
        let private_input = CreateOrderPrivateInput {
            signature,
            public_key: signer.public_key.clone(),
        };
        let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");
        run_action(
            &mut light,
            &mut full,
            alice,
            PermissionnedOrderbookAction::CreateOrder(market_order),
            private_payload,
        );

        // Update expected balances
        expected_balances.entry(alice).and_modify(|entry| {
            // Market Ask of 10 consumes half of "ask-lim4" (10@11)
            entry.base += 10;
            entry.quote -= (10 * 11) as i128;
        });

        expected_balances.entry("alice").and_modify(|entry| {
            entry.quote += (10 * 11) as i128;
        });

        assert_balances(
            "after market order 3",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );
    }

    // Step 9: Create a market order that will execute all orders
    {
        let market_order = Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Bid,
            price: None,
            pair: pair.clone(),
            quantity: 10 + 25 + 35 + 30, // this consumes half of "ask-lim4", and fully ask-lim3, ask-lim2 and ask-lim1
        };
        let alice = users[0];
        let signer = &signers[0];
        let user_info = full.get_user_info(alice).expect("user info for signature");

        let msg = format!(
            "{}:{}:create_order:{}",
            alice, user_info.nonce, market_order.order_id
        );
        let signature = signer.sign(&msg);
        let private_input = CreateOrderPrivateInput {
            signature,
            public_key: signer.public_key.clone(),
        };
        let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");
        run_action(
            &mut light,
            &mut full,
            alice,
            PermissionnedOrderbookAction::CreateOrder(market_order),
            private_payload,
        );

        // Update expected balances
        expected_balances.entry(alice).and_modify(|entry| {
            // Market Ask of 10 consumes half of "ask-lim4" (10@11)
            entry.base += 10;
            entry.quote -= (10 * 11) as i128;
            // Market Ask of 10 consumes half of "ask-lim3" (25@12)
            entry.base += 25;
            entry.quote -= (25 * 12) as i128;
            // Market Ask of 10 consumes half of "ask-lim2" (35@13)
            entry.base += 35;
            entry.quote -= (35 * 13) as i128;
            // Market Ask of 10 consumes half of "ask-lim1" (30@14)
            entry.base += 30;
            entry.quote -= (30 * 14) as i128;
        });

        expected_balances.entry("alice").and_modify(|entry| {
            entry.quote += (10 * 11) as i128;
            entry.quote += (30 * 14) as i128;
        });

        expected_balances.entry("bob").and_modify(|entry| {
            entry.quote += (35 * 13) as i128;
        });

        expected_balances.entry("charlie").and_modify(|entry| {
            entry.quote += (25 * 12) as i128;
        });

        assert_balances(
            "after market order 4",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );
    }

    let sell_orders = light.order_manager.sell_orders.get(&pair).unwrap().clone();
    assert!(sell_orders.is_empty(), "all sell orders should be filled");

    // Step 10 Create a market order that will partially match bid-lim1
    {
        let market_order = Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 10, // this consumes half of "bid-lim1"
        };
        let alice = users[0];
        let signer = &signers[0];
        let user_info = full.get_user_info(alice).expect("user info for signature");

        let msg = format!(
            "{}:{}:create_order:{}",
            alice, user_info.nonce, market_order.order_id
        );
        let signature = signer.sign(&msg);
        let private_input = CreateOrderPrivateInput {
            signature,
            public_key: signer.public_key.clone(),
        };
        let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");
        run_action(
            &mut light,
            &mut full,
            alice,
            PermissionnedOrderbookAction::CreateOrder(market_order),
            private_payload,
        );

        // Update expected balances
        expected_balances.entry(alice).and_modify(|entry| {
            // Market Bid of 10 consumes half of "bid-lim1" (10@8)
            entry.base -= 10;
            entry.quote += 10 * 8;
        });

        expected_balances
            .entry("alice")
            .and_modify(|entry| entry.base += 10);

        assert_balances(
            "after market order 5",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );
    }

    // Step 11 Create a market order that will fully match bid-lim1 and partually match bid-lim2
    {
        let market_order = Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 10 + 10, // this consumes last half of "bid-lim1" and half of "bid-lim2"
        };
        let alice = users[0];
        let signer = &signers[0];
        let user_info = full.get_user_info(alice).expect("user info for signature");

        let msg = format!(
            "{}:{}:create_order:{}",
            alice, user_info.nonce, market_order.order_id
        );
        let signature = signer.sign(&msg);
        let private_input = CreateOrderPrivateInput {
            signature,
            public_key: signer.public_key.clone(),
        };
        let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");
        run_action(
            &mut light,
            &mut full,
            alice,
            PermissionnedOrderbookAction::CreateOrder(market_order),
            private_payload,
        );

        // Update expected balances
        expected_balances.entry(alice).and_modify(|entry| {
            // Market Bid of 20 consumes half of "bid-lim1" (10@8)
            entry.base -= 10;
            entry.quote += 10 * 8;
            // Market Bid of 10 consumes half of "bid-lim2" (10@7)
            entry.base -= 10;
            entry.quote += 10 * 7;
        });

        expected_balances.entry("alice").and_modify(|entry| {
            entry.base += 10;
        });
        expected_balances.entry("bob").and_modify(|entry| {
            entry.base += 10;
        });

        assert_balances(
            "after market order 6",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );
    }

    // Step 12 Create a market order that will fully match bid-lim2
    {
        let market_order = Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 5, // this consumes last half of "bid-lim2"
        };
        let alice = users[0];
        let signer = &signers[0];
        let user_info = full.get_user_info(alice).expect("user info for signature");

        let msg = format!(
            "{}:{}:create_order:{}",
            alice, user_info.nonce, market_order.order_id
        );
        let signature = signer.sign(&msg);
        let private_input = CreateOrderPrivateInput {
            signature,
            public_key: signer.public_key.clone(),
        };
        let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");
        run_action(
            &mut light,
            &mut full,
            alice,
            PermissionnedOrderbookAction::CreateOrder(market_order),
            private_payload,
        );

        // Update expected balances
        expected_balances.entry(alice).and_modify(|entry| {
            // Market Bid of 5 consumes half of "bid-lim2" (5@7)
            entry.base -= 5;
            entry.quote += 5 * 7;
        });

        expected_balances.entry("bob").and_modify(|entry| {
            entry.base += 5;
        });

        assert_balances(
            "after market order 7",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );
    }

    // Step 13 Create a market order that will execute all orders
    {
        let market_order = Order {
            order_id: "market1".to_string(),
            order_type: OrderType::Market,
            order_side: OrderSide::Ask,
            price: None,
            pair: pair.clone(),
            quantity: 25 + 20 + 15 + 10, // this consumes "bid-lim3", "bid-lim4", "bid-lim5" and "bid-lim6"
        };
        let alice = users[0];
        let signer = &signers[0];
        let user_info = full.get_user_info(alice).expect("user info for signature");

        let msg = format!(
            "{}:{}:create_order:{}",
            alice, user_info.nonce, market_order.order_id
        );
        let signature = signer.sign(&msg);
        let private_input = CreateOrderPrivateInput {
            signature,
            public_key: signer.public_key.clone(),
        };
        let private_payload = borsh::to_vec(&private_input).expect("serialize create order input");
        run_action(
            &mut light,
            &mut full,
            alice,
            PermissionnedOrderbookAction::CreateOrder(market_order),
            private_payload,
        );

        // Update expected balances
        expected_balances.entry(alice).and_modify(|entry| {
            // Market Bid of 5 consumes half of "bid-lim3" (25@6)
            entry.base -= 25;
            entry.quote += 25 * 6;
            // Market Bid of 5 consumes half of "bid-lim4" (20@5)
            entry.base -= 20;
            entry.quote += 20 * 5;
            // Market Bid of 5 consumes half of "bid-lim5" (15@4)
            entry.base -= 15;
            entry.quote += 15 * 4;
            // Market Bid of 5 consumes half of "bid-lim6" (10@3)
            entry.base -= 10;
            entry.quote += 10 * 3;
        });

        expected_balances.entry("charlie").and_modify(|entry| {
            entry.base += 25;
        });
        expected_balances.entry("alice").and_modify(|entry| {
            entry.base += 20;
        });
        expected_balances.entry("bob").and_modify(|entry| {
            entry.base += 15;
        });
        expected_balances.entry("charlie").and_modify(|entry| {
            entry.base += 10;
        });

        assert_balances(
            "after market order 8",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );
    }

    let sell_orders = light.order_manager.sell_orders.get(&pair).unwrap().clone();
    assert!(sell_orders.is_empty(), "all sell orders should be filled");
    let buy_orders = light.order_manager.buy_orders.get(&pair).unwrap().clone();
    assert!(buy_orders.is_empty(), "all buy orders should be filled");
}
