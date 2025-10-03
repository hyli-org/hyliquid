#[cfg(test)]
mod orderbook_tests {
    use k256::ecdsa::signature::DigestSigner;
    use k256::ecdsa::{Signature, SigningKey};
    use orderbook::orderbook::{
        ExecutionMode, OrderSide, OrderType, Orderbook, PairInfo, TokenPair,
    };
    use orderbook::smt_values::UserInfo;
    use orderbook::{
        AddSessionKeyPrivateInput, CreateOrderPrivateInput, OrderbookAction,
        PermissionnedOrderbookAction, PermissionnedPrivateInput,
    };
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

        sdk::info!("light.order_manager before exec: {:?}", light.order_manager);
        sdk::info!("full.order_manager before exec: {:?}\n", full.order_manager);

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

        sdk::info!("light.order_manager after exec: {:?}", light.order_manager);
        sdk::info!("full.order_manager after exec: {:?}\n", full.order_manager);

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
            private_input: borsh::to_vec(&permissioned_private_input)
                .expect("serialize private input"),
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

        let mut light =
            Orderbook::init(lane_id.clone(), ExecutionMode::Light, secret.clone()).unwrap();
        let mut full =
            Orderbook::init(lane_id.clone(), ExecutionMode::Full, secret.clone()).unwrap();

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
            PermissionnedOrderbookAction::CreateOrder {
                order_id: bid_order_id.clone(),
                order_side: OrderSide::Bid,
                order_type: OrderType::Limit,
                price: bid_price,
                pair: pair.clone(),
                quantity: bid_quantity,
            },
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
            PermissionnedOrderbookAction::CreateOrder {
                order_id: ask_order_id.clone(),
                order_side: OrderSide::Ask,
                order_type: OrderType::Limit,
                price: ask_price,
                pair: pair.clone(),
                quantity: ask_quantity,
            },
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

        let mut light =
            Orderbook::init(lane_id.clone(), ExecutionMode::Light, secret.clone()).unwrap();
        let mut full =
            Orderbook::init(lane_id.clone(), ExecutionMode::Full, secret.clone()).unwrap();

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

        #[derive(Clone, Copy)]
        struct BalanceExpectation {
            base: i128,
            quote: i128,
        }

        let mut expected_balances: std::collections::HashMap<&str, BalanceExpectation> = users
            .iter()
            .map(|&user| (user, BalanceExpectation { base: 0, quote: 0 }))
            .collect();

        // Step 3: Fund all users with both tokens and record expected balances
        for &user_name in &users {
            run_action(
                &mut light,
                &mut full,
                user_name,
                PermissionnedOrderbookAction::Deposit {
                    token: base_token.clone(),
                    amount: 10_000_u64,
                },
                Vec::new(),
            );
            run_action(
                &mut light,
                &mut full,
                user_name,
                PermissionnedOrderbookAction::Deposit {
                    token: quote_token.clone(),
                    amount: 10_000_u64,
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

        let assert_balances = |stage: &str,
                               light: &Orderbook,
                               full: &Orderbook,
                               expected: &std::collections::HashMap<&str, BalanceExpectation>,
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
                    "{stage}: user {user} base balance mismatch for light (expected {expected_base}, got {light_base:?})"
                );
                assert_eq!(
                    full_base.0, expected_base,
                    "{stage}: user {user} base balance mismatch for full (expected {expected_base}, got {full_base:?})"
                );
                assert_eq!(
                    light_quote.0, expected_quote,
                    "{stage}: user {user} quote balance mismatch for light (expected {expected_quote}, got {light_quote:?})"
                );
                assert_eq!(
                    full_quote.0, expected_quote,
                    "{stage}: user {user} quote balance mismatch for full (expected {expected_quote}, got {full_quote:?})"
                );
            }
        };

        assert_balances(
            "after deposits",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );

        #[derive(Clone)]
        struct StoredLimitOrder {
            owner: String,
            side: OrderSide,
            price: u64,
            quantity: u64,
        }

        #[derive(Clone)]
        struct LimitOrderSpec {
            side: OrderSide,
            price: u64,
            quantity: u64,
        }

