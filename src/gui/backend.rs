//! Bridge between the egui thread and the async backend: DB access from the UI
//! without blocking a frame. Each call spawns onto the backend's Tokio runtime
//! and the result comes back as a `DbMsg` the UI drains every frame.

use std::future::Future;
use std::sync::mpsc::{channel, Receiver, Sender};

use sqlx::SqlitePool;

use crate::storage::paper_trades::{self, PaperTrade};
use crate::storage::watchlists::{self, Watchlist, WatchlistItem};

/// Sent once from the backend thread when the DB pool is ready.
#[derive(Clone)]
pub struct Backend {
    pub pool: SqlitePool,
    pub rt: tokio::runtime::Handle,
}

pub enum DbMsg {
    Watchlists(Vec<Watchlist>),
    Items { watchlist_id: i64, items: Vec<WatchlistItem> },
    Trades(Vec<PaperTrade>),
    Error(String),
}

pub struct Db {
    backend: Option<Backend>,
    tx: Sender<DbMsg>,
    pub rx: Receiver<DbMsg>,
}

impl Db {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self { backend: None, tx, rx }
    }

    pub fn is_ready(&self) -> bool {
        self.backend.is_some()
    }

    pub fn set_backend(&mut self, backend: Backend) {
        self.backend = Some(backend);
    }

    fn run<F, Fut>(&self, f: F)
    where
        F: FnOnce(SqlitePool) -> Fut,
        Fut: Future<Output = anyhow::Result<Vec<DbMsg>>> + Send + 'static,
    {
        let Some(b) = &self.backend else { return };
        let fut = f(b.pool.clone());
        let tx = self.tx.clone();
        b.rt.spawn(async move {
            match fut.await {
                Ok(msgs) => {
                    for m in msgs {
                        let _ = tx.send(m);
                    }
                }
                Err(e) => {
                    let _ = tx.send(DbMsg::Error(e.to_string()));
                }
            }
        });
    }

    // ---- watchlists -------------------------------------------------------

    /// Lists watchlists; seeds a "Default" one with two indices on first run.
    pub fn refresh_watchlists(&self) {
        self.run(|pool| async move {
            let mut lists = watchlists::list_watchlists(&pool).await?;
            if lists.is_empty() {
                let id = watchlists::create_watchlist(&pool, "Default").await?;
                for s in ["NIFTY 50", "NIFTY BANK"] {
                    watchlists::add_item(&pool, id, s, "index").await?;
                }
                lists = watchlists::list_watchlists(&pool).await?;
            }
            Ok(vec![DbMsg::Watchlists(lists)])
        });
    }

    pub fn refresh_items(&self, watchlist_id: i64) {
        self.run(move |pool| async move {
            let items = watchlists::list_items(&pool, watchlist_id).await?;
            Ok(vec![DbMsg::Items { watchlist_id, items }])
        });
    }

    pub fn create_watchlist(&self, name: String) {
        self.run(move |pool| async move {
            watchlists::create_watchlist(&pool, &name).await?;
            Ok(vec![DbMsg::Watchlists(watchlists::list_watchlists(&pool).await?)])
        });
    }

    pub fn delete_watchlist(&self, id: i64) {
        self.run(move |pool| async move {
            watchlists::delete_watchlist(&pool, id).await?;
            Ok(vec![DbMsg::Watchlists(watchlists::list_watchlists(&pool).await?)])
        });
    }

    pub fn add_item(&self, watchlist_id: i64, symbol: String, instrument_type: String) {
        self.run(move |pool| async move {
            watchlists::add_item(&pool, watchlist_id, &symbol, &instrument_type).await?;
            let items = watchlists::list_items(&pool, watchlist_id).await?;
            Ok(vec![DbMsg::Items { watchlist_id, items }])
        });
    }

    pub fn remove_item(&self, watchlist_id: i64, item_id: i64) {
        self.run(move |pool| async move {
            watchlists::remove_item(&pool, item_id).await?;
            let items = watchlists::list_items(&pool, watchlist_id).await?;
            Ok(vec![DbMsg::Items { watchlist_id, items }])
        });
    }

    // ---- paper trades -----------------------------------------------------

    pub fn refresh_trades(&self) {
        self.run(|pool| async move { Ok(vec![DbMsg::Trades(paper_trades::list_trades(&pool, None).await?)]) });
    }

    pub fn open_trade(
        &self,
        symbol: String,
        instrument_type: String,
        side: &'static str,
        quantity: f64,
        price: f64,
        notes: Option<String>,
    ) {
        self.run(move |pool| async move {
            paper_trades::open_trade(
                &pool,
                &symbol,
                &instrument_type,
                side,
                quantity,
                price,
                notes.as_deref(),
            )
            .await?;
            Ok(vec![DbMsg::Trades(paper_trades::list_trades(&pool, None).await?)])
        });
    }

    pub fn close_trade(&self, id: i64, exit_price: f64) {
        self.run(move |pool| async move {
            paper_trades::close_trade(&pool, id, exit_price).await?;
            Ok(vec![DbMsg::Trades(paper_trades::list_trades(&pool, None).await?)])
        });
    }
}