//! Live option chain: tick store, contract keys, and the chain table UI.

use std::collections::{BTreeMap, HashMap};

use eframe::egui;

use crate::storage::OptionTickRow;

const UP: egui::Color32 = egui::Color32::from_rgb(38, 166, 154);
const DOWN: egui::Color32 = egui::Color32::from_rgb(239, 68, 68);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ce,
    Pe,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Ce => "CE",
            Kind::Pe => "PE",
        }
    }
}

/// One option contract. Stored in watchlists / paper trades as a single
/// string key (`SYMBOL|EXPIRY|STRIKE|CE`) so it fits the existing `symbol` column.
#[derive(Clone, Debug, PartialEq)]
pub struct Contract {
    pub symbol: String,
    pub expiry: String,
    pub strike: f64,
    pub kind: Kind,
}

impl Contract {
    pub fn key(&self) -> String {
        format!("{}|{}|{}|{}", self.symbol, self.expiry, self.strike, self.kind.as_str())
    }

    pub fn parse(key: &str) -> Option<Self> {
        let mut p = key.splitn(4, '|');
        let symbol = p.next()?.to_string();
        let expiry = p.next()?.to_string();
        let strike: f64 = p.next()?.parse().ok()?;
        let kind = match p.next()? {
            "CE" => Kind::Ce,
            "PE" => Kind::Pe,
            _ => return None,
        };
        Some(Self { symbol, expiry, strike, kind })
    }

    pub fn label(&self) -> String {
        format!("{} {} {} ({})", self.symbol, self.strike, self.kind.as_str(), self.expiry)
    }
}

/// Human-readable form of a stored symbol (option keys are expanded).
pub fn display_symbol(symbol: &str) -> String {
    Contract::parse(symbol).map(|c| c.label()).unwrap_or_else(|| symbol.to_string())
}

/// F&O underlying symbol -> NSE index name used by the index stream (for spot / ATM).
pub fn spot_index_name(symbol: &str) -> Option<&'static str> {
    match symbol {
        "NIFTY" => Some("NIFTY 50"),
        "BANKNIFTY" => Some("NIFTY BANK"),
        "FINNIFTY" => Some("NIFTY FINANCIAL SERVICES"),
        "MIDCPNIFTY" => Some("NIFTY MIDCAP SELECT"),
        "NIFTYNXT50" => Some("NIFTY NEXT 50"),
        _ => None,
    }
}

fn strike_key(strike: f64) -> i64 {
    (strike * 100.0).round() as i64
}

/// Latest tick per (symbol, expiry, strike).
#[derive(Default)]
pub struct ChainStore {
    chains: HashMap<(String, String), BTreeMap<i64, OptionTickRow>>,
    pub tick_count: u64,
}

impl ChainStore {
    pub fn apply(&mut self, mut row: OptionTickRow) {
        self.tick_count += 1;
        let chain = self.chains.entry((row.symbol.clone(), row.expiry.clone())).or_default();
        let k = strike_key(row.strike_price);
        // Ticks can omit fields; keep the previous value rather than blanking the cell.
        if let Some(old) = chain.get(&k) {
            row.ce_last_price = row.ce_last_price.or(old.ce_last_price);
            row.ce_change = row.ce_change.or(old.ce_change);
            row.ce_volume = row.ce_volume.or(old.ce_volume);
            row.ce_oi = row.ce_oi.or(old.ce_oi);
            row.ce_bid = row.ce_bid.or(old.ce_bid);
            row.ce_ask = row.ce_ask.or(old.ce_ask);
            row.pe_last_price = row.pe_last_price.or(old.pe_last_price);
            row.pe_change = row.pe_change.or(old.pe_change);
            row.pe_volume = row.pe_volume.or(old.pe_volume);
            row.pe_oi = row.pe_oi.or(old.pe_oi);
            row.pe_bid = row.pe_bid.or(old.pe_bid);
            row.pe_ask = row.pe_ask.or(old.pe_ask);
        }
        chain.insert(k, row);
    }

