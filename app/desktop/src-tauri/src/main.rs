// ShellX Cut — Tauri desktop binary entry.
// Hide the extra console window on Windows release builds. On Linux/macOS this
// attribute is a no-op.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    harden_process_search_path();
    // All logic lives in lib.rs::run() so the same entry serves desktop +
    // future mobile targets (Tauri 2 recommended layout).
    shellx_cut_lib::run()
}

/// Desktop twin of `cut_media::toolpath::harden_process_search_path` in the
/// separate engine workspace. Call before Tauri starts any thread or child.
fn harden_process_search_path() {
    let path = std::env::var_os("PATH");
    if let Some(original) = path.as_deref() {
        match absolute_path_value(Some(original)) {
            Some(clean) if clean.as_os_str() != original => std::env::set_var("PATH", clean),
            Some(_) => {}
            None => std::env::remove_var("PATH"),
        }
    }
    if cfg!(windows) {
        std::env::set_var("NoDefaultCurrentDirectoryInExePath", "1");
    }
}

fn absolute_path_value(path: Option<&std::ffi::OsStr>) -> Option<std::ffi::OsString> {
    let entries: Vec<_> = std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .collect();
    (!entries.is_empty())
        .then(|| std::env::join_paths(entries).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_path_filter_keeps_only_absolute_entries_in_order() {
        let absolute = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let input =
            std::env::join_paths([".".into(), absolute.clone(), "relative".into()]).unwrap();
        let expected = std::env::join_paths([absolute]).unwrap();
        assert_eq!(absolute_path_value(Some(&input)), Some(expected));
        assert_eq!(absolute_path_value(Some(".".as_ref())), None);
        assert_eq!(absolute_path_value(None), None);
    }
}
