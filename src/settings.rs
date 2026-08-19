use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;
use tracing::{error, info};

pub const APP_NAME: &str = "kstocks";

// ============================================================================
// APP FOLDERS
// ============================================================================
//
// Mirrors the server's `settings::setup_app_folders`, but with a
// client-specific settings filename so a desktop install and a
// server process on the same machine never fight over one file.

pub struct AppPaths {
    pub root: PathBuf,
    pub db_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub settings_file: PathBuf,
}

pub fn setup_app_folders() -> io::Result<AppPaths> {
    let base_path = dirs::data_local_dir()
        .or_else(|| env::current_dir().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Could not determine a storage location"))?;

    let root = base_path.join(format!(".{}", APP_NAME));
    let db_dir = root.join("db");
    let logs_dir = root.join("logs");
    // Client uses its own settings file, separate from the server's
    // settings_server.json, so the two processes never fight over the same file.
    let settings_file = root.join("settings_client.json");

    let paths = AppPaths { root, db_dir, logs_dir, settings_file };

    fs::create_dir_all(&paths.root)?;
    fs::create_dir_all(&paths.db_dir)?;
    fs::create_dir_all(&paths.logs_dir)?;

    Ok(paths)
}

// ============================================================================
// CONFIGURATION STRUCTURES
// ============================================================================

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DatabaseConfig {
    /// Path to the local SQLite database file, e.g. `<db_dir>/kstocks.db`.
    pub connection_string: String,
    pub max_connections: u32,
    /// Max ticks to buffer in memory before a forced flush (mirrors server).
    pub batch_max_rows: usize,
    /// Max time to wait before flushing a partial batch (mirrors server).
    pub batch_max_wait_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    /// Base URL of the kstocks-server instance this client talks to.
    /// Defaults to localhost during development; update to the deployed
    /// server's IP/domain when publishing.
    pub base_url: String,
    /// Client API key returned by `/register`, stored after first launch.
    /// `None` until the user has registered.
    pub api_key: Option<String>,
}

// ----------------------------------------------------------------------
// Market/streaming config — ported from kstocks-server's settings.rs.
// Same shapes so the ported market/storage code can consume `AppConfig`
// (this file's, not the server's) without modification.
// ----------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiEndpoint {
    pub base: String,
    pub desc: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemConfig {
    /// wss://streamer.nseindia.com/streams/indices/high/windices
    pub indices_streamer: ApiEndpoint,
    /// wss://streamer.nseindia.com/streams/fo/mbp  (symbol + expiry params appended at runtime)
    pub option_ticks: ApiEndpoint,
    /// https://www.nseindia.com/api/NextApi/apiClient/indexTrackerApi?functionName=getAllIndices
    pub indices_info: ApiEndpoint,
    /// https://www.nseindia.com/api/option-chain-contract-info?symbol=<fnoIndexName>
    pub option_info: ApiEndpoint,
    /// https://www.nseindia.com/api/NextApi/dynamicApi?functionName=getCurrentTime
    pub current_time: ApiEndpoint,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MarketRuntimeConfig {
    /// How often (seconds) to refresh the symbol/expiry list from NSE.
    pub symbol_refresh_interval_seconds: u64,
    /// Seconds to wait before reconnecting a dropped WebSocket.
    pub reconnect_delay_seconds: u64,
    /// If true, restrict streaming to the NSE trading window (9:00-15:30 IST, Mon-Fri).
    pub restrict_to_trading_window: bool,
    /// Seconds of no real (non-heartbeat) tick activity within the trading
    /// window before switching from Active to Idle mode. Default 3600 (1 hour).
    pub inactive_switch_after_secs: i64,
    /// Seconds between polls while in Idle mode. Default 3600 (1 hour).
    pub idle_poll_interval_secs: u64,
    /// How long (seconds) to hold each Idle-mode poll connection open.
    pub idle_poll_listen_secs: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AggregationConfig {
    /// How often (seconds) the 1-minute OHLC aggregation job runs.
    pub run_interval_secs: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RetentionConfig {
    /// Raw index_ticks/option_ticks older than this many trading days are
    /// purged (once *_ohlc_1m coverage is confirmed).
    pub raw_ticks_keep_trading_days: i64,
    /// index_ohlc_1m rows older than this many days are purged (once
    /// index_ohlc_1d coverage is confirmed).
    pub index_ohlc_1m_keep_days: i64,
    /// option_ohlc_1m rows are purged once `expiry_date < today - this many days`.
    pub option_ohlc_1m_expiry_grace_days: i64,
    /// index_ohlc_1d rows older than this many days are purged. 0 = keep forever.
    pub index_ohlc_1d_keep_days: i64,
}

impl AggregationConfig {
    fn default_config() -> Self {
        Self { run_interval_secs: 300 }
    }
}

impl RetentionConfig {
    fn default_config() -> Self {
        Self {
            raw_ticks_keep_trading_days: 2,
            index_ohlc_1m_keep_days: 60,
            option_ohlc_1m_expiry_grace_days: 7,
            index_ohlc_1d_keep_days: 365 * 3,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub database: DatabaseConfig,
    pub server: ServerConfig,
    pub system: SystemConfig,
    pub market_runtime: MarketRuntimeConfig,
    #[serde(default = "AggregationConfig::default_config")]
    pub aggregation: AggregationConfig,
    #[serde(default = "RetentionConfig::default_config")]
    pub retention: RetentionConfig,
}

impl AppConfig {
    pub fn default(paths: &AppPaths) -> Self {
        let db_path = paths.db_dir.join("kstocks.db");
        AppConfig {
            database: DatabaseConfig {
                connection_string: db_path.to_string_lossy().to_string(),
                max_connections: 5,
                batch_max_rows: 500,
                batch_max_wait_ms: 500,
            },
            server: ServerConfig {
                base_url: "http://localhost:8787".to_string(),
                api_key: None,
            },
            system: SystemConfig {
                indices_streamer: ApiEndpoint {
                    base: "wss://streamer.nseindia.com/streams/indices/high/windices".to_string(),
                    desc: "WebSocket stream for real-time index data".to_string(),
                },
                option_ticks: ApiEndpoint {
                    base: "wss://streamer.nseindia.com/streams/fo/mbp".to_string(),
                    desc: "Websocket to get CE and PE price movements".to_string(),
                },
                indices_info: ApiEndpoint {
                    base: "https://www.nseindia.com/api/NextApi/apiClient/indexTrackerApi?functionName=getAllIndices".to_string(),
                    desc: "Get F&O index name, underlying index names, short and long names".to_string(),
                },
                option_info: ApiEndpoint {
                    base: "https://www.nseindia.com/api/option-chain-contract-info".to_string(),
                    desc: "Fetch the expiry dates and strike prices for the index".to_string(),
                },
                current_time: ApiEndpoint {
                    base: "https://www.nseindia.com/api/NextApi/dynamicApi?functionName=getCurrentTime".to_string(),
                    desc: "Get NSE's current IST server time".to_string(),
                },
            },
            market_runtime: MarketRuntimeConfig {
                symbol_refresh_interval_seconds: 3600,
                reconnect_delay_seconds: 5,
                restrict_to_trading_window: false,
                inactive_switch_after_secs: 3600,
                idle_poll_interval_secs: 3600,
                idle_poll_listen_secs: 15,
            },
            aggregation: AggregationConfig::default_config(),
            retention: RetentionConfig::default_config(),
        }
    }
}

pub fn load_or_create_config(paths: &AppPaths) -> io::Result<AppConfig> {
    let settings_file = &paths.settings_file;
    if settings_file.exists() {
        match fs::read_to_string(settings_file) {
            Ok(content) => match serde_json::from_str::<AppConfig>(&content) {
                Ok(config) => {
                    info!("Loaded existing configuration from: {}", settings_file.display());
                    return Ok(config);
                }
                Err(e) => {
                    error!("Failed to parse settings_client.json: {}. Using defaults.", e);
                }
            },
            Err(e) => {
                error!("Failed to read settings_client.json: {}. Using defaults.", e);
            }
        }
    }

    let default_config = AppConfig::default(paths);
    save_config(paths, &default_config)?;
    info!("Created default configuration at: {}", settings_file.display());
    Ok(default_config)
}

pub fn save_config(paths: &AppPaths, config: &AppConfig) -> io::Result<()> {
    let config_json = serde_json::to_string_pretty(config)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    fs::write(&paths.settings_file, &config_json)?;
    Ok(())
}