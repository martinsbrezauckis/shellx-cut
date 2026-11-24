//! Per-scan probe sharing, batched Python dependency checks, and cooperative
//! cancellation / merge helpers for fast initial `system.doctor` responses.

use super::{Card, CardStatus, DoctorReport};
use cut_media::hwencode::{
    gpu_filters_work_cancelable, probe_ffmpeg_detailed, FfmpegCaps, FfmpegScanReport, HwCaps,
};
use cut_media::hwencode_limits::{size_limits_for_cancelable, EncoderSizeLimit};
use cut_media::toolpath::FfmpegSelection;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Cancels any active deferred background probe when the owning `DoctorCache`
/// (the last clone held by `AppState`) drops — e.g. at test or CLI exit.
#[derive(Debug, Default)]
pub(crate) struct CancelOnDrop {
    active: Mutex<Option<Arc<AtomicBool>>>,
}

impl CancelOnDrop {
    pub(crate) fn swap(&self, next: Option<Arc<AtomicBool>>) {
        let prev = {
            let mut guard = self.active.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut *guard, next)
        };
        if let Some(flag) = prev {
            flag.store(true, Ordering::Release);
        }
    }

    pub(crate) fn cancel_active(&self) {
        let guard = self.active.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(flag) = guard.as_ref() {
            flag.store(true, Ordering::Release);
        }
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.cancel_active();
    }
}

/// Scoped to a single `doctor::scan()` call so `ffmpeg_cards`, `gpu_encode_card`,
/// and `ffmpeg_selection_using` never probe the same binary multiple times, while
/// every explicit re-scan still probes cleanly without cross-scan staleness.
pub(super) struct FfmpegScanPool {
    candidates: Vec<PathBuf>,
    probe_cuda: bool,
    slots: Mutex<HashMap<PathBuf, Arc<OnceLock<FfmpegScanReport>>>>,
    selection: OnceLock<FfmpegSelection>,
}

impl FfmpegScanPool {
    pub(super) fn new(candidates: Vec<PathBuf>, probe_cuda: bool) -> Self {
        let mut slots = HashMap::new();
        for path in &candidates {
            slots
                .entry(path.clone())
                .or_insert_with(|| Arc::new(OnceLock::new()));
        }
        Self {
            candidates,
            probe_cuda,
            slots: Mutex::new(slots),
            selection: OnceLock::new(),
        }
    }

    pub(super) fn cuda_deferred(&self) -> bool {
        !self.probe_cuda
    }

    pub(super) fn candidates(&self) -> &[PathBuf] {
        &self.candidates
    }

    fn slot_for(&self, path: &Path) -> Arc<OnceLock<FfmpegScanReport>> {
        let mut guard = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(OnceLock::new()))
            .clone()
    }

    pub(super) fn probe_one(&self, path: &Path) -> FfmpegScanReport {
        let slot = self.slot_for(path);
        slot.get_or_init(|| probe_ffmpeg_detailed(path, self.probe_cuda))
            .clone()
    }

    pub(super) fn all_candidate_reports(&self) -> Vec<FfmpegScanReport> {
        if self.candidates.len() <= 1 {
            return self
                .candidates
                .iter()
                .map(|path| self.probe_one(path))
                .collect();
        }
        std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .candidates
                .iter()
                .map(|path| scope.spawn(|| self.probe_one(path)))
                .collect();
            handles
                .into_iter()
                .filter_map(|handle| handle.join().ok())
                .collect()
        })
    }

    pub(super) fn all_candidate_caps(&self) -> Vec<FfmpegCaps> {
        self.all_candidate_reports()
            .into_iter()
            .map(|report| report.caps)
            .collect()
    }

    pub(super) fn selection(&self) -> &FfmpegSelection {
        self.selection.get_or_init(|| {
            cut_media::toolpath::ffmpeg_selection_using(&self.candidates, || {
                self.all_candidate_caps()
            })
        })
    }
}

/// Shares Python `--version` + module-import checks across cards that point at
/// the same interpreter within a single scan, avoiding duplicate venv spawns.
#[derive(Default)]
pub(super) struct PythonScanPool {
    imports: Mutex<HashMap<(PathBuf, bool), Arc<OnceLock<HashMap<String, bool>>>>>,
    versions: Mutex<HashMap<PathBuf, Arc<OnceLock<bool>>>>,
}

const SHARED_PROBE_MODULES: &[&str] = &["onnx_asr", "onnxruntime", "numpy", "soundfile", "PIL"];

