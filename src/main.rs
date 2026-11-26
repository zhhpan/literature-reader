#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod auth_popup;
mod citations;
mod digest;
mod document_credits;
mod hyphenation;
mod layout;
mod math;
#[cfg(windows)]
mod math_layout;
mod model_settings;
mod pdf;
mod reader;
mod reading_position;
mod recent_documents;
mod runtime;
mod tern;
mod translation;
mod translation_cache;
#[cfg(windows)]
mod ui;
#[cfg(windows)]
mod update;
#[cfg(target_os = "linux")]
mod ui_wayland;
#[cfg(target_os = "linux")]
mod wayland_update;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--layout-probe") {
        std::process::exit(layout::probe(&args[2..]));
    }
    if args.get(1).map(String::as_str) == Some("--tern") {
        let status = tern::run();
        #[cfg(target_os = "linux")]
        translation::shutdown_bridges();
        std::process::exit(status);
    }
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
