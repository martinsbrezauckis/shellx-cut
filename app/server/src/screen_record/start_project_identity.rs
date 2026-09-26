//! Bind an optional recording request to the project the caller observed.

use crate::dispatch::{no_project, path_free_project_identity};
use crate::state::AppState;
use cut_core::{error_codes, CutError};
use serde_json::Value;

/// Caller holds `project_transition` through this check and capture admission.
pub(super) async fn admit(state: &AppState, expected: Option<&Value>) -> Result<(), CutError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let project = state.project.read().await;
    let store = project.as_ref().ok_or_else(no_project)?;
    if path_free_project_identity(store)? != *expected {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "recording project changed",
            "expected_project_identity does not match the open project",
        )
        .with_suggested_action("refresh project.state and start recording for that project"));
    }
    Ok(())
}
