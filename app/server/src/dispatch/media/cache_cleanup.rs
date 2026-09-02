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
