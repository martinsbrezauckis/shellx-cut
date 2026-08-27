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
pub(super) async fn remove_owned_cache_outputs(
    state: &AppState,
    project_dir: &Path,
    asset: &str,
    outputs: [(crate::cache_lifecycle::CacheKind, Option<&str>); 2],
    freed: &mut Vec<String>,
    warnings: &mut Vec<cut_core::VerbWarning>,
) {
    if outputs.iter().all(|(_, relative)| relative.is_none()) {
        return;
    }
    let _cache_lease = state.cache_lifecycle_lease.write().await;
    for (kind, relative) in outputs {
        let Some(relative) = relative else {
            continue;
        };
        match crate::cache_lifecycle::remove_owned_output(project_dir, kind, asset, relative) {
            Ok(crate::cache_lifecycle::OwnedRemoval::Retired) => freed.push(relative.into()),
            Ok(crate::cache_lifecycle::OwnedRemoval::UnlinkedLedgerPending(error)) => {
                freed.push(relative.into());
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
