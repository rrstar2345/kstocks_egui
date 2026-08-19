//! Backfill: pulls historical `index_ohlc_1m` / `index_ohlc_1d` bars from
//! kstocks-server's read-only `/ohlc/index` API and upserts them into the
//! client's own local tables, so the chart has continuity across periods
//! the app wasn't running to stream live (app closed overnight, laptop
//! asleep, etc.).
//!
//! Deliberately narrow scope, matching the server's role as described in
//! its README: "the desktop app's job once it connects directly to NSE's
//! WSS for gap-fill" — this module IS that gap-fill, and only that.
//! It never touches raw ticks (`index_ticks`/`option_ticks`) or
//! `aggregation_state`; those are exclusively owned by the local streaming
//! + aggregation pipeline (`market::streamers`, `storage::ohlc`). Option
//! backfill (`backfill_option_1m` / `run_startup_option_backfill`) mirrors
//! the same upsert pattern for `option_ohlc_1m`.

use anyhow::{anyhow, Result};
use sqlx::SqlitePool;
use tracing::{error, info, warn};

use crate::api::ApiClient;

/// Upsert one page of `/ohlc/index` bars into `index_ohlc_1m`. Same
/// `ON CONFLICT ... DO UPDATE` shape as the server's own aggregation, so a
/// re-run (e.g. overlapping ranges) is always safe to repeat.
async fn upsert_index_1m_bars(
    pool: &SqlitePool,
    index_name: &str,
    bars: &[crate::api::client::IndexBar],
) -> Result<usize> {
    let mut tx = pool.begin().await?;
    let mut count = 0;

    for bar in bars {
        sqlx::query(
            r#"
            INSERT INTO index_ohlc_1m (index_name, bucket_start, open, high, low, close, tick_count)
            VALUES (?, ?, ?, ?, ?, ?, 0)
            ON CONFLICT (index_name, bucket_start) DO UPDATE SET
                open = excluded.open,
                high = excluded.high,
                low = excluded.low,
                close = excluded.close
            "#,
        )
        .bind(index_name)
        .bind(&bar.bucket_start)
        .bind(bar.open)
        .bind(bar.high)
        .bind(bar.low)
        .bind(bar.close)
        .execute(&mut *tx)
        .await?;
        count += 1;
    }

    tx.commit().await?;
    Ok(count)
}

/// Upsert one page of `/ohlc/index` bars into `index_ohlc_1d`, same shape.
async fn upsert_index_1d_bars(
    pool: &SqlitePool,
    index_name: &str,
    bars: &[crate::api::client::IndexBar],
) -> Result<usize> {
    let mut tx = pool.begin().await?;
    let mut count = 0;

    for bar in bars {
        sqlx::query(
            r#"
            INSERT INTO index_ohlc_1d (index_name, bucket_start, open, high, low, close, tick_count)
            VALUES (?, ?, ?, ?, ?, ?, 0)
            ON CONFLICT (index_name, bucket_start) DO UPDATE SET
                open = excluded.open,
                high = excluded.high,
                low = excluded.low,
                close = excluded.close
            "#,
        )
        .bind(index_name)
        .bind(&bar.bucket_start)
        .bind(bar.open)
        .bind(bar.high)
        .bind(bar.low)
        .bind(bar.close)
        .execute(&mut *tx)
        .await?;
        count += 1;
    }

    tx.commit().await?;
    Ok(count)
}

/// Fetch and upsert `index_ohlc_1m` bars for one symbol/range/interval.
/// `interval` should be a 1-minute-tier interval valid for the server's
/// `/ohlc/index` (see server README's range/interval table, e.g. `1m` for
/// `range=1d`).
pub async fn backfill_index_1m(
    api_client: &ApiClient,
    pool: &SqlitePool,
    symbol: &str,
    range: &str,
    interval: &str,
) -> Result<usize> {
    let bars = api_client
        .ohlc_index(symbol, range, interval)
        .await
        .map_err(|e| anyhow!("backfill fetch failed for {} {}/{}: {}", symbol, range, interval, e))?;

    let n = upsert_index_1m_bars(pool, symbol, &bars).await?;
    info!("Backfilled {} index_ohlc_1m bar(s) for {} ({}/{})", n, symbol, range, interval);
    Ok(n)
}

