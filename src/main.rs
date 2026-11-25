#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod auth_popup;
mod citations;
mod digest;
mod document_credits;
mod hyphenation;
mod math;
#[cfg(windows)]
mod math_layout;
mod model_settings;
mod pdf;
mod reader;
mod reading_position;
mod recent_documents;
mod runtime;
mod translation;
mod translation_cache;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod update;
#[cfg(target_os = "linux")]
mod ui_wayland;

fn main() {
    #[cfg(windows)]
    ui::run();
    #[cfg(target_os = "linux")]
    {
        let status = ui_wayland::run();
        translation::shutdown_bridges();
        if status != 0 {
            std::process::exit(status);
        }
    }
}
