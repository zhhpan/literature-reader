#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod citations;
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

fn main() {
    ui::run();
}