    pub fn symbols(&self) -> Vec<String> {
        let mut v: Vec<String> = self.chains.keys().map(|(s, _)| s.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    pub fn expiries(&self, symbol: &str) -> Vec<String> {
        let mut v: Vec<String> = self
            .chains
            .keys()
            .filter(|(s, _)| s == symbol)
            .map(|(_, e)| e.clone())
            .collect();
        v.sort();
        v
    }

    /// (last price, change) for a contract.
    pub fn quote(&self, c: &Contract) -> Option<(f64, Option<f64>)> {
        let row = self
            .chains
            .get(&(c.symbol.clone(), c.expiry.clone()))?
            .get(&strike_key(c.strike))?;
        match c.kind {
            Kind::Ce => row.ce_last_price.map(|p| (p, row.ce_change)),
            Kind::Pe => row.pe_last_price.map(|p| (p, row.pe_change)),
        }
    }
}

pub struct ChainView {
    pub symbol: Option<String>,
    pub expiry: Option<String>,
    /// Strikes shown on each side of ATM.
    pub window: usize,
}

impl Default for ChainView {
    fn default() -> Self {
        Self { symbol: None, expiry: None, window: 10 }
    }
}

fn opt(v: Option<f64>, dp: usize) -> String {
    v.map(|x| format!("{x:.dp$}")).unwrap_or_else(|| "-".into())
}

fn chg(ui: &mut egui::Ui, v: Option<f64>) {
    match v {
        Some(x) => {
            let color = if x >= 0.0 { UP } else { DOWN };
            ui.colored_label(color, format!("{x:+.2}"));
        }
        None => {
            ui.label("-");
        }
    }
}

/// Returns the contract whose LTP the user clicked, if any.
pub fn show(
    ui: &mut egui::Ui,
    view: &mut ChainView,
    store: &ChainStore,
    spot_of: &dyn Fn(&str) -> Option<f64>,
) -> Option<Contract> {
    let symbols = store.symbols();
    if symbols.is_empty() {
        ui.add_space(12.0);
        ui.label("Waiting for option ticks… (nothing is streamed while the market is closed)");
        return None;
    }

    if view.symbol.as_ref().map_or(true, |s| !symbols.contains(s)) {
        view.symbol = symbols.first().cloned();
        view.expiry = None;
    }
    let symbol = view.symbol.clone().unwrap();
    let expiries = store.expiries(&symbol);
    if view.expiry.as_ref().map_or(true, |e| !expiries.contains(e)) {
        view.expiry = expiries.first().cloned();
    }
    let Some(expiry) = view.expiry.clone() else { return None };

    let spot = spot_of(&symbol);

    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("chain_symbol")
            .selected_text(&symbol)
            .show_ui(ui, |ui| {
                for s in &symbols {
                    ui.selectable_value(&mut view.symbol, Some(s.clone()), s);
                }
            });
        egui::ComboBox::from_id_salt("chain_expiry")
            .selected_text(&expiry)
            .show_ui(ui, |ui| {
                for e in &expiries {
                    ui.selectable_value(&mut view.expiry, Some(e.clone()), e);
                }
            });
        ui.label("±strikes:");
        ui.add(egui::DragValue::new(&mut view.window).range(3..=60));
        ui.separator();
        match spot {
            Some(s) => ui.label(format!("Spot: {s:.2}")),
            None => ui.label("Spot: -"),
        };
        ui.label(format!("ticks: {}", store.tick_count));
    });
    ui.separator();

    let Some(chain) = store.chains.get(&(symbol.clone(), expiry.clone())) else { return None };
    let rows: Vec<&OptionTickRow> = chain.values().collect();
    if rows.is_empty() {
        return None;
    }

    let atm_idx = match spot {
        Some(s) => rows
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1.strike_price - s)
                    .abs()
                    .partial_cmp(&(b.1.strike_price - s).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i)
            .unwrap_or(0),
        None => rows.len() / 2,
    };
    let lo = atm_idx.saturating_sub(view.window);
    let hi = (atm_idx + view.window + 1).min(rows.len());

    let mut picked = None;

    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("chain_grid").striped(true).num_columns(9).show(ui, |ui| {
            for h in ["CE OI", "CE Vol", "CE Chg", "CE LTP", "Strike", "PE LTP", "PE Chg", "PE Vol", "PE OI"] {
                ui.strong(h);
            }
            ui.end_row();

            for (i, r) in rows[lo..hi].iter().enumerate() {
                let is_atm = lo + i == atm_idx && spot.is_some();

                ui.label(opt(r.ce_oi, 0));
                ui.label(r.ce_volume.map(|v| v.to_string()).unwrap_or_else(|| "-".into()));
                chg(ui, r.ce_change);
                if ui
                    .add(egui::Button::new(opt(r.ce_last_price, 2)).frame(false))
                    .on_hover_text(format!("bid {} / ask {} — click to trade", opt(r.ce_bid, 2), opt(r.ce_ask, 2)))
                    .clicked()
                {
                    picked = Some(Contract {
                        symbol: symbol.clone(),
                        expiry: expiry.clone(),
                        strike: r.strike_price,
                        kind: Kind::Ce,
                    });
                }

                let strike_txt = egui::RichText::new(format!("{}", r.strike_price)).strong();
                ui.label(if is_atm { strike_txt.background_color(egui::Color32::from_gray(70)) } else { strike_txt });

                if ui
                    .add(egui::Button::new(opt(r.pe_last_price, 2)).frame(false))
                    .on_hover_text(format!("bid {} / ask {} — click to trade", opt(r.pe_bid, 2), opt(r.pe_ask, 2)))
                    .clicked()
                {
                    picked = Some(Contract {
                        symbol: symbol.clone(),
                        expiry: expiry.clone(),
                        strike: r.strike_price,
                        kind: Kind::Pe,
                    });
                }
                chg(ui, r.pe_change);
                ui.label(r.pe_volume.map(|v| v.to_string()).unwrap_or_else(|| "-".into()));
                ui.label(opt(r.pe_oi, 0));
                ui.end_row();
            }
        });
    });

    picked
}