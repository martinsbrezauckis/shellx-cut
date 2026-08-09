//! Read-only project-local recovery-status root lookup.

use std::path::Path;

use cut_core::CutError;

use super::{status_page, RecoveryStatusPage};

pub(super) fn for_project(
    project_dir: &Path,
    after: Option<&str>,
    limit: usize,
) -> Result<RecoveryStatusPage, CutError> {
    match crate::screen_record::containment::existing_cache_dir(project_dir)? {
        Some(cache) => status_page(&cache, after, limit),
        None => Ok(RecoveryStatusPage {
            captures: vec![],
            next_cursor: None,
        }),
    }
}
