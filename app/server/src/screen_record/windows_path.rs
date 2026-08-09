//! Windows Graphics Capture output-path admission for `screen_record.start`.

use std::path::Path;

#[cfg(test)]
use std::path::PathBuf;

#[cfg(any(windows, test))]
use cut_core::error_codes;
use cut_core::CutError;

#[cfg(any(windows, test))]
pub(super) fn ensure_wgc_checkpoint_path_supported(capture_dir: &Path) -> Result<(), CutError> {
    let budget = record_recovery::windows_wgc_path_budget(capture_dir);
    if budget.supported() {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::INVALID_ARGS,
        "project path is too long for Windows screen recording",
        format!(
            "Windows Graphics Capture would receive a {} UTF-16-code-unit checkpoint path (including its terminator); the supported limit is {}",
            budget.utf16_units_with_nul, budget.max_utf16_units_with_nul
        ),
    )
    .with_suggested_action(
        "move or open the project under a shorter path, then start recording again",
    ))
}

#[cfg(all(not(windows), not(test)))]
pub(super) fn ensure_wgc_checkpoint_path_supported(_capture_dir: &Path) -> Result<(), CutError> {
    Ok(())
}

#[cfg(any(windows, test))]
pub(super) fn ensure_pre_marker_path(project_dir: &Path, capture_id: &str) -> Result<(), CutError> {
    let capture_dir = super::screen_record_cache_dir(project_dir)?.join(capture_id);
    ensure_wgc_checkpoint_path_supported(&super::strip_verbatim_prefix(&capture_dir))
}

#[cfg(all(not(windows), not(test)))]
pub(super) fn ensure_pre_marker_path(
    _project_dir: &Path,
    _capture_id: &str,
) -> Result<(), CutError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ascii_root_with_utf16_units(units: usize) -> PathBuf {
        #[cfg(windows)]
        let prefix = r"C:\";
        #[cfg(not(windows))]
        let prefix = "/";

        assert!(units >= prefix.len());
        let root = PathBuf::from(format!("{prefix}{}", "x".repeat(units - prefix.len())));
        assert_eq!(root.to_string_lossy().encode_utf16().count(), units);
        root
    }

    #[test]
    fn wgc_path_budget_accepts_supported_root_and_rejects_first_unsupported_root() {
        let supported_root = ascii_root_with_utf16_units(225);
        let supported_budget = record_recovery::windows_wgc_path_budget(&supported_root);
        assert_eq!(supported_budget.utf16_units_with_nul, 259);
        assert!(supported_budget.supported());
        assert!(ensure_wgc_checkpoint_path_supported(&supported_root).is_ok());

        let final_supported_root = ascii_root_with_utf16_units(226);
        let final_supported_budget =
            record_recovery::windows_wgc_path_budget(&final_supported_root);
        assert_eq!(final_supported_budget.utf16_units_with_nul, 260);
        assert!(final_supported_budget.supported());

        let first_unsupported_root = ascii_root_with_utf16_units(227);
        let error = ensure_wgc_checkpoint_path_supported(&first_unsupported_root).unwrap_err();
        assert_eq!(error.code, error_codes::INVALID_ARGS);
        assert!(error.message.contains("too long"));
        assert!(error.cause.contains("261"));
        assert!(error
            .suggested_action
            .as_deref()
            .is_some_and(|action| action.contains("shorter path")));
    }

    #[test]
    fn wgc_budget_uses_the_same_stripped_path_as_the_native_encoder() {
        let plain = ascii_root_with_utf16_units(225);
        let verbatim = PathBuf::from(format!(r"\\?\{}", plain.display()));

        assert_eq!(super::super::strip_verbatim_prefix(&verbatim), plain);
        assert_eq!(
            record_recovery::windows_wgc_path_budget(&plain),
            record_recovery::windows_wgc_path_budget(&super::super::strip_verbatim_prefix(
                &verbatim
            ))
        );
    }
}
