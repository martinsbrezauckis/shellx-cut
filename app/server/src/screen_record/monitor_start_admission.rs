//! Exact native monitor admission for `screen_record.start`.
//!
//! A monitor identity is authoritative only when it is copied unchanged from a
//! fresh Doctor row. This module intentionally does not inspect display names,
//! ordinals, primary state, or geometry: any mismatch asks the caller to reopen
//! the source picker instead of selecting a plausible replacement.

use super::MonitorInfo;
use cut_core::{error_codes, CutError};

const MAX_MONITOR_ID_BYTES: usize = 1_024;

/// Server-admitted monitor target for one capture start.
///
/// `legacy_index` remains the compatibility path when no exact Doctor identity
/// was supplied. It is deliberately retained separately so it cannot become a
/// replacement for `exact_id`.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) legacy_index: Option<u32>,
    pub(super) exact_id: Option<String>,
}

/// Admit an optional exact monitor identity from the current Doctor result.
///
/// A supplied value must match one currently enumerated opaque identity exactly;
/// it is returned unchanged for the native backend's second, just-before-capture
/// revalidation. Omission keeps the ordinary monitor-index/default-start path.
pub(super) fn admit(
    legacy_index: Option<u32>,
    requested: Option<&str>,
    has_window: bool,
    monitors: &[MonitorInfo],
) -> Result<Target, CutError> {
    if has_window && requested.is_some() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "window and monitor_id cannot be selected together",
            "choose one source from the current screen_record.doctor result",
        ));
    }
    let Some(requested) = requested else {
        return Ok(Target {
            legacy_index,
            exact_id: None,
        });
    };
    if requested.trim().is_empty() || requested.len() > MAX_MONITOR_ID_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "monitor_id must be a current opaque monitor identity",
            "refresh the source picker and choose a listed display",
        ));
    }
    if !monitors
        .iter()
        .any(|monitor| monitor.id.as_deref() == Some(requested))
    {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "the selected display is no longer available",
            "the exact monitor identity was absent from the current screen_record.doctor result",
        )
        .with_suggested_action("reopen the source picker and choose the display again"));
    }
    Ok(Target {
        legacy_index,
        exact_id: Some(requested.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(id: Option<&str>, index: u32) -> MonitorInfo {
        MonitorInfo {
            id: id.map(str::to_owned),
            index,
            name: "Display copy must not select it".into(),
            width: 1_920,
            height: 1_080,
            primary: index == 1,
        }
    }

    #[test]
    fn exact_current_identity_is_preserved_unchanged() {
        let id = "shellx-monitor-v1:windows:exact";
        assert_eq!(
            admit(Some(2), Some(id), false, &[monitor(Some(id), 2)]).unwrap(),
            Target {
                legacy_index: Some(2),
                exact_id: Some(id.into()),
            }
        );
    }

    #[test]
    fn missing_identity_never_falls_back_to_matching_display_copy_or_ordinal() {
        let error = admit(
            Some(2),
            Some("shellx-monitor-v1:windows:retired"),
            false,
            &[monitor(Some("shellx-monitor-v1:windows:replacement"), 2)],
        )
        .unwrap_err();
        assert_eq!(error.code, error_codes::NOT_FOUND);
    }

    #[test]
    fn monitor_id_and_window_are_mutually_exclusive() {
        let error = admit(
            Some(2),
            Some("shellx-monitor-v1:windows:exact"),
            true,
            &[monitor(Some("shellx-monitor-v1:windows:exact"), 2)],
        )
        .unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
    }

    #[test]
    fn omitted_identity_preserves_the_ordinary_legacy_index() {
        assert_eq!(
            admit(Some(2), None, false, &[monitor(None, 2)]).unwrap(),
            Target {
                legacy_index: Some(2),
                exact_id: None,
            }
        );
    }

    #[test]
    fn malformed_identity_is_rejected_before_capture_start() {
        let error = admit(None, Some("   "), false, &[]).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
    }
}
