//! Queries against `paper_trades` — simulated orders, priced off live
//! market data but executed independently of any live broker.

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PaperTrade {
    pub id: i64,
    pub symbol: String,
    pub instrument_type: String,
    pub side: String,
    pub quantity: f64,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub status: String,
    pub opened_at: String,
    pub closed_at: Option<String>,
    pub notes: Option<String>,
}

pub async fn open_trade(
    pool: &SqlitePool,
    symbol: &str,
    instrument_type: &str,
    side: &str,
    quantity: f64,
    entry_price: f64,
    notes: Option<&str>,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query(
        r#"
        INSERT INTO paper_trades
            (symbol, instrument_type, side, quantity, entry_price, status, opened_at, notes)
        VALUES (?, ?, ?, ?, ?, 'open', ?, ?)
        "#,
    )
    .bind(symbol)
    .bind(instrument_type)
    .bind(side)
    .bind(quantity)
    .bind(entry_price)
    .bind(&now)
    .bind(notes)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

pub async fn close_trade(pool: &SqlitePool, id: i64, exit_price: f64) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE paper_trades SET exit_price = ?, status = 'closed', closed_at = ? WHERE id = ?",
    )
    .bind(exit_price)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_trades(pool: &SqlitePool, status: Option<&str>) -> Result<Vec<PaperTrade>> {
    let rows = match status {
        Some(s) => {
            sqlx::query_as::<_, PaperTrade>(
                "SELECT * FROM paper_trades WHERE status = ? ORDER BY opened_at DESC",
            )
            .bind(s)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, PaperTrade>("SELECT * FROM paper_trades ORDER BY opened_at DESC")
                .fetch_all(pool)
                .await?
        }
    };
    Ok(rows)
}
