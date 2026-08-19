//! Event emission for live market ticks. The streamers write every tick to SQLite
//! (via the batched writers in `storage::ticks`) and also send each tick straight
//! to the GUI via channels so the UI can update in real time instead of polling.
//!
//! This module provides traits and functions that work with both the old Tauri-based
//! event system and the new channel-based GUI emitter.

use tracing::warn;

use crate::storage::{IndexTickRow, OptionTickRow};

/// Trait for event emission - implemented by both Tauri AppHandle and GuiEventEmitter
pub trait EventEmitter: Send + Sync {
    fn emit_index_tick(&self, row: &IndexTickRow);
    fn emit_option_tick(&self, row: &OptionTickRow);
}

/// Emitter for the new egui-based GUI using channels
#[derive(Clone)]
pub struct GuiEventEmitter {
    index_tx: std::sync::mpsc::Sender<IndexTickRow>,
    // Future: add option_tx for option streaming
}

impl GuiEventEmitter {
    pub fn new(index_tx: std::sync::mpsc::Sender<IndexTickRow>) -> Self {
        Self { index_tx }
    }
}

impl EventEmitter for GuiEventEmitter {
    fn emit_index_tick(&self, row: &IndexTickRow) {
        if let Err(e) = self.index_tx.send(row.clone()) {
            warn!("Failed to send index tick to GUI: {}", e);
        }
    }

    fn emit_option_tick(&self, _row: &OptionTickRow) {
        // TODO: implement option streaming to GUI
        warn!("Option tick streaming not yet implemented for egui");
    }
}

/// Legacy constants for backwards compatibility
pub const INDEX_TICK_EVENT: &str = "index-tick";
pub const OPTION_TICK_EVENT: &str = "option-tick";

/// Legacy function - kept for compatibility during transition
pub fn emit_index_tick<E: EventEmitter>(emitter: &E, row: &IndexTickRow) {
    emitter.emit_index_tick(row);
}

/// Legacy function - kept for compatibility during transition
pub fn emit_option_tick<E: EventEmitter>(emitter: &E, row: &OptionTickRow) {
    emitter.emit_option_tick(row);
}