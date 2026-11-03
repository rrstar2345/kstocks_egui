//! egui-based GUI: candlestick chart, watchlist, live option chain and paper trading.
//! Pure egui implementation with no Tauri dependencies.

use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_plot::{BoxElem, BoxPlot, BoxSpread, HPlacement, Plot, PlotPoint};
use serde::{Deserialize, Serialize};
use tracing::info;

pub mod backend;
mod options;
mod paper;
mod watchlist;

use crate::storage::OptionTickRow;
use backend::{Backend, Db, DbMsg};
use options::{ChainStore, ChainView, Contract};
use paper::{PaperAction, Ticket};
use watchlist::{WatchlistState, WlAction};

/// Everything the backend streams to the UI.
pub enum GuiEvent {
    Index(IndexTickRow),
    Option(OptionTickRow),
}

/// Latest price + change for any instrument (used by watchlist and paper trading).
pub struct QuoteView {
    pub price: f64,
    pub change: Option<f64>,
    /// `true` if `change` is a percentage (indices), `false` if absolute (options).
    pub pct: bool,
}

struct Quote {
    price: f64,
    per_change: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Chart,
    Options,
    Paper,
}

fn price_lookup(
    quotes: &HashMap<String, Quote>,
    chain: &ChainStore,
    kind: &str,
    symbol: &str,
) -> Option<QuoteView> {
    if kind == "option" {
        let c = Contract::parse(symbol)?;
        chain.quote(&c).map(|(price, change)| QuoteView { price, change, pct: false })
    } else {
        quotes.get(symbol).map(|q| QuoteView { price: q.price, change: Some(q.per_change), pct: true })
    }
}

/// Matches `IndexTickRow` from storage::ticks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexTickRow {
    pub time: String,
    pub index_name: String,
    pub current_price: f64,
    pub change: f64,
    pub per_change: f64,
    pub previous_close: f64,
    pub open: f64,
    pub low: f64,
    pub high: f64,
    pub ind_status: String,
    pub mkt_status: String,
    pub dissemination_time: String,
}

/// One OHLC candle
#[derive(Clone, Copy, Debug)]
struct Candle {
    bucket: i64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

impl Candle {
    fn new(bucket: i64, price: f64) -> Self {
        Self {
            bucket,
            open: price,
            high: price,
            low: price,
            close: price,
        }
    }

    fn apply_tick(&mut self, price: f64) {
        self.high = self.high.max(price);
        self.low = self.low.min(price);
        self.close = price;
    }
}

/// Candle series with bucketing
struct CandleSeries {
    bars: Vec<Candle>,
    bucket_seconds: i64,
}

impl CandleSeries {
    fn new(bucket_seconds: i64) -> Self {
        Self {
            bars: Vec::new(),
            bucket_seconds,
        }
    }

    fn push_tick(&mut self, elapsed_secs: f64, price: f64) {
        let bucket = (elapsed_secs as i64) / self.bucket_seconds;
        match self.bars.last_mut() {
            Some(last) if last.bucket == bucket => {
                last.apply_tick(price);
            }
            _ => {
                self.bars.push(Candle::new(bucket, price));
            }
        }
    }
}

/// Main chart application
pub struct CandlestickApp {
    series: CandleSeries,
    tick_rx: Receiver<GuiEvent>,
    backend_rx: Receiver<Backend>,
    db: Db,
    tab: Tab,
    quotes: HashMap<String, Quote>,
    chain: ChainStore,
    chain_view: ChainView,
    watchlist: WatchlistState,
    ticket: Ticket,
    trades: Vec<crate::storage::paper_trades::PaperTrade>,
    status_error: Option<String>,
    frame_times: Vec<f32>,
    last_frame_at: Instant,
    tick_count: u64,
    follow: bool,
    visible_bars: usize,
    selected_index: String,
    connection_status: String,
    error_message: Option<String>,
}

impl CandlestickApp {
    pub fn new(tick_rx: Receiver<GuiEvent>, backend_rx: Receiver<Backend>) -> Self {
        let now = Instant::now();
        Self {
            series: CandleSeries::new(1),
            tick_rx,
            backend_rx,
            db: Db::new(),
            tab: Tab::Chart,
            quotes: HashMap::new(),
            chain: ChainStore::default(),
            chain_view: ChainView::default(),
            watchlist: WatchlistState::default(),
            ticket: Ticket::default(),
            trades: Vec::new(),
            status_error: None,
            frame_times: Vec::with_capacity(240),
            last_frame_at: now,
            tick_count: 0,
            follow: true,
            visible_bars: 60,
            selected_index: "NIFTY 50".to_string(),
            connection_status: "Connecting".to_string(),
            error_message: None,
        }
    }

