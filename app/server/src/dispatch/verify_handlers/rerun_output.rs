//! Final-leaf and authorized-root validation for `verify.rerun` outputs.

use crate::output_paths::fenced_existing_export_read;
use cut_core::{error_codes, CutError};
use std::path::{Path, PathBuf};

pub(super) fn fenced_output_for_receipt(
    project_dir: &Path,
    receipt_output_path: &str,
    expected: Option<&Path>,
) -> Result<PathBuf, CutError> {
    let declared = Path::new(receipt_output_path);
    let declared_plain = record_recovery::is_plain_regular_file(declared).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not inspect the receipt-bound render output",
            error.to_string(),
        )
    })?;
    if !declared_plain {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "receipt-bound render output is not a local regular file",
            format!("refusing unsafe output leaf {}", declared.display()),
        )
        .with_suggested_action("render again before re-running output checks"));
    }
    let output = fenced_existing_export_read(
        project_dir,
        declared,
        "render output",
        "render again to an authorized export folder, then re-run output checks",
    )?;
    let resolved_plain = record_recovery::is_plain_regular_file(&output).map_err(|error| {
        CutError::new(
            error_codes::IO,
            "could not inspect the resolved render output",
            error.to_string(),
        )
    })?;
    if !resolved_plain {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "resolved render output is not a local regular file",
            format!("refusing unsafe output path {}", output.display()),
        )
        .with_suggested_action("render again before re-running output checks"));
    }
    if expected.is_some_and(|expected| expected != output) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "rendered output no longer resolves to the receipt-bound artifact",
            format!(
                "receipt output now resolves to {}; expected {}",
                output.display(),
                expected.unwrap().display()
            ),
        )
        .with_suggested_action("render again before re-running output checks"));
    }
    Ok(output)
}
