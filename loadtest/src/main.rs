mod auth;
mod checks;
mod config;
mod http_client;
mod metrics;
mod scenarios;
mod state;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use goose::prelude::*;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use config::{CliArgs, Config, LoadModel};
use scenarios::{cancellation_scenario, maker_scenario, taker_scenario};
use state::SharedState;
use std::sync::Mutex;

// Global state for config and shared state to be accessed by user sessions
static GLOBAL_CONFIG: Mutex<Option<Config>> = Mutex::new(None);
static GLOBAL_SHARED_STATE: Mutex<Option<SharedState>> = Mutex::new(None);

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let args = CliArgs::parse();

    // Setup logging
    setup_logging(args.verbose);

    // Load and validate configuration
    let config = Config::load(&args).context("Failed to load configuration")?;

    tracing::info!("Starting load test with configuration:");
    tracing::info!("  Base URL: {}", config.server.base_url);
    tracing::info!("  Instrument: {}", config.instrument_symbol());
    tracing::info!("  Load model: {:?}", config.load.model);
    tracing::info!("  Duration: {}s", config.load.duration);

    // Dry run mode: validate and exit
    if args.dry_run {
        println!("✅ Configuration valid (dry-run mode)");
        println!("\nConfiguration summary:");
        println!("  Server: {}", config.server.base_url);
        println!("  Instrument: {}", config.instrument_symbol());
        println!("  Model: {:?}", config.load.model);
        match config.load.model {
            LoadModel::Closed => println!("  Users: {}", config.load.users),
            LoadModel::Open => println!("  RPS: {}", config.load.rps),
        }
        println!("  Duration: {}s", config.load.duration);
        println!(
            "  Maker: {}",
            if config.maker.enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        println!(
            "  Taker: {}",
            if config.taker.enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        println!(
            "  Cancellation: {}",
            if config.cancellation.enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        return Ok(());
    }

    // Pre-flight checks
    checks::preflight_checks(&config.server.base_url)?;

    // Record test start time
    let start_time = Utc::now();

    // Create shared state
    let shared_state = SharedState::new(config.rng.seed);

    // Update order tracker max size from config
    {
        let mut tracker = shared_state.order_tracker.lock().unwrap();
        *tracker = state::OrderTracker::with_max_size(config.cancellation.max_tracked_orders);
    }

    // Initialize global state for user sessions
    {
        let mut global_config = GLOBAL_CONFIG.lock().unwrap();
        *global_config = Some(config.clone());
    }
    {
        let mut global_shared_state = GLOBAL_SHARED_STATE.lock().unwrap();
        *global_shared_state = Some(shared_state.clone());
    }

    // Build Goose attack
    let goose_metrics = run_goose_test(config.clone(), shared_state).await?;

    // Export metrics
    let summary = metrics::export_metrics(&goose_metrics, &config.metrics, start_time)
        .context("Failed to export metrics")?;

    // Print summary
    metrics::print_summary(&summary, config.metrics.verbose);

    // Validate SLA
    if let Err(e) = checks::validate_sla(&summary, &config.sla) {
        tracing::error!("SLA validation failed: {}", e);
        std::process::exit(1);
    }

    tracing::info!("Load test completed successfully");
    Ok(())
}

async fn run_goose_test(config: Config, _shared_state: SharedState) -> Result<GooseMetrics> {
    // Determine user count based on load model
    let users = match config.load.model {
        LoadModel::Closed => config.load.users as usize,
        LoadModel::Open => {
            // For open model, we simulate with users + throttle
            // Goose doesn't have direct RPS mode, so we approximate
            (config.load.rps / 10).max(1) as usize // Heuristic: ~10 RPS per user
        }
    };

    // Build base Goose configuration with chained calls
    tracing::info!("Building Goose configuration...");
    let mut goose_builder = GooseAttack::initialize()
        .context("Failed to initialize Goose")?
        .set_default(GooseDefault::Host, config.server.base_url.as_str())
        .context("Failed to set host")?
        .set_default(GooseDefault::RunTime, config.load.duration as usize)
        .context("Failed to set runtime")?
        .set_default(GooseDefault::Users, users)
        .context("Failed to set users")?
        .set_default(GooseDefault::LogLevel, 1)
        .context("Failed to set log level")? // Info level
        .set_default(GooseDefault::NoMetrics, false)
        .context("Failed to set no metrics")?;

    // Configure ramp-up if specified
    if config.load.ramp_duration > 0 && config.load.ramp_users_per_second > 0 {
        let str = format!("{}", config.load.ramp_users_per_second);
        goose_builder = goose_builder
            .set_default(GooseDefault::HatchRate, str.as_str())
            .map_err(|e| anyhow::anyhow!("Failed to set hatch rate: {:?}", e))?;
    }

    // Register setup scenario (always enabled)
    tracing::info!("Registering setup scenario...");
    let mut attack = *goose_builder;

    // Register maker scenario if enabled
    if config.maker.enabled {
        tracing::info!("Registering maker scenario...");
        let maker = maker_scenario().set_weight(config.maker.weight as usize)?;
        attack = attack.register_scenario(maker);
    }

    // Register taker scenario if enabled
    if config.taker.enabled {
        tracing::info!("Registering taker scenario...");
        let taker = taker_scenario().set_weight(config.taker.weight as usize)?;
        attack = attack.register_scenario(taker);
    }

    // Register cancellation scenario if enabled
    if config.cancellation.enabled {
        tracing::info!("Registering cancellation scenario...");
        let cancellation =
            cancellation_scenario().set_weight(config.cancellation.weight as usize)?;
        attack = attack.register_scenario(cancellation);
    }

    // Execute the load test
    tracing::info!("Starting Goose load test...");
    let metrics = attack
        .execute()
        .await
        .context("Goose attack execution failed")?;

    tracing::info!("Load test execution completed");
    Ok(metrics)
}

fn setup_logging(verbose: bool) {
    let log_level = if verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("loadtest={},goose={}", log_level, log_level).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}
