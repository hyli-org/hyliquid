use config::{Config, Environment, File};
use hyli_modules::modules::websocket::WebSocketConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Conf {
    pub id: String,
    /// The log format to use - "json", "node" or "full" (default)
    pub log_format: String,
    /// Directory name to store node state.
    pub data_directory: PathBuf,
    /// When running only the indexer, the address of the DA server to connect to
    pub da_read_from: String,
    pub node_url: String,
    pub indexer_url: String,

    pub database_url: String,
    pub database_name: String,

    pub rest_server_port: u16,
    pub rest_server_max_body_size: usize,

    pub buffer_blocks: u32,
    pub max_txs_per_proof: usize,
    pub tx_working_window_size: usize,

    // Bridge configuration
    pub bridge: BridgeConfig,

    /// Websocket configuration
    pub websocket: WebSocketConfig,

    /// URL to trigger L2 book updates
    pub trigger_url: String,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub eth_contract_vault_address: String,
    pub eth_contract_address: String,
    pub eth_contract_deploy_block: u64,
    pub eth_rpc_ws_url: String,
    pub eth_rpc_http_url: String,
    pub eth_signer_private_key: String,
}

impl Conf {
    pub fn new(config_files: Vec<String>) -> Result<Self, anyhow::Error> {
        let mut s = Config::builder().add_source(File::from_str(
            include_str!("conf_defaults.toml"),
            config::FileFormat::Toml,
        ));
        // Priority order: config file, then environment variables
        for config_file in config_files {
            s = s.add_source(File::with_name(&config_file).required(false));
        }
        let conf: Self = s
            .add_source(
                Environment::with_prefix("hyli")
                    .separator("__")
                    .prefix_separator("_")
                    .list_separator(","),
            )
            .build()?
            .try_deserialize()?;
        Ok(conf)
    }
}
