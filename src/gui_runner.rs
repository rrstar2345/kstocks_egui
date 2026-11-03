//! Hybrid runner: initializes backend with streamers and DB in a separate thread,
//! then runs egui on the main thread.
//!
//! Note: egui requires the main thread for its event loop on Linux/X11/Wayland.

use std::sync::mpsc::channel;
use tracing::info;

use crate::gui;
use crate::gui::backend::Backend;
use crate::gui::{GuiEvent, IndexTickRow as GuiIndexTickRow};
use crate::market::{market_clock, streamers, symbols, events::EventEmitter};
use crate::storage::{self, IndexTickRow};
use crate::{api, settings, stats};

/// Run the full hybrid app: initialize backend async, then run egui on main thread
pub fn run() {
    let (tx, rx) = channel::<GuiEvent>();
    let (ready_tx, ready_rx) = channel::<Backend>();

    // Spawn backend initialization in a dedicated background thread
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            if let Err(e) = init_backend_with_ticks(tx, ready_tx).await {
                eprintln!("Backend initialization error: {}", e);
            }

            // Keep runtime alive indefinitely
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        });
    });

    // Small delay to allow backend to start up before GUI
    std::thread::sleep(std::time::Duration::from_millis(500));
    info!("Backend initialized, starting egui GUI");

    // Run egui on main thread (required for X11/Wayland/Windows event loops)
    let app = gui::create_app(rx, ready_rx);
    gui::run_app(app);
}

/// Initialize the backend: DB, streamers, aggregation, retention
async fn init_backend_with_ticks(
    gui_tx: std::sync::mpsc::Sender<GuiEvent>,
    ready_tx: std::sync::mpsc::Sender<Backend>,
) -> anyhow::Result<()> {
    let app_settings = settings::setup_app_folders()?;
    let config = settings::load_or_create_config(&app_settings)?;

    info!("App root: {}", app_settings.root.display());
    info!("Local DB: {}", config.database.connection_string);
    info!("Server URL: {}", config.server.base_url);

    let db = storage::init_pool(&config.database).await?;
    info!("SQLite connected and schema verified");

    // Hand the pool + runtime handle to the UI (watchlists / paper trades)
    let _ = ready_tx.send(Backend {
        pool: db.clone(),
        rt: tokio::runtime::Handle::current(),
    });

    let app_stats = stats::new_shared_stats();
    let session = market_clock::new_shared_session_state();
    market_clock::refresh_if_stale(&config).await;
    {
        let mut s = app_stats.write().await;
        s.session_mode_label = session.mode().await.label().to_string();
    }

    // Market clock supervisor
    let supervisor_config = config.clone();
    let supervisor_stats = app_stats.clone();
    let supervisor_session = session.clone();
    tokio::spawn(async move {
        loop {
            market_clock::refresh_if_stale(&supervisor_config).await;
            let mode = supervisor_session
                .tick(supervisor_config.market_runtime.inactive_switch_after_secs)
                .await;
            {
                let mut s = supervisor_stats.write().await;
                s.session_mode_label = mode.label().to_string();
            }
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        }
    });

    // Tick writers
    let (index_tx, _index_writer_handle) = storage::start_index_tick_writer(
        db.clone(),
        config.database.clone(),
        app_stats.clone(),
    );
    let (option_tx, _option_writer_handle) = storage::start_option_tick_writer(
        db.clone(),
        config.database.clone(),
        app_stats.clone(),
    );

    // Create a GUI event emitter
    let gui_emitter = GuiEventEmitter { tx: gui_tx };

    // Resolve F&O symbols
    info!("Resolving F&O symbols and nearest expiries...");
    match symbols::resolve_symbol_expiries(&config).await {
        Ok(symbol_expiries) => {
            for se in &symbol_expiries {
                info!("Will stream options for {} / {}", se.symbol, se.expiry);
            }

            // Index streamer
            let emitter = gui_emitter.clone();
            let cfg = config.clone();
            let idx_tx = index_tx.clone();
            let st = app_stats.clone();
            let ses = session.clone();
            tokio::spawn(async move {
                streamers::indices::run(cfg, idx_tx, st, ses, emitter).await;
            });

            // Option streamers
            for se in symbol_expiries {
                let emitter = gui_emitter.clone();
                let cfg = config.clone();
                let opt_tx = option_tx.clone();
                let st = app_stats.clone();
                let ses = session.clone();
                let sym = se.symbol.clone();
                let exp = se.expiry.clone();
                tokio::spawn(async move {
                    streamers::options::run(cfg, sym, exp, opt_tx, st, ses, emitter).await;
                });
            }
        }
        Err(e) => {
            info!("Failed to resolve F&O symbols, indices only: {}", e);
            let emitter = gui_emitter.clone();
            let cfg = config.clone();
            let idx_tx = index_tx.clone();
            let st = app_stats.clone();
            let ses = session.clone();
            tokio::spawn(async move {
                streamers::indices::run(cfg, idx_tx, st, ses, emitter).await;
            });
        }
    }

    // Aggregation & retention
    storage::ohlc::spawn_1m_aggregation_loop(db.clone(), config.aggregation.run_interval_secs);
    storage::ohlc::spawn_daily_rollup_loop(db.clone());
    storage::retention::spawn_retention_loop(db.clone(), config.retention.clone());

    // Backfill
    if config.server.api_key.is_some() {
        let api_client = api::ApiClient::new(
            config.server.base_url.clone(),
            config.server.api_key.clone(),
        );
        let backfill_db = db.clone();
        let default_symbols = vec!["NIFTY".to_string(), "BANKNIFTY".to_string()];
        let option_backfill_client = api_client.clone();
        let option_backfill_db = db.clone();
        tokio::spawn(async move {
            storage::backfill::run_startup_backfill(&api_client, &backfill_db, &default_symbols)
                .await;
            storage::backfill::run_startup_option_backfill(&option_backfill_client, &option_backfill_db)
                .await;
        });
    }

    info!("Backend initialized successfully");
    Ok(())
}

/// GUI event emitter - bridges backend streamers to GUI
/// Replaces Tauri's AppHandle for the egui standalone app
#[derive(Clone)]
pub struct GuiEventEmitter {
    tx: std::sync::mpsc::Sender<GuiEvent>,
}

impl GuiEventEmitter {
    /// Emit an event to the GUI
    pub fn emit(&self, ev: GuiEvent) {
        let _ = self.tx.send(ev);
    }
}

impl EventEmitter for GuiEventEmitter {
    fn emit_index_tick(&self, row: &IndexTickRow) {
        // Convert storage::IndexTickRow to gui::IndexTickRow
        let gui_row = GuiIndexTickRow {
            time: row.time.to_rfc3339(),
            index_name: row.index_name.clone(),
            current_price: row.current_price,
            change: row.change,
            per_change: row.per_change,
            previous_close: row.previous_close,
            open: row.open,
            low: row.low,
            high: row.high,
            ind_status: row.ind_status.clone(),
            mkt_status: row.mkt_status.clone(),
            dissemination_time: row.dissemination_time.clone(),
        };
        let _ = self.tx.send(GuiEvent::Index(gui_row));
    }

    fn emit_option_tick(&self, row: &storage::OptionTickRow) {
        let _ = self.tx.send(GuiEvent::Option(row.clone()));
    }
}