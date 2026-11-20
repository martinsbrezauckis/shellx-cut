//! One test-process-wide isolation of every per-user location.
//!
//! A unit test must never read or write the person's real ShellX Cut: the state
//! folder with `engine.addr`, the recent-projects list and asset library, the
//! downloaded tools folder (FFmpeg, Python sidecar) and the provider CLIs'
//! install dirs. They all derive from `SHELLX_CUT_HOME`, `HOME`, `USERPROFILE`,
//! `LOCALAPPDATA`, `APPDATA` and `XDG_DATA_HOME`, so the test binary points them
//! at one temp folder before `main` runs (see [`isolate_test_process!`]).
//! Child processes a test starts inherit it. PATH and every variable the person
//! or the check script set for a test (`SHELLX_CUT_FFMPEG*`, `RUSTUP_HOME`, ...)
//! are left alone.

use std::path::PathBuf;

/// Marker the isolation sets; a process that has it is already isolated.
pub const MARKER: &str = "SHELLX_CUT_TEST_ISOLATED";

/// Point every per-user location at a per-process temp folder. Idempotent.
/// Call it only before other threads run (the registered hook does).
pub fn install_for_this_process() {
    if std::env::var_os(MARKER).is_some() {
        return;
    }
    // rustup and cargo find their toolchains through HOME: pin them to the real
    // location first so tests that start `rustc` still work.
    let real_home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    if let Some(real_home) = real_home {
        let real_home = PathBuf::from(real_home);
        for (key, leaf) in [("RUSTUP_HOME", ".rustup"), ("CARGO_HOME", ".cargo")] {
            if std::env::var_os(key).is_none() {
                std::env::set_var(key, real_home.join(leaf));
            }
        }
    }
    let root = std::env::temp_dir().join(format!("cut-test-process-{}", std::process::id()));
    let user = root.join("user");
    let state = root.join("state");
    for dir in [&user, &state] {
        let _ = std::fs::create_dir_all(dir);
    }
    std::env::set_var("SHELLX_CUT_HOME", &state);
    std::env::set_var("HOME", &user);
    std::env::set_var("USERPROFILE", &user);
    std::env::set_var("LOCALAPPDATA", user.join("AppData").join("Local"));
    std::env::set_var("APPDATA", user.join("AppData").join("Roaming"));
    std::env::set_var("XDG_DATA_HOME", user.join(".local").join("share"));
    std::env::set_var("SHELLX_CUT_AGENT_SEARCH", "path");
    std::env::set_var(MARKER, "1");
}

/// The folders a test process must never touch, for a guard test: true when
/// `path` lies under the per-process isolation root.
pub fn is_isolated_path(path: &std::path::Path) -> bool {
    path.starts_with(std::env::temp_dir()) && path.to_string_lossy().contains("cut-test-process-")
}

/// Register [`install_for_this_process`] to run when the test binary loads,
/// before `main` and before any test thread. Use once per crate, in test cfg:
/// `cut_core::isolate_test_process!();`
#[macro_export]
macro_rules! isolate_test_process {
    () => {
        #[cfg(test)]
        mod __isolate_test_process {
            extern "C" fn init() {
                $crate::test_isolation::install_for_this_process();
            }
            #[cfg_attr(
                any(target_os = "linux", target_os = "android", target_os = "freebsd"),
                link_section = ".init_array"
            )]
            #[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
            #[cfg_attr(windows, link_section = ".CRT$XCU")]
            #[used]
            static INIT: extern "C" fn() = init;
        }
    };
}
