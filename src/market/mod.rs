//! Everything related to fetching/streaming NSE market data: the shared
//! HTTP client, NSE-clock-derived session state, F&O symbol/expiry
//! resolution, and the live WSS streamers.
//!
//! Ported from kstocks-server's `src/market/` — same code, only the
//! `crate::settings::AppConfig` fields it reads (`system`, `market_runtime`)
//! were renamed on the client side to avoid clashing with `ServerConfig`
//! (the kstocks-server API client's own config struct). See PROGRESS.md.

pub mod events;
pub mod http;
pub mod market_clock;
pub mod streamers;
pub mod symbols;