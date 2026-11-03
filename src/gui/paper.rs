//! Paper trading tab: order ticket, open positions with live P&L, history.
//! Orders fill instantly at the latest live price (no slippage / fees modelled).

use eframe::egui;

use super::options::display_symbol;
use super::QuoteView;
use crate::storage::paper_trades::PaperTrade;

const UP: egui::Color32 = egui::Color32::from_rgb(38, 166, 154);
const DOWN: egui::Color32 = egui::Color32::from_rgb(239, 68, 68);

pub struct Ticket {
    /// "index" or "option"
    pub instrument_type: String,
    /// Index name, or an option contract key (see `options::Contract`).
    pub symbol: String,
    pub buy: bool,
    pub qty: f64,
    pub notes: String,
}

impl Default for Ticket {
    fn default() -> Self {
        Self {
            instrument_type: "index".into(),
            symbol: "NIFTY 50".into(),
            buy: true,
            qty: 1.0,
            notes: String::new(),
        }
    }
}

pub enum PaperAction {
    Open {
        symbol: String,
        kind: String,
        side: &'static str,
        qty: f64,
        price: f64,
        notes: Option<String>,
    },
    Close { id: i64, price: f64 },
    AddToWatchlist { symbol: String, kind: String },
}

fn pnl(side: &str, entry: f64, exit: f64, qty: f64) -> f64 {
    let sign = if side == "buy" { 1.0 } else { -1.0 };
    (exit - entry) * qty * sign
}

fn pnl_label(ui: &mut egui::Ui, v: f64) {
    ui.colored_label(if v >= 0.0 { UP } else { DOWN }, format!("{v:+.2}"));
}

fn short_time(t: &str) -> String {
    t.get(..19).unwrap_or(t).replace('T', " ")
}

