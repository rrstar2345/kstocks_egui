//! Queries against `watchlists` / `watchlist_items`.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Watchlist {
    pub id: i64,
    pub name: String,
    pub sort_order: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WatchlistItem {
    pub id: i64,
    pub watchlist_id: i64,
    pub symbol: String,
    pub instrument_type: String,
    pub sort_order: i64,
    pub added_at: String,
}

pub async fn list_watchlists(pool: &SqlitePool) -> Result<Vec<Watchlist>> {
    let rows = sqlx::query_as::<_, Watchlist>("SELECT * FROM watchlists ORDER BY sort_order, id")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn create_watchlist(pool: &SqlitePool, name: &str) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query("INSERT INTO watchlists (name, sort_order, created_at) VALUES (?, 0, ?)")
        .bind(name)
        .bind(&now)
        .execute(pool)
        .await?;
    Ok(res.last_insert_rowid())
}

pub async fn delete_watchlist(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM watchlists WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_items(pool: &SqlitePool, watchlist_id: i64) -> Result<Vec<WatchlistItem>> {
    let rows = sqlx::query_as::<_, WatchlistItem>(
        "SELECT * FROM watchlist_items WHERE watchlist_id = ? ORDER BY sort_order, id",
    )
    .bind(watchlist_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn add_item(
    pool: &SqlitePool,
    watchlist_id: i64,
    symbol: &str,
    instrument_type: &str,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query(
        r#"
        INSERT INTO watchlist_items (watchlist_id, symbol, instrument_type, sort_order, added_at)
        VALUES (?, ?, ?, 0, ?)
        ON CONFLICT(watchlist_id, symbol) DO NOTHING
        "#,
    )
    .bind(watchlist_id)
    .bind(symbol)
    .bind(instrument_type)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

pub async fn remove_item(pool: &SqlitePool, item_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM watchlist_items WHERE id = ?")
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}