        let limit_order_specs = [
            LimitOrderSpec {
                side: OrderSide::Ask,
                price: 130,
                quantity: 5,
            },
            LimitOrderSpec {
                side: OrderSide::Ask,
                price: 120,
                quantity: 5,
            },
            LimitOrderSpec {
                side: OrderSide::Ask,
                price: 115,
                quantity: 5,
            },
            LimitOrderSpec {
                side: OrderSide::Ask,
                price: 110,
                quantity: 5,
            },
            LimitOrderSpec {
                side: OrderSide::Ask,
                price: 105,
                quantity: 5,
            },
            LimitOrderSpec {
                side: OrderSide::Bid,
                price: 104,
                quantity: 5,
            },
        ];

        let mut limit_orders_metadata: std::collections::HashMap<String, StoredLimitOrder> =
            std::collections::HashMap::new();

        let mut order_sequence = 0_u32;

        for (idx, spec) in limit_order_specs.iter().enumerate() {
            order_sequence += 1;
            let user = users[idx % users.len()];
            let side = spec.side.clone();
            let price = Some(spec.price);
            let quantity = spec.quantity;
            let order_id = format!("{user}-limit-{order_sequence}");

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
            let private_payload =
                borsh::to_vec(&private_input).expect("serialize create order input");

            run_action(
                &mut light,
                &mut full,
                user,
                PermissionnedOrderbookAction::CreateOrder {
                    order_id: order_id.clone(),
                    order_side: side.clone(),
                    order_type: OrderType::Limit,
                    price,
                    pair: pair.clone(),
                    quantity,
                },
                private_payload,
            );

            {
                let entry = expected_balances
                    .get_mut(user)
                    .expect("expected balance entry");
                match side {
                    OrderSide::Ask => {
                        entry.base += quantity as i128;
                    }
                    OrderSide::Bid => {
                        entry.quote -= (quantity as i128) * (spec.price as i128);
                    }
                }
            }

            let expected_entry = expected_balances.get(user).expect("expected balance entry");
            let expected_base: u64 = expected_entry
                .base
                .try_into()
                .expect("base balance should stay non-negative");
            let expected_quote: u64 = expected_entry
                .quote
                .try_into()
                .expect("quote balance should stay non-negative");

            let light_user_state = light
                .get_user_info(user)
                .expect("light user info after limit order");
            let full_user_state = full
                .get_user_info(user)
                .expect("full user info after limit order");
            let light_base_after = light.get_balance(&light_user_state, &base_token).0;
            let full_base_after = full.get_balance(&full_user_state, &base_token).0;
            assert_eq!(
                light_base_after, expected_base,
                "light base balance mismatch for {user} after limit order"
            );
            assert_eq!(
                full_base_after, expected_base,
                "full base balance mismatch for {user} after limit order"
            );
            let light_quote_after = light.get_balance(&light_user_state, &quote_token).0;
            let full_quote_after = full.get_balance(&full_user_state, &quote_token).0;
            assert_eq!(
                light_quote_after, expected_quote,
                "light quote balance mismatch for {user} after limit order"
            );
            assert_eq!(
                full_quote_after, expected_quote,
                "full quote balance mismatch for {user} after limit order"
            );

            limit_orders_metadata.insert(
                order_id,
                StoredLimitOrder {
                    owner: user.to_string(),
                    side: side.clone(),
                    price: spec.price,
                    quantity,
                },
            );
        }

        assert_balances(
            "after limit orders",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );

        assert_eq!(
            light.order_manager.orders.len(),
            limit_order_specs.len(),
            "light order count after limit placement"
        );
        assert_eq!(
            full.order_manager.orders.len(),
            limit_order_specs.len(),
            "full order count after limit placement"
        );

        let mut expected_sell_orders: Vec<(u64, String)> = limit_orders_metadata
            .iter()
            .filter_map(|(order_id, order)| {
                if order.side == OrderSide::Ask {
                    Some((order.price, order_id.clone()))
                } else {
                    None
                }
            })
            .collect();
        expected_sell_orders.sort_by_key(|(price, _)| *price);
        let expected_sell_ids: Vec<String> = expected_sell_orders
            .into_iter()
            .map(|(_, order_id)| order_id)
            .collect();

