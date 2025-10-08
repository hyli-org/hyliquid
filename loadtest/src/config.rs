use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub instrument: InstrumentConfig,
    pub load: LoadConfig,
    pub maker: MakerConfig,
    pub taker: TakerConfig,
    pub cancellation: CancellationConfig,
    pub http: HttpConfig,
    pub user_setup: UserSetupConfig,
    pub rng: RngConfig,
    pub sla: SlaConfig,
    pub metrics: MetricsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentConfig {
    pub base_asset: String,
    pub quote_asset: String,
    pub price_tick: u64,
    pub qty_step: u64,
    pub price_scale: u32,
    pub qty_scale: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadConfig {
    pub model: LoadModel,
    pub prefix: String,
    pub users: u32,
    pub rps: u32,
    pub duration: u64,
    pub ramp_users_per_second: u32,
    pub ramp_duration: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LoadModel {
    Closed,
    Open,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerConfig {
    pub enabled: bool,
    pub weight: u32,
    pub ladder_levels: u32,
    pub min_spread_ticks: u64,
    pub level_spacing_ticks: u64,
    pub min_quantity_steps: u64,
    pub max_quantity_steps: u64,
    pub mid_drift_ticks: i64,
    pub mid_initial: u64,
    pub cycle_interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakerConfig {
    pub enabled: bool,
    pub weight: u32,
    pub cross_ticks: u64,
    pub min_quantity_steps: u64,
    pub max_quantity_steps: u64,
    pub interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancellationConfig {
    pub enabled: bool,
    pub weight: u32,
    pub cancel_percentage: u32,
    pub max_tracked_orders: usize,
    pub interval_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    pub timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub max_retries: u32,
    pub retry_backoff_ms: u64,
    pub max_requests_per_second: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSetupConfig {
    pub initial_deposit_base: u64,
    pub initial_deposit_quote: u64,
    pub minimal_balance_base: u64,
    pub minimal_balance_quote: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RngConfig {
    pub seed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlaConfig {
    pub enabled: bool,
    pub p50_max_ms: u64,
    pub p95_max_ms: u64,
    pub p99_max_ms: u64,
    pub max_error_rate_percent: f64,
    pub min_fills: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    pub export_json: bool,
    pub export_csv: bool,
    pub output_dir: String,
    pub verbose: bool,
}

#[derive(Parser, Debug)]
#[command(name = "loadtest_goose")]
#[command(about = "Goose-based load testing for orderbook market maker simulation")]
pub struct CliArgs {
    /// Path to configuration file
    #[arg(long, default_value = "loadtest.toml")]
    pub config: PathBuf,

    /// Prepare the test environment
    #[arg(long)]
    pub prepare: bool,

    /// Override: Server base URL
    #[arg(long)]
    pub base_url: Option<String>,

    /// Override: Base asset symbol
    #[arg(long)]
    pub base_asset: Option<String>,

    /// Override: Quote asset symbol
    #[arg(long)]
    pub quote_asset: Option<String>,

    /// Override: Number of virtual users (closed model)
    #[arg(long)]
    pub users: Option<u32>,

    /// Override: Prefix for user identities
    #[arg(long)]
    pub prefix: Option<String>,

    /// Override: Requests per second (open model)
    #[arg(long)]
    pub rps: Option<u32>,

    /// Override: Test duration in seconds
    #[arg(long)]
    pub duration: Option<u64>,

    /// Override: RNG seed
    #[arg(long)]
    pub seed: Option<u64>,

    /// Override: Load model (closed or open)
    #[arg(long)]
    pub model: Option<String>,

    /// Override: Price tick
    #[arg(long)]
    pub price_tick: Option<u64>,

    /// Override: Quantity step
    #[arg(long)]
    pub qty_step: Option<u64>,

    /// Override: Output directory for reports
    #[arg(long)]
    pub report_dir: Option<String>,

    /// Dry run: validate configuration without executing
    #[arg(long)]
    pub dry_run: bool,

    /// Verbose output
    #[arg(long, short)]
    pub verbose: bool,
}

impl Config {
    /// Load configuration from TOML file and apply CLI overrides
    pub fn load(args: &CliArgs) -> Result<Self> {
        // Load TOML configuration
        let config_str = std::fs::read_to_string(&args.config)
            .with_context(|| format!("Failed to read config file: {:?}", args.config))?;

        let mut config: Config = toml::from_str(&config_str)
            .with_context(|| format!("Failed to parse config file: {:?}", args.config))?;

        // Apply environment variable overrides first
        if let Ok(val) = std::env::var("BASE_URL") {
            config.server.base_url = val;
        }
        if let Ok(val) = std::env::var("BASE_ASSET") {
            config.instrument.base_asset = val;
        }
        if let Ok(val) = std::env::var("QUOTE_ASSET") {
            config.instrument.quote_asset = val;
        }
        if let Ok(val) = std::env::var("USERS") {
            if let Ok(users) = val.parse() {
                config.load.users = users;
            }
        }
        if let Ok(val) = std::env::var("RPS") {
            if let Ok(rps) = val.parse() {
                config.load.rps = rps;
            }
        }
        if let Ok(val) = std::env::var("DURATION") {
            if let Ok(duration) = val.parse() {
                config.load.duration = duration;
            }
        }
        if let Ok(val) = std::env::var("SEED") {
            if let Ok(seed) = val.parse() {
                config.rng.seed = seed;
            }
        }
        if let Ok(val) = std::env::var("MODEL") {
            config.load.model = match val.to_lowercase().as_str() {
                "closed" => LoadModel::Closed,
                "open" => LoadModel::Open,
                _ => config.load.model,
            };
        }
        if let Ok(val) = std::env::var("PRICE_TICK") {
            if let Ok(tick) = val.parse() {
                config.instrument.price_tick = tick;
            }
        }
        if let Ok(val) = std::env::var("QTY_STEP") {
            if let Ok(step) = val.parse() {
                config.instrument.qty_step = step;
            }
        }
        if let Ok(val) = std::env::var("REPORT_DIR") {
            config.metrics.output_dir = val;
        }

        // Apply CLI overrides (these take precedence over env vars)
        if let Some(base_url) = &args.base_url {
            config.server.base_url = base_url.clone();
        }

        if let Some(base_asset) = &args.base_asset {
            config.instrument.base_asset = base_asset.clone();
        }

        if let Some(quote_asset) = &args.quote_asset {
            config.instrument.quote_asset = quote_asset.clone();
        }

        if let Some(users) = args.users {
            config.load.users = users;
        }

        if let Some(prefix) = &args.prefix {
            config.load.prefix = prefix.clone();
        }

        if let Some(rps) = args.rps {
            config.load.rps = rps;
        }

        if let Some(duration) = args.duration {
            config.load.duration = duration;
        }

        if let Some(seed) = args.seed {
            config.rng.seed = seed;
        }

        if let Some(model) = &args.model {
            config.load.model = match model.to_lowercase().as_str() {
                "closed" => LoadModel::Closed,
                "open" => LoadModel::Open,
                _ => anyhow::bail!("Invalid load model. Must be 'closed' or 'open'"),
            };
        }

        if let Some(price_tick) = args.price_tick {
            config.instrument.price_tick = price_tick;
        }

        if let Some(qty_step) = args.qty_step {
            config.instrument.qty_step = qty_step;
        }

        if let Some(report_dir) = &args.report_dir {
            config.metrics.output_dir = report_dir.clone();
        }

        if args.verbose {
            config.metrics.verbose = true;
        }

        // Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Validate configuration consistency
    pub fn validate(&self) -> Result<()> {
        // Check base URL
        if self.server.base_url.is_empty() {
            anyhow::bail!("Server base URL cannot be empty");
        }

        // Warn if pointing to production
        if self.server.base_url.contains("prod") || self.server.base_url.contains("production") {
            tracing::warn!(
                "⚠️  WARNING: Configuration points to production URL: {}",
                self.server.base_url
            );
            tracing::warn!("⚠️  Load testing against production is NOT recommended!");
        }

        // Check instrument
        if self.instrument.base_asset.is_empty() || self.instrument.quote_asset.is_empty() {
            anyhow::bail!("Base and quote assets must be specified");
        }

        if self.instrument.base_asset == self.instrument.quote_asset {
            anyhow::bail!("Base and quote assets cannot be the same");
        }

        // Check load configuration
        if self.load.duration == 0 {
            anyhow::bail!("Test duration must be greater than 0");
        }

        match self.load.model {
            LoadModel::Closed => {
                if self.load.users == 0 {
                    anyhow::bail!("Number of users must be greater than 0 for closed model");
                }
                if self.load.users > 10000 {
                    tracing::warn!("⚠️  WARNING: Very high number of users ({}). This may overwhelm the system.", self.load.users);
                }
            }
            LoadModel::Open => {
                if self.load.rps == 0 {
                    anyhow::bail!("RPS must be greater than 0 for open model");
                }
                if self.load.rps > 100000 {
                    tracing::warn!(
                        "⚠️  WARNING: Very high RPS target ({}). This may overwhelm the system.",
                        self.load.rps
                    );
                }
            }
        }

        // Check that at least one scenario is enabled
        if !self.maker.enabled && !self.taker.enabled && !self.cancellation.enabled {
            anyhow::bail!("At least one scenario (maker, taker, or cancellation) must be enabled");
        }

        Ok(())
    }

    /// Get the trading pair as tuple
    pub fn pair(&self) -> (String, String) {
        (
            self.instrument.base_asset.clone(),
            self.instrument.quote_asset.clone(),
        )
    }

    /// Get the instrument symbol (BASE/QUOTE)
    pub fn instrument_symbol(&self) -> String {
        format!(
            "{}/{}",
            self.instrument.base_asset, self.instrument.quote_asset
        )
    }
}
