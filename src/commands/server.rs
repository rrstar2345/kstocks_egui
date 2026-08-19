//! Tauri commands wrapping the kstocks-server API client.

use tauri::State;

use crate::api::client::{HealthResponse, IndexBar, OptionBar, RegisterResponse, ValidateResponse};
use crate::settings::save_config;
use crate::state::AppState;

#[tauri::command]
pub async fn register_client(
    state: State<'_, AppState>,
    username: String,
) -> Result<RegisterResponse, String> {
    let result = {
        let client = state.api_client.read().await;
        client.register(&username).await.map_err(|e| e.to_string())?
    };

    // Persist the returned key immediately — it's shown exactly once.
    {
        let mut config = state.config.write().await;
        config.server.api_key = Some(result.api_key.clone());
        save_config(&state.paths, &config).map_err(|e| e.to_string())?;
    }
    {
        let mut client = state.api_client.write().await;
        client.set_api_key(Some(result.api_key.clone()));
    }

    Ok(result)
}

#[tauri::command]
pub async fn validate_client(state: State<'_, AppState>) -> Result<ValidateResponse, String> {
    let client = state.api_client.read().await;
    client.validate().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn server_health(state: State<'_, AppState>) -> Result<HealthResponse, String> {
    let client = state.api_client.read().await;
    client.health().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn fetch_index_ohlc(
    state: State<'_, AppState>,
    symbol: String,
    range: String,
    interval: String,
) -> Result<Vec<IndexBar>, String> {
    let client = state.api_client.read().await;
    client
        .ohlc_index(&symbol, &range, &interval)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn fetch_option_ohlc(
    state: State<'_, AppState>,
    symbol: String,
    expiry: String,
    strike: f64,
    range: String,
    interval: String,
    leg: String,
) -> Result<Vec<OptionBar>, String> {
    let client = state.api_client.read().await;
    client
        .ohlc_option(&symbol, &expiry, strike, &range, &interval, &leg)
        .await
        .map_err(|e| e.to_string())
}

/// Update the server base URL at runtime (e.g. switching from
/// `http://localhost:8787` to a deployed server's IP/domain) and persist it.
#[tauri::command]
pub async fn set_server_url(state: State<'_, AppState>, base_url: String) -> Result<(), String> {
    {
        let mut config = state.config.write().await;
        config.server.base_url = base_url.clone();
        save_config(&state.paths, &config).map_err(|e| e.to_string())?;
    }
    {
        let mut client = state.api_client.write().await;
        client.set_base_url(base_url);
    }
    Ok(())
}

/// Sanitized view of the server config for the UI: the API key itself is
/// never sent to the frontend after registration (it's shown exactly once,
/// at the moment `register_client` returns it) — only whether one is
/// stored, so screens can render "connected" state without handling
/// secret material.
#[derive(serde::Serialize)]
pub struct ServerConfigView {
    pub base_url: String,
    pub has_api_key: bool,
}

#[tauri::command]
pub async fn get_server_config(state: State<'_, AppState>) -> Result<ServerConfigView, String> {
    let config = state.config.read().await;
    Ok(ServerConfigView {
        base_url: config.server.base_url.clone(),
        has_api_key: config.server.api_key.is_some(),
    })
}

/// Clear the stored API key (e.g. "log out" / "re-register"). Does not
/// touch `base_url`.
#[tauri::command]
pub async fn clear_api_key(state: State<'_, AppState>) -> Result<(), String> {
    {
        let mut config = state.config.write().await;
        config.server.api_key = None;
        save_config(&state.paths, &config).map_err(|e| e.to_string())?;
    }
    {
        let mut client = state.api_client.write().await;
        client.set_api_key(None);
    }
    Ok(())
}
