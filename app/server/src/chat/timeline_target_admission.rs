//! Open-project admission for the immutable timeline context attached to Agent Chat.

use crate::{
    chat::{self, timeline_target::TimelineTarget},
    dispatch::path_free_project_identity,
    state::AppState,
};
use cut_core::{error_codes, CutError};

/// Resolve asset references and validate the target while the exact project
/// snapshot remains read-locked. The caller releases that lock before spawning
/// a provider, so a target cannot be accepted for one project/revision then run
/// against another.
pub(crate) async fn validate_request_scope(
    state: &AppState,
    attachment_ids: &[String],
    target: Option<TimelineTarget>,
) -> Result<(Vec<String>, Option<TimelineTarget>), CutError> {
    let project = state.project.read().await;
    let store = project.as_ref().ok_or_else(crate::dispatch::no_project)?;
    let attachments =
        chat::validate_attachment_ids(attachment_ids, |id| store.project.assets.contains_key(id))
            .map_err(|detail| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "invalid chat attachments",
                detail,
            )
        })?;
    let target = target.map(|candidate| {
        let identity = path_free_project_identity(store)?;
        let revision = store.log.current_revision()?.ok_or_else(|| CutError::new(
            error_codes::CONFLICT,
            "timeline target cannot be checked against an empty project history",
            "make an initial project edit or choose the target again after the project state is available",
        ))?;
        chat::timeline_target::validate(candidate, &store.project, &identity, Some(&revision))
            .map_err(|detail| CutError::new(
                error_codes::CONFLICT,
                "timeline target changed before Agent Chat started",
                detail,
            ).with_suggested_action("choose the target again from the current timeline"))
    }).transpose()?;
    Ok((attachments, target))
}
