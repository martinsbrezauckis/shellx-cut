//! Project-transition admission for project-owned native captures.

use super::{canonical_project_dir, capture_sessions};
use crate::state::AppState;
use cut_core::{error_codes, CutError};
use std::path::Path;

/// A live recorder owns its project directory through native worker teardown.
/// Callers hold `AppState::project_transition` before awaiting this check. The
/// project snapshot is taken before the synchronous registry lock, so no await
/// is ever performed while the registry is held.
///
/// `delete_target` deliberately has different semantics from replacement:
/// deleting an unrelated closed project remains available during a capture,
/// while deleting the capture's own project is refused even from another
/// `AppState`. An unbound native reservation (rehearsal or screenshot) remains
/// fail-closed because it cannot prove an unaffected project root.
pub(crate) async fn refuse_project_transition_if_active(
    state: &AppState,
    delete_target: Option<&Path>,
) -> Result<(), CutError> {
    let current_project = {
        let project = state.project.read().await;
        project
            .as_ref()
            .map(|store| canonical_project_dir(&store.dir))
    };
    let delete_target = delete_target.map(canonical_project_dir);
    let active = capture_sessions()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .next()
        .map(|(capture_id, entry)| (capture_id.clone(), entry.project_dir.clone()));
    let Some((capture_id, owned_project)) = active else {
        return Ok(());
    };

    let blocks_transition = match (owned_project, delete_target) {
        (None, _) => true,
        (Some(owned), Some(target)) => owned == target,
        (Some(owned), None) => current_project.is_some_and(|current| current == owned),
    };
    if !blocks_transition {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        "cannot change projects while a native capture is active",
        format!(
            "capture {capture_id} still owns native capture and the current project directory"
        ),
    )
    .with_suggested_action(
        "wait for the active native capture to finish before opening, creating, closing, or deleting its project",
    ))
}

#[cfg(test)]
mod tests {
    use super::super::{capture_test_lock, reserve_capture};
    use super::*;
    use crate::screen_record::CaptureSessionControl;

    const UNBOUND_CAPTURE_TRANSITION_CHILD: &str =
        "SHELLX_CUT_UNBOUND_CAPTURE_TRANSITION_TEST_CHILD";
    const TEST_NAME: &str =
        "screen_record::capture_registry::project_transition::tests::unbound_reservation_remains_fail_closed";

    #[tokio::test]
    async fn unbound_reservation_remains_fail_closed() {
        // An unbound reservation deliberately rejects every project transition.
        // Run that process-global state in an exact-test child so it cannot
        // affect unrelated parallel project fixtures in the parent process.
        if std::env::var_os(UNBOUND_CAPTURE_TRANSITION_CHILD).is_none() {
            let status = std::process::Command::new(
                std::env::current_exe().expect("resolve server test executable"),
            )
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(UNBOUND_CAPTURE_TRANSITION_CHILD, "1")
            .status()
            .expect("start isolated unbound-capture test");
            assert!(status.success(), "isolated unbound-capture test failed");
            return;
        }

        let _capture_lock = capture_test_lock().lock().await;
        let reservation = reserve_capture(
            format!("cap_unbound_transition_{}", std::process::id()),
            CaptureSessionControl::new(None, false, false, false),
        )
        .expect("unbound reservation must be admitted in its isolated process");
        let state = AppState::new();
        let _transition = state.project_transition.lock().await;
        let unrelated_delete_root = tempfile::tempdir().expect("unrelated delete root");

        let replacement = refuse_project_transition_if_active(&state, None)
            .await
            .expect_err("unbound reservation must refuse project replacement");
        assert_eq!(replacement.code, error_codes::CONFLICT);
        let delete =
            refuse_project_transition_if_active(&state, Some(unrelated_delete_root.path()))
                .await
                .expect_err("unbound reservation must refuse an unrelated project deletion");
        assert_eq!(delete.code, error_codes::CONFLICT);

        drop(reservation);
        assert!(
            refuse_project_transition_if_active(&state, None)
                .await
                .is_ok(),
            "replacement becomes available after unbound reservation release"
        );
        assert!(
            refuse_project_transition_if_active(&state, Some(unrelated_delete_root.path()))
                .await
                .is_ok(),
            "deletion becomes available after unbound reservation release"
        );
    }
}
