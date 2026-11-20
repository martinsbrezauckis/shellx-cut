//! Exact native monitor admission for `screen_record.start`.
//!
//! A monitor identity is authoritative only when it is copied unchanged from a
//! fresh Doctor row. This module intentionally does not inspect display names,
//! ordinals, primary state, or geometry: any mismatch asks the caller to reopen
//! the source picker instead of selecting a plausible replacement.

use super::{MonitorInfo, WindowInfo};
use cut_core::{error_codes, CutError};

const MAX_MONITOR_ID_BYTES: usize = 1_024;
const MAX_WINDOW_ID_BYTES: usize = 1_024;

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

/// Admit an optional window identity before anything is created or started.
///
/// The window must be one of the windows the same fresh Doctor result lists.
/// Without this check a closed or invented window id returned success at
/// Start and failed only at Stop, after a capture directory and journal had
/// already been created.
pub(super) fn admit_window(
    requested: Option<&str>,
    window_capture_supported: bool,
    windows: &[WindowInfo],
) -> Result<(), CutError> {
    let Some(requested) = requested else {
        return Ok(());
    };
    if !window_capture_supported {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "window recording is not available on this system",
            "this recorder captures whole displays only on this platform",
        )
        .with_suggested_action("choose a display from screen_record.doctor instead of a window"));
    }
    if requested.trim().is_empty() || requested.len() > MAX_WINDOW_ID_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "window must be a current opaque window identity",
            "the window value is empty or too long to be a window id from screen_record.doctor",
        )
        .with_suggested_action("reopen the source picker and choose a listed window"));
    }
    if !windows.iter().any(|window| window.id == requested) {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "the selected window is no longer available",
            format!(
                "window {requested} is not in the current screen_record.doctor window list: it was closed, or the id did not come from that list"
            ),
        )
        .with_suggested_action("reopen the source picker and choose the window again"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str) -> WindowInfo {
        WindowInfo {
            id: id.into(),
            title: "Notes".into(),
            app: "notes.exe".into(),
        }
    }

    #[test]
    fn listed_window_is_admitted_and_absent_window_is_refused_before_start() {
        let listed = "windows-hwnd-v1:0000000000010203:42";
        admit_window(Some(listed), true, &[window(listed)]).unwrap();
        admit_window(None, true, &[]).unwrap();
        let error = admit_window(
            Some("windows-hwnd-v1:00000000deadbeef:1"),
            true,
            &[window(listed)],
        )
        .unwrap_err();
        assert_eq!(error.code, error_codes::NOT_FOUND);
        assert_eq!(error.message, "the selected window is no longer available");
        assert!(error.cause.contains("windows-hwnd-v1:00000000deadbeef:1"));
        assert_eq!(
            error.suggested_action.as_deref(),
            Some("reopen the source picker and choose the window again")
        );
    }

    #[test]
    fn window_on_a_display_only_platform_is_refused_with_a_display_action() {
        let error = admit_window(Some("any"), false, &[]).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
        assert_eq!(
            error.message,
            "window recording is not available on this system"
        );
        assert_eq!(
            error.suggested_action.as_deref(),
            Some("choose a display from screen_record.doctor instead of a window")
        );
        let error = admit_window(Some("  "), true, &[]).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
    }

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
