use crate::conf::Conf;
use anyhow::{Context, Result};
use client_sdk::rest_client::{IndexerApiHttpClient, NodeApiClient, NodeApiHttpClient};
use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    trace::{self, BatchConfigBuilder, SdkTracerProvider},
    Resource,
};
use sdk::LaneId;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, level_filters::LevelFilter};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./src/migrations");

async fn connect_database(config: &Conf) -> Result<PgPool> {
    info!("Connecting to database: {}", config.database_url);
    let pool = PgPoolOptions::new()
        .max_connections(150)
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect(&config.database_url)
        .await
        .context("Failed to connect to the config database")?;

    if config.database_url.ends_with(&config.database_name) {
        return Ok(pool);
    }

    // Check if database exists
    let database_exists = sqlx::query(
        format!(
            "SELECT 1 FROM pg_database WHERE datname = '{}'",
            config.database_name
        )
        .as_str(),
    )
    .fetch_optional(&pool)
    .await?;

    if database_exists.is_none() {
        info!("Creating database: {}", config.database_name);
        sqlx::query(format!("CREATE DATABASE {}", config.database_name).as_str())
            .execute(&pool)
            .await?;
    }

    let database_url = format!("{}/{}", config.database_url, config.database_name);
    info!("Connecting to database: {}", database_url);

    let pool = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(std::time::Duration::from_secs(1))
        .connect(&database_url)
        .await
        .context("Failed to connect to the created database")?;

    Ok(pool)
}

pub async fn setup_database(config: &Conf, clean_db: bool) -> Result<PgPool> {
    let pool = connect_database(config).await?;

    if clean_db {
        info!("Cleaning database: {}", config.database_name);
        sqlx::query("DROP SCHEMA public CASCADE;")
            .execute(&pool)
            .await
            .context("cleaning database")?;
        sqlx::query("CREATE SCHEMA public;")
            .execute(&pool)
            .await
            .context("creating public schema")?;
    }

    info!("Running database migrations");
    MIGRATOR.run(&pool).await?;
    info!("Database migrations completed");

    Ok(pool)
}

pub fn init_tracing() -> opentelemetry_sdk::trace::SdkTracerProvider {
    // Set up W3C trace context propagator
    global::set_text_map_propagator(opentelemetry_sdk::propagation::TraceContextPropagator::new());

    // Configure resource with service name
    let resource = Resource::builder_empty()
        .with_service_name("hyliquid-orderbook")
        .build();

    // Build OTLP exporter using tonic (grpc)
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint("http://localhost:4317")
        .build()
        .expect("Failed to create OTLP exporter");

    // Create batch span processor
    let batch_config = BatchConfigBuilder::default().build();
    let batch_processor = trace::BatchSpanProcessor::builder(otlp_exporter)
        .with_batch_config(batch_config)
        .build();

    // Create tracer provider
    let tracer_provider = SdkTracerProvider::builder()
        .with_span_processor(batch_processor)
        .with_resource(resource)
        .build();

    // Get tracer before setting as global
    let tracer = tracer_provider.tracer("hyliquid-orderbook");

    // Set as global tracer provider (keep a clone so caller can choose to flush/shutdown)
    let _ = global::set_tracer_provider(tracer_provider.clone());

    // Configure tracing subscriber with env filter
    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // Initialize the tracing subscriber with both console output and OTLP
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(OpenTelemetryLayer::new(tracer))
        .init();

    tracing::info!("Tracing initialized with OTLP exporter to http://localhost:4317");

    tracer_provider
}

pub struct ServiceContext {
    pub user_service: Arc<RwLock<crate::services::user_service::UserService>>,
    pub asset_service: Arc<RwLock<crate::services::asset_service::AssetService>>,
    pub bridge_service: Arc<RwLock<crate::services::bridge_service::BridgeService>>,
    pub book_service: Arc<RwLock<crate::services::book_service::BookService>>,
    pub node_client: Arc<NodeApiHttpClient>,
    pub indexer_client: Arc<IndexerApiHttpClient>,
    pub validator_lane_id: LaneId,
}

pub async fn setup_services(config: &Conf, pool: PgPool) -> Result<ServiceContext> {
    // Initialize services
    let user_service = Arc::new(RwLock::new(
        crate::services::user_service::UserService::new(pool.clone()).await,
    ));
    let asset_service = Arc::new(RwLock::new(
        crate::services::asset_service::AssetService::new(pool.clone()).await,
    ));
    let bridge_service = Arc::new(RwLock::new(
        crate::services::bridge_service::BridgeService::new(pool.clone(), &config.bridge).await?,
    ));
    let book_service = Arc::new(RwLock::new(
        crate::services::book_service::BookService::new(pool.clone()),
    ));

    // Initialize node client
    let node_client = Arc::new(
        NodeApiHttpClient::new(config.node_url.clone()).context("Failed to build node client")?,
    );

    // Initialize indexer client
    let indexer_client = Arc::new(
        IndexerApiHttpClient::new(config.indexer_url.clone())
            .context("Failed to build indexer client")?,
    );

    // Get validator lane ID
    let validator_lane_id = node_client
        .get_node_info()
        .await?
        .pubkey
        .map(LaneId)
        .ok_or_else(|| {
            error!("Validator lane id not found");
            anyhow::anyhow!("Validator lane id not found")
        })?;

    Ok(ServiceContext {
        user_service,
        asset_service,
        bridge_service,
        book_service,
        node_client,
        indexer_client,
        validator_lane_id,
    })
}
