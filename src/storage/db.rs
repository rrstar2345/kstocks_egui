//! `kstocks.db`: the client's local SQLite database. Holds everything that
//! belongs to this desktop install: app settings, watchlists, layouts,
//! simulated (paper) trades, AND (ported from kstocks-server) the raw
//! tick / OHLC tables that the local NSE WSS streamers write into and the
//! local aggregation job rolls up — this file is the client's own market
//! data store, not just a cache of the server.
//!
//! The kstocks-server backfill (see `storage::backfill`) only ever writes
//! into `index_ohlc_1m` / `index_ohlc_1d` (gap-fill for periods the app
//! wasn't running to stream live) — it never touches raw ticks, which are
//! exclusively populated by this client's own WSS connection.

use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tracing::info;

use crate::settings::DatabaseConfig;

/// Open (creating if missing) the local database and ensure its schema
/// exists. Same WAL/NORMAL pragma pattern as the server for consistency.
pub async fn init_pool(db_config: &DatabaseConfig) -> Result<SqlitePool> {
    let connect_options = SqliteConnectOptions::new()
        .filename(&db_config.connection_string)
        .create_if_missing(true);

    let pool = SqlitePool::connect_with(connect_options)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to local database: {}", e))?;

    sqlx::query("PRAGMA journal_mode = WAL;").execute(&pool).await?;
    sqlx::query("PRAGMA synchronous = NORMAL;").execute(&pool).await?;
    sqlx::query("PRAGMA foreign_keys = ON;").execute(&pool).await?;

    create_schema(&pool).await?;

    Ok(pool)
}

async fn create_schema(pool: &SqlitePool) -> Result<()> {
    // Generic key/value settings the UI can read/write freely (theme,
    // last-selected workspace, feature flags, etc.) without needing a
    // schema migration for every new preference.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Watchlists: a named group of symbols the user tracks.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS watchlists (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS watchlist_items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            watchlist_id INTEGER NOT NULL REFERENCES watchlists(id) ON DELETE CASCADE,
            symbol TEXT NOT NULL,
            instrument_type TEXT NOT NULL DEFAULT 'index',
            sort_order INTEGER NOT NULL DEFAULT 0,
            added_at TEXT NOT NULL,
            UNIQUE(watchlist_id, symbol)
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Chart/window layouts, saved as opaque JSON blobs the frontend owns
    // the shape of (panel positions, indicators, timeframe, etc.).
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS layouts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            layout_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Simulated (paper) trades, priced off live/streamed market data but
    // executed entirely client-side — independent of any live broker.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS paper_trades (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            symbol TEXT NOT NULL,
            instrument_type TEXT NOT NULL DEFAULT 'index',
            side TEXT NOT NULL CHECK (side IN ('buy', 'sell')),
            quantity REAL NOT NULL,
            entry_price REAL NOT NULL,
            exit_price REAL,
            status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'closed', 'cancelled')),
            opened_at TEXT NOT NULL,
            closed_at TEXT,
            notes TEXT
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_watchlist_items_watchlist_id ON watchlist_items(watchlist_id);")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_paper_trades_status ON paper_trades(status);")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_paper_trades_symbol ON paper_trades(symbol);")
        .execute(pool)
        .await?;

    create_market_data_schema(pool).await?;

    info!("Local database schema verified: app_settings, watchlists, watchlist_items, layouts, paper_trades, market data tables");

    Ok(())
}

/// Raw tick + OHLC tables, ported from kstocks-server's
/// `storage::ticks::create_schema`. Identical shape so the ported
/// aggregation/retention code (see `storage::ohlc`, `storage::retention`)
/// and the server's `/ohlc/*` response shapes line up without translation.
async fn create_market_data_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS index_ticks (
            time TEXT NOT NULL,
            index_name TEXT NOT NULL,
            current_price REAL,
            change REAL,
            per_change REAL,
            previous_close REAL,
            open REAL,
            low REAL,
            high REAL,
            ind_status TEXT,
            mkt_status TEXT,
            dissemination_time TEXT
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_index_ticks_name_time ON index_ticks(index_name, time);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS option_ticks (
            time TEXT NOT NULL,
            symbol TEXT NOT NULL,
            expiry TEXT NOT NULL,
            strike_price REAL,

            ce_last_price REAL,
            ce_change REAL,
            ce_volume INTEGER,
            ce_oi REAL,
            ce_bid REAL,
            ce_ask REAL,

            pe_last_price REAL,
            pe_change REAL,
            pe_volume INTEGER,
            pe_oi REAL,
            pe_bid REAL,
            pe_ask REAL
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_option_ticks_symbol_expiry_strike_time
        ON option_ticks(symbol, expiry, strike_price, time);
        "#,
    )
    .execute(pool)
    .await?;

    // ------------------------------------------------------------------
    // Aggregated OHLC tables. Populated by BOTH the local aggregation job
    // (from this client's own raw ticks) AND the server backfill (gap-fill
    // for `index_ohlc_1m` / `index_ohlc_1d` only) — both paths upsert into
    // the same tables, keyed identically, so they merge without conflict.
    // ------------------------------------------------------------------

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS index_ohlc_1m (
            index_name TEXT NOT NULL,
            bucket_start TEXT NOT NULL,
            open REAL NOT NULL,
            high REAL NOT NULL,
            low REAL NOT NULL,
            close REAL NOT NULL,
            tick_count INTEGER NOT NULL,
            PRIMARY KEY (index_name, bucket_start)
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_index_ohlc_1m_bucket ON index_ohlc_1m(bucket_start);")
        .execute(pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS index_ohlc_1d (
            index_name TEXT NOT NULL,
            bucket_start TEXT NOT NULL,
            open REAL NOT NULL,
            high REAL NOT NULL,
            low REAL NOT NULL,
            close REAL NOT NULL,
            tick_count INTEGER NOT NULL,
            PRIMARY KEY (index_name, bucket_start)
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_index_ohlc_1d_bucket ON index_ohlc_1d(bucket_start);")
        .execute(pool)
        .await?;

    // `expiry_date` mirrors the server: a real ISO date column separate
    // from the free-form `expiry` string, so retention can compare against
    // "today" cheaply.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS option_ohlc_1m (
            symbol TEXT NOT NULL,
            expiry TEXT NOT NULL,
            expiry_date DATE NOT NULL,
            strike_price REAL NOT NULL,
            bucket_start TEXT NOT NULL,

            ce_open REAL,
            ce_high REAL,
            ce_low REAL,
            ce_close REAL,
            ce_volume INTEGER,
            ce_oi_close REAL,

            pe_open REAL,
            pe_high REAL,
            pe_low REAL,
            pe_close REAL,
            pe_volume INTEGER,
            pe_oi_close REAL,

            tick_count INTEGER NOT NULL,
            PRIMARY KEY (symbol, expiry, strike_price, bucket_start)
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_option_ohlc_1m_bucket ON option_ohlc_1m(bucket_start);")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_option_ohlc_1m_expiry_date ON option_ohlc_1m(expiry_date);")
        .execute(pool)
        .await?;

    // Watermark table: tracks how far each aggregation tier has scanned.
    // NOTE: the server backfill path does NOT use this watermark — it's
    // driven by its own last-synced-bucket bookkeeping (see
    // `storage::backfill`), so a fresh install can gap-fill from the server
    // without waiting for local streaming to establish a baseline.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS aggregation_state (
            table_name TEXT PRIMARY KEY,
            last_bucket_end TEXT NOT NULL
        );
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}