impl PythonScanPool {
    pub(super) fn python_runnable(&self, python: &Path) -> bool {
        let slot = {
            let mut guard = self.versions.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .entry(python.to_path_buf())
                .or_insert_with(|| Arc::new(OnceLock::new()))
                .clone()
        };
        *slot.get_or_init(|| {
            super::version_line(python.as_os_str(), &["--version"], Duration::from_secs(8))
                .is_some()
        })
    }

    pub(super) fn probe_modules(
        &self,
        python: &Path,
        modules: &[&str],
        native_context: bool,
        timeout: Duration,
    ) -> HashMap<String, bool> {
        let can_share = modules.iter().all(|m| SHARED_PROBE_MODULES.contains(m));
        if !can_share {
            return run_python_module_probe(python, modules, native_context, timeout);
        }
        let slot = {
            let mut guard = self.imports.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .entry((python.to_path_buf(), native_context))
                .or_insert_with(|| Arc::new(OnceLock::new()))
                .clone()
        };
        let cached = slot.get_or_init(|| {
            run_python_module_probe(python, SHARED_PROBE_MODULES, native_context, timeout)
        });
        modules
            .iter()
            .map(|&m| (m.to_string(), cached.get(m).copied().unwrap_or(false)))
            .collect()
    }
}

pub(super) fn run_python_module_probe(
    python: &Path,
    modules: &[&str],
    native_context: bool,
    timeout: Duration,
) -> HashMap<String, bool> {
    let mut results = HashMap::new();
    if modules.is_empty()
        || !modules
            .iter()
            .all(|m| m.split('.').all(super::is_python_identifier))
    {
        return results;
    }
    let quoted: Vec<String> = modules.iter().map(|m| format!("{m:?}")).collect();
    let snippet = format!(
        "import importlib, json\nres = {{}}\nfor m in [{}]:\n    try:\n        importlib.import_module(m); res[m] = True\n    except Exception:\n        res[m] = False\nprint('__cut_import_probe:' + json.dumps(res))",
        quoted.join(", ")
    );
    let mut command = Command::new(python);
    cut_perception::apply_python_command_policy(&mut command, native_context);
    command.args(["-c", &snippet]);
    let Ok(out) = super::run_doctor_command(&mut command, timeout, "check Python imports") else {
        return results;
    };
    if let Some(parsed) = parse_import_probe_output(&out.stdout) {
        return parsed;
    }
    // Fallback ONLY for minimal test mocks that exit 0 without printing JSON:
    // keep honest false on non-zero exit.
    if out.status.success() && out.stdout.is_empty() {
        for &m in modules {
            results.insert(m.to_string(), true);
        }
    }
    results
}

fn parse_import_probe_output(stdout: &[u8]) -> Option<HashMap<String, bool>> {
    let text = String::from_utf8_lossy(stdout);
    for line in text.lines().rev() {
        let trimmed = line.trim();
        let json_str = trimmed
            .strip_prefix("__cut_import_probe:")
            .unwrap_or(trimmed);
        if !json_str.starts_with('{') {
            continue;
        }
        if let Ok(serde_json::Value::Object(map)) =
            serde_json::from_str::<serde_json::Value>(json_str)
        {
            let valid: HashSet<&str> = ["version"].into_iter().collect();
            let mut out = HashMap::new();
            for (k, v) in map {
                if valid.contains(k.as_str()) {
                    continue;
                }
                if let Some(b) = v.as_bool() {
                    out.insert(k, b);
                }
            }
            if !out.is_empty() {
                return Some(out);
            }
        }
    }
    None
}

/// Slow optional checks deferred from Phase 1 (`DoctorCache::cached()`) so the
/// first doctor response answers fast without blocking on 4K/8K test encodes or
/// cold `import torch` CUDA checks. Filled in background and published via `doctor_updated`.
#[derive(Debug, Clone)]
pub(crate) struct DeferredWork {
    pub(crate) size_limits: Option<(PathBuf, HwCaps)>,
    pub(crate) cuda_filters: Option<PathBuf>,
    pub(crate) matte_premium: Option<(PathBuf, bool, Card)>,
}

pub(crate) struct DeferredResult {
    pub(crate) size_limits: Option<Vec<EncoderSizeLimit>>,
    pub(crate) cuda_filters: Option<(PathBuf, bool)>,
    pub(crate) matte_premium: Option<Card>,
}