        let sell_orders_light: Vec<String> = light
            .order_manager
            .sell_orders
            .get(&pair)
            .expect("light sell queue")
            .iter()
            .cloned()
            .collect();
        let sell_orders_full: Vec<String> = full
            .order_manager
            .sell_orders
            .get(&pair)
            .expect("full sell queue")
            .iter()
            .cloned()
            .collect();
        assert_eq!(
            sell_orders_light, expected_sell_ids,
            "sell order queue for light"
        );
        assert_eq!(
            sell_orders_full, expected_sell_ids,
            "sell order queue for full"
        );

        let mut expected_buy_orders: Vec<(u64, String)> = limit_orders_metadata
            .iter()
            .filter_map(|(order_id, order)| {
                if order.side == OrderSide::Bid {
                    Some((order.price, order_id.clone()))
                } else {
                    None
                }
            })
            .collect();
        expected_buy_orders.sort_by(|a, b| b.0.cmp(&a.0));
        let expected_buy_ids: Vec<String> = expected_buy_orders
            .into_iter()
            .map(|(_, order_id)| order_id)
            .collect();

        let buy_orders_light: Vec<String> = light
            .order_manager
            .buy_orders
            .get(&pair)
            .expect("light buy queue")
            .iter()
            .cloned()
            .collect();
        let buy_orders_full: Vec<String> = full
            .order_manager
            .buy_orders
            .get(&pair)
            .expect("full buy queue")
            .iter()
            .cloned()
            .collect();
        assert_eq!(
            buy_orders_light, expected_buy_ids,
            "buy order queue for light"
        );
        assert_eq!(
            buy_orders_full, expected_buy_ids,
            "buy order queue for full"
        );

        let find_order_id = |orders: &std::collections::HashMap<String, StoredLimitOrder>,
                             side: OrderSide,
                             price: u64|
         -> String {
            orders
                .iter()
                .find_map(|(order_id, order)| {
                    if order.side == side && order.price == price {
                        Some(order_id.clone())
                    } else {
                        None
                    }
                })
                .expect("order id for price")
        };

        let lowest_ask_order_id = find_order_id(&limit_orders_metadata, OrderSide::Ask, 105);
        let resting_bid_order_id = find_order_id(&limit_orders_metadata, OrderSide::Bid, 104);
        let next_ask_order_id = find_order_id(&limit_orders_metadata, OrderSide::Ask, 110);

        #[derive(Clone)]
        struct MarketOrderSpec {
            user: &'static str,
            side: OrderSide,
            quantity: u64,
            matches: Vec<String>,
        }

        let market_order_specs = [
            MarketOrderSpec {
                user: "alice",
                side: OrderSide::Bid,
                quantity: 5,
                matches: vec![lowest_ask_order_id],
            },
            MarketOrderSpec {
                user: "bob",
                side: OrderSide::Ask,
                quantity: 5,
                matches: vec![resting_bid_order_id],
            },
            MarketOrderSpec {
                user: "charlie",
                side: OrderSide::Bid,
                quantity: 5,
                matches: vec![next_ask_order_id],
            },
        ];

