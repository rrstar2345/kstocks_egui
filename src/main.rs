// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use kstocks::gui_runner;

/// Entry point for standalone egui desktop app
fn main() {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Run the hybrid backend + egui app
    gui_runner::run();
}