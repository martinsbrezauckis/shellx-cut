//! Optional immutable-origin admission for project-owned asynchronous work.

use crate::dispatch::{no_project, path_free_project_identity};
use crate::state::AppState;
use cut_core::{error_codes, CutError};

/// The caller holds `project_transition` through the protected operation.
pub(crate) async fn admit(state: &AppState, expected: Option<&str>) -> Result<(), CutError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let project = state.project.read().await;
    let store = project.as_ref().ok_or_else(no_project)?;
    let current = path_free_project_identity(store)?;
    if current["origin_path_sha256"] != expected {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "job project changed",
            "expected_origin_path_sha256 does not match the open project",
        )
        .with_suggested_action("return to the job's project and retry there"));
    }
    Ok(())
}
