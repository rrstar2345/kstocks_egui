# Technical Documentation: kstocks_egui

This document describes the architecture, data flow, storage schema, and key modules of the `kstocks` desktop client. For setup and usage, see the [README](../README.md).

## 1. Architecture overview

The app is a single process with two halves:

- **Backend** (Tokio runtime on a dedicated OS thread): settings, database, market clock, WebSocket streamers, tick writers, aggregation, retention, and backfill.
- **Frontend** (egui on the main thread): renders the candlestick chart from data the backend sends over a channel.

```
                 +-------------------------- backend thread (tokio) ---------------------------+
NSE REST  ---->  | market_clock / symbols                                                     |
NSE WSS   ---->  | streamers::indices, streamers::options                                     |
                 |        |                       |                                           |
                 |        v                       v                                           |
                 |  tick writers (batched)  -> SQLite (kstocks.db, WAL)                       |
                 |        |                                    ^                              |
                 |        | GuiEventEmitter                    | aggregation / retention       |
                 +--------|-----------------------------------|------------------------------+
                          | std::sync::mpsc                    |
                          v                                    |
                 +---- main thread (egui/eframe) -------+      |
                 | CandlestickApp: CandleSeries, plot   |     (optional)
                 +--------------------------------------+   kstocks-server
                                                            (ohlc backfill)
```

## 2. Startup sequence

Defined in `src/gui_runner.rs`:

1. `main.rs` initialises `tracing_subscriber`, then calls `gui_runner::run()`.
2. `run()` creates an `mpsc` channel and spawns a thread that builds a Tokio runtime and runs `init_backend_with_ticks`.
3. The main thread sleeps 500 ms (a fixed delay, not a readiness signal), then creates and runs the egui app.
4. `init_backend_with_ticks`:
   1. Creates app folders and loads or creates `settings_client.json`.
   2. Opens the SQLite pool and creates the schema (`storage::db::init_pool`).
   3. Refreshes the NSE clock if stale and starts the market-clock supervisor (every 30 s).
   4. Starts the index and option tick writers.
   5. Resolves F&O symbols and nearest expiries. On success it starts one index streamer and one option streamer per symbol. On failure it starts only the index streamer.
   6. Starts the 1-minute aggregation loop, the daily rollup loop, and the retention loop.
   7. If `server.api_key` is set, runs startup backfill for index bars (NIFTY, BANKNIFTY) and option bars.
5. The backend thread then stays alive, sleeping in one-hour intervals.

## 3. Module reference

### 3.1 `settings`
- `setup_app_folders()` creates `<data_local_dir>/.kstocks/{db,logs}` and returns `AppPaths`.
- `AppConfig` groups `database`, `server`, `system` (NSE endpoints), `market_runtime`, `aggregation`, and `retention`. The last two use defaults when missing from the file.
- `load_or_create_config` falls back to defaults if the file is missing or unparseable. It does not repair a broken file.

### 3.2 `market`
| Module | Responsibility |
|---|---|
| `http` | Shared `reqwest` client with NSE headers. `warm_nse_session()` primes cookies. `connect_nse_ws()` opens a WebSocket and attaches NSE cookies to the upgrade request. |
| `market_clock` | Derives IST time from NSE's server time endpoint. Tracks `SessionMode` (Active or Idle) with `SessionState`. Switches to Idle after `inactive_switch_after_secs` without real ticks. |
| `symbols` | Fetches the F&O index list and nearest expiry per symbol. Produces `SymbolExpiry` values. |
| `streamers::indices` | Index WebSocket. Active mode holds the socket open. Idle mode polls every `idle_poll_interval_secs` and listens for `idle_poll_listen_secs`. |
| `streamers::options` | Per-symbol option WebSocket (`?symbol=…&expiry=…`), with the same Active/Idle behaviour. |
| `events` | `EventEmitter` trait, implemented by `gui_runner::GuiEventEmitter` to forward index ticks to the GUI. |

### 3.3 `storage`
| Module | Responsibility |
|---|---|
| `db` | Pool setup (WAL, `synchronous=NORMAL`, foreign keys on) and schema creation. |
| `ticks` | `IndexTickRow` and `OptionTickRow` types. Batched writers flush on `batch_max_rows` or `batch_max_wait_ms`. |
| `ohlc` | 1-minute aggregation for index and option ticks, and a daily rollup from `index_ohlc_1m`. Uses `aggregation_state` watermarks so each run resumes where the last one ended. |
| `retention` | Daily purge of raw ticks, 1m bars, and 1d bars based on watermarks and config. Runs `PRAGMA optimize` and `VACUUM` afterward. |
| `backfill` | Fetches bars from `kstocks-server` `/ohlc/index` and `/ohlc/option` and upserts them. It only writes bar tables, never raw ticks. |
| `watchlists`, `paper_trades`, `settings_store` | CRUD helpers for the local tables. |

### 3.4 `api`
`ApiClient` wraps the `kstocks-server` endpoints `/register`, `/validate`, `/health`, `/ohlc/index`, and `/ohlc/option`. It sends the API key with requests when one is set.

### 3.5 `stats`
`AppStats` is a shared `RwLock` struct with per-stream connection state (`ConnState`), DB write counters, and the current session label. The backend updates it and the UI reads it.

### 3.6 `gui` and `gui_runner`
- `CandlestickApp` drains the tick channel each frame (`process_pending_ticks`), groups prices into `Candle`s via `CandleSeries`, and draws them with `egui_plot::BoxPlot`.
- `CandleSeries::new(1)` means one candle per second in the current build.
- `GuiEventEmitter::emit_option_tick` is a TODO, so option data is not displayed.