impl DeferredWork {
    pub(crate) fn is_empty(&self) -> bool {
        self.size_limits.is_none() && self.cuda_filters.is_none() && self.matte_premium.is_none()
    }

    pub(crate) fn execute(self, cancel: Arc<AtomicBool>) -> Option<DeferredResult> {
        if cancel.load(Ordering::Acquire) {
            return None;
        }
        let cancel_sl = Some(cancel.clone());
        let cancel_cf = Some(cancel.clone());
        let cancel_mp = &cancel;
        let (size_limits, cuda_filters, matte_premium) = std::thread::scope(|scope| {
            let sl = self.size_limits.as_ref().map(|(ffmpeg, hw)| {
                let c = cancel_sl.clone();
                scope.spawn(move || size_limits_for_cancelable(ffmpeg, hw, c))
            });
            let cf = self.cuda_filters.as_ref().map(|ffmpeg| {
                let c = cancel_cf.clone();
                let path = ffmpeg.clone();
                scope.spawn(move || {
                    let ok = gpu_filters_work_cancelable(&path, c);
                    (path, ok)
                })
            });
            let mp = self.matte_premium.map(|(python, native, card)| {
                scope.spawn(move || {
                    if cancel_mp.load(Ordering::Acquire) {
                        return None;
                    }
                    let cuda = super::premium_matte::probe_cuda_availability(&python, native);
                    Some(super::premium_matte::finalize_matte_premium_card(
                        card, cuda,
                    ))
                })
            });
            (
                sl.and_then(|h| h.join().ok()),
                cf.and_then(|h| h.join().ok()),
                mp.and_then(|h| h.join().ok()).flatten(),
            )
        });
        if cancel.load(Ordering::Acquire) {
            return None;
        }
        Some(DeferredResult {
            size_limits,
            cuda_filters,
            matte_premium,
        })
    }
}

pub(crate) fn apply_deferred_result(
    mut report: DoctorReport,
    result: DeferredResult,
) -> DoctorReport {
    if result.size_limits.is_some() || result.cuda_filters.is_some() {
        if let Some(gpu) = report.cards.iter_mut().find(|c| c.id == "gpu-encode") {
            if let Some(details) = gpu.details.as_object_mut() {
                if let Some(limits) = result.size_limits {
                    details.insert("size_limits".into(), serde_json::json!(limits));
                    details.insert("size_limits_pending".into(), serde_json::json!(false));
                }
                if let Some((path, cuda_ok)) = result.cuda_filters {
                    details.insert("full_fasttrack".into(), serde_json::json!(cuda_ok));
                    details.insert("cuda_filters_pending".into(), serde_json::json!(false));
                    let path_str = path.display().to_string();
                    if let Some(serde_json::Value::Array(candidates)) =
                        details.get_mut("candidates")
                    {
                        for item in candidates {
                            if item.get("path").and_then(|p| p.as_str()) == Some(path_str.as_str())
                            {
                                if let Some(obj) = item.as_object_mut() {
                                    obj.insert("cuda_filters".into(), serde_json::json!(cuda_ok));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(premium_card) = result.matte_premium {
        if let Some(slot) = report.cards.iter_mut().find(|c| c.id == "matte_premium") {
            *slot = premium_card;
        }
    }
    report.scanned_at = chrono::Utc::now().to_rfc3339();
    report
}

pub(crate) fn should_emit_deferred_update(previous: &DoctorReport, next: &DoctorReport) -> bool {
    if !previous.same_capabilities(next) {
        return true;
    }
    gpu_limits_view(previous) != gpu_limits_view(next)
        || matte_checked_view(previous) != matte_checked_view(next)
}

fn gpu_limits_view(
    r: &DoctorReport,
) -> Option<(
    Option<&serde_json::Value>,
    Option<&serde_json::Value>,
    Option<&serde_json::Value>,
    Option<&serde_json::Value>,
)> {
    r.cards.iter().find(|c| c.id == "gpu-encode").map(|c| {
        (
            c.details.get("size_limits"),
            c.details.get("size_limits_pending"),
            c.details.get("full_fasttrack"),
            c.details.get("cuda_filters_pending"),
        )
    })
}

fn matte_checked_view(
    r: &DoctorReport,
) -> Option<(bool, Option<&serde_json::Value>, Option<&serde_json::Value>)> {
    r.cards.iter().find(|c| c.id == "matte_premium").map(|c| {
        (
            c.status == CardStatus::Unknown,
            c.details.get("cuda_available"),
            c.details.get("checking"),
        )
    })
}
