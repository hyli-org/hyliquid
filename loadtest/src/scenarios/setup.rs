use std::time::Duration;

use goose::prelude::*;
use server::services::user_service::Balance;
use tracing::{debug, info};

use crate::http_client::OrderbookClient;
use crate::state::UserState;
use crate::GLOBAL_CONFIG;

/// Transaction: Initialize user state and add session key
async fn init_user_state(user: &mut GooseUser) -> TransactionResult {
    // Check if user is already setup
    if user.get_session_data::<UserState>().is_some() {
        return Ok(()); // Already initialized
    }

    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    // Create user state
    let user_state = UserState::new(user.weighted_users_index, &config.load.prefix).unwrap();
    info!(
        "Initializing user {} (index {})",
        user_state.auth.identity, user.weighted_users_index
    );

    // Store user state in session data
    user.set_session_data(user_state);

    Ok(())
}

/// Transaction: Add session key
async fn add_session_key_transaction(user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let user_state = user.get_session_data::<UserState>().unwrap();

    if user_state.session_key_added {
        return Ok(()); // Already added
    }

    let user_auth = user_state.auth.clone();

    debug!("Adding session key for {}", user_auth.identity);

    let client = OrderbookClient::new(&config).unwrap();
    client.add_session_key(user, &user_auth).await?;

    let user_state = user.get_session_data_mut::<UserState>().unwrap();
    user_state.session_key_added = true;

    Ok(())
}

/// Transaction: Create trading pair (first user only)
async fn create_pair_transaction(user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    if user.weighted_users_index == 0 {
        let user_state = user.get_session_data::<UserState>().unwrap();
        let user_auth = user_state.auth.clone();

        info!("Creating trading pair: {}", config.instrument_symbol());

        let client = OrderbookClient::new(&config).unwrap();
        let _ = client.create_pair(user, &user_auth, config.pair()).await; // Ignore errors (pair might exist)
    }

    Ok(())
}

/// Transaction: Get user balances
async fn get_balances_transaction(user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let user_state = user.get_session_data::<UserState>().unwrap();
    let user_auth = user_state.auth.clone();

    let client = OrderbookClient::new(&config).unwrap();
    let balance = client.get_balances(user, &user_auth).await?;

    let base_asset_balance = balance
        .balances
        .iter()
        .find(|b| b.symbol == config.instrument.base_asset)
        .cloned()
        .unwrap_or(Balance {
            symbol: config.instrument.base_asset.clone(),
            available: 0,
            total: 0,
            reserved: 0,
        });
    let quote_asset_balance = balance
        .balances
        .iter()
        .find(|b| b.symbol == config.instrument.quote_asset)
        .cloned()
        .unwrap_or(Balance {
            symbol: config.instrument.quote_asset.clone(),
            available: 0,
            total: 0,
            reserved: 0,
        });

    debug!(
        "User {} base asset balance: available={}, total={}, reserved={}",
        user_auth.identity,
        base_asset_balance.available,
        base_asset_balance.total,
        base_asset_balance.reserved
    );
    debug!(
        "User {} quote asset balance: available={}, total={}, reserved={}",
        user_auth.identity,
        quote_asset_balance.available,
        quote_asset_balance.total,
        quote_asset_balance.reserved
    );
    let user_state = user.get_session_data_mut::<UserState>().unwrap();

    user_state.base_balance = base_asset_balance.available as u64;
    user_state.quote_balance = quote_asset_balance.available as u64;

    Ok(())
}

/// Transaction: Deposit base asset
async fn deposit_base_asset_transaction(user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let user_state = user.get_session_data::<UserState>().unwrap();
    let user_auth = user_state.auth.clone();

    let client = OrderbookClient::new(&config).unwrap();

    if user_state.base_balance < config.user_setup.minimal_balance_base {
        debug!(
            "Depositing {} {} for {}",
            config.user_setup.initial_deposit_base,
            config.instrument.base_asset,
            user_auth.identity
        );
        client
            .deposit(
                user,
                &user_auth,
                &config.instrument.base_asset,
                config.user_setup.initial_deposit_base,
            )
            .await?;
    }

    Ok(())
}

/// Transaction: Deposit quote asset
async fn deposit_quote_asset_transaction(user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let user_state = user.get_session_data::<UserState>().unwrap();
    let user_auth = user_state.auth.clone();

    let client = OrderbookClient::new(&config).unwrap();

    let user_state = user.get_session_data::<UserState>().unwrap();

    if user_state.quote_balance < config.user_setup.minimal_balance_quote {
        debug!(
            "Depositing {} {} for {}",
            config.user_setup.initial_deposit_quote,
            config.instrument.quote_asset,
            user_auth.identity
        );
        client
            .deposit(
                user,
                &user_auth,
                &config.instrument.quote_asset,
                config.user_setup.initial_deposit_quote,
            )
            .await?;
    }

    Ok(())
}

/// Transaction: Set nonce
async fn get_nonce_transaction(user: &mut GooseUser) -> TransactionResult {
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };

    let user_state = user.get_session_data_mut::<UserState>().unwrap();

    let client = OrderbookClient::new(&config).unwrap();
    let current_nonce = loop {
        match client.get_nonce(&user_state.auth).await {
            Ok(nonce) => break Ok(nonce),
            Err(e) => {
                if e.to_string().contains("404") {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                break Err(e);
            }
        }
    }
    .unwrap();

    user_state.nonce = current_nonce;

    Ok(())
}

/// Creates the setup scenario with all its transactions
pub fn setup_scenario(name: &str) -> Scenario {
    scenario!(name)
        .set_wait_time(Duration::from_millis(20), Duration::from_millis(200))
        .unwrap()
        .register_transaction(
            transaction!(init_user_state)
                .set_name("init_user_state")
                .set_sequence(1)
                .set_on_start(),
        )
        .register_transaction(
            transaction!(add_session_key_transaction)
                .set_name("add_session_key")
                .set_sequence(2)
                .set_on_start(),
        )
        .register_transaction(
            transaction!(create_pair_transaction)
                .set_name("create_pair")
                .set_sequence(3)
                .set_on_start(),
        )
        .register_transaction(
            transaction!(get_balances_transaction)
                .set_name("get_balances")
                .set_sequence(4)
                .set_on_start(),
        )
        .register_transaction(
            transaction!(deposit_base_asset_transaction)
                .set_name("deposit_base_asset")
                .set_sequence(5)
                .set_on_start(),
        )
        .register_transaction(
            transaction!(deposit_quote_asset_transaction)
                .set_name("deposit_quote_asset")
                .set_sequence(6)
                .set_on_start(),
        )
        .register_transaction(
            transaction!(get_nonce_transaction)
                .set_name("get_nonce")
                .set_sequence(7)
                .set_on_start(),
        )
}
