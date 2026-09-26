//! Exact, serialized proxy/filmstrip cleanup after a committed media mutation.

use super::*;
use std::path::Path;

fn output_label(kind: crate::cache_lifecycle::CacheKind) -> &'static str {
    match kind {
        crate::cache_lifecycle::CacheKind::Proxies => "proxy",
        crate::cache_lifecycle::CacheKind::Thumbnails => "filmstrip",
    }
}

fn cache_cleanup_warning(
    kind: crate::cache_lifecycle::CacheKind,
    asset: &str,
    action: &str,
    error: CutError,
) -> cut_core::VerbWarning {
    cut_core::VerbWarning {
        code: "cache_ownership_cleanup_pending".into(),
        message: format!(
            "{action} {} cache for asset '{asset}', but cache ownership cleanup needs attention: {} ({})",
            output_label(kind),
            error.message,
            error.cause
        ),
        detail: Default::default(),
    }
}

/// Remove proxy and filmstrip outputs through the ownership ledger. Callers
/// must have dropped `AppState::project`'s write guard: this exclusive cache
/// lease serializes its read-modify-write ledger update against all cache
/// producers and other retirement requests.
pub(super) fn remove_owned_cache_outputs_locked(
    project_dir: &Path,
    asset: &str,
    outputs: impl IntoIterator<Item = (crate::cache_lifecycle::CacheKind, Option<String>)>,
    freed: &mut Vec<String>,
    warnings: &mut Vec<cut_core::VerbWarning>,
) {
    let mut seen: Vec<(crate::cache_lifecycle::CacheKind, String)> = Vec::new();
    for (kind, relative) in outputs {
        let Some(relative) = relative else {
            continue;
        };
        if seen
            .iter()
            .any(|(seen_kind, seen_relative)| *seen_kind == kind && seen_relative == &relative)
        {
            continue;
        }
        seen.push((kind, relative.clone()));
        match crate::cache_lifecycle::remove_owned_output(project_dir, kind, asset, &relative) {
            Ok(crate::cache_lifecycle::OwnedRemoval::Retired) => freed.push(relative),
            Ok(crate::cache_lifecycle::OwnedRemoval::LedgerRetiredMissing) => {
                warnings.push(cache_cleanup_warning(
                    kind,
                    asset,
                    "retired the missing ownership record for",
                    CutError::new(
                        cut_core::error_codes::CONFLICT,
                        "cache file was already absent",
                        "the pending rebuild reservation was removed without deleting a file",
                    ),
                ))
            }
            Ok(crate::cache_lifecycle::OwnedRemoval::UnlinkedLedgerPending(error)) => {
                freed.push(relative);
                warnings.push(cache_cleanup_warning(
                    kind,
                    asset,
                    "removed the exact",
                    error,
                ));
            }
            Err(error) => warnings.push(cache_cleanup_warning(kind, asset, "kept the", error)),
        }
    }
}

/// Retire only the receipt names generated for this asset. Cached transcript
/// and perception pointers come from project.json and are untrusted input;
/// joining one directly to the project directory could unlink an unrelated
/// file. The literal receipts directory and each leaf must be plain entries.
pub(super) fn remove_asset_receipts(
    project_dir: &Path,
    asset: &str,
    transcript: Option<&str>,
    perception: Option<&str>,
    freed: &mut Vec<String>,
    warnings: &mut Vec<cut_core::VerbWarning>,
) {
    if asset.is_empty()
        || asset.len() > 128
        || !asset
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        warnings.push(receipt_warning(asset, "invalid asset id"));
        return;
    }
    let expected_words = format!("receipts/{asset}.words.json");
    let expected_perception = format!("receipts/{asset}.perception.json");
    let expected_probe = format!("receipts/{asset}.probe.json");
    for (pointer, expected) in [
        (transcript, expected_words.as_str()),
        (perception, expected_perception.as_str()),
    ] {
        if pointer.is_some_and(|value| value != expected) {
            warnings.push(receipt_warning(asset, "unrecognized derived receipt path"));
        }
    }
    let receipts = match crate::output_paths::existing_plain_project_relative_dir(
        project_dir,
        Path::new("receipts"),
    ) {
        Ok(path) => path,
        Err(_) => {
            warnings.push(receipt_warning(asset, "receipts directory is not plain"));
            return;
        }
    };
    for relative in [
        transcript.filter(|value| *value == expected_words.as_str()),
        perception.filter(|value| *value == expected_perception.as_str()),
        Some(expected_probe.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        let Some(name) = Path::new(relative).file_name() else {
            continue;
        };
        let path = receipts.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if plain_receipt_file(&metadata) => {
                if std::fs::remove_file(&path).is_ok() {
                    freed.push(relative.to_string());
                }
            }
            Ok(_) => warnings.push(receipt_warning(
                asset,
                "derived receipt is not a plain file",
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => warnings.push(receipt_warning(
                asset,
                "derived receipt could not be inspected",
            )),
        }
    }
}

fn receipt_warning(asset: &str, cause: &str) -> cut_core::VerbWarning {
    cut_core::VerbWarning {
        code: "derived_receipt_cleanup_pending".into(),
        message: format!("kept derived receipt for asset '{asset}': {cause}"),
        detail: Default::default(),
    }
}

fn plain_receipt_file(metadata: &std::fs::Metadata) -> bool {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return false;
        }
    }
    true
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn receipt_retirement_keeps_untrusted_pointer_and_link_targets() {
        let scratch = tempfile::tempdir().unwrap();
        let project = scratch.path().join("project.cutproj");
        let receipts = project.join("receipts");
        std::fs::create_dir_all(&receipts).unwrap();
        let outside = scratch.path().join("outside.json");
        std::fs::write(&outside, b"keep me").unwrap();
        symlink(&outside, receipts.join("a1.perception.json")).unwrap();
        let valid = receipts.join("a1.words.json");
        std::fs::write(&valid, b"retire me").unwrap();
        let mut freed = Vec::new();
        let mut warnings = Vec::new();

        remove_asset_receipts(
            &project,
            "a1",
            Some("receipts/a1.words.json"),
            Some("../outside.json"),
            &mut freed,
            &mut warnings,
        );
        assert!(!valid.exists());
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep me");
        assert!(!warnings.is_empty());
        assert_eq!(freed, ["receipts/a1.words.json"]);

        remove_asset_receipts(
            &project,
            "a1",
            None,
            Some("receipts/a1.perception.json"),
            &mut freed,
            &mut warnings,
        );
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep me");
        assert!(std::fs::symlink_metadata(receipts.join("a1.perception.json")).is_ok());
    }
}
