//! Tauri commands wrapping local paper-trade storage.

use tauri::State;

use crate::state::AppState;
use crate::storage::paper_trades::{self, PaperTrade};

#[tauri::command]
pub async fn open_paper_trade(
    state: State<'_, AppState>,
    symbol: String,
    instrument_type: String,
    side: String,
    quantity: f64,
    entry_price: f64,
    notes: Option<String>,
) -> Result<i64, String> {
    paper_trades::open_trade(
        &state.db,
        &symbol,
        &instrument_type,
        &side,
        quantity,
        entry_price,
        notes.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_paper_trade(
    state: State<'_, AppState>,
    id: i64,
    exit_price: f64,
) -> Result<(), String> {
    paper_trades::close_trade(&state.db, id, exit_price).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_paper_trades(
    state: State<'_, AppState>,
    status: Option<String>,
) -> Result<Vec<PaperTrade>, String> {
    paper_trades::list_trades(&state.db, status.as_deref())
        .await
        .map_err(|e| e.to_string())
}
