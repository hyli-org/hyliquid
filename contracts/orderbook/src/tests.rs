#[cfg(test)]
mod orderbook_tests {
    use sdk::{hyli_model_utils::TimestampMs, LaneId};

    use crate::{
        orderbook::{OrderSide, OrderbookEvent},
        *,
    };

    fn setup() -> (String, String, Orderbook) {
        let mut orderbook = Orderbook::init(LaneId::default(), true, vec![]);
        let eth_user = "eth_user".to_string();
        let usd_user = "usd_user".to_string();

        let mut eth_token = BTreeMap::new();
        eth_token.insert(
            eth_user.clone(),
            orderbook::UserInfo {
                balance: 10,
                secret: Vec::new(),
            },
        );
        orderbook.balances.insert("ETH".to_string(), eth_token);

        let mut usd_token = BTreeMap::new();
        usd_token.insert(
            usd_user.clone(),
            orderbook::UserInfo {
                balance: 3000,
                secret: Vec::new(),
            },
        );
        orderbook.balances.insert("USD".to_string(), usd_token);

        (eth_user, usd_user, orderbook)
    }

    #[test_log::test]
    fn test_limit_sell_order_create() {
        let (eth_user, _, mut orderbook) = setup();

        // Create a limit sell order
        let order = Order {
            order_id: "order1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };

        let events = orderbook
            .execute_order(&eth_user, order.clone(), BTreeMap::default())
            .unwrap();

        // Check that the order was created
        assert_eq!(events.len(), 3);
        let created_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderCreated { .. }))
            .count();
        let balance_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(created_count, 1);
        assert_eq!(balance_count, 2);

