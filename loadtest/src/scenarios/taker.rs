use goose::prelude::*;
use orderbook::model::{OrderSide, OrderType};
use tracing::{debug, info, warn};

use crate::http_client::{build_order, OrderbookClient};
use crate::scenarios::setup_scenario;
use crate::state::UserState;
use crate::GLOBAL_CONFIG;
use crate::GLOBAL_SHARED_STATE;

/// Transaction: Fetch current orderbook to get best bid/ask
async fn get_orderbook_transaction(user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let client = OrderbookClient::new(&config).unwrap();

    // Fetch current orderbook to get best bid/ask
    let _orderbook = match client
        .get_orderbook(
            user,
            &config.instrument.base_asset,
            &config.instrument.quote_asset,
            1, // Only need top level
        )
        .await
    {
        Ok(book) => book,
        Err(e) => {
            warn!("Taker: failed to fetch orderbook: {:?}", e);
            return Ok(());
        }
    };

    Ok(())
}

/// Transaction: Place taker order (buy or sell)
async fn place_taker_order_transaction(user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let shared_state = {
        let global_shared_state = GLOBAL_SHARED_STATE.lock().unwrap();
        global_shared_state.clone().unwrap()
    };

    let client = OrderbookClient::new(&config).unwrap();

    // Fetch current orderbook to get best bid/ask
    let orderbook = match client
        .get_orderbook(
            user,
            &config.instrument.base_asset,
            &config.instrument.quote_asset,
            1, // Only need top level
        )
        .await
    {
        Ok(book) => book,
        Err(e) => {
            warn!("Taker: failed to fetch orderbook: {:?}", e);
            return Ok(());
        }
    };

    let best_bid = orderbook.best_bid();
    let best_ask = orderbook.best_ask();

    if best_bid.is_none() && best_ask.is_none() {
        debug!("Taker: orderbook is empty, skipping");
        return Ok(());
    }

    // Randomly decide whether to buy or sell
    let is_buy = shared_state.random_bool(0.5);

    if is_buy {
        // Buy: cross the ask
        if let Some(ask) = best_ask {
            let user_state = user.get_session_data_mut::<UserState>().unwrap();
            let user_auth = user_state.auth.clone();
            let cross_price =
                (ask.price as u64) + (config.taker.cross_ticks * config.instrument.price_tick);
            let quantity = shared_state.random_range(
                config.taker.min_quantity_steps,
                config.taker.max_quantity_steps,
            ) * config.instrument.qty_step;

            let order_id = user_state.generate_order_id("taker_buy");
            let nonce = user_state.next_nonce();

            let order = build_order(
                order_id.clone(),
                OrderSide::Bid,
                OrderType::Limit,
                Some(cross_price),
                config.pair(),
                quantity,
            );

            let signature = user_state.auth.sign_create_order(nonce, &order_id).unwrap();

            match client
                .create_order(user, &user_auth, &order, &signature)
                .await
            {
                Ok(_) => {
                    info!(
                        "Taker: BUY order {} @ {} (crossing ask @ {}) qty {}",
                        order_id, cross_price, ask.price, quantity
                    );
                }
                Err(e) => {
                    warn!("Taker: failed to place buy order: {:?}", e);
                }
            }
        } else {
            debug!("Taker: no asks available");
        }
    } else {
        // Sell: cross the bid
        if let Some(bid) = best_bid {
            let user_state = user.get_session_data_mut::<UserState>().unwrap();
            let user_auth = user_state.auth.clone();
            let cross_price =
                if bid.price as u64 > config.taker.cross_ticks * config.instrument.price_tick {
                    (bid.price as u64) - (config.taker.cross_ticks * config.instrument.price_tick)
                } else {
                    1 // Minimum price
                };

            let quantity = shared_state.random_range(
                config.taker.min_quantity_steps,
                config.taker.max_quantity_steps,
            ) * config.instrument.qty_step;

            let order_id = user_state.generate_order_id("taker_sell");
            let nonce = user_state.next_nonce();

            let order = build_order(
                order_id.clone(),
                OrderSide::Ask,
                OrderType::Limit,
                Some(cross_price),
                config.pair(),
                quantity,
            );

            let signature = user_state.auth.sign_create_order(nonce, &order_id).unwrap();

            match client
                .create_order(user, &user_auth, &order, &signature)
                .await
            {
                Ok(_) => {
                    info!(
                        "Taker: SELL order {} @ {} (crossing bid @ {}) qty {}",
                        order_id, cross_price, bid.price, quantity
                    );
                }
                Err(e) => {
                    warn!("Taker: failed to place sell order: {:?}", e);
                }
            }
        } else {
            debug!("Taker: no bids available");
        }
    }

    Ok(())
}

/// Transaction: Wait before next taker order
async fn taker_wait_transaction(_user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    // Wait before next taker order
    tokio::time::sleep(tokio::time::Duration::from_millis(config.taker.interval_ms)).await;

    Ok(())
}

/// Creates the taker scenario with all its transactions
pub fn taker_scenario() -> Scenario {
    setup_scenario("Taker")
        .register_transaction(
            transaction!(get_orderbook_transaction)
                .set_name("get_orderbook")
                .set_sequence(10),
        )
        .register_transaction(
            transaction!(place_taker_order_transaction)
                .set_name("place_order")
                .set_sequence(20),
        )
        .register_transaction(
            transaction!(taker_wait_transaction)
                .set_name("wait_interval")
                .set_sequence(30),
        )
}
