# kstocks_egui

A standalone desktop app written in Rust with [egui](https://github.com/emilk/egui) (via `eframe`). It streams live NSE (National Stock Exchange of India) index data, builds candlesticks in real time, and stores market data in a local SQLite database.

> The app is a client. It can optionally backfill historical bars from a separate `kstocks-server` instance, which is not part of this repository.

## Features

- Live NSE index streaming over WebSocket (`wss://streamer.nseindia.com`)
- Real-time candlestick chart for the selected index (1-second buckets in the current GUI)
- Automatic F&O symbol and nearest-expiry discovery
- Option tick streaming and storage (option data is stored but not yet shown in the GUI)
- Local SQLite store with raw ticks, 1-minute and daily OHLC tables
- Background aggregation (raw ticks to 1m/1d OHLC) and retention/purge jobs
- Market-session awareness: Active (held-open socket) vs Idle (periodic polling) modes
- Optional startup backfill of index and option bars from `kstocks-server`

## Requirements

- Rust toolchain (edition 2021, stable)
- Desktop environment supported by eframe (X11, Wayland, Windows, or macOS)
- Network access to `nseindia.com` and `streamer.nseindia.com`
- Optional: a running `kstocks-server` and an API key, for backfill

## Build and run

```bash
git clone https://github.com/rrstar2345/kstocks_egui.git
cd kstocks_egui
cargo run --release
```

Logging uses `tracing`. Set `RUST_LOG` to control verbosity, for example:

```bash
RUST_LOG=info cargo run
```

## Configuration

On first launch the app creates its folders and a settings file:

| Path (Linux example) | Purpose |
|---|---|
| `~/.local/share/.kstocks/` | App root (`dirs::data_local_dir()`) |
| `~/.local/share/.kstocks/db/kstocks.db` | SQLite database |
| `~/.local/share/.kstocks/logs/` | Log directory (created; file logging not yet wired) |
| `~/.local/share/.kstocks/settings_client.json` | Runtime configuration |

On Windows and macOS the root is under the platform's local data directory.

Key settings in `settings_client.json`:

- `server.base_url`: `kstocks-server` URL (default `http://localhost:8787`)
- `server.api_key`: set after registering with the server; `null` disables backfill
- `market_runtime.*`: idle/active switching and reconnect timing
- `aggregation.run_interval_secs`: how often 1m aggregation runs (default 300)
- `retention.*`: how long each table is kept

## Project layout

```
src/
├── main.rs            Binary entry point (initialises tracing, calls gui_runner::run)
├── lib.rs             Library crate root (exports modules)
├── gui_runner.rs      Starts backend on a background thread, runs egui on main thread
├── gui.rs             egui candlestick UI
├── settings.rs        App paths and configuration load/save
├── stats.rs           Shared runtime statistics (stream state, DB counters)
├── api/client.rs      HTTP client for kstocks-server
├── market/            NSE clock, symbols, HTTP helpers, WebSocket streamers
└── storage/           SQLite schema, tick writers, aggregation, retention, backfill
```

## Documentation

- [Technical documentation](docs/TECHNICAL.md): architecture, threading, data flow, schema, and extension points

## Status and known gaps

- Option ticks are received and stored, but the GUI does not yet display them (`emit_option_tick` is a TODO).
- The GUI shows a single index candlestick view.
- The paper-trading, watchlist, and layout storage modules exist, but this repository does not wire them into the UI.
- Log files are not written to `logs/` yet.
- Registration with `kstocks-server` is implemented in the API client but is not exposed in the UI.

