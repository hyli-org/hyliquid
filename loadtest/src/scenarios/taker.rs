use goose::prelude::*;
use orderbook::model::{OrderSide, OrderType};
use tracing::{info, warn};

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

    // Randomly decide whether to buy or sell
    let is_buy = shared_state.random_bool(0.5);

    if is_buy {
        // Buy: cross the ask
        if let Some(ask) = best_ask {
            place_crossing_order(
                user,
                &client,
                &shared_state,
                &config,
                OrderSide::Bid,
                ask.price as u64,
                "taker_buy",
            )
            .await?;
        } else {
            place_self_cross_pair(user, &client, &shared_state, &config).await?;
        }
    } else {
        // Sell: cross the bid
        if let Some(bid) = best_bid {
            place_crossing_order(
                user,
                &client,
                &shared_state,
                &config,
                OrderSide::Ask,
                bid.price as u64,
                "taker_sell",
            )
            .await?;
        } else {
            place_self_cross_pair(user, &client, &shared_state, &config).await?;
        }
    }

    Ok(())
}

/// Place an aggressive order that crosses the provided best price
async fn place_crossing_order(
    user: &mut GooseUser,
    client: &OrderbookClient,
    shared_state: &crate::state::SharedState,
    config: &crate::config::Config,
    side: OrderSide,
    best_price: u64,
    prefix: &str,
) -> TransactionResult {
    let order_side = side.clone();
    let price_tick = config.instrument.price_tick;
    let cross_price = match order_side {
        OrderSide::Bid => best_price + (config.taker.cross_ticks * price_tick),
        OrderSide::Ask => best_price
            .saturating_sub(config.taker.cross_ticks * price_tick)
            .max(1),
    };

    let quantity = shared_state.random_range(
        config.taker.min_quantity_steps,
        config.taker.max_quantity_steps,
    ) * config.instrument.qty_step;

    let (order_id, signature, auth, order) = {
        let user_state = user.get_session_data_mut::<UserState>().unwrap();
        let order_id = user_state.generate_order_id(prefix);
        let nonce = user_state.next_nonce();
        let auth = user_state.auth.clone();
        let order = build_order(
            order_id.clone(),
            order_side.clone(),
            OrderType::Limit,
            Some(cross_price),
            config.pair(),
            quantity,
        );
        let signature = auth.sign_create_order(nonce, &order_id).unwrap();
        (order_id, signature, auth, order)
    };

    let result = client.create_order(user, &auth, &order, &signature).await;

    if let Err(e) = result {
        warn!(
            "Taker: failed to place {} order: {:?}",
            match side {
                OrderSide::Bid => "buy",
                OrderSide::Ask => "sell",
            },
            e
        );
        let user_state = user.get_session_data_mut::<UserState>().unwrap();
        user_state.revert_nonce();
        return Ok(());
    }

    {
        shared_state
            .order_tracker
            .lock()
            .unwrap()
            .add_order(order_id.clone());
    }

    info!(
        "Taker: placed {} order {} @ {} qty {} (best {})",
        match order_side {
            OrderSide::Bid => "buy",
            OrderSide::Ask => "sell",
        },
        order_id,
        cross_price,
        quantity,
        best_price
    );

    Ok(())
}

/// When no liquidity exists, create a crossed bid/ask pair using the shared mid
async fn place_self_cross_pair(
    user: &mut GooseUser,
    client: &OrderbookClient,
    shared_state: &crate::state::SharedState,
    config: &crate::config::Config,
) -> TransactionResult {
    let mid = shared_state.mid_price.lock().unwrap().get();
    let price_tick = config.instrument.price_tick;
    let cross = config.taker.cross_ticks * price_tick;

    let bid_price = mid.saturating_add(cross);
    let ask_price = (mid.saturating_sub(cross)).max(price_tick);

    let quantity = shared_state.random_range(
        config.taker.min_quantity_steps,
        config.taker.max_quantity_steps,
    ) * config.instrument.qty_step;

    // Place bid
    let (bid_order_id, bid_signature, bid_auth, bid_order) = {
        let user_state = user.get_session_data_mut::<UserState>().unwrap();
        let order_id = user_state.generate_order_id("taker_cross_bid");
        let nonce = user_state.next_nonce();
        let auth = user_state.auth.clone();
        let order = build_order(
            order_id.clone(),
            OrderSide::Bid,
            OrderType::Limit,
            Some(bid_price),
            config.pair(),
            quantity,
        );
        let signature = auth.sign_create_order(nonce, &order_id).unwrap();
        (order_id, signature, auth, order)
    };

    let bid_res = client
        .create_order(user, &bid_auth, &bid_order, &bid_signature)
        .await;

    if let Err(e) = bid_res {
        warn!("Taker: failed to place crossed bid: {:?}", e);
        let user_state = user.get_session_data_mut::<UserState>().unwrap();
        user_state.revert_nonce();
        return Ok(());
    }

    {
        shared_state
            .order_tracker
            .lock()
            .unwrap()
            .add_order(bid_order_id.clone());
    }

    // Place ask that crosses the bid
    let (ask_order_id, ask_signature, ask_auth, ask_order) = {
        let user_state = user.get_session_data_mut::<UserState>().unwrap();
        let order_id = user_state.generate_order_id("taker_cross_ask");
        let nonce = user_state.next_nonce();
        let auth = user_state.auth.clone();
        let order = build_order(
            order_id.clone(),
            OrderSide::Ask,
            OrderType::Limit,
            Some(ask_price),
            config.pair(),
            quantity,
        );
        let signature = auth.sign_create_order(nonce, &order_id).unwrap();
        (order_id, signature, auth, order)
    };

    let ask_res = client
        .create_order(user, &ask_auth, &ask_order, &ask_signature)
        .await;

    if let Err(e) = ask_res {
        warn!("Taker: failed to place crossed ask: {:?}", e);
        let user_state = user.get_session_data_mut::<UserState>().unwrap();
        user_state.revert_nonce();
        return Ok(());
    }

    {
        shared_state
            .order_tracker
            .lock()
            .unwrap()
            .add_order(ask_order_id.clone());
    }

    info!(
        "Taker: placed self-crossed bid/ask -> bid {} @ {}, ask {} @ {}, qty {}, mid {}",
        bid_order_id, bid_price, ask_order_id, ask_price, quantity, mid
    );

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
}
