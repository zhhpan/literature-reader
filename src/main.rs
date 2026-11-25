#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod auth_popup;
mod citations;
mod digest;
mod hyphenation;
mod math;
mod math_layout;
mod model_settings;
mod pdf;
mod reader;
mod reading_position;
mod recent_documents;
mod runtime;
mod translation;
mod translation_cache;
mod ui;
mod update;

fn main() {
    ui::run();
}
