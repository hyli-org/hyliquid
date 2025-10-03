use goose::prelude::*;
use server::services::user_service::Balance;
use tracing::{debug, info};

use crate::auth::UserAuth;
use crate::config::Config;
use crate::http_client::OrderbookClient;
use crate::state::UserState;
use crate::GLOBAL_CONFIG;

/// Setup task: initialize user with session key, create pair if needed, deposit funds
pub async fn user_setup(user: &mut GooseUser) -> TransactionResult {
    // Get configuration from user data
    let config = {
        let global_config = GLOBAL_CONFIG.lock().unwrap();
        global_config.clone().unwrap()
    };
    // Create HTTP client
    let client = OrderbookClient::new(&config).unwrap();

    // check if user is already setup
    let user_state = user.get_session_data::<UserState>();

    if user_state.is_some() {
        info!("User {} already setup", user_state.unwrap().auth.identity);
        let user_state = user_state.unwrap();
        let user_auth = user_state.auth.clone();
        user_deposit(user, &client, &user_auth, &config).await?;
        return Ok(());
    } else {
        // Create user state
        let user_state = UserState::new(user.weighted_users_index).unwrap();
        let user_auth = user_state.auth.clone();

        info!(
            "Setting up user {} (index {})",
            user_state.auth.identity, user.weighted_users_index
        );

        // Step 1: Add session key
        debug!("Adding session key for {}", user_state.auth.identity);
        client.add_session_key(user, &user_auth).await?;

        // Step 2: Create pair (first user only, others will fail gracefully)
        if user.weighted_users_index == 0 {
            info!("Creating trading pair: {}", config.instrument_symbol());
            let _ = client.create_pair(user, &user_auth, config.pair()).await; // Ignore errors (pair might exist)
        }

        // Small delay to let pair creation propagate
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        info!("User {} setup complete", user_auth.identity);

        user_deposit(user, &client, &user_auth, &config).await?;

        // Store user state in session data
        user.set_session_data(user_state);
    }

    Ok(())
}

async fn user_deposit(
    user: &mut GooseUser,
    client: &OrderbookClient,
    user_auth: &UserAuth,
    config: &Config,
) -> TransactionResult {
    // Check user balance
    let balance = client.get_balances(user, user_auth).await?;

    let base_asset_balance = balance
        .balances
        .iter()
        .find(|b| b.token == config.instrument.base_asset)
        .cloned()
        .unwrap_or(Balance {
            token: config.instrument.base_asset.clone(),
            available: 0,
            total: 0,
            reserved: 0,
        });
    let quote_asset_balance = balance
        .balances
        .iter()
        .find(|b| b.token == config.instrument.quote_asset)
        .cloned()
        .unwrap_or(Balance {
            token: config.instrument.quote_asset.clone(),
            available: 0,
            total: 0,
            reserved: 0,
        });

    debug!(
        "User {} base asset balance: {:?}",
        user_auth.identity, base_asset_balance
    );
    debug!(
        "User {} quote asset balance: {:?}",
        user_auth.identity, quote_asset_balance
    );

    // Step 3: Deposit base asset
    if (base_asset_balance.available as u64) < config.user_setup.minimal_balance_base {
        debug!(
            "Depositing {} {} for {}",
            config.user_setup.initial_deposit_base,
            config.instrument.base_asset,
            user_auth.identity
        );
        client
            .deposit(
                user,
                user_auth,
                &config.instrument.base_asset,
                config.user_setup.initial_deposit_base,
            )
            .await?;
    }

    // Step 4: Deposit quote asset
    if (quote_asset_balance.available as u64) < config.user_setup.minimal_balance_quote {
        debug!(
            "Depositing {} {} for {}",
            config.user_setup.initial_deposit_quote,
            config.instrument.quote_asset,
            user_auth.identity
        );
        client
            .deposit(
                user,
                user_auth,
                &config.instrument.quote_asset,
                config.user_setup.initial_deposit_quote,
            )
            .await?;
    }

    Ok(())
}
