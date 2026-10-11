#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod host;
mod logging;
mod protocol;

fn main() {
    if let Err(error) = host::run() {
        logging::write(&format!("Host failed: {error}"));
        host::show_startup_error(&error.to_string());
        std::process::exit(1);
    }
}
