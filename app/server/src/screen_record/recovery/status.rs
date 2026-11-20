//! Read-only project-local recovery-status root lookup.

use std::path::Path;

use cut_core::{error_codes, CutError};
use record_recovery::{is_plain_dir, recovery_status, CaptureRecoveryStatus};
use serde::Serialize;

use super::validate_capture_id;

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

#[derive(Debug, Serialize)]
pub(crate) struct RecoveryStatusPage {
    pub captures: Vec<RecoveryStatusItem>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RecoveryStatusItem {
    pub capture_id: String,
    #[serde(flatten)]
    pub status: CaptureRecoveryStatus,
    /// Whether screen_record.recover can make a clip from this capture's files
    /// (sealed video, unsealed checkpoints, or sound). Read from file names and
    /// sizes only.
    pub recoverable: bool,
}

/// Enumerate the capture-cache status without calling a media tool or changing
/// anything on disk. Cursor values and source paths never escape this owner.
pub(crate) fn status_page(
    cache: &Path,
    after: Option<&str>,
    limit: usize,
) -> Result<RecoveryStatusPage, CutError> {
    if !cache.exists() {
        return Ok(RecoveryStatusPage {
            captures: Vec::new(),
            next_cursor: None,
        });
    }
    let entries = std::fs::read_dir(cache).map_err(|_error| {
        CutError::new(
            error_codes::IO,
            "could not read screen-record recovery cache",
            "check that the project cache directory is readable",
        )
    })?;
    // One pass keeps only the next page: a received project may hold any
    // number of folders in its capture cache.
    let limit = limit.clamp(1, 100);
    let mut cursor_seen = after.is_none();
    let mut page = std::collections::BTreeSet::new();
    for entry in entries.flatten() {
        let Some(id) = entry
            .file_type()
            .ok()
            .filter(|kind| kind.is_dir())
            .filter(|_| is_plain_dir(&entry.path()).unwrap_or(false))
            .and_then(|_| entry.file_name().into_string().ok())
            .filter(|id| validate_capture_id(id).is_ok())
        else {
            continue;
        };
        if let Some(cursor) = after {
            if id == cursor {
                cursor_seen = true;
            }
            if id.as_str() <= cursor {
                continue;
            }
        }
        page.insert(id);
        if page.len() > limit + 1 {
            page.pop_last();
        }
    }
    if !cursor_seen {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "recovery-status cursor was not emitted for this project",
            "request the first page, then pass its exact next_cursor value",
        )
        .with_suggested_action("pass an exact next_cursor from the previous response"));
    }
    let more = page.len() > limit;
    let selected = page.iter().take(limit);
    let captures: Vec<_> = selected
        .map(|capture_id| RecoveryStatusItem {
            capture_id: capture_id.clone(),
            status: recovery_status(&cache.join(capture_id)),
            recoverable: super::super::pending_recordings::has_recoverable_media(
                &cache.join(capture_id),
            ),
        })
        .collect();
    let next_cursor = more
        .then(|| captures.last().map(|item| item.capture_id.clone()))
        .flatten();
    Ok(RecoveryStatusPage {
        captures,
        next_cursor,
    })
}

#[cfg(test)]
mod paging_tests {
    use super::status_page;

    /// The listing keeps one page at a time, so paging must still visit every
    /// capture once, in order, whatever the folder order on disk.
    #[test]
    fn status_pages_visit_every_capture_once_in_order() {
        let temp = tempfile::tempdir().unwrap();
        let mut expected: Vec<String> = (0..37).map(|n| format!("capture-{n:03}")).collect();
        for id in expected.iter().rev() {
            std::fs::create_dir(temp.path().join(id)).unwrap();
        }
        std::fs::write(temp.path().join("capture-file"), b"not a folder").unwrap();
        expected.sort();
        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = status_page(temp.path(), cursor.as_deref(), 5).unwrap();
            assert!(page.captures.len() <= 5);
            seen.extend(page.captures.iter().map(|item| item.capture_id.clone()));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        assert_eq!(seen, expected);
    }
}
