//! Verified work-unit accounting for cache rebuild admission.
//!
//! Keep this separate from scheduling and worker publication so the estimate
//! never grows into a speculative duration predictor or a second job path.

use super::super::CacheKind;
use super::RebuildAssetPlan;
use serde_json::{json, Value};
use std::path::Path;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy)]
pub(super) struct SourceCheck {
    pub(super) matches: bool,
    pub(super) readable: bool,
    pub(super) bytes: Option<u64>,
}

/// Portable source metadata taken around the content hash. A stable size and
/// modification timestamp make the reported bytes belong to the same ordinary
/// filesystem observation as the hash; the rebuild worker still hashes again
/// before it publishes any derived output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceSnapshot {
    bytes: u64,
    modified: SystemTime,
}

impl SourceCheck {
    pub(super) fn verify(source: &Path, expected_hash: &str) -> Self {
        let before = source_snapshot(source);
        let actual = cut_core::hash_file(source).ok();
        let after = source_snapshot(source);
        Self::from_observations(before, actual.as_deref(), after, expected_hash)
    }

    fn from_observations(
        before: Option<SourceSnapshot>,
        actual_hash: Option<&str>,
        after: Option<SourceSnapshot>,
        expected_hash: &str,
    ) -> Self {
        let stable = before.zip(after).filter(|(before, after)| before == after);
        let readable = actual_hash.is_some() && stable.is_some();
        Self {
            matches: readable && actual_hash == Some(expected_hash),
            readable,
            bytes: readable.then(|| {
                stable
                    .expect("readable requires stable source metadata")
                    .0
                    .bytes
            }),
        }
    }
}

fn source_snapshot(source: &Path) -> Option<SourceSnapshot> {
    let metadata = std::fs::metadata(source).ok()?;
    Some(SourceSnapshot {
        bytes: metadata.len(),
        modified: metadata.modified().ok()?,
    })
}

/// Cost is expressed as verified work units instead of a fragile wall-clock
/// promise. Source content can still change before worker publication, where
/// the worker's existing second identity check will refuse it.
#[derive(Default)]
pub(super) struct RebuildEstimate {
    assets: u64,
    verified_source_bytes: u64,
    proxy_outputs: u64,
    filmstrip_outputs: u64,
    proxy_duration_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(bytes: u64, seconds: u64) -> SourceSnapshot {
        SourceSnapshot {
            bytes,
            modified: std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds),
        }
    }

    #[test]
    fn source_check_never_pairs_a_hash_with_replaced_file_metadata() {
        let stable = SourceCheck::from_observations(
            Some(snapshot(42, 1)),
            Some("sha256:source"),
            Some(snapshot(42, 1)),
            "sha256:source",
        );
        assert!(stable.readable);
        assert!(stable.matches);
        assert_eq!(stable.bytes, Some(42));

        let replaced = SourceCheck::from_observations(
            Some(snapshot(42, 1)),
            Some("sha256:source"),
            Some(snapshot(64, 2)),
            "sha256:source",
        );
        assert!(!replaced.readable);
        assert!(!replaced.matches);
        assert_eq!(replaced.bytes, None);
    }
}

impl RebuildEstimate {
    pub(super) fn from_plans(plans: &[RebuildAssetPlan]) -> Self {
        let mut estimate = Self::default();
        for plan in plans {
            estimate.assets = estimate.assets.saturating_add(1);
            estimate.verified_source_bytes = estimate
                .verified_source_bytes
                .saturating_add(plan.source_bytes);
            for (kind, _) in &plan.outputs {
                match kind {
                    CacheKind::Proxies => {
                        estimate.proxy_outputs = estimate.proxy_outputs.saturating_add(1);
                        estimate.proxy_duration_ms = estimate
                            .proxy_duration_ms
                            .saturating_add(plan.asset.duration_ms.unwrap_or_default());
                    }
                    CacheKind::Thumbnails => {
                        estimate.filmstrip_outputs = estimate.filmstrip_outputs.saturating_add(1)
                    }
                }
            }
        }
        estimate
    }

    pub(super) fn public(&self) -> Value {
        json!({
            "basis": "source_hash_match_and_current_import_metadata",
            "assets": self.assets,
            "verified_source_bytes": self.verified_source_bytes,
            "proxy_outputs": self.proxy_outputs,
            "filmstrip_outputs": self.filmstrip_outputs,
            "proxy_duration_ms": self.proxy_duration_ms,
        })
    }
}
