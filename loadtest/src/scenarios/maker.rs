use goose::prelude::*;
use orderbook::orderbook::{OrderSide, OrderType};
use tracing::{debug, warn};

use crate::http_client::{build_order, OrderbookClient};
use crate::state::UserState;
use crate::GLOBAL_CONFIG;
use crate::GLOBAL_SHARED_STATE;

/// Maker scenario: place bid and ask orders around dynamic mid price
pub async fn maker_scenario(user: &mut GooseUser) -> TransactionResult {
    // Get session data (clone to avoid borrow conflicts)
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let shared_state = {
        let global_shared_state = GLOBAL_SHARED_STATE.lock().unwrap();
        global_shared_state.clone().unwrap()
    };

    let user_state = user.get_session_data_mut::<UserState>().unwrap();

    // Create HTTP client
    let client = OrderbookClient::new(&config).unwrap();

    // Get current nonce from server
    let current_nonce = client.get_nonce(&user_state.auth).await.unwrap();

    user_state.nonce = current_nonce;

    // Update mid price with random walk
    let drift = shared_state.random_drift(config.maker.mid_drift_ticks);
    {
        let mut mid_price = shared_state.mid_price.lock().unwrap();
        mid_price.apply_drift(drift);
    }

    let mid = shared_state.mid_price.lock().unwrap().get();
    debug!("Maker: mid price = {} (drift: {})", mid, drift);

    // Place bid orders (buy side)
    for level in 0..config.maker.ladder_levels {
        let user_state = user.get_session_data_mut::<UserState>().unwrap();
        let user_auth = user_state.auth.clone();
        let price_offset =
            config.maker.min_spread_ticks + (level as u64 * config.maker.level_spacing_ticks);
        let price = mid.saturating_sub(price_offset * config.instrument.price_tick);

        if price == 0 {
            continue; // Skip invalid prices
        }

        let quantity = shared_state.random_range(
            config.maker.min_quantity_steps,
            config.maker.max_quantity_steps,
        ) * config.instrument.qty_step;

        let order_id = user_state.generate_order_id("maker_bid");
        let nonce = user_state.next_nonce();

        let order = build_order(
            order_id.clone(),
            OrderSide::Bid,
            OrderType::Limit,
            Some(price),
            config.pair(),
            quantity,
        );

        // Sign the order
        let signature = user_state.auth.sign_create_order(nonce, &order_id).unwrap();

        // Send order
        match client
            .create_order(user, &user_auth, &order, &signature)
            .await
        {
            Ok(_) => {
                debug!(
                    "Maker: placed bid {} @ {} qty {}",
                    order_id, price, quantity
                );
                // Track order for potential cancellation
                shared_state.order_tracker.lock().unwrap().add_order(
                    order_id,
                    config.pair(),
                    nonce,
                );
            }
            Err(e) => {
                warn!("Maker: failed to place bid: {:?}", e);
                // Don't fail the entire scenario, just log and continue
            }
        }

        // Small delay between orders to avoid overwhelming the server
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    // Place ask orders (sell side)
    for level in 0..config.maker.ladder_levels {
        let user_state = user.get_session_data_mut::<UserState>().unwrap();
        let user_auth = user_state.auth.clone();
        let price_offset =
            config.maker.min_spread_ticks + (level as u64 * config.maker.level_spacing_ticks);
        let price = mid.saturating_add(price_offset * config.instrument.price_tick);

        let quantity = shared_state.random_range(
            config.maker.min_quantity_steps,
            config.maker.max_quantity_steps,
        ) * config.instrument.qty_step;

        let order_id = user_state.generate_order_id("maker_ask");
        let nonce = user_state.next_nonce();

        let order = build_order(
            order_id.clone(),
            OrderSide::Ask,
            OrderType::Limit,
            Some(price),
            config.pair(),
            quantity,
        );

        // Sign the order
        let signature = user_state.auth.sign_create_order(nonce, &order_id).unwrap();

        // Send order
        match client
            .create_order(user, &user_auth, &order, &signature)
            .await
        {
            Ok(_) => {
                debug!(
                    "Maker: placed ask {} @ {} qty {}",
                    order_id, price, quantity
                );
                // Track order for potential cancellation
                shared_state.order_tracker.lock().unwrap().add_order(
                    order_id,
                    config.pair(),
                    nonce,
                );
            }
            Err(e) => {
                warn!("Maker: failed to place ask: {:?}", e);
                // Don't fail the entire scenario, just log and continue
            }
        }

        // Small delay between orders
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    // Wait before next maker cycle
    tokio::time::sleep(tokio::time::Duration::from_millis(
        config.maker.cycle_interval_ms,
    ))
    .await;

    Ok(())
}
