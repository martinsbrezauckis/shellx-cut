//! Process-lifetime Windows Runtime state shared by WGC capture sessions.
//!
//! `windows-capture` initializes and uninitializes WinRT on each capture thread.
//! The generated `windows` bindings cache agile activation factories for the
//! lifetime of the process. If the last MTA usage cookie is released between
//! sessions, Windows may unload the DLL that owns a cached factory's vtable;
//! the next capture then calls through that stale pointer. Keep one MTA usage
//! reference until process exit so those process-global caches remain valid.

use std::sync::OnceLock;

use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

static PROCESS_MTA_PIN: OnceLock<Result<usize, String>> = OnceLock::new();

pub(crate) fn pin_process_mta() -> Result<(), String> {
    match PROCESS_MTA_PIN.get_or_init(|| {
        // SAFETY: CoIncrementMTAUsage returns an opaque process-wide usage cookie.
        // We deliberately retain it until process exit and therefore must not call
        // CoDecrementMTAUsage while generated WinRT factory caches can still exist.
        unsafe { CoIncrementMTAUsage() }
            .map(|cookie| cookie.0 as usize)
            .map_err(|error| format!("CoIncrementMTAUsage: {error}"))
    }) {
        Ok(_) => Ok(()),
        Err(error) => Err(error.clone()),
    }
}

/// Keep the capture worker in the same physical, per-monitor coordinate space
/// as WGC frames and the low-level mouse hook. `cutd` has no UI manifest of its
/// own, so relying on a desktop shell's DPI context would let Windows virtualize
/// monitor geometry before it reaches the Region input mapper.
pub(crate) struct PerMonitorDpiContext {
    previous: DPI_AWARENESS_CONTEXT,
}

impl Drop for PerMonitorDpiContext {
    fn drop(&mut self) {
        // SAFETY: `previous` is the thread-local context returned by the paired
        // successful call below. Restoring it at capture exit cannot affect a
        // different thread or the foreground shell process.
        unsafe {
            let _ = SetThreadDpiAwarenessContext(self.previous);
        }
    }
}

/// Refuse capture if Windows cannot expose physical per-monitor coordinates.
/// A virtualized monitor rectangle could still look geometrically valid while
/// mapping WGC pixels and `MSLLHOOKSTRUCT::pt` to different desktop positions.
pub(crate) fn enter_per_monitor_dpi_v2() -> Result<PerMonitorDpiContext, String> {
    // SAFETY: this changes only the calling capture worker's temporary DPI
    // context. The returned previous context is restored by `Drop`.
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if previous.0.is_null() {
        return Err("SetThreadDpiAwarenessContext(PER_MONITOR_AWARE_V2) failed".to_string());
    }
    Ok(PerMonitorDpiContext { previous })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_mta_pin_is_idempotent() {
        pin_process_mta().expect("first process MTA pin");
        pin_process_mta().expect("second process MTA pin");
        assert!(matches!(PROCESS_MTA_PIN.get(), Some(Ok(_))));
    }
}
