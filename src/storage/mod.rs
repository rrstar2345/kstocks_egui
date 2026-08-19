//! Local storage layer for `kstocks.db` — app settings, watchlists,
//! layouts, simulated (paper) trades, and (ported from kstocks-server)
//! this client's own raw tick / OHLC market data tables.
//!
//! `ticks`, `ohlc`, and `retention` are ported near-verbatim from
//! kstocks-server; `backfill` is new client-only code that calls the
//! server's read-only API to gap-fill `index_ohlc_1m`/`index_ohlc_1d`.
//! See PROGRESS.md for what's wired up vs. still pending.

pub mod backfill;
pub mod db;
pub mod ohlc;
pub mod paper_trades;
pub mod retention;
pub mod settings_store;
pub mod ticks;
pub mod watchlists;

pub use db::init_pool;
pub use ohlc::ohlc_expiry_date_str;
pub use ticks::{
    start_index_tick_writer, start_option_tick_writer, IndexTickRow, IndexTickSender,
    OptionTickRow, OptionTickSender,
};