    fn process_pending_ticks(&mut self) {
        while let Ok(ev) = self.tick_rx.try_recv() {
            match ev {
                GuiEvent::Index(tick) => {
                    // Skip heartbeats
                    if tick.index_name == "HEARTBEAT" {
                        continue;
                    }

                    self.quotes.insert(
                        tick.index_name.clone(),
                        Quote { price: tick.current_price, per_change: tick.per_change },
                    );
                    self.connection_status = "Connected".to_string();
                    self.error_message = None;

                    // Only the selected index feeds the candle series
                    if tick.index_name == self.selected_index {
                        let elapsed = self.tick_count as f64 * 0.02;
                        self.series.push_tick(elapsed, tick.current_price);
                        self.tick_count += 1;
                    }
                }
                GuiEvent::Option(row) => self.chain.apply(row),
            }
        }
    }

    fn poll_backend(&mut self) {
        if let Ok(backend) = self.backend_rx.try_recv() {
            self.db.set_backend(backend);
            self.db.refresh_watchlists();
            self.db.refresh_trades();
        }
    }

    fn process_db_msgs(&mut self) {
        while let Ok(msg) = self.db.rx.try_recv() {
            match msg {
                DbMsg::Watchlists(lists) => {
                    let still_valid = self.watchlist.selected.map_or(false, |id| lists.iter().any(|l| l.id == id));
                    if !still_valid {
                        self.watchlist.selected = lists.first().map(|l| l.id);
                        self.watchlist.items.clear();
                        if let Some(id) = self.watchlist.selected {
                            self.db.refresh_items(id);
                        }
                    }
                    self.watchlist.lists = lists;
                }
                DbMsg::Items { watchlist_id, items } => {
                    if Some(watchlist_id) == self.watchlist.selected {
                        self.watchlist.items = items;
                    }
                }
                DbMsg::Trades(t) => self.trades = t,
                DbMsg::Error(e) => self.status_error = Some(e),
            }
        }
    }

    fn select_index(&mut self, name: &str) {
        if self.selected_index != name {
            self.selected_index = name.to_string();
            self.series = CandleSeries::new(1);
            self.tick_count = 0;
            self.follow = true;
        }
    }

    fn index_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.quotes.keys().cloned().collect();
        if v.is_empty() {
            v.push(self.selected_index.clone());
        }
        v.sort();
        v
    }

    fn handle_watchlist_action(&mut self, a: WlAction) {
        match a {
            WlAction::SelectList(id) => {
                self.watchlist.selected = Some(id);
                self.watchlist.items.clear();
                self.db.refresh_items(id);
            }
            WlAction::Create(name) => self.db.create_watchlist(name),
            WlAction::Delete(id) => self.db.delete_watchlist(id),
            WlAction::Add { symbol, kind } => {
                if let Some(id) = self.watchlist.selected {
                    self.db.add_item(id, symbol, kind);
                }
            }
            WlAction::Remove(item_id) => {
                if let Some(id) = self.watchlist.selected {
                    self.db.remove_item(id, item_id);
                }
            }
            WlAction::Open(item) => {
                if item.instrument_type == "option" {
                    self.ticket.instrument_type = "option".into();
                    self.ticket.symbol = item.symbol;
                    self.tab = Tab::Paper;
                } else {
                    self.select_index(&item.symbol);
                    self.ticket.instrument_type = "index".into();
                    self.ticket.symbol = item.symbol;
                    self.tab = Tab::Chart;
                }
            }
        }
    }

    fn handle_paper_action(&mut self, a: PaperAction) {
        match a {
            PaperAction::Open { symbol, kind, side, qty, price, notes } => {
                self.db.open_trade(symbol, kind, side, qty, price, notes)
            }
            PaperAction::Close { id, price } => self.db.close_trade(id, price),
            PaperAction::AddToWatchlist { symbol, kind } => {
                if let Some(id) = self.watchlist.selected {
                    self.db.add_item(id, symbol, kind);
                }
            }
        }
    }
}