/// Fetch and upsert `index_ohlc_1d` bars for one symbol/range/interval
/// (e.g. `range=1y`, `interval=1mo` or similar daily-tier combination —
/// see server README).
pub async fn backfill_index_1d(
    api_client: &ApiClient,
    pool: &SqlitePool,
    symbol: &str,
    range: &str,
    interval: &str,
) -> Result<usize> {
    let bars = api_client
        .ohlc_index(symbol, range, interval)
        .await
        .map_err(|e| anyhow!("backfill fetch failed for {} {}/{}: {}", symbol, range, interval, e))?;

    let n = upsert_index_1d_bars(pool, symbol, &bars).await?;
    info!("Backfilled {} index_ohlc_1d bar(s) for {} ({}/{})", n, symbol, range, interval);
    Ok(n)
}

/// Upsert one page of `/ohlc/option` bars into `option_ohlc_1m` for a
/// single symbol/expiry/strike. Same upsert shape as the local option
/// aggregation, so it merges cleanly with whatever the local streamer has
/// already written for the same bucket.
async fn upsert_option_1m_bars(
    pool: &SqlitePool,
    symbol: &str,
    expiry: &str,
    strike: f64,
    bars: &[crate::api::client::OptionBar],
) -> Result<usize> {
    let expiry_date = crate::storage::ohlc_expiry_date_str(expiry);
    let mut tx = pool.begin().await?;
    let mut count = 0;

    for bar in bars {
        sqlx::query(
            r#"
            INSERT INTO option_ohlc_1m (
                symbol, expiry, expiry_date, strike_price, bucket_start,
                ce_open, ce_high, ce_low, ce_close, ce_volume, ce_oi_close,
                pe_open, pe_high, pe_low, pe_close, pe_volume, pe_oi_close,
                tick_count
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)
            ON CONFLICT (symbol, expiry, strike_price, bucket_start) DO UPDATE SET
                ce_open = COALESCE(excluded.ce_open, option_ohlc_1m.ce_open),
                ce_high = COALESCE(excluded.ce_high, option_ohlc_1m.ce_high),
                ce_low = COALESCE(excluded.ce_low, option_ohlc_1m.ce_low),
                ce_close = COALESCE(excluded.ce_close, option_ohlc_1m.ce_close),
                ce_volume = COALESCE(excluded.ce_volume, option_ohlc_1m.ce_volume),
                ce_oi_close = COALESCE(excluded.ce_oi_close, option_ohlc_1m.ce_oi_close),
                pe_open = COALESCE(excluded.pe_open, option_ohlc_1m.pe_open),
                pe_high = COALESCE(excluded.pe_high, option_ohlc_1m.pe_high),
                pe_low = COALESCE(excluded.pe_low, option_ohlc_1m.pe_low),
                pe_close = COALESCE(excluded.pe_close, option_ohlc_1m.pe_close),
                pe_volume = COALESCE(excluded.pe_volume, option_ohlc_1m.pe_volume),
                pe_oi_close = COALESCE(excluded.pe_oi_close, option_ohlc_1m.pe_oi_close)
            "#,
        )
        .bind(symbol)
        .bind(expiry)
        .bind(&expiry_date)
        .bind(strike)
        .bind(&bar.bucket_start)
        .bind(bar.ce_open)
        .bind(bar.ce_high)
        .bind(bar.ce_low)
        .bind(bar.ce_close)
        .bind(bar.ce_volume)
        .bind(bar.ce_oi_close)
        .bind(bar.pe_open)
        .bind(bar.pe_high)
        .bind(bar.pe_low)
        .bind(bar.pe_close)
        .bind(bar.pe_volume)
        .bind(bar.pe_oi_close)
        .execute(&mut *tx)
        .await?;
        count += 1;
    }

    tx.commit().await?;
    Ok(count)
}

