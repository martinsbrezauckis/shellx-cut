//! Shell-owned Windows controller capture exclusion.
//!
//! `SetWindowDisplayAffinity` requires this process to own the HWND. The
//! spawned cutd must receive only the resulting redacted conclusion, never a
//! native handle.

use tauri::WebviewWindow;

pub const ENV_CONTROLLER_PLACEMENT: &str = "SHELLX_CUT_CONTROLLER_PLACEMENT";
pub const ENV_CONTROLLER_PLACEMENT_OWNER: &str = "SHELLX_CUT_CONTROLLER_PLACEMENT_OWNER";
pub const PLACEMENT_OWNER: &str = "tauri-shell-v1";

const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;

#[link(name = "user32")]
extern "system" {
    fn SetWindowDisplayAffinity(hwnd: *mut core::ffi::c_void, affinity: u32) -> i32;
    fn GetWindowDisplayAffinity(hwnd: *mut core::ffi::c_void, affinity: *mut u32) -> i32;
}

/// Set and read back affinity while the Tauri shell still owns the top-level
/// HWND. `None` means no current shell window was available; callers must not
/// turn that absence into an exclusion claim.
pub fn observe(window: &WebviewWindow) -> Option<&'static str> {
    let hwnd = window.hwnd().ok()?;
    if hwnd.0.is_null() {
        return None;
    }
    let mut affinity = 0_u32;
    // SAFETY: Tauri returned this shell process's live HWND. `affinity` is
    // writable output storage for the immediate user32 readback.
    let set_ok = unsafe { SetWindowDisplayAffinity(hwnd.0, WDA_EXCLUDEFROMCAPTURE) } != 0;
    // SAFETY: same shell-owned HWND and initialized output storage as above.
    let read_ok = unsafe { GetWindowDisplayAffinity(hwnd.0, &mut affinity) } != 0;
    Some(placement_from_readback(set_ok, read_ok, affinity))
}

const fn placement_from_readback(set_ok: bool, read_ok: bool, affinity: u32) -> &'static str {
    if set_ok && read_ok && affinity == WDA_EXCLUDEFROMCAPTURE {
        "excluded"
    } else {
        "refused"
    }
}

#[cfg(test)]
mod tests {
    use super::placement_from_readback;

    #[test]
    fn only_successful_shell_readback_reports_excluded() {
        assert_eq!(placement_from_readback(true, true, 0x11), "excluded");
        assert_eq!(placement_from_readback(false, true, 0x11), "refused");
        assert_eq!(placement_from_readback(true, false, 0x11), "refused");
        assert_eq!(placement_from_readback(true, true, 0), "refused");
    }
}