impl CandlestickApp {
    fn chart_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Index:");
            let names = self.index_names();
            let mut chosen = self.selected_index.clone();
            egui::ComboBox::from_id_salt("chart_index")
                .selected_text(&chosen)
                .show_ui(ui, |ui| {
                    for n in &names {
                        ui.selectable_value(&mut chosen, n.clone(), n);
                    }
                });
            if chosen != self.selected_index {
                self.select_index(&chosen);
            }
        });

        if let Some(ref err) = self.error_message {
            ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
        }

        ui.separator();

        let up_color = egui::Color32::from_rgb(38, 166, 154);
        let down_color = egui::Color32::from_rgb(239, 68, 68);

        let boxes: Vec<BoxElem> = self
            .series
            .bars
            .iter()
            .map(|c| {
                let is_up = c.close >= c.open;
                let (fill, stroke_color) = if is_up {
                    (up_color, up_color)
                } else {
                    (down_color, down_color)
                };
                let body_top = c.open.max(c.close);
                let body_bottom = c.open.min(c.close);
                BoxElem::new(
                    c.bucket as f64,
                    BoxSpread::new(c.low, body_bottom, c.close, body_top, c.high),
                )
                .name(format!("{:.2}|{:.2}|{is_up}", c.open, c.close))
                .box_width(0.7)
                .whisker_width(0.0)
                .fill(fill)
                .stroke(egui::Stroke::new(1.0, stroke_color))
            })
            .collect();

        let box_plot = BoxPlot::new("candles", boxes).vertical().element_formatter(Box::new(
            |elem: &egui_plot::BoxElem, _plot: &BoxPlot| {
                let mut parts = elem.name.splitn(3, '|');
                let open: f64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(elem.spread.quartile1);
                let close: f64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(elem.spread.median);
                format!(
                    "time: {}s\nO: {:.2}\nH: {:.2}\nL: {:.2}\nC: {:.2}",
                    elem.argument as i64,
                    open,
                    elem.spread.upper_whisker,
                    elem.spread.lower_whisker,
                    close,
                )
            },
        ));

        let last_close = self.series.bars.last().map(|c| c.close);
        let last_bucket = self.series.bars.last().map(|c| c.bucket as f64);
        let is_last_up = self
            .series
            .bars
            .last()
            .map(|c| c.close >= c.open)
            .unwrap_or(true);
        let last_color = if is_last_up { up_color } else { down_color };

        let bar_count = self.series.bars.len();
        let window = self.visible_bars.max(1);
        let (window_min_bucket, window_max_bucket) = if bar_count == 0 {
            (0.0, window as f64)
        } else {
            let last_bucket = self.series.bars[bar_count - 1].bucket as f64;
            let first_visible_bucket = self.series.bars[bar_count.saturating_sub(window)].bucket as f64;
            if bar_count <= window {
                let first_bucket = self.series.bars[0].bucket as f64;
                (first_bucket, first_bucket + window as f64)
            } else {
                (first_visible_bucket, last_bucket + 1.0)
            }
        };

        let visible_slice_start = bar_count.saturating_sub(window);
        let (y_min, y_max) = {
            let visible = &self.series.bars[visible_slice_start..];
            if visible.is_empty() {
                (0.0, 1.0)
            } else {
                let lo = visible.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
                let hi = visible.iter().map(|c| c.high).fold(f64::NEG_INFINITY, f64::max);
                let pad = ((hi - lo).max(0.01)) * 0.08;
                (lo - pad, hi + pad)
            }
        };

        let plot_response = Plot::new("candlestick_plot")
            .height(ui.available_height() - 8.0)
            .show_grid(true)
            .allow_scroll(true)
            .allow_zoom(true)
            .y_axis_position(HPlacement::Right)
            .x_axis_formatter(|mark, _range| format!("{}s", mark.value as i64))
            .y_axis_formatter(|mark, _range| format!("{:.2}", mark.value))
            .show(ui, |plot_ui| {
                if self.follow {
                    plot_ui.set_plot_bounds(egui_plot::PlotBounds::from_min_max(
                        [window_min_bucket, y_min],
                        [window_max_bucket, y_max],
                    ));
                }

                plot_ui.box_plot(box_plot);

                if let (Some(price), Some(bucket)) = (last_close, last_bucket) {
                    let x_max = plot_ui.plot_bounds().max()[0];
                    let line = egui_plot::Line::new("last_price", vec![[bucket, price], [x_max, price]])
                        .color(last_color)
                        .style(egui_plot::LineStyle::Dotted { spacing: 4.0 })
                        .width(1.0);
                    plot_ui.line(line);
                }
            });

        let resp = &plot_response.response;
        if resp.dragged() || resp.hovered() && ui.input(|i| i.smooth_scroll_delta != egui::Vec2::ZERO) {
            self.follow = false;
        }

        if let Some(price) = last_close {
            let transform = &plot_response.transform;
            let plot_rect = *transform.frame();
            let y = transform.position_from_point(&PlotPoint::new(0.0, price)).y;

            let label = format!("{price:.2}");
            let painter = ui.painter_at(plot_rect.expand2(egui::vec2(60.0, 0.0)));
            let font = egui::FontId::proportional(12.0);
            let galley = painter.layout_no_wrap(label.clone(), font.clone(), egui::Color32::WHITE);
            let padding = egui::vec2(6.0, 3.0);
            let tag_size = galley.size() + padding * 2.0;
            let tag_min = egui::pos2(plot_rect.right(), y - tag_size.y / 2.0);
            let tag_rect = egui::Rect::from_min_size(tag_min, tag_size);

            painter.rect_filled(tag_rect, egui::CornerRadius::same(2), last_color);
            painter.text(
                tag_rect.left_center() + egui::vec2(padding.x, 0.0),
                egui::Align2::LEFT_CENTER,
                label,
                font,
                egui::Color32::WHITE,
            );
        }

    }
}