        for spec in market_order_specs.iter() {
            order_sequence += 1;
            let order_id = format!("{}-market-{}", spec.user, order_sequence);

            let user_index = users
                .iter()
                .position(|candidate| *candidate == spec.user)
                .expect("user index");
            let signer = &signers[user_index];
            let user_info = full
                .get_user_info(spec.user)
                .expect("user info for signature");
            let msg = format!(
                "{}:{}:create_order:{}",
                spec.user, user_info.nonce, order_id
            );
            let signature = signer.sign(&msg);
            let private_input = CreateOrderPrivateInput {
                signature,
                public_key: signer.public_key.clone(),
            };
            let private_payload =
                borsh::to_vec(&private_input).expect("serialize create order input");

            run_action(
                &mut light,
                &mut full,
                spec.user,
                PermissionnedOrderbookAction::CreateOrder {
                    order_id,
                    order_side: spec.side.clone(),
                    order_type: OrderType::Market,
                    price: None,
                    pair: pair.clone(),
                    quantity: spec.quantity,
                },
                private_payload,
            );

            let matched_pairs: Vec<(String, StoredLimitOrder)> = spec
                .matches
                .iter()
                .map(|match_id| {
                    let order = limit_orders_metadata
                        .get(match_id)
                        .expect("limit order exists for match")
                        .clone();
                    (match_id.clone(), order)
                })
                .collect();

            let matched_quantity: u64 = matched_pairs.iter().map(|(_, order)| order.quantity).sum();
            assert_eq!(
                matched_quantity, spec.quantity,
                "market order quantity should equal matched liquidity"
            );

            let mut taker_base_delta: i128 = 0;
            let mut taker_quote_delta: i128 = 0;
            let mut maker_deltas: std::collections::HashMap<String, (i128, i128)> =
                std::collections::HashMap::new();

            for (_, matched_order) in matched_pairs.iter() {
                let amount = (matched_order.quantity as i128) * (matched_order.price as i128);
                match matched_order.side {
                    OrderSide::Ask => {
                        taker_base_delta += matched_order.quantity as i128;
                        taker_quote_delta -= amount;
                        let entry = maker_deltas
                            .entry(matched_order.owner.clone())
                            .or_insert((0, 0));
                        entry.1 += amount;
                    }
                    OrderSide::Bid => {
                        taker_base_delta -= matched_order.quantity as i128;
                        taker_quote_delta += amount;
                        let entry = maker_deltas
                            .entry(matched_order.owner.clone())
                            .or_insert((0, 0));
                        entry.0 += matched_order.quantity as i128;
                    }
                }
            }

            {
                let entry = expected_balances
                    .get_mut(spec.user)
                    .expect("expected taker balance entry");
                entry.base += taker_base_delta;
                entry.quote += taker_quote_delta;
            }

            for (maker, (base_delta, quote_delta)) in maker_deltas.iter() {
                let entry = expected_balances
                    .get_mut(maker.as_str())
                    .expect("expected maker balance entry");
                entry.base += *base_delta;
                entry.quote += *quote_delta;
            }

            let expected_taker = expected_balances
                .get(spec.user)
                .expect("expected taker balance entry");
            let expected_taker_base: u64 = expected_taker
                .base
                .try_into()
                .expect("taker base balance should stay non-negative");
            let expected_taker_quote: u64 = expected_taker
                .quote
                .try_into()
                .expect("taker quote balance should stay non-negative");
            let taker_light_user = light
                .get_user_info(spec.user)
                .expect("light taker user info after market order");
            let taker_full_user = full
                .get_user_info(spec.user)
                .expect("full taker user info after market order");
            let taker_light_base = light.get_balance(&taker_light_user, &base_token).0;
            let taker_full_base = full.get_balance(&taker_full_user, &base_token).0;
            assert_eq!(
                taker_light_base, expected_taker_base,
                "light base balance mismatch for {} after market order",
                spec.user
            );
            assert_eq!(
                taker_full_base, expected_taker_base,
                "full base balance mismatch for {} after market order",
                spec.user
            );
            let taker_light_quote = light.get_balance(&taker_light_user, &quote_token).0;
            let taker_full_quote = full.get_balance(&taker_full_user, &quote_token).0;
            assert_eq!(
                taker_light_quote, expected_taker_quote,
                "light quote balance mismatch for {} after market order",
                spec.user
            );
            assert_eq!(
                taker_full_quote, expected_taker_quote,
                "full quote balance mismatch for {} after market order",
                spec.user
            );

            for (maker, _) in maker_deltas.iter() {
                let expected_maker = expected_balances
                    .get(maker.as_str())
                    .expect("expected maker balance entry");
                let expected_maker_base: u64 = expected_maker
                    .base
                    .try_into()
                    .expect("maker base balance should stay non-negative");
                let expected_maker_quote: u64 = expected_maker
                    .quote
                    .try_into()
                    .expect("maker quote balance should stay non-negative");
                let maker_light_user = light
                    .get_user_info(maker.as_str())
                    .expect("light maker user info after market order");
                let maker_full_user = full
                    .get_user_info(maker.as_str())
                    .expect("full maker user info after market order");
                let maker_light_base = light.get_balance(&maker_light_user, &base_token).0;
                let maker_full_base = full.get_balance(&maker_full_user, &base_token).0;
                assert_eq!(
                    maker_light_base, expected_maker_base,
                    "light base balance mismatch for {maker} after market order"
                );
                assert_eq!(
                    maker_full_base, expected_maker_base,
                    "full base balance mismatch for {maker} after market order"
                );
                let maker_light_quote = light.get_balance(&maker_light_user, &quote_token).0;
                let maker_full_quote = full.get_balance(&maker_full_user, &quote_token).0;
                assert_eq!(
                    maker_light_quote, expected_maker_quote,
                    "light quote balance mismatch for {maker} after market order"
                );
                assert_eq!(
                    maker_full_quote, expected_maker_quote,
                    "full quote balance mismatch for {maker} after market order"
                );
            }

            for (matched_id, _) in matched_pairs.iter() {
                limit_orders_metadata
                    .remove(matched_id)
                    .expect("remove executed limit order");
                assert!(
                    !light.order_manager.orders.contains_key(matched_id),
                    "light orderbook should not keep executed order {matched_id}"
                );
                assert!(
                    !full.order_manager.orders.contains_key(matched_id),
                    "full orderbook should not keep executed order {matched_id}"
                );
            }
        }

