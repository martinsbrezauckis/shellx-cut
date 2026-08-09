//! Coherent updater handoff for the desktop shell's owned `cutd` sidecar.
//!
//! The Windows updater exits through `std::process::exit`, which bypasses the
//! normal Tauri run-event cleanup. The installer must therefore receive a
//! verified, quiescent application: download/signature verification happens
//! first, then this module stops and reaps the exact child spawned by this
//! shell, and only then may the installer start.

use tauri::Manager;

use crate::EngineProcess;
#[cfg(windows)]
use crate::{EngineState, EngineStatus};

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EngineMode {
    Spawned,
    External,
    Unwired,
}

#[cfg(any(windows, test))]
fn handoff_policy(mode: EngineMode, has_owned_child: bool) -> Result<bool, &'static str> {
    match (mode, has_owned_child) {
        (EngineMode::Spawned, true) => Ok(true),
        (EngineMode::Spawned, false) => Err("the owned engine handle is missing"),
        (EngineMode::External, _) => Err("ShellX Cut is using an external engine"),
        (EngineMode::Unwired, false) => Ok(false),
        (EngineMode::Unwired, true) => Err("an untracked owned engine is still running"),
    }
}

#[cfg(windows)]
fn live_mode(app: &tauri::AppHandle) -> EngineMode {
    let Some(state) = app.try_state::<EngineStatus>() else {
        return EngineMode::Unwired;
    };
    let status = state.0.lock().unwrap();
    match &*status {
        EngineState::Wired {
            mode: "spawned", ..
        } => EngineMode::Spawned,
        EngineState::Wired { .. } => EngineMode::External,
        EngineState::Unwired { .. } => EngineMode::Unwired,
    }
}

/// Stop and reap the exact `cutd` child owned by this shell before an update.
///
/// An adopted external engine is deliberately never killed. The update fails
/// closed and asks the user to close the other Cut instance instead of risking
/// a locked or mixed-version installation.
#[cfg(windows)]
pub(crate) fn stop_owned_engine_for_update(app: &tauri::AppHandle) -> Result<bool, String> {
    let mode = live_mode(app);
    let state = app
        .try_state::<EngineProcess>()
        .ok_or_else(|| "the desktop engine process manager is unavailable".to_string())?;
    let mut slot = state.0.lock().unwrap();
    let should_stop = handoff_policy(mode, slot.is_some()).map_err(|reason| {
        if mode == EngineMode::External {
            format!(
                "{reason}. Close every other ShellX Cut window and engine process, then try the update again"
            )
        } else {
            format!("cannot prepare the updater: {reason}")
        }
    })?;
    if !should_stop {
        return Ok(false);
    }

    let child = slot
        .as_mut()
        .expect("handoff policy requires an owned child");
    if child
        .try_wait()
        .map_err(|e| format!("could not inspect the ShellX Cut engine before update: {e}"))?
        .is_none()
    {
        child
            .kill()
            .map_err(|e| format!("could not stop the ShellX Cut engine before update: {e}"))?;
        child
            .wait()
            .map_err(|e| format!("could not confirm the ShellX Cut engine stopped: {e}"))?;
    }
    slot.take();
    Ok(true)
}

/// Normal window/app exit stays best-effort: only the exact stored child is
/// touched, and an adopted external engine remains outside our ownership.
pub(crate) fn stop_owned_engine_best_effort(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<EngineProcess>() {
        if let Some(mut child) = state.0.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updater_stops_only_a_tracked_spawned_engine() {
        assert_eq!(handoff_policy(EngineMode::Spawned, true), Ok(true));
        assert!(handoff_policy(EngineMode::Spawned, false).is_err());
        assert!(handoff_policy(EngineMode::External, false).is_err());
        assert_eq!(handoff_policy(EngineMode::Unwired, false), Ok(false));
        assert!(handoff_policy(EngineMode::Unwired, true).is_err());
    }
}
