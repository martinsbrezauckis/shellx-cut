//! Screen-capture access bookkeeping that never starts a capture.
//!
//! Every native screen access in this crate (the delivery probe, display and
//! window lists, the permission read) passes through [`admit_native_screen_access`].
//! That lets a status read prove it touched nothing, and lets a test process
//! refuse every native screen access and every permission request, so a test
//! run can never start a capture or show a system prompt on the desktop it
//! runs on. Setting `SHELLX_CUT_NATIVE_CAPTURE_OFF=1` in the environment has
//! the same effect for the whole process.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

static NATIVE_SCREEN_ACCESS_FORBIDDEN: AtomicBool = AtomicBool::new(false);
static NATIVE_CAPTURE_OFF_BY_ENVIRONMENT: OnceLock<bool> = OnceLock::new();

thread_local! {
    static NATIVE_SCREEN_ACCESS_ATTEMPTS: Cell<u64> = const { Cell::new(0) };
    static SCREEN_PROBE_ATTEMPTS: Cell<u64> = const { Cell::new(0) };
}

/// Screen-delivery checks (the short real capture) attempted on this thread.
pub fn screen_probe_attempts() -> u64 {
    SCREEN_PROBE_ATTEMPTS.with(Cell::get)
}

/// Count one screen-delivery check on this thread.
pub(crate) fn count_screen_probe() {
    SCREEN_PROBE_ATTEMPTS.with(|attempts| attempts.set(attempts.get() + 1));
}

/// Refuse every later native screen access and permission request in this
/// process. Meant for test processes; it cannot be undone.
pub fn forbid_native_capture_access() {
    NATIVE_SCREEN_ACCESS_FORBIDDEN.store(true, Ordering::SeqCst);
}

/// True when native screen access and permission requests are refused.
pub(crate) fn native_capture_access_forbidden() -> bool {
    NATIVE_SCREEN_ACCESS_FORBIDDEN.load(Ordering::SeqCst)
        || *NATIVE_CAPTURE_OFF_BY_ENVIRONMENT.get_or_init(|| {
            std::env::var_os("SHELLX_CUT_NATIVE_CAPTURE_OFF").is_some_and(|value| value == "1")
        })
}

/// Native screen accesses attempted on the calling thread, refused or not.
pub fn native_screen_access_attempts() -> u64 {
    NATIVE_SCREEN_ACCESS_ATTEMPTS.with(Cell::get)
}

/// Count one native screen access on this thread and say whether it may run.
pub(crate) fn admit_native_screen_access() -> bool {
    NATIVE_SCREEN_ACCESS_ATTEMPTS.with(|attempts| attempts.set(attempts.get() + 1));
    !native_capture_access_forbidden()
}

/// Whether the OS already lets this app capture the screen, read without
/// asking. macOS answers from `CGPreflightScreenCaptureAccess`, which never
/// shows a prompt. Other platforms have no such permission and return `None`,
/// as does a process where native screen access is refused.
pub fn screen_capture_access_preflight() -> Option<bool> {
    #[cfg(all(target_os = "macos", feature = "capture-macos"))]
    {
        if !admit_native_screen_access() {
            return None;
        }
        #[link(name = "CoreGraphics", kind = "framework")]
        extern "C" {
            fn CGPreflightScreenCaptureAccess() -> bool;
        }
        // SAFETY: CoreGraphics owns the permission state and takes no Rust pointers.
        return Some(unsafe { CGPreflightScreenCaptureAccess() });
    }
    #[allow(unreachable_code)]
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempts_are_counted_per_thread() {
        let before = native_screen_access_attempts();
        assert!(admit_native_screen_access() || native_capture_access_forbidden());
        assert_eq!(native_screen_access_attempts(), before + 1);
        let other = std::thread::spawn(native_screen_access_attempts)
            .join()
            .unwrap();
        assert_eq!(other, 0);
    }
}