        assert_balances(
            "after market orders",
            &light,
            &full,
            &expected_balances,
            &users,
            &base_token,
            &quote_token,
        );

        let remaining_orders: Vec<(u64, String, OrderSide)> = limit_orders_metadata
            .iter()
            .map(|(order_id, order)| (order.price, order_id.clone(), order.side.clone()))
            .collect();

        let mut expected_remaining_sell: Vec<(u64, String)> = remaining_orders
            .iter()
            .filter_map(|(price, order_id, side)| {
                if *side == OrderSide::Ask {
                    Some((*price, order_id.clone()))
                } else {
                    None
                }
            })
            .collect();
        expected_remaining_sell.sort_by_key(|(price, _)| *price);
        let expected_remaining_sell_ids: Vec<String> = expected_remaining_sell
            .into_iter()
            .map(|(_, order_id)| order_id)
            .collect();

        let mut expected_remaining_buy: Vec<(u64, String)> = remaining_orders
            .iter()
            .filter_map(|(price, order_id, side)| {
                if *side == OrderSide::Bid {
                    Some((*price, order_id.clone()))
                } else {
                    None
                }
            })
            .collect();
        expected_remaining_buy.sort_by(|a, b| b.0.cmp(&a.0));
        let expected_remaining_buy_ids: Vec<String> = expected_remaining_buy
            .into_iter()
            .map(|(_, order_id)| order_id)
            .collect();

        let remaining_sell_light: Vec<String> = light
            .order_manager
            .sell_orders
            .get(&pair)
            .map(|orders| orders.iter().cloned().collect())
            .unwrap_or_else(Vec::new);
        let remaining_sell_full: Vec<String> = full
            .order_manager
            .sell_orders
            .get(&pair)
            .map(|orders| orders.iter().cloned().collect())
            .unwrap_or_else(Vec::new);
        assert_eq!(
            remaining_sell_light, expected_remaining_sell_ids,
            "remaining sell orders for light"
        );
        assert_eq!(
            remaining_sell_full, expected_remaining_sell_ids,
            "remaining sell orders for full"
        );

        let remaining_buy_light: Vec<String> = light
            .order_manager
            .buy_orders
            .get(&pair)
            .map(|orders| orders.iter().cloned().collect())
            .unwrap_or_else(Vec::new);
        let remaining_buy_full: Vec<String> = full
            .order_manager
            .buy_orders
            .get(&pair)
            .map(|orders| orders.iter().cloned().collect())
            .unwrap_or_else(Vec::new);
        assert_eq!(
            remaining_buy_light, expected_remaining_buy_ids,
            "remaining buy orders for light"
        );
        assert_eq!(
            remaining_buy_full, expected_remaining_buy_ids,
            "remaining buy orders for full"
        );

        assert_eq!(
            light.order_manager, full.order_manager,
            "Light and full order managers diverged"
        );

        for &user_name in &users {
            let light_user = light.get_user_info(user_name).expect("light user info");
            let full_user = full.get_user_info(user_name).expect("full user info");

            let light_base = light.get_balance(&light_user, &base_token);
            let full_base = full.get_balance(&full_user, &base_token);
            let light_quote = light.get_balance(&light_user, &quote_token);
            let full_quote = full.get_balance(&full_user, &quote_token);

            assert_eq!(
                light_base, full_base,
                "User {user_name} {base_token} balance diverged between light and full"
            );
            assert_eq!(
                light_quote, full_quote,
                "User {user_name} {quote_token} balance diverged between light and full"
            );
        }
    }
}
