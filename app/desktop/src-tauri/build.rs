// ShellX Cut — Tauri build script.
// Runs tauri-build's codegen (parses tauri.conf.json, generates the context,
// capability schemas under gen/, and platform metadata). All configuration
// lives in tauri.conf.json.
fn main() {
    tauri_build::build();
}
