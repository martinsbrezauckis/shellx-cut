//! Start check for Windows N/KN editions without the Media Feature Pack.
//!
//! The engine (`cutd`) links the Media Foundation DLLs directly, so the Windows
//! loader refuses to start it when they are absent and shows only a system
//! error about a missing DLL. The shell does not import them, so it can start,
//! notice first, and give the person (and, through `engine_status`, an agent)
//! one plain reason with the way forward.

/// System DLLs the engine's import table names (see the Windows import gate in
/// `scripts/lib/windows-pe-imports.mjs`).
const MEDIA_FOUNDATION_DLLS: [&str; 3] = ["mfplat.dll", "mf.dll", "mfreadwrite.dll"];

/// Names from [`MEDIA_FOUNDATION_DLLS`] that `is_present` does not find.
fn missing_media_foundation(is_present: impl Fn(&str) -> bool) -> Vec<&'static str> {
    MEDIA_FOUNDATION_DLLS
        .into_iter()
        .filter(|name| !is_present(name))
        .collect()
}

fn media_feature_pack_reason(missing: &[&str]) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "engine unavailable: this Windows edition has no Media Foundation ({} not found), which the ShellX Cut engine needs to start. \
         Windows N and KN editions need the Media Feature Pack: open Settings > Optional features \
         (Windows 11: System > Optional features > View features; Windows 10: Apps > Optional features > Add a feature), \
         install \"Media Feature Pack\", then restart ShellX Cut.",
        missing.join(", ")
    ))
}

/// Whether `name` is in the Windows system folder, where the loader finds it.
/// `None` when that folder cannot be located, so an unusual setup is never
/// blocked on a guess.
#[cfg(windows)]
fn present_in_system_folder(name: &str) -> Option<bool> {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let folder = std::path::Path::new(&root).join("System32");
    folder.is_dir().then(|| folder.join(name).is_file())
}

#[cfg(not(windows))]
fn present_in_system_folder(_name: &str) -> Option<bool> {
    None
}

/// The reason the engine cannot start on this machine, or `None` when it can
/// (or when this cannot be determined).
pub(crate) fn engine_start_blocker() -> Option<String> {
    media_feature_pack_reason(&missing_media_foundation(|name| {
        present_in_system_folder(name).unwrap_or(true)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_dll_blocks_the_start_with_a_plain_reason_and_the_way_forward() {
        let missing = missing_media_foundation(|name| name != "mfplat.dll");
        assert_eq!(missing, ["mfplat.dll"]);
        let reason = media_feature_pack_reason(&missing).expect("a missing DLL has a reason");
        assert!(reason.starts_with("engine unavailable:"), "{reason}");
        assert!(reason.contains("mfplat.dll"), "{reason}");
        assert!(reason.contains("Media Feature Pack"), "{reason}");
        assert!(reason.contains("Settings > Optional features"), "{reason}");
    }

    #[test]
    fn every_missing_dll_is_named() {
        let missing = missing_media_foundation(|_| false);
        assert_eq!(missing, MEDIA_FOUNDATION_DLLS);
        let reason = media_feature_pack_reason(&missing).unwrap();
        for name in MEDIA_FOUNDATION_DLLS {
            assert!(reason.contains(name), "{reason}");
        }
    }

    #[test]
    fn nothing_missing_means_no_blocker() {
        assert!(missing_media_foundation(|_| true).is_empty());
        assert_eq!(media_feature_pack_reason(&[]), None);
    }

    #[cfg(windows)]
    #[test]
    fn this_windows_machine_reports_its_real_system_folder() {
        // A machine running this test has Media Foundation unless it is an
        // N edition; the check must at least locate the system folder.
        assert!(present_in_system_folder("kernel32.dll").unwrap());
        assert_eq!(present_in_system_folder("no-such-library.dll"), Some(false));
    }
}
