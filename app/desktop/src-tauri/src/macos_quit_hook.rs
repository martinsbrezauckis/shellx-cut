//! macOS: the app menu's Quit, Cmd+Q, Quit in the Dock, an Apple Event quit and
//! logging out go through the same close guard as closing the window.
//!
//! All of them call `-[NSApplication terminate:]`. The windowing layer turns
//! that only into the final exit event, without the exit request the guard
//! listens to, so a quit used to stop the engine in the middle of a recording
//! (the take was left "interrupted") or an export. `macos_quit_hook.m` adds
//! `applicationShouldTerminate:` to the app delegate; it asks
//! [`crate::close_guard::on_close_request`], cancels this terminate while the
//! guard holds, and the guard quits the app itself once the recording is saved
//! or the person answered the export question. Logging out while held is
//! cancelled by macOS with its own notice; the app then quits by itself.

use std::sync::OnceLock;

use tauri::AppHandle;

static APP: OnceLock<AppHandle> = OnceLock::new();

extern "C" {
    fn sxc_macos_install_quit_hook(callback: extern "C" fn() -> i32) -> i32;
    #[cfg(test)]
    fn sxc_macos_quit_hook_selftest(hold: i32, delegate_answers_itself: i32) -> std::ffi::c_long;
}

/// 1 holds the quit, 0 lets it go ahead. Never unwinds into AppKit.
extern "C" fn should_hold_quit() -> i32 {
    let Some(app) = APP.get() else {
        return 0;
    };
    let decision = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::close_guard::on_close_request(app, true)
    }));
    i32::from(matches!(decision, Ok(crate::close_guard::Decision::Hold)))
}

/// Install once, on the main thread, after the app finished launching.
pub(crate) fn install(app: &AppHandle) {
    let _ = APP.set(app.clone());
    // SAFETY: called from the setup hook on the AppKit main thread; the
    // callback is a plain `extern "C"` function that lives for the process.
    let status = unsafe { sxc_macos_install_quit_hook(should_hold_quit) };
    if status != 0 {
        // Without the hook a quit still does not cut a recording off (see
        // close_guard::on_unasked_exit), but it no longer asks first.
        eprintln!(
            "[shellx-cut] quit guard not installed (status {status}); Quit will not ask first"
        );
    }
}

#[cfg(test)]
mod tests {
    // NSApplicationTerminateReply values.
    const CANCEL: std::ffi::c_long = 0;
    const NOW: std::ffi::c_long = 1;
    const LATER: std::ffi::c_long = 2;

    #[test]
    fn a_terminate_request_asks_the_guard_first() {
        // SAFETY: the self-test builds its own delegate class and object.
        let reply = |hold, own| unsafe { super::sxc_macos_quit_hook_selftest(hold, own) };
        // Quit from the menu, Cmd+Q, Dock, Apple Event or logout while the
        // guard holds (a recording is being saved, an export asks): cancelled.
        assert_eq!(reply(1, 0), CANCEL);
        // Nothing to save or ask about: the quit goes ahead.
        assert_eq!(reply(0, 0), NOW);
        // A delegate that already answers keeps its answer once the guard lets
        // the quit go, and is overruled while the guard holds.
        assert_eq!(reply(0, 1), LATER);
        assert_eq!(reply(1, 1), CANCEL);
    }
}
