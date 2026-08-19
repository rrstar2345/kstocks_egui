//! Tauri commands wrapping local watchlist storage.

use tauri::State;

use crate::state::AppState;
use crate::storage::watchlists::{self, Watchlist, WatchlistItem};

#[tauri::command]
pub async fn list_watchlists(state: State<'_, AppState>) -> Result<Vec<Watchlist>, String> {
    watchlists::list_watchlists(&state.db).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_watchlist(state: State<'_, AppState>, name: String) -> Result<i64, String> {
    watchlists::create_watchlist(&state.db, &name).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_watchlist(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    watchlists::delete_watchlist(&state.db, id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_watchlist_items(
    state: State<'_, AppState>,
    watchlist_id: i64,
) -> Result<Vec<WatchlistItem>, String> {
    watchlists::list_items(&state.db, watchlist_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_watchlist_item(
    state: State<'_, AppState>,
    watchlist_id: i64,
    symbol: String,
    instrument_type: String,
) -> Result<i64, String> {
    watchlists::add_item(&state.db, watchlist_id, &symbol, &instrument_type)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_watchlist_item(state: State<'_, AppState>, item_id: i64) -> Result<(), String> {
    watchlists::remove_item(&state.db, item_id).await.map_err(|e| e.to_string())
}