/// Fetch and upsert `option_ohlc_1m` bars (both legs) for one
/// symbol/expiry/strike, gap-filling since the app last streamed live.
/// `leg` is passed through to the server as-is (server returns both legs
/// regardless per the documented `/ohlc/option` contract; kept as a
/// parameter in case the server narrows the response by leg later).
pub async fn backfill_option_1m(
    api_client: &ApiClient,
    pool: &SqlitePool,
    symbol: &str,
    expiry: &str,
    strike: f64,
    leg: &str,
) -> Result<usize> {
    let bars = api_client
        .ohlc_option(symbol, expiry, strike, "1d", "1m", leg)
        .await
        .map_err(|e| anyhow!("option backfill fetch failed for {} {} {}: {}", symbol, expiry, strike, e))?;

    let n = upsert_option_1m_bars(pool, symbol, expiry, strike, &bars).await?;
    info!("Backfilled {} option_ohlc_1m bar(s) for {} {} {}", n, symbol, expiry, strike);
    Ok(n)
}

/// Run a backfill pass for a set of index symbols on app startup: recent 1m
/// history (gap-fill since last close) plus daily history. Best-effort per
/// symbol — one symbol's failure (e.g. not yet approved, network down)
/// doesn't stop the others.
pub async fn run_startup_backfill(api_client: &ApiClient, pool: &SqlitePool, symbols: &[String]) {
    for symbol in symbols {
        if let Err(e) = backfill_index_1m(api_client, pool, symbol, "1d", "1m").await {
            warn!("Startup 1m backfill failed for {}: {}", symbol, e);
        }
        if let Err(e) = backfill_index_1d(api_client, pool, symbol, "3mo", "1d").await {
            warn!("Startup 1d backfill failed for {}: {}", symbol, e);
        }
    }
}

/// Backfill missing `option_ohlc_1m` data (both CE/PE legs) for every
/// strike currently seen locally (from the streamer-populated
/// `option_ticks`/`option_ohlc_1m`), one symbol/expiry pair at a time.
/// Best-effort — one strike's failure doesn't stop the rest. Intended to
/// run after the index backfill on startup, only for registered
/// (API-key-holding) users, so the option chain/list views have
/// continuity across periods the app wasn't running.
pub async fn run_startup_option_backfill(api_client: &ApiClient, pool: &SqlitePool) {
    let symbols = match crate::storage::ohlc::list_option_symbols(pool).await {
        Ok(s) => s,
        Err(e) => {
            warn!("Could not list local option symbols for backfill: {}", e);
            return;
        }
    };

    for symbol in symbols {
        let expiries = match crate::storage::ohlc::list_option_expiries(pool, &symbol).await {
            Ok(e) => e,
            Err(e) => {
                warn!("Could not list expiries for {} backfill: {}", symbol, e);
                continue;
            }
        };
        for expiry in expiries {
            let strikes =
                match crate::storage::ohlc::list_option_strikes(pool, &symbol, &expiry).await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Could not list strikes for {}/{} backfill: {}", symbol, expiry, e);
                        continue;
                    }
                };
            for strike in strikes {
                if let Err(e) = backfill_option_1m(api_client, pool, &symbol, &expiry, strike, "both").await {
                    warn!("Startup option backfill failed for {}/{}/{}: {}", symbol, expiry, strike, e);
                }
            }
        }
    }
}

/// Errors are logged, never propagated — backfill is a best-effort
/// convenience layered on top of local streaming, never a hard dependency
/// for the app to function (per CONTEXT.md: "Keep the UI completely
/// independent of market data providers").
#[allow(dead_code)]
fn log_backfill_error(context: &str, e: &anyhow::Error) {
    error!("{}: {}", context, e);
}