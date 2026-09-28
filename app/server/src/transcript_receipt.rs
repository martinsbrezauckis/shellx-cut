//! Read the deterministic, project-owned word receipt named by an asset.

use std::path::Path;

use cut_core::{error_codes, CutError};

// Long recordings can have large word lists. Keep the same generous bound used
// for media-evidence receipts while rejecting hostile files before JSON parsing.
const MAX_TRANSCRIPT_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn read(
    project_dir: &Path,
    asset_id: &str,
    relative: &str,
) -> Result<Vec<u8>, CutError> {
    let expected = format!("receipts/{asset_id}.words.json");
    if relative != expected {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "transcript receipt path does not match its asset",
            "the cached transcript pointer must name this asset's project-local receipt",
        )
        .with_suggested_action("re-run media.transcribe for this asset"));
    }
    let path = crate::output_paths::resolve_existing_project_file(
        project_dir,
        &expected,
        "transcript receipt",
        "re-run media.transcribe for this asset",
    )?;
    crate::vissearch::read_bounded_file(&path, MAX_TRANSCRIPT_BYTES).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "transcript receipt cannot be read safely",
            error.to_string(),
        )
        .with_suggested_action("re-run media.transcribe for this asset")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_bounded_receipt_for_the_selected_asset() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("test.cutproj");
        std::fs::create_dir_all(project.join("receipts")).unwrap();
        std::fs::write(project.join("receipts/a1.words.json"), b"{}").unwrap();
        std::fs::write(root.path().join("outside.json"), b"secret").unwrap();

        assert_eq!(
            read(&project, "a1", "receipts/a1.words.json").unwrap(),
            b"{}"
        );
        assert!(read(&project, "a1", "../outside.json").is_err());
        assert!(read(
            &project,
            "a1",
            root.path().join("outside.json").to_str().unwrap()
        )
        .is_err());

        let receipt = std::fs::File::create(project.join("receipts/a2.words.json")).unwrap();
        receipt.set_len(MAX_TRANSCRIPT_BYTES + 1).unwrap();
        assert!(read(&project, "a2", "receipts/a2.words.json").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn linked_receipt_cannot_read_outside_the_project() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("test.cutproj");
        std::fs::create_dir_all(project.join("receipts")).unwrap();
        let outside = root.path().join("outside.json");
        std::fs::write(&outside, b"{}").unwrap();
        std::os::unix::fs::symlink(outside, project.join("receipts/a1.words.json")).unwrap();
        assert!(read(&project, "a1", "receipts/a1.words.json").is_err());
    }
}
