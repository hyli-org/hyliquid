#[cfg(test)]
mod orderbook_tests {
    use std::collections::BTreeMap;

    use sdk::LaneId;

    use crate::{
        orderbook::*,
        smt_values::{Balance, UserInfo},
    };

    fn test_user(name: &str) -> UserInfo {
        let mut ui = UserInfo::new(name.to_string(), name.as_bytes().to_vec());
        ui.nonce = 1;
        ui.session_keys = Vec::new();
        ui
    }

    fn session_key(seed: u8) -> Vec<u8> {
        vec![seed; 32]
    }

    fn limit_order(
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

    fn register_user(orderbook: &mut Orderbook, user_info: &UserInfo) {
        if !matches!(orderbook.execution_state, ExecutionState::Full(_)) {
            panic!("register_user requires full execution state");
        }

        orderbook
            .update_user_info_merkle_root(user_info)
            .expect("register user in SMT");
    }

    #[test_log::test]
    fn executes_partial_match_and_updates_balances() {
        let pair = ("ETH".to_string(), "USDC".to_string());
        let mut orderbook =
            Orderbook::init(LaneId::default(), ExecutionMode::Full, b"secret".to_vec())
                .expect("orderbook initialization");

        sdk::info!("Creating pair {pair:?}");
        orderbook
            .create_pair(
                &pair,
                &PairInfo {
                    base_scale: 0,
                    quote_scale: 0,
                },
            )
            .expect("pair creation");

        let bob = test_user("bob");
        let alice = test_user("alice");
        let carol = test_user("carol");

        register_user(&mut orderbook, &bob);
        register_user(&mut orderbook, &alice);
        register_user(&mut orderbook, &carol);

        // Fund accounts

        sdk::info!("Deposit for bob: {:?}", bob.get_key());
        orderbook
            .deposit(&pair.1, 1_000, &bob)
            .expect("bob quote deposit");
        sdk::info!("Deposit for alice: {:?}", alice.get_key());
        orderbook
            .deposit(&pair.1, 500, &alice)
            .expect("alice quote deposit");
        sdk::info!("Deposit for carol: {:?}", carol.get_key());
        orderbook
            .deposit(&pair.0, 80, &carol)
            .expect("carol base deposit");

        // Bob posts two bids, Alice posts one bid
        let bid_bob_primary = limit_order("bid-bob-primary", OrderSide::Bid, 10, 50, &pair);
        let bid_bob_secondary = limit_order("bid-bob-secondary", OrderSide::Bid, 8, 30, &pair);
        let bid_alice = limit_order("bid-alice", OrderSide::Bid, 9, 20, &pair);

        sdk::info!("Placing order for bob primary");
        orderbook
            .execute_order(&bob, bid_bob_primary.clone())
            .expect("bob primary order");

        sdk::info!("Placing order for bob secondary");
        orderbook
            .execute_order(&bob, bid_bob_secondary.clone())
            .expect("bob secondary order");

        sdk::info!("Placing order for alice");
        orderbook
            .execute_order(&alice, bid_alice.clone())
            .expect("alice bid order");

        assert_eq!(
            orderbook.order_manager.orders.len(),
            3,
            "three resting bids"
        );

        // Carol sells part of Bob's primary order.
        let carol_order = limit_order("ask-carol", OrderSide::Ask, 9, 40, &pair);

        sdk::info!("Carol placing order to match against book");
        let events = orderbook
            .execute_order(&carol, carol_order.clone())
            .expect("carol executes against book");

        // Ensure matching produced a partial fill update on Bob's primary order
        let update_event = events.iter().find_map(|event| match event {
            OrderbookEvent::OrderUpdate {
                order_id,
                taker_order_id,
                executed_quantity,
                remaining_quantity,
                ..
            } if order_id == &bid_bob_primary.order_id
                && taker_order_id == &carol_order.order_id =>
            {
                Some((*executed_quantity, *remaining_quantity))
            }
            _ => None,
        });

        assert_eq!(
            update_event,
            Some((40, 10)),
            "bob primary should be partially filled"
        );

        let expected_balance_events = BTreeMap::from([
            ((bob.user.clone(), pair.0.clone()), 40_u64),
            ((bob.user.clone(), pair.1.clone()), 260_u64),
            ((carol.user.clone(), pair.0.clone()), 40_u64),
            ((carol.user.clone(), pair.1.clone()), 400_u64),
        ]);

        let actual_balance_events: BTreeMap<(String, String), u64> = events
            .iter()
            .filter_map(|event| match event {
                OrderbookEvent::BalanceUpdated {
                    user,
                    token,
                    amount,
                } => Some(((user.clone(), token.clone()), *amount)),
                _ => None,
            })
            .collect();

        assert_eq!(
            actual_balance_events, expected_balance_events,
            "BalanceUpdated events should reflect updated balances"
        );

        // Order book state should reflect remaining quantity and untouched secondary orders
        let bob_primary = orderbook
            .order_manager
            .orders
            .get(&bid_bob_primary.order_id)
            .expect("primary order persists");
        assert_eq!(bob_primary.quantity, 10);

        let buy_queue = orderbook
            .order_manager
            .buy_orders
            .get(&pair)
            .expect("buy queue exists");
        assert_eq!(buy_queue.len(), 3);
        assert_eq!(buy_queue.front(), Some(&bid_bob_primary.order_id));

        // Balances should reflect the trade: Carol sold 40 ETH for 400 USDC
        assert_eq!(orderbook.get_balance(&bob, &pair.0).0, 40);
        assert_eq!(orderbook.get_balance(&bob, &pair.1).0, 260);
        assert_eq!(orderbook.get_balance(&carol, &pair.0).0, 40);
        assert_eq!(orderbook.get_balance(&carol, &pair.1).0, 400);

        let buy_queue_ids: Vec<_> = buy_queue.iter().collect();
        assert_eq!(
            buy_queue_ids,
            vec![
                &bid_bob_primary.order_id,
                &bid_alice.order_id,
                &bid_bob_secondary.order_id
            ],
            "orders should be sorted by best price (highest) first"
        );
    }

    #[test_log::test]
    fn complete_order_matching_btc_usd_pair() {
        let pair = ("BTC".to_string(), "USD".to_string());
        let mut orderbook =
            Orderbook::init(LaneId::default(), ExecutionMode::Full, b"secret".to_vec())
                .expect("orderbook initialization");

        // Create BTC/USD pair
        sdk::info!("Creating pair");
        orderbook
            .create_pair(
                &pair,
                &PairInfo {
                    base_scale: 0,
                    quote_scale: 0,
                },
            )
            .expect("pair creation");

        let user1 = test_user("user1");
        let user2 = test_user("user2");

        register_user(&mut orderbook, &user1);
        register_user(&mut orderbook, &user2);

        // user1 add session key
        sdk::info!("Adding session key for user1");
        orderbook
            .add_session_key(user1.clone(), &session_key(1))
            .expect("user1 session key");

        // user2 add session key
        sdk::info!("Adding session key for user2");
        orderbook
            .add_session_key(user2.clone(), &session_key(2))
            .expect("user2 session key");

        sdk::info!("Deposits for user1: {:?}", user1.get_key());
        // user1 deposit 100 USD (to buy BTC with USD - bid order)
        orderbook
            .deposit(&pair.1, 100, &user1)
            .expect("user1 USD deposit");

        sdk::info!("Deposits for user2: {:?}", user2.get_key());
        // user2 deposit 100 BTC (to sell BTC for USD - ask order)
        orderbook
            .deposit(&pair.0, 100, &user2)
            .expect("user2 BTC deposit");

        // Verify initial balances
        assert_eq!(
            orderbook.get_balance(&user1, &pair.0).0,
            0,
            "user1 should have 0 BTC"
        );
        assert_eq!(
            orderbook.get_balance(&user1, &pair.1).0,
            100,
            "user1 should have 100 USD"
        );
        assert_eq!(
            orderbook.get_balance(&user2, &pair.0).0,
            100,
            "user2 should have 100 BTC"
        );
        assert_eq!(
            orderbook.get_balance(&user2, &pair.1).0,
            0,
            "user2 should have 0 USD"
        );

        // user1 create-order --order-id id1 --order-type limit --order-side bid --pair-token1 BTC --pair-token2 USD --quantity 2 --price 1
        let user1_order = limit_order("id1", OrderSide::Bid, 1, 2, &pair);
        sdk::info!("user1 placing bid order for user1");
        orderbook
            .execute_order(&user1, user1_order.clone())
            .expect("user1 places bid order");

        // Verify user1's order is in the book
        assert_eq!(
            orderbook.order_manager.orders.len(),
            1,
            "one order should be in the book"
        );
        assert!(
            orderbook.order_manager.orders.contains_key("id1"),
            "user1 order should be in the book"
        );

        // user2 create-order --order-id id2 --order-type limit --order-side ask --pair-token1 BTC --pair-token2 USD --quantity 2 --price 1
        let user2_order = limit_order("id2", OrderSide::Ask, 1, 2, &pair);

        sdk::info!("user2 placing ask order for user2");
        let events = orderbook
            .execute_order(&user2, user2_order.clone())
            .expect("user2 executes ask order against user1's bid");

        // Verify that both orders were executed (should generate OrderExecuted events)
        let execution_events: Vec<_> = events
            .iter()
            .filter(|event| matches!(event, OrderbookEvent::OrderExecuted { .. }))
            .collect();

        assert!(
            !execution_events.is_empty(),
            "orders should have been executed"
        );

        // Verify that user1 received 2 BTC for 2 USD (he was buying BTC with USD)
        // user1 had 100 BTC initially, spent 2 USD to buy 2 BTC, so should have 100 BTC and -2 USD (but he didn't have USD initially, so this is handled via the trade)
        // Actually, user1 was bidding (buying BTC with USD), but he deposited BTC, not USD. Let me reconsider the scenario.
        //
        // Wait, I think there's a misunderstanding. Let me re-read the request:
        // user1 deposits 100 BTC and places a BID (wants to buy more BTC with USD) - this doesn't make sense
        // user2 deposits 100 USD and places an ASK (wants to sell BTC for USD) - but doesn't have BTC
        //
        // Let me correct this:
        // user1 should deposit USD to buy BTC (bid)
        // user2 should deposit BTC to sell BTC (ask)

        // Verify final balances after the trade
        let user1_btc_balance = orderbook.get_balance(&user1, &pair.0).0;
        let user1_usd_balance = orderbook.get_balance(&user1, &pair.1).0;
        let user2_btc_balance = orderbook.get_balance(&user2, &pair.0).0;
        let user2_usd_balance = orderbook.get_balance(&user2, &pair.1).0;

        // user1 should have received 2 BTC and spent 2 USD (100 - 2 = 98 USD, 0 + 2 = 2 BTC)
        assert_eq!(user1_btc_balance, 2, "user1 should have received 2 BTC");
        assert_eq!(user1_usd_balance, 98, "user1 should have spent 2 USD");

        // user2 should have received 2 USD and spent 2 BTC (100 - 2 = 98 BTC, 0 + 2 = 2 USD)
        assert_eq!(user2_btc_balance, 98, "user2 should have spent 2 BTC");
        assert_eq!(user2_usd_balance, 2, "user2 should have received 2 USD");

        // The orders should no longer be in the book since they were fully matched
        assert_eq!(
            orderbook.order_manager.orders.len(),
            0,
            "all orders should be executed and removed from book"
        );
    }

    #[test]
    fn zkvm_witness_verification_uses_state_proof() {
        let pair = ("ETH".to_string(), "USDC".to_string());
        let mut orderbook =
            Orderbook::init(LaneId::default(), ExecutionMode::Full, b"secret".to_vec())
                .expect("orderbook initialization");

        orderbook
            .create_pair(
                &pair,
                &PairInfo {
                    base_scale: 0,
                    quote_scale: 0,
                },
            )
            .expect("pair creation");

        let bob = test_user("bob");
        register_user(&mut orderbook, &bob);

        let events = orderbook.deposit(&pair.1, 100, &bob).expect("bob deposit");

        let (zk_state, order_manager) = orderbook
            .for_zkvm(&bob, &events)
            .expect("derive zkvm state from events");

        let mut zk_orderbook = orderbook.clone();
        zk_orderbook.order_manager = order_manager;
        zk_orderbook.execution_state = ExecutionState::ZkVm(zk_state.clone());

        zk_orderbook
            .verify_users_info_proof()
            .expect("users info proof matches state");
        zk_orderbook
            .verify_balances_proof()
            .expect("balances proof matches state");

        let mut tampered_state = zk_state;
        let balance_witness = tampered_state
            .balances
            .get_mut(&pair.1)
            .expect("balance witness for token");
        let (_, balance) = balance_witness
            .value
            .iter_mut()
            .next()
            .expect("at least one balance entry");
        let incorrect_amount = balance.0 + 1;
        *balance = Balance(incorrect_amount);

        let mut tampered_orderbook = orderbook.clone();
        tampered_orderbook.execution_state = ExecutionState::ZkVm(tampered_state);

        let err = tampered_orderbook
            .verify_balances_proof()
            .expect_err("tampered balance proof should fail");
        assert!(
            err.contains("Invalid balances proof"),
            "unexpected error: {err}"
        );
    }

    // #[test]
    // fn zkvm_order_execution_requires_user_witness() {
    //     let pair = ("HYLLAR".to_string(), "ORANJ".to_string());
    //     let mut orderbook =
    //         Orderbook::init(LaneId::default(), ExecutionMode::Full, b"secret".to_vec())
    //             .expect("orderbook initialization");

    //     orderbook
    //         .create_pair(
    //             &pair,
    //             &PairInfo {
    //                 base_scale: 0,
    //                 quote_scale: 0,
    //             },
    //         )
    //         .expect("pair creation");

    //     let user1 = test_user("user1");
    //     let user2 = test_user("user2");

    //     register_user(&mut orderbook, &user1);
    //     register_user(&mut orderbook, &user2);

    //     orderbook
    //         .add_session_key(user1.clone(), &session_key(1))
    //         .expect("user1 session key");

    //     orderbook
    //         .deposit(&pair.1, 100, &user1)
    //         .expect("user1 deposit");

    //     let maker_order = limit_order("id1", OrderSide::Bid, 1, 2, &pair);
    //     orderbook
    //         .execute_order(&user1, maker_order.clone())
    //         .expect("user1 places order");

    //     orderbook
    //         .add_session_key(user2.clone(), &session_key(2))
    //         .expect("user2 session key");

    //     orderbook
    //         .deposit(&pair.0, 100, &user2)
    //         .expect("user2 deposit");

    //     let taker_info = orderbook
    //         .get_user_info(&user2.user)
    //         .expect("fetch user2 info before execution");

    //     let taker_order = limit_order("id2", OrderSide::Ask, 1, 2, &pair);
    //     let events = orderbook
    //         .execute_order(&taker_info, taker_order.clone())
    //         .expect("user2 executes order");

    //     assert!(
    //         events
    //             .iter()
    //             .any(|event| matches!(event, OrderbookEvent::OrderExecuted { .. })),
    //         "execution should emit OrderExecuted events"
    //     );

    //     let private_input = CreateOrderPrivateInput {
    //         signature: Vec::new(),
    //         public_key: Vec::new(),
    //     };

    //     let zk_state = orderbook
    //         .as_zkvm(&taker_info, &events)
    //         .expect("derive zkvm state");

    //     let mut zk_orderbook = orderbook.clone();
    //     zk_orderbook.execution_state = ExecutionState::ZkVm(zk_state);

    //     let maker_key = user1.get_key();
    //     zk_orderbook
    //         .order_manager
    //         .orders
    //         .insert(maker_order.order_id.clone(), maker_order.clone());
    //     zk_orderbook
    //         .order_manager
    //         .orders_owner
    //         .insert(maker_order.order_id.clone(), maker_key);

    //     let action = PermissionnedOrderbookAction::CreateOrder {
    //         order_id: taker_order.order_id.clone(),
    //         order_side: taker_order.order_side.clone(),
    //         order_type: taker_order.order_type.clone(),
    //         price: taker_order.price,
    //         pair: taker_order.pair.clone(),
    //         quantity: taker_order.quantity,
    //     };

    //     let _events = zk_orderbook
    //         .execute_permissionned_action(
    //             taker_info.clone(),
    //             action,
    //             &borsh::to_vec(&private_input).expect("serialize private input"),
    //         )
    //         .expect("expected user witness validation to fail");
    // }
}
