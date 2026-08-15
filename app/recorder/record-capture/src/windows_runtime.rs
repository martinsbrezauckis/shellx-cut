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