        // Check that the order is in the sell orders list
        assert!(orderbook.orders.contains_key("order1"));
        assert!(orderbook
            .sell_orders
            .get(&("ETH".to_string(), "USD".to_string()))
            .unwrap()
            .contains(&"order1".to_string()));
    }

    #[test_log::test]
    fn test_limit_buy_order_create() {
        let (_, usd_user, mut orderbook) = setup();

        // Create a limit buy order
        let order = Order {
            order_id: "order1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };

        let events = orderbook
            .execute_order(&usd_user, order.clone(), BTreeMap::default())
            .unwrap();

        // Check that the order was created
        assert_eq!(events.len(), 3);
        let created_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderCreated { .. }))
            .count();
        let balance_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(created_count, 1);
        assert_eq!(balance_count, 2);

        // Check that the order is in the buy orders list
        assert!(orderbook.orders.contains_key("order1"));
        assert!(orderbook
            .buy_orders
            .get(&("ETH".to_string(), "USD".to_string()))
            .unwrap()
            .contains(&"order1".to_string()));
    }

    #[test_log::test]
    fn test_limit_order_match_same_quantity_same_price() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit sell order first
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&eth_user, sell_order, BTreeMap::default())
            .unwrap();

        // Create a matching buy order
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &usd_user,
                buy_order,
                BTreeMap::from([("sell1".to_string(), eth_user.clone())]),
            )
            .unwrap();

        // Check that the order was executed
        assert_eq!(events.len(), 6);
        let executed_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderExecuted { .. }))
            .count();
        let balance_updated_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(executed_count, 2);
        // usd_user received ETH
        // usd_user sent USD
        // orderbook sent ETH
        // eth_user received USD
        assert_eq!(balance_updated_count, 4);
        // usd_user received ETH
        // usd_user sent USD
        // orderbook sent ETH
        // eth_user received USD

        // Check balances were updated correctly
        let eth_user_usd = orderbook
            .balances
            .get("USD")
            .unwrap()
            .get(&eth_user)
            .unwrap();
        let usd_user_eth = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&usd_user)
            .unwrap();

        assert_eq!(eth_user_usd.balance, 2000); // Seller received USD
        assert_eq!(usd_user_eth.balance, 1); // Buyer received ETH
    }

    #[test_log::test]
    fn test_limit_order_match_same_quantity_lower_price() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit sell order first
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&eth_user, sell_order, BTreeMap::default())
            .unwrap();

        // Create a matching buy order
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(1900),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(1),
        };
        let events = orderbook
            .execute_order(
                &usd_user,
                buy_order,
                BTreeMap::from([("sell1".to_string(), eth_user.clone())]),
            )
            .unwrap();

        // Check that the order was NOT executed
        assert_eq!(events.len(), 3);
        let created_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderCreated { .. }))
            .count();
        let balance_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(created_count, 1);
        assert_eq!(balance_count, 2);
        // Check balances were updated correctly
        let eth_user_usd = orderbook
            .balances
            .get("USD")
            .unwrap()
            .get(&eth_user)
            .cloned()
            .unwrap_or_default();
        let usd_user_eth = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&usd_user)
            .cloned()
            .unwrap_or_default();

        assert_eq!(eth_user_usd.balance, 0); // Seller did not received USD
        assert_eq!(usd_user_eth.balance, 0); // Buyer did not received ETH

        // Check user correctly desposited the amount
        let eth_user_eth = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&eth_user)
            .unwrap();
        let usd_user_usd = orderbook
            .balances
            .get("USD")
            .unwrap()
            .get(&usd_user)
            .unwrap();

        assert_eq!(eth_user_eth.balance, 10 - 1); // Seller did not received USD
        assert_eq!(usd_user_usd.balance, 3000 - 1900); // Buyer did not received ETH
    }

    #[test_log::test]
    fn test_limit_order_match_same_quantity_lower_price_bis() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit buy order first
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(1900),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&usd_user, sell_order, BTreeMap::default())
            .unwrap();

        // Create a matching sell order
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(1),
        };
        let events = orderbook
            .execute_order(
                &eth_user,
                buy_order,
                BTreeMap::from([("sell1".to_string(), usd_user.clone())]),
            )
            .unwrap();

        // Check that the order was NOT executed
        assert_eq!(events.len(), 3);
        let created_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderCreated { .. }))
            .count();
        let balance_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(created_count, 1);
        assert_eq!(balance_count, 2);

        // Check balances were updated correctly
        let eth_user_usd = orderbook
            .balances
            .get("USD")
            .unwrap()
            .get(&eth_user)
            .cloned()
            .unwrap_or_default();
        let usd_user_eth = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&usd_user)
            .cloned()
            .unwrap_or_default();

        assert_eq!(eth_user_usd.balance, 0); // Seller did not received USD
        assert_eq!(usd_user_eth.balance, 0); // Buyer did not received ETH

        // Check user correctly desposited the amount
        let eth_user_eth = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&eth_user)
            .cloned()
            .unwrap_or_default();
        let usd_user_usd = orderbook
            .balances
            .get("USD")
            .unwrap()
            .get(&usd_user)
            .cloned()
            .unwrap_or_default();

        assert_eq!(eth_user_eth.balance, 10 - 1); // Seller did not received USD
        assert_eq!(usd_user_usd.balance, 3000 - 1900); // Buyer did not received ETH
    }

    #[test_log::test]
    fn test_limit_order_match_same_quantity_higher_price() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit sell order first
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&eth_user, sell_order, BTreeMap::default())
            .unwrap();

        // Create a matching buy order
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(2100),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &usd_user,
                buy_order,
                BTreeMap::from([("sell1".to_string(), eth_user.clone())]),
            )
            .unwrap();

        // Check that the order was executed
        assert_eq!(events.len(), 6);
        let executed_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderExecuted { .. }))
            .count();
        let balance_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(executed_count, 2);
        // usd_user received ETH
        // usd_user sent USD
        // orderbook sent ETH
        // eth_user received USD
        assert_eq!(balance_count, 4);

        // Check balances were updated correctly
        let usd_balances = orderbook.balances.get("USD").unwrap();
        let eth_balances = orderbook.balances.get("ETH").unwrap();

        assert_eq!(usd_balances.get(&eth_user).unwrap().balance, 2000);
        assert_eq!(usd_balances.get(&usd_user).unwrap().balance, 1000);

        assert_eq!(eth_balances.get(&eth_user).unwrap().balance, 9);
        assert_eq!(eth_balances.get(&usd_user).unwrap().balance, 1);
    }

    #[test_log::test]
    fn test_limit_order_match_less_sell_quantity_same_price() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit sell order for 1 ETH
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(1000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&eth_user, sell_order, BTreeMap::default())
            .unwrap();

        // Create a buy order for 2 ETH
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(1000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 2,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &usd_user,
                buy_order,
                BTreeMap::from([("sell1".to_string(), eth_user.clone())]),
            )
            .unwrap();

        // Check that the order was NOT executed
        assert_eq!(events.len(), 7);
        let executed_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderExecuted { .. }))
            .count();
        let created_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderCreated { .. }))
            .count();
        let balance_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(executed_count, 1);
        assert_eq!(created_count, 1);
        // eth_user received USD
        // usd_user sent USD
        // usd_user received ETH
        // orderbook received USD
        // orderbook sent ETH
        assert_eq!(balance_count, 5);

        assert_eq!(orderbook.orders.len(), 1);
        let only_order = orderbook.orders.values().next().unwrap();
        assert!(matches!(only_order.order_side, OrderSide::Bid));

        // Check balances were updated correctly
        let usd_balances = orderbook.balances.get("USD").unwrap();
        let eth_balances = orderbook.balances.get("ETH").unwrap();

        assert_eq!(usd_balances.get(&eth_user).unwrap().balance, 1000);
        assert_eq!(usd_balances.get(&usd_user).unwrap().balance, 1000);
        assert_eq!(usd_balances.get("orderbook").unwrap().balance, 1000);

        assert_eq!(eth_balances.get(&eth_user).unwrap().balance, 9);
        assert_eq!(eth_balances.get(&usd_user).unwrap().balance, 1);
        assert_eq!(eth_balances.get("orderbook").unwrap().balance, 0);
    }

    #[test_log::test]
    fn test_partial_order_execution_same_price() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit sell order for 2 ETH
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 2,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&eth_user, sell_order, BTreeMap::default())
            .unwrap();

        // Create a buy order for 1 ETH
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &usd_user,
                buy_order,
                BTreeMap::from([("sell1".to_string(), eth_user.clone())]),
            )
            .unwrap();

        // Check that we got an OrderUpdate event
        assert!(events.iter().any(|event| matches!(event,
            OrderbookEvent::OrderUpdate {
                order_id,
                remaining_quantity,
                pair: _
            } if order_id == "sell1" && *remaining_quantity == 1
        )));

        // Check balance updates
        let balance_updated_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        // eth_user received USD
        // usd_user sent USD
        // usd_user received ETH
        // orderbook sent ETH
        assert_eq!(balance_updated_count, 4);

        // Check balances were updated correctly
        let seller_usd = orderbook
            .balances
            .get("USD")
            .unwrap()
            .get(&eth_user)
            .unwrap();
        let buyer_eth = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&usd_user)
            .unwrap();

        assert_eq!(seller_usd.balance, 2000); // Seller received USD for 1 ETH
        assert_eq!(buyer_eth.balance, 1); // Buyer received 1 ETH

        // Check that the sell order is still in the orderbook with updated quantity
        let remaining_order = orderbook.orders.get("sell1").unwrap();
        assert_eq!(remaining_order.quantity, 1);
    }

    #[test_log::test]
    fn test_partial_order_execution_higher_price() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit sell order for 2 ETH
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 2,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&eth_user, sell_order, BTreeMap::default())
            .unwrap();

        // Create a buy order for 1 ETH at a higher price
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(2100),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &usd_user,
                buy_order,
                BTreeMap::from([("sell1".to_string(), eth_user.clone())]),
            )
            .unwrap();

        // Check that we got an OrderUpdate event
        assert!(events.iter().any(|event| matches!(event,
            OrderbookEvent::OrderUpdate {
                order_id,
                remaining_quantity,
                pair: _
            } if order_id == "sell1" && *remaining_quantity == 1
        )));

        // Check balance updates
        let balance_updated_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        // eth_user received USD
        // usd_user sent USD
        // usd_user received ETH
        // orderbook sent ETH
        assert_eq!(balance_updated_count, 4);

        // Check balances were updated correctly
        let seller_usd = orderbook
            .balances
            .get("USD")
            .unwrap()
            .get(&eth_user)
            .unwrap();
        let buyer_eth = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&usd_user)
            .unwrap();

        assert_eq!(seller_usd.balance, 2000); // Seller received USD for 1 ETH (at sell price)
        assert_eq!(buyer_eth.balance, 1); // Buyer received 1 ETH

        // Check that the sell order is still in the orderbook with updated quantity
        let remaining_order = orderbook.orders.get("sell1").unwrap();
        assert_eq!(remaining_order.quantity, 1);
    }

    #[test_log::test]
    fn test_market_sell_order_against_larger_buy_order() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit buy order first for 2 ETH
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(1000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 2,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&usd_user, buy_order, BTreeMap::default())
            .unwrap();

        // Create a market sell order for 1 ETH
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Market,
            price: None, // Market order
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &eth_user,
                sell_order,
                BTreeMap::from([("buy1".to_string(), usd_user.clone())]),
            )
            .unwrap();

        // Order should be executed immediately at the buy order's price
        assert!(events
            .iter()
            .any(|event| matches!(event, OrderbookEvent::OrderUpdate {
            order_id,
            remaining_quantity,
            pair: _
        } if order_id == "buy1" && *remaining_quantity == 1)));

        // Check balance updates
        let balance_updated_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        // eth_user sent ETH
        // usd_user received ETH
        // orderbook sent USD
        // eth_user received USD
        assert_eq!(balance_updated_count, 4);

        // Check balances were updated correctly
        let eth_user_usd = orderbook
            .balances
            .get("USD")
            .unwrap()
            .get(&eth_user)
            .unwrap();
        let usd_user_eth = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&usd_user)
            .unwrap();

        assert_eq!(eth_user_usd.balance, 1000); // Seller got the buy order's price
        assert_eq!(usd_user_eth.balance, 1); // Buyer got their ETH
    }

    #[test_log::test]
    fn test_market_sell_order_against_exact_buy_order() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit buy order first for 1 ETH
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&usd_user, buy_order, BTreeMap::default())
            .unwrap();

        // Create a market sell order for 1 ETH
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Market,
            price: None, // Market order
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &eth_user,
                sell_order,
                BTreeMap::from([("buy1".to_string(), usd_user.clone())]),
            )
            .unwrap();

        assert_eq!(events.len(), 5);
        let executed_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderExecuted { .. }))
            .count();
        let balance_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(executed_count, 1);
        // eth_user sent ETH
        // usd_user received ETH
        // orderbook sent USD
        // eth_user received USD
        assert_eq!(balance_count, 4);

        // Assert orderbook is empty
        assert_eq!(orderbook.orders.len(), 0);

        // Check that balances have been updated correctly
        let eth_balances = orderbook.balances.get("ETH").unwrap();
        let usd_balances = orderbook.balances.get("USD").unwrap();

        assert_eq!(eth_balances.get(&eth_user).unwrap().balance, 9); // eth_user sold 1 ETH ...
        assert_eq!(usd_balances.get(&eth_user).unwrap().balance, 2000); // .. for 2000 USD

        assert_eq!(eth_balances.get(&usd_user).unwrap().balance, 1); // usd_user bought 1 ETH ...
        assert_eq!(usd_balances.get(&usd_user).unwrap().balance, 1000); // .. for 2000 USD
    }

    #[test_log::test]
    fn test_market_sell_order_against_smaller_buy_order() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit buy order first for 1 ETH
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };

        orderbook
            .execute_order(&usd_user, buy_order, BTreeMap::default())
            .unwrap();

        // Create a market sell order for 2 ETH
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Market,
            price: None, // Market order
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 2,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &eth_user,
                sell_order,
                BTreeMap::from([("buy1".to_string(), usd_user.clone())]),
            )
            .unwrap();

        assert_eq!(events.len(), 5);
        let executed_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderExecuted { .. }))
            .count();
        let balance_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(executed_count, 1);
        // eth_user sent ETH
        // usd_user received ETH
        // orderbook sent USD
        // eth_user received USD
        assert_eq!(balance_count, 4);

        // Assert orderbook is empty
        assert_eq!(orderbook.orders.len(), 0);

        // Check that balances have been updated correctly
        let eth_balances = orderbook.balances.get("ETH").unwrap();
        let usd_balances = orderbook.balances.get("USD").unwrap();

        assert_eq!(eth_balances.get(&eth_user).unwrap().balance, 9); // eth_user sold 1 ETH ...
        assert_eq!(usd_balances.get(&eth_user).unwrap().balance, 2000); // .. for 2000 USD

        assert_eq!(eth_balances.get(&usd_user).unwrap().balance, 1); // usd_user bought 1 ETH ...
        assert_eq!(usd_balances.get(&usd_user).unwrap().balance, 1000); // .. for 2000 USD
    }

    // Tests with existing sell orders
    #[test_log::test]
    fn test_market_buy_order_against_larger_sell_order() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit sell order first for 2 ETH
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 2,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&eth_user, sell_order, BTreeMap::default())
            .unwrap();

        // Create a market buy order for 1 ETH
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Market,
            price: None, // Market order
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &usd_user,
                buy_order,
                BTreeMap::from([("sell1".to_string(), eth_user.clone())]),
            )
            .unwrap();

        // Order should be executed immediately at the sell order's price
        assert!(events
            .iter()
            .any(|event| matches!(event, OrderbookEvent::OrderUpdate {
            order_id,
            remaining_quantity,
            pair: _
        } if order_id == "sell1" && *remaining_quantity == 1)));

        // Check balance updates
        let balance_updated_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        // eth_user received USD
        // usd_user sent USD
        // usd_user received ETH
        // orderbook sent ETH
        assert_eq!(balance_updated_count, 4);

        // Check balances were updated correctly
        let eth_user_usd = orderbook
            .balances
            .get("USD")
            .unwrap()
            .get(&eth_user)
            .unwrap();
        let usd_user_eth = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&usd_user)
            .unwrap();

        assert_eq!(eth_user_usd.balance, 2000); // Seller got their asking price
        assert_eq!(usd_user_eth.balance, 1); // Buyer got their ETH
    }

    #[test_log::test]
    fn test_market_buy_order_against_exact_sell_order() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit sell order first for 1 ETH
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&eth_user, sell_order, BTreeMap::default())
            .unwrap();

        // Create a market buy order for 1 ETH
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Market,
            price: None, // Market order
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &usd_user,
                buy_order,
                BTreeMap::from([("sell1".to_string(), eth_user.clone())]),
            )
            .unwrap();

        // Both orders should be fully executed
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, OrderbookEvent::OrderExecuted { .. }))
                .count(),
            2
        );

        // Check balance updates
        let balance_updated_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        // eth_user sent ETH
        // usd_user received ETH
        // usd_user sent USD
        // eth_user received USD
        assert_eq!(balance_updated_count, 4);

        // Check balances
        let eth_user_usd = orderbook
            .balances
            .get("USD")
            .unwrap()
            .get(&eth_user)
            .unwrap();
        let usd_user_eth = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&usd_user)
            .unwrap();

        assert_eq!(eth_user_usd.balance, 2000);
        assert_eq!(usd_user_eth.balance, 1);
    }

    #[test_log::test]
    fn test_market_buy_order_against_smaller_sell_order() {
        let (eth_user, usd_user, mut orderbook) = setup();

        // Create a limit sell order first for 1 ETH
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };
        orderbook
            .execute_order(&eth_user, sell_order, BTreeMap::default())
            .unwrap();

        // Create a market buy order for 2 ETH
        let buy_order = Order {
            order_id: "buy1".to_string(),
            order_side: OrderSide::Bid,
            order_type: OrderType::Market,
            price: None, // Market order
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 2,
            timestamp: TimestampMs(1),
        };

        let events = orderbook
            .execute_order(
                &usd_user,
                buy_order,
                BTreeMap::from([("sell1".to_string(), eth_user.clone())]),
            )
            .unwrap();

        assert_eq!(events.len(), 5);
        let executed_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderExecuted { .. }))
            .count();
        let balance_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(executed_count, 1);
        // eth_user sent ETH
        // usd_user received ETH
        // orderbook sent USD
        // eth_user received USD
        assert_eq!(balance_count, 4);

        // Assert orderbook is empty
        assert_eq!(orderbook.orders.len(), 0);

        // Check that balances have been updated correctly
        let eth_balances = orderbook.balances.get("ETH").unwrap();
        let usd_balances = orderbook.balances.get("USD").unwrap();

        assert_eq!(eth_balances.get(&eth_user).unwrap().balance, 9); // eth_user sold 1 ETH ...
        assert_eq!(usd_balances.get(&eth_user).unwrap().balance, 2000); // .. for 2000 USD

        assert_eq!(eth_balances.get(&usd_user).unwrap().balance, 1); // usd_user bought 1 ETH ...
        assert_eq!(usd_balances.get(&usd_user).unwrap().balance, 1000); // .. for 2000 USD
    }

    #[test_log::test]
    fn test_cancel_order() {
        let (eth_user, _, mut orderbook) = setup();

        // Create a limit sell order first
        let sell_order = Order {
            order_id: "sell1".to_string(),
            order_side: OrderSide::Ask,
            order_type: OrderType::Limit,
            price: Some(2000),
            pair: ("ETH".to_string(), "USD".to_string()),
            quantity: 1,
            timestamp: TimestampMs(0),
        };

        // Execute the order
        orderbook
            .execute_order(&eth_user, sell_order, BTreeMap::default())
            .unwrap();

        // Verify order exists
        assert!(orderbook.orders.contains_key("sell1"));
        assert_eq!(orderbook.orders.len(), 1);

        // Check initial balances after order creation
        let eth_user_balance_before = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&eth_user)
            .unwrap()
            .balance;
        let orderbook_balance_before = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get("orderbook")
            .unwrap()
            .balance;

        assert_eq!(eth_user_balance_before, 9); // 10 - 1 (reserved for order)
        assert_eq!(orderbook_balance_before, 1); // 1 reserved for the order

        // Cancel the order
        let events = orderbook
            .cancel_order("sell1".to_string(), &eth_user)
            .unwrap();

        // Check events
        assert_eq!(events.len(), 3);
        let cancelled_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::OrderCancelled { .. }))
            .count();
        let balance_count = events
            .iter()
            .filter(|e| matches!(e, OrderbookEvent::BalanceUpdated { .. }))
            .count();
        assert_eq!(cancelled_count, 1);
        assert_eq!(balance_count, 2);

        // Verify order was removed
        assert!(!orderbook.orders.contains_key("sell1"));
        assert_eq!(orderbook.orders.len(), 0);

        // Check balances were restored
        let eth_user_balance_after = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&eth_user)
            .unwrap()
            .balance;
        let orderbook_balance_after = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get("orderbook")
            .unwrap()
            .balance;

        assert_eq!(eth_user_balance_after, 10); // Balance restored
        assert_eq!(orderbook_balance_after, 0); // Orderbook balance back to 0
    }

    #[test_log::test]
    fn test_withdraw() {
        let (eth_user, _, mut orderbook) = setup();

        // Check initial balance
        let initial_balance = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&eth_user)
            .unwrap()
            .balance;
        assert_eq!(initial_balance, 10);

        // Test successful withdrawal
        let events = orderbook.withdraw("ETH".to_string(), 3, &eth_user).unwrap();

        // Check events
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], OrderbookEvent::BalanceUpdated { .. }));

        // Check balance was updated
        let new_balance = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&eth_user)
            .unwrap()
            .balance;
        assert_eq!(new_balance, 7); // 10 - 3

        // Test withdrawal with insufficient balance
        let result = orderbook.withdraw("ETH".to_string(), 10, &eth_user);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Insufficient balance"));
        assert!(err.contains(&eth_user));
        assert!(err.contains("has 7 ETH tokens"));
        assert!(err.contains("trying to withdraw 10"));

        // Verify balance unchanged after failed withdrawal
        let balance_after_fail = orderbook
            .balances
            .get("ETH")
            .unwrap()
            .get(&eth_user)
            .unwrap()
            .balance;
        assert_eq!(balance_after_fail, 7);
    }

    // TODO: This test is disabled as get_latest_deposit_mut and BlockHeight are not implemented
    // #[test_log::test]
    // fn test_order_execution_blocked_after_recent_deposit() {
    //     let (eth_user, _, mut orderbook) = setup();

    //     // Set a more recent deposit block height for eth_user
    //     *orderbook.get_latest_deposit_mut(&eth_user, "ETH") = BlockHeight(4);

    //     // Try to create a sell order when deposit was too recent
    //     let sell_order = Order {
    //         order_id: "sell1".to_string(),
    //         order_side: OrderSide::Sell,
    //         order_type: OrderType::Limit,
    //         price: Some(2000),
    //         pair: ("ETH".to_string(), "USD".to_string()),
    //         quantity: 1,

    //         timestamp: TimestampMs(0),
    //     };

    //     // Execute order with tx_ctx at block height 6 (< deposit block + 5)
    //     let result = orderbook.execute_order(&eth_user, sell_order, BTreeMap::default());

    //     // Should fail because not enough blocks have passed since deposit
    //     assert!(result.is_err());
    //     let err = result.unwrap_err();
    //     assert!(err.contains("too soon after the last deposit"));
    //     assert!(err.contains("5 blocks are required"));

    //     // Check no balances were modified
    //     let eth_user_eth = orderbook
    //         .balances
    //         .get("ETH")
    //         .unwrap()
    //         .get(&eth_user)
    //         .unwrap();
    //     assert_eq!(eth_user_eth.balance, 10); // Original balance unchanged

    //     // Check no orders were created
    //     assert_eq!(orderbook.orders.len(), 0);
    // }
}
