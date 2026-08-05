// ShellX Cut — Tauri desktop binary entry.
// Hide the extra console window on Windows release builds. On Linux/macOS this
// attribute is a no-op.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // All logic lives in lib.rs::run() so the same entry serves desktop +
    // future mobile targets (Tauri 2 recommended layout).
    shellx_cut_lib::run()
}
