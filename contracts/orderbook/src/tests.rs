#[cfg(test)]
mod orderbook_tests {
    use std::collections::BTreeMap;

    use sdk::{merkle_utils::BorshableMerkleProof, LaneId};
    use sparse_merkle_tree::MerkleProof;

    use crate::{
        orderbook::*,
        smt_values::{Balance, UserInfo},
    };

    fn test_user(name: &str) -> UserInfo {
        UserInfo::new(name.to_string(), name.as_bytes().to_vec())
    }

    fn empty_proof() -> BorshableMerkleProof {
        BorshableMerkleProof(MerkleProof::new(vec![], vec![]))
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

    fn balance_snapshot(
        orderbook: &Orderbook,
        tokens: &[TokenName],
        users: &[UserInfo],
    ) -> BTreeMap<TokenName, BTreeMap<UserInfo, Balance>> {
        let mut snapshot = BTreeMap::new();
        for token in tokens {
            let mut token_balances = BTreeMap::new();
            for user in users {
                token_balances.insert(user.clone(), orderbook.get_balance(user, token));
            }
            snapshot.insert(token.clone(), token_balances);
        }
        snapshot
    }

    fn empty_balance_proofs(tokens: &[TokenName]) -> BTreeMap<TokenName, BorshableMerkleProof> {
        tokens
            .iter()
            .map(|token| (token.clone(), empty_proof()))
            .collect()
    }

    fn register_user(orderbook: &mut Orderbook, user: &UserInfo) {
        orderbook
            .users_info_salt
            .insert(user.user.clone(), user.salt.clone());
        orderbook
            .users_info_mt
            .update(user.get_key(), user.clone())
            .expect("inserting user in SMT");
    }

    #[test]
    fn executes_partial_match_and_updates_balances() {
        let pair = ("ETH".to_string(), "USDC".to_string());
        let mut orderbook = Orderbook::init(LaneId::default(), true, b"secret".to_vec())
            .expect("orderbook initialization");

        orderbook
            .create_pair(
                pair.clone(),
                PairInfo {
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
        orderbook
            .deposit(pair.1.clone(), 1_000, &bob, &mut Balance(0), &empty_proof())
            .expect("bob quote deposit");
        orderbook
            .deposit(pair.1.clone(), 500, &alice, &mut Balance(0), &empty_proof())
            .expect("alice quote deposit");
        orderbook
            .deposit(pair.0.clone(), 80, &carol, &mut Balance(0), &empty_proof())
            .expect("carol base deposit");

        // Bob posts two bids, Alice posts one bid
        let bid_bob_primary = limit_order("bid-bob-primary", OrderSide::Bid, 10, 50, &pair);
        let bid_bob_secondary = limit_order("bid-bob-secondary", OrderSide::Bid, 8, 30, &pair);
        let bid_alice = limit_order("bid-alice", OrderSide::Bid, 9, 20, &pair);
        let tokens = vec![pair.0.clone(), pair.1.clone()];

        let balances = balance_snapshot(&orderbook, &tokens, &[bob.clone()]);
        let proofs = empty_balance_proofs(&tokens);
        orderbook
            .execute_order(
                &bob,
                bid_bob_primary.clone(),
                BTreeMap::new(),
                &balances,
                &proofs,
            )
            .expect("bob primary order");

        let balances = balance_snapshot(&orderbook, &tokens, &[bob.clone()]);
        let proofs = empty_balance_proofs(&tokens);
        orderbook
            .execute_order(
                &bob,
                bid_bob_secondary.clone(),
                BTreeMap::new(),
                &balances,
                &proofs,
            )
            .expect("bob secondary order");

        let balances = balance_snapshot(&orderbook, &tokens, &[alice.clone()]);
        let proofs = empty_balance_proofs(&tokens);
        orderbook
            .execute_order(
                &alice,
                bid_alice.clone(),
                BTreeMap::new(),
                &balances,
                &proofs,
            )
            .expect("alice bid order");

        assert_eq!(
            orderbook.order_manager.orders.len(),
            3,
            "three resting bids"
        );

        // Carol sells part of Bob's primary order.
        let mut order_user_map = BTreeMap::new();
        order_user_map.insert(bid_bob_primary.order_id.clone(), bob.clone());
        order_user_map.insert(bid_bob_secondary.order_id.clone(), bob.clone());
        order_user_map.insert(bid_alice.order_id.clone(), alice.clone());

        let carol_users = vec![bob.clone(), carol.clone()];
        let balances = balance_snapshot(&orderbook, &tokens, &carol_users);
        let proofs = empty_balance_proofs(&tokens);
        let carol_order = limit_order("ask-carol", OrderSide::Ask, 9, 40, &pair);
        let events = orderbook
            .execute_order(
                &carol,
                carol_order.clone(),
                order_user_map,
                &balances,
                &proofs,
            )
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
                OrderbookEvent::BalanceUpdated { user, token, amount } => {
                    Some(((user.clone(), token.clone()), *amount))
                }
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
            .get_order(&bid_bob_primary.order_id)
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
}
