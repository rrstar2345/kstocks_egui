use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::api::ApiClient;
use crate::market::market_clock::SharedSessionState;
use crate::settings::{AppConfig, AppPaths};
use crate::stats::SharedStats;

/// Shared application state, managed by Tauri and injected into commands.
/// The API client is behind an `RwLock` because the base URL / API key can
/// change at runtime (e.g. after `/register`, or when pointed at a
/// different server).
///
/// `stats` and `session` are the same shared types the server's streamers
/// use (`crate::stats`, `crate::market::market_clock`) — kept in
/// `AppState` so a future Tauri command can expose live stream
/// status/session mode to the frontend (see PROGRESS.md).
pub struct AppState {
    pub db: SqlitePool,
    pub paths: AppPaths,
    pub config: RwLock<AppConfig>,
    pub api_client: RwLock<ApiClient>,
    pub stats: SharedStats,
    pub session: SharedSessionState,
}

impl AppState {
    pub fn new(
        db: SqlitePool,
        paths: AppPaths,
        config: AppConfig,
        stats: SharedStats,
        session: SharedSessionState,
    ) -> Self {
        let api_client = ApiClient::new(config.server.base_url.clone(), config.server.api_key.clone());
        Self {
            db,
            paths,
            config: RwLock::new(config),
            api_client: RwLock::new(api_client),
            stats,
            session,
        }
    }
}