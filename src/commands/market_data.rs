//! Tauri commands for reading locally-aggregated OHLC data (fast,
//! offline-capable path used by chart widgets). For explicit historical
//! backfill from the server, see `commands::server::fetch_index_ohlc`.

use tauri::State;

use crate::state::AppState;
use crate::storage::ohlc::{
    self, IndexSnapshot, OhlcBar, OptionChainRow, OptionLegBar,
};

/// Recent bars for a single index/symbol, oldest-first, ready to hand
/// straight to a chart component.
#[tauri::command]
pub async fn get_recent_index_bars(
    state: State<'_, AppState>,
    symbol: String,
    interval: String,
    limit: Option<i64>,
) -> Result<Vec<OhlcBar>, String> {
    ohlc::recent_index_bars(&state.db, &symbol, &interval, limit.unwrap_or(200))
        .await
        .map_err(|e| e.to_string())
}

/// Recent bars for a single option leg (CE or PE), oldest-first, for the
/// chart widget's option-chain mode.
#[tauri::command]
pub async fn get_recent_option_bars(
    state: State<'_, AppState>,
    symbol: String,
    expiry: String,
    strike: f64,
    leg: String,
    limit: Option<i64>,
) -> Result<Vec<OptionLegBar>, String> {
    ohlc::recent_option_bars(&state.db, &symbol, &expiry, strike, &leg, limit.unwrap_or(200))
        .await
        .map_err(|e| e.to_string())
}

/// Distinct option symbols with local data available (populates the
/// option-chain symbol picker).
#[tauri::command]
pub async fn list_option_symbols(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    ohlc::list_option_symbols(&state.db).await.map_err(|e| e.to_string())
}

/// Distinct index symbols with local data available (populates the index
/// picker in `SelectionPicker` — previously a hardcoded frontend list).
#[tauri::command]
pub async fn list_index_symbols(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    ohlc::list_index_symbols(&state.db).await.map_err(|e| e.to_string())
}

/// Distinct expiries for a symbol, nearest first.
#[tauri::command]
pub async fn list_option_expiries(state: State<'_, AppState>, symbol: String) -> Result<Vec<String>, String> {
    ohlc::list_option_expiries(&state.db, &symbol).await.map_err(|e| e.to_string())
}

/// Distinct strike prices for a symbol+expiry, ascending.
#[tauri::command]
pub async fn list_option_strikes(
    state: State<'_, AppState>,
    symbol: String,
    expiry: String,
) -> Result<Vec<f64>, String> {
    ohlc::list_option_strikes(&state.db, &symbol, &expiry).await.map_err(|e| e.to_string())
}

/// Latest snapshot of the full option chain for symbol+expiry, one row
/// per strike (CE left / PE right), ascending strike order.
#[tauri::command]
pub async fn get_option_chain(
    state: State<'_, AppState>,
    symbol: String,
    expiry: String,
) -> Result<Vec<OptionChainRow>, String> {
    ohlc::option_chain_snapshot(&state.db, &symbol, &expiry).await.map_err(|e| e.to_string())
}

/// Latest tick snapshot for every distinct index seen locally — powers the
/// watchlist and the index list-view.
#[tauri::command]
pub async fn get_all_index_snapshots(state: State<'_, AppState>) -> Result<Vec<IndexSnapshot>, String> {
    ohlc::all_index_snapshots(&state.db).await.map_err(|e| e.to_string())
}

/// Latest tick snapshot for a single index by name.
#[tauri::command]
pub async fn get_index_snapshot(
    state: State<'_, AppState>,
    symbol: String,
) -> Result<Option<IndexSnapshot>, String> {
    ohlc::index_snapshot(&state.db, &symbol).await.map_err(|e| e.to_string())
}