pub fn show(
    ui: &mut egui::Ui,
    ticket: &mut Ticket,
    trades: &[PaperTrade],
    index_names: &[String],
    quote_of: &dyn Fn(&str, &str) -> Option<QuoteView>,
    db_ready: bool,
    can_watch: bool,
) -> Vec<PaperAction> {
    let mut actions = Vec::new();

    // ---- order ticket -----------------------------------------------------
    ui.heading("Order ticket");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ticket.instrument_type, "index".to_string(), "Index");
        ui.selectable_value(&mut ticket.instrument_type, "option".to_string(), "Option");
        ui.separator();

        if ticket.instrument_type == "index" {
            if is_option_key(&ticket.symbol) {
                ticket.symbol = index_names.first().cloned().unwrap_or_else(|| "NIFTY 50".into());
            }
            egui::ComboBox::from_id_salt("ticket_index")
                .selected_text(&ticket.symbol)
                .show_ui(ui, |ui| {
                    for n in index_names {
                        ui.selectable_value(&mut ticket.symbol, n.clone(), n);
                    }
                });
        } else if is_option_key(&ticket.symbol) {
            ui.label(display_symbol(&ticket.symbol));
        } else {
            ui.weak("Pick a contract by clicking an LTP in the Options tab");
        }
    });

    let live = quote_of(&ticket.instrument_type, &ticket.symbol);
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ticket.buy, true, "Buy");
        ui.selectable_value(&mut ticket.buy, false, "Sell");
        ui.label("Qty");
        ui.add(egui::DragValue::new(&mut ticket.qty).range(1.0..=1_000_000.0).speed(1.0));
        ui.label("Price");
        match &live {
            Some(q) => ui.strong(format!("{:.2}", q.price)),
            None => ui.weak("no live price"),
        };
    });
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut ticket.notes).hint_text("Notes (optional)").desired_width(260.0));
    });
    ui.horizontal(|ui| {
        let valid = db_ready
            && live.is_some()
            && ticket.qty > 0.0
            && (ticket.instrument_type == "index" || is_option_key(&ticket.symbol));
        let label = if ticket.buy { "Place BUY (paper)" } else { "Place SELL (paper)" };
        if ui.add_enabled(valid, egui::Button::new(label)).clicked() {
            if let Some(q) = &live {
                actions.push(PaperAction::Open {
                    symbol: ticket.symbol.clone(),
                    kind: ticket.instrument_type.clone(),
                    side: if ticket.buy { "buy" } else { "sell" },
                    qty: ticket.qty,
                    price: q.price,
                    notes: if ticket.notes.trim().is_empty() { None } else { Some(ticket.notes.trim().to_string()) },
                });
                ticket.notes.clear();
            }
        }
        let watch_ok = can_watch && (ticket.instrument_type == "index" || is_option_key(&ticket.symbol));
        if ui.add_enabled(watch_ok, egui::Button::new("Add to watchlist")).clicked() {
            actions.push(PaperAction::AddToWatchlist {
                symbol: ticket.symbol.clone(),
                kind: ticket.instrument_type.clone(),
            });
        }
    });

    ui.separator();

    // ---- positions --------------------------------------------------------
    let open: Vec<&PaperTrade> = trades.iter().filter(|t| t.status == "open").collect();
    let closed: Vec<&PaperTrade> = trades.iter().filter(|t| t.status == "closed").collect();

    let mut unrealized = 0.0;
    let mut unrealized_known = true;
    for t in &open {
        match quote_of(&t.instrument_type, &t.symbol) {
            Some(q) => unrealized += pnl(&t.side, t.entry_price, q.price, t.quantity),
            None => unrealized_known = false,
        }
    }
    let realized: f64 = closed
        .iter()
        .filter_map(|t| t.exit_price.map(|x| pnl(&t.side, t.entry_price, x, t.quantity)))
        .sum();

    ui.horizontal(|ui| {
        ui.strong("Open positions");
        ui.separator();
        ui.label("Unrealized:");
        pnl_label(ui, unrealized);
        if !unrealized_known {
            ui.weak("(some prices missing)");
        }
        ui.separator();
        ui.label("Realized:");
        pnl_label(ui, realized);
    });

    egui::ScrollArea::vertical().id_salt("paper_scroll").auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("open_grid").striped(true).show(ui, |ui| {
            for h in ["Opened", "Instrument", "Side", "Qty", "Entry", "Live", "P&L", ""] {
                ui.strong(h);
            }
            ui.end_row();
            for t in &open {
                let q = quote_of(&t.instrument_type, &t.symbol);
                ui.label(short_time(&t.opened_at));
                ui.label(display_symbol(&t.symbol));
                ui.label(t.side.to_uppercase());
                ui.label(format!("{}", t.quantity));
                ui.label(format!("{:.2}", t.entry_price));
                match &q {
                    Some(q) => {
                        ui.label(format!("{:.2}", q.price));
                        pnl_label(ui, pnl(&t.side, t.entry_price, q.price, t.quantity));
                    }
                    None => {
                        ui.weak("—");
                        ui.weak("—");
                    }
                }
                if ui.add_enabled(q.is_some() && db_ready, egui::Button::new("Close")).clicked() {
                    if let Some(q) = &q {
                        actions.push(PaperAction::Close { id: t.id, price: q.price });
                    }
                }
                ui.end_row();
            }
        });
        if open.is_empty() {
            ui.weak("No open positions.");
        }

        ui.add_space(10.0);
        ui.strong("History");
        egui::Grid::new("closed_grid").striped(true).show(ui, |ui| {
            for h in ["Closed", "Instrument", "Side", "Qty", "Entry", "Exit", "P&L"] {
                ui.strong(h);
            }
            ui.end_row();
            for t in &closed {
                let exit = t.exit_price.unwrap_or(t.entry_price);
                ui.label(short_time(t.closed_at.as_deref().unwrap_or("")));
                ui.label(display_symbol(&t.symbol));
                ui.label(t.side.to_uppercase());
                ui.label(format!("{}", t.quantity));
                ui.label(format!("{:.2}", t.entry_price));
                ui.label(format!("{exit:.2}"));
                pnl_label(ui, pnl(&t.side, t.entry_price, exit, t.quantity));
                ui.end_row();
            }
        });
        if closed.is_empty() {
            ui.weak("No closed trades.");
        }
    });

    actions
}

fn is_option_key(symbol: &str) -> bool {
    super::options::Contract::parse(symbol).is_some()
}