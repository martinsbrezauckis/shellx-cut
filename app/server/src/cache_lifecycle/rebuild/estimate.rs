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
pub(crate) struct SourceCheck {
    pub(crate) matches: bool,
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
    pub(crate) fn verify(source: &Path, expected_hash: &str) -> Self {
        let before = source_snapshot(source);
        if before.is_none() {
            return Self::from_observations(None, None, None, expected_hash);
        }
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
    if !metadata.is_file() {
        return None;
    }
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

    #[cfg(unix)]
    #[test]
    fn special_source_is_rejected_without_opening_a_pipe() {
        use std::os::unix::ffi::OsStrExt;
        const CHILD: &str = "SHELLX_CUT_SOURCE_PIPE_CHECK_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let root = tempfile::tempdir().unwrap();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "cache_lifecycle::rebuild::estimate::tests::special_source_is_rejected_without_opening_a_pipe", "--nocapture"])
                .env(CHILD, root.path()).spawn().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success(), "special source admission failed");
                    return;
                }
                if std::time::Instant::now() >= deadline {
                    child.kill().unwrap();
                    child.wait().unwrap();
                    panic!("source identity opened a pipe instead of refusing it");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        let root = std::path::PathBuf::from(std::env::var_os(CHILD).unwrap());
        let path = root.join("source.pipe");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: the path is a live NUL-terminated byte string in this test's private directory.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let check = SourceCheck::verify(&path, "sha256:expected");
        assert!(!check.readable);
        assert!(!check.matches);
        assert_eq!(check.bytes, None);
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