## 4. Data flow

1. **Ingest.** A streamer parses each WebSocket message into `IndexTickRow` or `OptionTickRow` and sends it to the matching writer channel.
2. **Persist.** Writers batch rows and insert them into `index_ticks` or `option_ticks`.
3. **Display.** The index streamer also sends a copy through `GuiEventEmitter` to the GUI channel.
4. **Aggregate.** Every `run_interval_secs`, the 1m job reads ticks after the watermark, builds OHLC per bucket, upserts into `index_ohlc_1m` or `option_ohlc_1m`, and advances the watermark. The daily job rolls up `index_ohlc_1m` into `index_ohlc_1d`.
5. **Purge.** The retention job deletes data older than the configured window, but only up to a watermark that shows the data has already been aggregated.
6. **Backfill (optional).** On startup, if an API key is set, bars for periods the app did not stream are fetched from `kstocks-server` and upserted into the bar tables.

## 5. Database schema

SQLite file: `<root>/db/kstocks.db`.

**Application tables**

| Table | Key columns | Notes |
|---|---|---|
| `app_settings` | key (PK), value | Generic key/value store |
| `watchlists` | id, name | |
| `watchlist_items` | id, watchlist_id (FK), symbol | Indexed on `watchlist_id` |
| `layouts` | id, … | Saved UI layouts |
| `paper_trades` | id, symbol, status, … | Indexed on `status` and `symbol` |

**Market data tables**

| Table | Key columns | Indexes |
|---|---|---|
| `index_ticks` | index_name, time, … | `(index_name, time)` |
| `option_ticks` | symbol, expiry, strike, time, … | `(symbol, expiry, strike, time)` |
| `index_ohlc_1m` | index_name, bucket_start, OHLC | `bucket_start` |
| `index_ohlc_1d` | index_name, bucket_start, OHLC | `bucket_start` |
| `option_ohlc_1m` | symbol, expiry_date, strike, bucket_start, OHLC | `bucket_start`, `expiry_date` |
| `aggregation_state` | table_name (PK), last_bucket_end | Watermarks |

See `src/storage/db.rs` for exact column definitions.

## 6. Configuration reference

| Key | Default | Meaning |
|---|---|---|
| `database.batch_max_rows` | 500 | Max rows buffered before a flush |
| `database.batch_max_wait_ms` | 500 | Max wait before flushing a partial batch |
| `server.base_url` | `http://localhost:8787` | kstocks-server base URL |
| `server.api_key` | `null` | Enables backfill when set |
| `market_runtime.symbol_refresh_interval_seconds` | 3600 | Symbol list refresh interval |
| `market_runtime.reconnect_delay_seconds` | 5 | Delay before reconnecting a dropped socket |
| `market_runtime.restrict_to_trading_window` | false | Limit streaming to 09:00–15:30 IST, Mon–Fri |
| `market_runtime.inactive_switch_after_secs` | 3600 | Idle-switch timeout |
| `market_runtime.idle_poll_interval_secs` | 3600 | Idle poll interval |
| `market_runtime.idle_poll_listen_secs` | 15 | Listen time per idle poll |
| `aggregation.run_interval_secs` | 300 | 1m aggregation interval |
| `retention.raw_ticks_keep_trading_days` | 2 | Raw tick retention |
| `retention.index_ohlc_1m_keep_days` | 60 | 1m index bar retention |
| `retention.option_ohlc_1m_expiry_grace_days` | 7 | Days after expiry before option 1m bars are purged |
| `retention.index_ohlc_1d_keep_days` | 1095 | Daily bar retention (0 keeps forever) |

## 7. External dependencies

**Data sources (NSE, unofficial endpoints):**
- `wss://streamer.nseindia.com/streams/indices/high/windices`
- `wss://streamer.nseindia.com/streams/fo/mbp`
- `https://www.nseindia.com/api/...` for index list, option expiry info, and server time

These endpoints are not a public API and may change without notice. The code sends browser-like headers and cookies. Check the NSE site's terms before using this software.

**Crates:** `eframe`/`egui`/`egui_plot` (GUI), `tokio` (async runtime), `tokio-tungstenite` (WebSocket), `reqwest` (HTTP), `sqlx` with SQLite (storage), `serde`/`serde_json` (config and payloads), `chrono` (time), `tracing` (logging).

## 8. Known limitations

- Option ticks are not shown in the UI.
- The sleep-based 500 ms GUI start delay is not a readiness check.
- The daily rollup and aggregation loops run regardless of whether the streamers are live.
- Log files are not written to `logs/`.
- Config parsing falls back to defaults silently on a broken file (an error is logged).
- `PROGRESS.md`, referenced in module doc comments, is not in the repository.
- No automated tests were found in the source tree.

## 9. Extending the app

- **Show option data:** implement `GuiEventEmitter::emit_option_tick` and add a corresponding channel to `CandlestickApp`.
- **Change candle timeframe:** adjust `CandleSeries::new(bucket_seconds)` in `gui.rs`, or make it a UI control.
- **Add a table:** add `CREATE TABLE IF NOT EXISTS` to `create_schema` in `storage/db.rs` and add a module under `storage/`.
- **Add an endpoint:** add a method to `ApiClient` and reuse `url()` for path joining.
- **Add a streamer:** follow `streamers/indices.rs`, which handles Active and Idle modes, and register its task in `gui_runner.rs`.