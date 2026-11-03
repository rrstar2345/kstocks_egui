//! Left-hand watchlist panel.

use eframe::egui;

use super::options::display_symbol;
use super::QuoteView;
use crate::storage::watchlists::{Watchlist, WatchlistItem};

const UP: egui::Color32 = egui::Color32::from_rgb(38, 166, 154);
const DOWN: egui::Color32 = egui::Color32::from_rgb(239, 68, 68);

#[derive(Default)]
pub struct WatchlistState {
    pub lists: Vec<Watchlist>,
    pub selected: Option<i64>,
    pub items: Vec<WatchlistItem>,
    pub new_name: String,
    pub add_index: String,
}

pub enum WlAction {
    SelectList(i64),
    Create(String),
    Delete(i64),
    Add { symbol: String, kind: String },
    Remove(i64),
    Open(WatchlistItem),
}

pub fn show(
    ui: &mut egui::Ui,
    state: &mut WatchlistState,
    index_names: &[String],
    quote_of: &dyn Fn(&str, &str) -> Option<QuoteView>,
) -> Vec<WlAction> {
    let mut actions = Vec::new();

    ui.heading("Watchlist");

    if state.lists.is_empty() {
        ui.label("Loading…");
    } else {
        ui.horizontal(|ui| {
            let current = state
                .lists
                .iter()
                .find(|l| Some(l.id) == state.selected)
                .map(|l| l.name.clone())
                .unwrap_or_default();
            egui::ComboBox::from_id_salt("wl_select").selected_text(current).show_ui(ui, |ui| {
                for l in &state.lists {
                    if ui.selectable_label(Some(l.id) == state.selected, &l.name).clicked() {
                        actions.push(WlAction::SelectList(l.id));
                    }
                }
            });
            if let Some(id) = state.selected {
                if ui.small_button("🗑").on_hover_text("Delete this watchlist").clicked() {
                    actions.push(WlAction::Delete(id));
                }
            }
        });
    }

    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut state.new_name).hint_text("New list").desired_width(110.0));
        if ui.button("Create").clicked() && !state.new_name.trim().is_empty() {
            actions.push(WlAction::Create(state.new_name.trim().to_string()));
            state.new_name.clear();
        }
    });

    ui.separator();

    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("wl_add_index")
            .selected_text(if state.add_index.is_empty() { "Add index…" } else { state.add_index.as_str() })
            .show_ui(ui, |ui| {
                for n in index_names {
                    ui.selectable_value(&mut state.add_index, n.clone(), n);
                }
            });
        if ui.button("Add").clicked() && !state.add_index.is_empty() {
            actions.push(WlAction::Add { symbol: state.add_index.clone(), kind: "index".into() });
        }
    });
    ui.small("Options: click an LTP in the Options tab, then “Add to watchlist” in Paper Trading.");

    ui.separator();

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if state.items.is_empty() {
            ui.label("Empty.");
        }
        for item in &state.items {
            ui.horizontal(|ui| {
                if ui.small_button("✕").on_hover_text("Remove").clicked() {
                    actions.push(WlAction::Remove(item.id));
                }
                let name = display_symbol(&item.symbol);
                if ui.add(egui::Button::new(name).frame(false)).clicked() {
                    actions.push(WlAction::Open(item.clone()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match quote_of(&item.instrument_type, &item.symbol) {
                        Some(q) => {
                            if let Some(c) = q.change {
                                let color = if c >= 0.0 { UP } else { DOWN };
                                let suffix = if q.pct { "%" } else { "" };
                                ui.colored_label(color, format!("{c:+.2}{suffix}"));
                            }
                            ui.label(format!("{:.2}", q.price));
                        }
                        None => {
                            ui.weak("—");
                        }
                    }
                });
            });
        }
    });

    actions
}