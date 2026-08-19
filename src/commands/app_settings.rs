//! Tauri commands wrapping local `app_settings` key/value storage.
//!
//! Used for anything the UI needs to persist across launches that isn't
//! big enough to deserve its own table: theme preference, dashboard widget
//! layout, last-selected watchlist, etc. Values are stored as opaque
//! strings (the frontend JSON-encodes/decodes as needed).

use tauri::State;

use crate::state::AppState;
use crate::storage::settings_store;

#[tauri::command]
pub async fn get_setting(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    settings_store::get(&state.db, &key).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_setting(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    settings_store::set(&state.db, &key, &value).await.map_err(|e| e.to_string())
}
