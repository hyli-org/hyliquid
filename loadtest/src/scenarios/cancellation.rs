use goose::prelude::*;
use tracing::{debug, info, warn};

use crate::http_client::OrderbookClient;
use crate::state::UserState;
use crate::GLOBAL_CONFIG;
use crate::GLOBAL_SHARED_STATE;

/// Transaction: Get nonce from server
async fn get_nonce_transaction(user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let user_state = user.get_session_data_mut::<UserState>().unwrap();

    let client = OrderbookClient::new(&config).unwrap();
    let current_nonce = client.get_nonce(&user_state.auth).await.unwrap();

    user_state.nonce = current_nonce;

    Ok(())
}

/// Transaction: Check if there are orders to cancel
async fn check_orders_to_cancel_transaction(_user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let shared_state = {
        let global_shared_state = GLOBAL_SHARED_STATE.lock().unwrap();
        global_shared_state.clone().unwrap()
    };

    // Check if there are orders to cancel (lock held briefly)
    let count_before = {
        let tracker = shared_state.order_tracker.lock().unwrap();
        tracker.count()
    };

    if count_before == 0 {
        debug!("Cancellation: no orders to cancel");
        return Ok(());
    }

    // Get orders to cancel from shared tracker (lock held briefly)
    let orders_to_cancel = {
        let mut tracker = shared_state.order_tracker.lock().unwrap();
        let orders = tracker.get_orders_to_cancel(config.cancellation.cancel_percentage);
        let count_after = tracker.count();

        info!(
            "Cancellation: selected {} orders to cancel (tracker: {} -> {})",
            orders.len(),
            count_before,
            count_after
        );

        orders
    };

    debug!("Orders to cancel: {}", orders_to_cancel.len());

    Ok(())
}

/// Transaction: Cancel orders
async fn cancel_orders_transaction(user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let shared_state = {
        let global_shared_state = GLOBAL_SHARED_STATE.lock().unwrap();
        global_shared_state.clone().unwrap()
    };

    // Check if there are orders to cancel (lock held briefly)
    let count_before = {
        let tracker = shared_state.order_tracker.lock().unwrap();
        tracker.count()
    };

    if count_before == 0 {
        return Ok(());
    }

    // Get orders to cancel from shared tracker (lock held briefly)
    let orders_to_cancel = {
        let mut tracker = shared_state.order_tracker.lock().unwrap();
        tracker.get_orders_to_cancel(config.cancellation.cancel_percentage)
    };

    if orders_to_cancel.is_empty() {
        return Ok(());
    }

    let client = OrderbookClient::new(&config).unwrap();

    // Cancel each order
    let mut cancelled_count = 0;
    let mut failed_count = 0;

    for order_info in orders_to_cancel {
        let user_state = user.get_session_data_mut::<UserState>().unwrap();
        let user_auth = user_state.auth.clone();
        let nonce = user_state.next_nonce();

        // Sign the cancellation
        let signature = user_state
            .auth
            .sign_cancel(nonce, &order_info.order_id)
            .unwrap();

        // Send cancellation request
        match client
            .cancel_order(user, &user_auth, &order_info.order_id, &signature)
            .await
        {
            Ok(_) => {
                debug!("Cancelled order: {}", order_info.order_id);
                cancelled_count += 1;
            }
            Err(e) => {
                // Order might already be filled or cancelled, don't fail the scenario
                warn!("Failed to cancel order {}: {:?}", order_info.order_id, e);
                failed_count += 1;
            }
        }

        // Small delay between cancellations
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }

    info!(
        "Cancellation cycle complete: {} cancelled, {} failed",
        cancelled_count, failed_count
    );

    Ok(())
}

/// Transaction: Wait before next cancellation cycle
async fn cancellation_wait_transaction(_user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    // Wait before next cancellation cycle
    tokio::time::sleep(tokio::time::Duration::from_millis(
        config.cancellation.interval_ms,
    ))
    .await;

    Ok(())
}

/// Creates the cancellation scenario with all its transactions
pub fn cancellation_scenario() -> Scenario {
    scenario!("Cancellation")
        .register_transaction(transaction!(get_nonce_transaction).set_name("get_nonce"))
        .register_transaction(
            transaction!(check_orders_to_cancel_transaction).set_name("check_orders"),
        )
        .register_transaction(transaction!(cancel_orders_transaction).set_name("cancel_orders"))
        .register_transaction(transaction!(cancellation_wait_transaction).set_name("wait_cycle"))
}