impl eframe::App for CandlestickApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        self.poll_backend();
        self.process_pending_ticks();
        self.process_db_msgs();

        // Frame timing
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame_at).as_secs_f32();
        self.last_frame_at = now;
        if dt > 0.0 {
            self.frame_times.push(dt);
            if self.frame_times.len() > 120 {
                self.frame_times.remove(0);
            }
        }
        let avg_dt = if self.frame_times.is_empty() {
            0.0
        } else {
            self.frame_times.iter().sum::<f32>() / self.frame_times.len() as f32
        };
        let fps = if avg_dt > 0.0 { 1.0 / avg_dt } else { 0.0 };

        // ---- top bar ------------------------------------------------------
        egui::Panel::top("top_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("kstocks");
                ui.separator();
                ui.selectable_value(&mut self.tab, Tab::Chart, "Chart");
                ui.selectable_value(&mut self.tab, Tab::Options, "Options");
                ui.selectable_value(&mut self.tab, Tab::Paper, "Paper Trading");
            });
            ui.horizontal(|ui| {
                let status_color = match self.connection_status.as_str() {
                    "Connected" => egui::Color32::GREEN,
                    "Connecting" => egui::Color32::YELLOW,
                    "Disconnected" => egui::Color32::RED,
                    _ => egui::Color32::GRAY,
                };
                ui.colored_label(status_color, format!("● {}", self.connection_status));
                ui.separator();
                ui.label(format!(
                    "bars: {} | ticks: {} | opt ticks: {} | fps: {:.1} | frame: {:.2}ms",
                    self.series.bars.len(),
                    self.tick_count,
                    self.chain.tick_count,
                    fps,
                    avg_dt * 1000.0
                ));
                if let Some(err) = self.status_error.clone() {
                    ui.separator();
                    ui.colored_label(egui::Color32::RED, format!("DB: {err}"));
                    if ui.small_button("dismiss").clicked() {
                        self.status_error = None;
                    }
                }
            });
        });

        // ---- watchlist side panel -----------------------------------------
        let index_names = self.index_names();
        let mut wl_actions: Vec<WlAction> = Vec::new();
        egui::Panel::left("watchlist_panel")
            .default_size(270.0)
            .show(ui, |ui| {
                wl_actions = watchlist::show(ui, &mut self.watchlist, &index_names, &|kind, sym| {
                    price_lookup(&self.quotes, &self.chain, kind, sym)
                });
            });

        // ---- central area -------------------------------------------------
        let mut chain_pick: Option<Contract> = None;
        let mut paper_actions: Vec<PaperAction> = Vec::new();
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Chart => self.chart_ui(ui),
            Tab::Options => {
                let quotes = &self.quotes;
                chain_pick = options::show(ui, &mut self.chain_view, &self.chain, &|sym| {
                    options::spot_index_name(sym).and_then(|n| quotes.get(n)).map(|q| q.price)
                });
            }
            Tab::Paper => {
                paper_actions = paper::show(
                    ui,
                    &mut self.ticket,
                    &self.trades,
                    &index_names,
                    &|kind, sym| price_lookup(&self.quotes, &self.chain, kind, sym),
                    self.db.is_ready(),
                    self.watchlist.selected.is_some(),
                );
            }
        });

        // ---- apply deferred actions ---------------------------------------
        if let Some(c) = chain_pick {
            self.ticket.instrument_type = "option".into();
            self.ticket.symbol = c.key();
            self.tab = Tab::Paper;
        }
        for a in wl_actions {
            self.handle_watchlist_action(a);
        }
        for a in paper_actions {
            self.handle_paper_action(a);
        }

        ctx.request_repaint_after(Duration::from_millis(33));
    }
}

/// Create an app instance with a tick receiver
pub fn create_app(tick_rx: Receiver<GuiEvent>, backend_rx: Receiver<Backend>) -> CandlestickApp {
    CandlestickApp::new(tick_rx, backend_rx)
}

/// Run the egui application (called from gui_runner on the main thread)
pub fn run_app(app: CandlestickApp) {
    info!("Starting egui GUI application");

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 800.0])
            .with_title("kstocks - Stock Trading Terminal"),
        ..Default::default()
    };

    let _ = eframe::run_native(
        "kstocks",
        native_options,
        Box::new(move |_cc| Ok(Box::new(app))),
    );
}