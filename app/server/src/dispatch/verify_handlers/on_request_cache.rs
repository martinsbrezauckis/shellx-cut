//! Bounded durable cache for standalone on-request checks (`verify.scopes`,
//! `verify.loudness`, `verify.delivery`, `render.qc`).
//!
//! Each check writes its own project-local `receipts/check_*.json` ledger via
//! bounded atomic I/O so both passed/flagged outcomes and engine errors survive
//! restarts without modifying timeline ops or source perception caches.

use super::*;
use cut_core::store::{read_bounded_cache, BoundedCacheRead};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

const MAX_CACHE_BYTES: u64 = 1024 * 1024;
static CACHE_WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SignalScopesCache {
    pub ts: String,
    pub at_op: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_hash: Option<String>,
    pub at_ms: u64,
    pub pass: bool,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CutError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SourceLoudnessEntry {
    pub asset_id: String,
    pub asset_hash: String,
    pub target_lufs: f64,
    pub integrated_lufs: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
    pub within_tolerance: bool,
    pub true_peak_ok: bool,
    #[serde(default)]
    pub recommendation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CutError>,
    pub ts: String,
    pub at_op: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SourceLoudnessCache {
    #[serde(default)]
    pub assets: BTreeMap<String, SourceLoudnessEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeliveryCache {
    pub ts: String,
    pub at_op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
    #[serde(default)]
    pub asset_hashes: BTreeMap<String, String>,
    pub word_count: usize,
    pub pass: bool,
    #[serde(default)]
    pub flags: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wpm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filler_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CutError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReframeQcCache {
    pub ts: String,
    pub at_op: String,
    pub reframe_id: String,
    pub review_count: usize,
    pub scene_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<CutError>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct LoadedOnRequestCaches {
    pub signal_scopes: Option<SignalScopesCache>,
    pub source_loudness: SourceLoudnessCache,
    pub delivery: Option<DeliveryCache>,
    pub reframe_qc: Option<ReframeQcCache>,
}

fn read_cache<T: serde::de::DeserializeOwned>(receipts: &Path, name: &str) -> Option<T> {
    let path = receipts.join(name);
    let BoundedCacheRead::Bytes(bytes) = read_bounded_cache(&path, MAX_CACHE_BYTES) else {
        return None;
    };
    serde_json::from_slice(&bytes).ok()
}

fn write_cache<T: Serialize>(project_dir: &Path, name: &str, value: &T) {
    let Ok(receipts) = super::rendering::checked_internal_dir(project_dir, Path::new("receipts"))
    else {
        return;
    };
    let path = receipts.join(name);
    if let Ok(text) = serde_json::to_string_pretty(value) {
        if write_output_atomic(&path, text).is_ok() {
            cut_media::cache_trust::remember(&path);
        }
    }
}

pub(crate) fn load_all(receipts: &Path) -> LoadedOnRequestCaches {
    LoadedOnRequestCaches {
        signal_scopes: read_cache(receipts, "check_signal_scopes.json"),
        source_loudness: read_cache(receipts, "check_source_loudness.json").unwrap_or_default(),
        delivery: read_cache(receipts, "check_delivery.json"),
        reframe_qc: read_cache(receipts, "check_reframe_qc.json"),
    }
}

pub(crate) fn record_signal_scopes(project_dir: &Path, entry: SignalScopesCache) {
    let _guard = CACHE_WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    write_cache(project_dir, "check_signal_scopes.json", &entry);
}

pub(crate) fn record_source_loudness(project_dir: &Path, entry: SourceLoudnessEntry) {
    let _guard = CACHE_WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut cache: SourceLoudnessCache =
        super::rendering::checked_internal_dir(project_dir, Path::new("receipts"))
            .ok()
            .and_then(|r| read_cache(&r, "check_source_loudness.json"))
            .unwrap_or_default();
    cache.assets.insert(entry.asset_id.clone(), entry);
    write_cache(project_dir, "check_source_loudness.json", &cache);
}

pub(crate) fn record_delivery(project_dir: &Path, entry: DeliveryCache) {
    let _guard = CACHE_WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    write_cache(project_dir, "check_delivery.json", &entry);
}

pub(crate) fn record_reframe_qc(project_dir: &Path, entry: ReframeQcCache) {
    let _guard = CACHE_WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    write_cache(project_dir, "check_reframe_qc.json", &entry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_request_caches_persist_outcomes_and_errors_without_cross_interference() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = dir.path().join("receipts");
        std::fs::create_dir_all(&receipts).unwrap();

        record_signal_scopes(
            dir.path(),
            SignalScopesCache {
                ts: "2026-11-24T00:00:00Z".into(),
                at_op: "op_000001".into(),
                scope: "timeline".into(),
                asset_id: None,
                asset_hash: None,
                at_ms: 0,
                pass: false,
                flags: vec!["clipped_highlights".into()],
                frame: None,
                error: None,
            },
        );
        record_source_loudness(
            dir.path(),
            SourceLoudnessEntry {
                asset_id: "a1".into(),
                asset_hash: "h1".into(),
                target_lufs: -14.0,
                integrated_lufs: None,
                true_peak_dbtp: None,
                within_tolerance: false,
                true_peak_ok: false,
                recommendation: String::new(),
                error: Some(CutError::new(
                    error_codes::FFMPEG,
                    "loudness probe failed",
                    "corrupt audio stream",
                )),
                ts: "2026-11-24T00:00:00Z".into(),
                at_op: "op_000001".into(),
            },
        );
        record_source_loudness(
            dir.path(),
            SourceLoudnessEntry {
                asset_id: "a2".into(),
                asset_hash: "h2".into(),
                target_lufs: -14.0,
                integrated_lufs: Some(-14.2),
                true_peak_dbtp: Some(-1.5),
                within_tolerance: true,
                true_peak_ok: true,
                recommendation: "ok".into(),
                error: None,
                ts: "2026-11-24T00:00:01Z".into(),
                at_op: "op_000001".into(),
            },
        );
        record_delivery(
            dir.path(),
            DeliveryCache {
                ts: "2026-11-24T00:00:00Z".into(),
                at_op: "op_000001".into(),
                asset: Some("a1".into()),
                asset_hashes: [("a1".into(), "h1".into())].into_iter().collect(),
                word_count: 100,
                pass: true,
                flags: serde_json::json!({}),
                wpm: Some(140.0),
                filler_pct: Some(0.0),
                error: None,
            },
        );

        let loaded = load_all(&receipts);
        assert_eq!(
            loaded.signal_scopes.unwrap().flags,
            vec!["clipped_highlights".to_string()]
        );
        assert_eq!(
            loaded.source_loudness.assets["a1"]
                .error
                .as_ref()
                .map(|e| e.message.as_str()),
            Some("loudness probe failed")
        );
        assert!(loaded.source_loudness.assets["a2"].within_tolerance);
        assert!(loaded.delivery.unwrap().pass);
    }

    #[cfg(unix)]
    #[test]
    fn planted_symlink_on_request_cache_is_never_read() {
        let dir = tempfile::tempdir().unwrap();
        let receipts = dir.path().join("receipts");
        std::fs::create_dir_all(&receipts).unwrap();
        let outside = dir.path().join("outside.json");
        std::fs::write(
            &outside,
            br#"{"ts":"x","at_op":"op_1","scope":"timeline","at_ms":0,"pass":true}"#,
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, receipts.join("check_signal_scopes.json")).unwrap();
        let loaded = load_all(&receipts);
        assert!(loaded.signal_scopes.is_none());
    }
}
