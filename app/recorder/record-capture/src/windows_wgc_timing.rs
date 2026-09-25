//! Bounded, private WGC clock comparison. This observes timing only; it never
//! changes a capture boundary, encoded frame, or checkpoint admission.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::Serialize;

#[derive(Clone)]
pub(crate) struct WgcTimingRecorder {
    origin: Instant,
    sidecar: PathBuf,
    summary: Arc<Mutex<WgcTimingSummary>>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
struct FrameTiming {
    native_timestamp_100ns: i64,
    capture_clock_elapsed_ns: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WgcTimingSummary {
    schema: &'static str,
    requested_fps: u32,
    accepted_frames: u64,
    first: Option<FrameTiming>,
    last: Option<FrameTiming>,
    native_timestamp_regressions: u64,
    max_native_gap_100ns: i64,
    close_callback_elapsed_ns: Option<u64>,
    encoder_finish_return_elapsed_ns: Option<u64>,
    encoder_finish_ok: Option<bool>,
    control_stop_return_elapsed_ns: Option<u64>,
    control_stop_ok: Option<bool>,
}

impl WgcTimingRecorder {
    pub(crate) fn new(origin: Instant, requested_fps: u32, staging: &Path) -> Self {
        // The native `s.mp4` lives in a random private staging directory that
        // publication removes. Keep the observation in its owning capture root.
        let stage_dir = staging.parent().expect("WGC stage has an owning directory");
        let capture_root = stage_dir.parent().expect("WGC stage has a capture root");
        let stage_name = stage_dir.file_name().expect("WGC stage is named");
        let mut sidecar_name = stage_name.to_os_string();
        sidecar_name.push(".wgc-timing.json");
        Self {
            origin,
            sidecar: capture_root.join(PathBuf::from(sidecar_name)),
            summary: Arc::new(Mutex::new(WgcTimingSummary {
                schema: "shellx-cut/wgc-timing-observation/1",
                requested_fps,
                accepted_frames: 0,
                first: None,
                last: None,
                native_timestamp_regressions: 0,
                max_native_gap_100ns: 0,
                close_callback_elapsed_ns: None,
                encoder_finish_return_elapsed_ns: None,
                encoder_finish_ok: None,
                control_stop_return_elapsed_ns: None,
                control_stop_ok: None,
            })),
        }
    }

    fn elapsed_ns(&self, at: Instant) -> u64 {
        u64::try_from(at.saturating_duration_since(self.origin).as_nanos()).unwrap_or(u64::MAX)
    }

    pub(crate) fn accepted_frame(&self, native_timestamp_100ns: i64, at: Instant) {
        let sample = FrameTiming {
            native_timestamp_100ns,
            capture_clock_elapsed_ns: self.elapsed_ns(at),
        };
        let mut summary = self
            .summary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = summary.last {
            if native_timestamp_100ns < previous.native_timestamp_100ns {
                summary.native_timestamp_regressions += 1;
            } else {
                summary.max_native_gap_100ns = summary
                    .max_native_gap_100ns
                    .max(native_timestamp_100ns.saturating_sub(previous.native_timestamp_100ns));
            }
        } else {
            summary.first = Some(sample);
        }
        summary.last = Some(sample);
        summary.accepted_frames = summary.accepted_frames.saturating_add(1);
    }

    pub(crate) fn close_callback(&self, at: Instant) {
        self.summary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .close_callback_elapsed_ns = Some(self.elapsed_ns(at));
    }

    pub(crate) fn encoder_finished(&self, at: Instant, ok: bool) {
        let mut summary = self
            .summary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        summary.encoder_finish_return_elapsed_ns = Some(self.elapsed_ns(at));
        summary.encoder_finish_ok = Some(ok);
    }

    pub(crate) fn control_stopped(&self, at: Instant, ok: bool) {
        let mut summary = self
            .summary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        summary.control_stop_return_elapsed_ns = Some(self.elapsed_ns(at));
        summary.control_stop_ok = Some(ok);
    }

    /// One create-new sidecar per physical segment. The caller treats a write
    /// failure as diagnostic loss, never as a reason to change Stop's result.
    pub(crate) fn persist(&self) -> std::io::Result<()> {
        let summary = self
            .summary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = serde_json::to_vec_pretty(&*summary)?;
        drop(summary);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.sidecar)?;
        file.write_all(&bytes)?;
        file.sync_all()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::WgcTimingRecorder;

    #[test]
    fn keeps_only_accepted_first_last_and_bounded_timing_summary() {
        let root = tempfile::tempdir().unwrap();
        let origin = Instant::now();
        let timing = WgcTimingRecorder::new(origin, 60, &root.path().join("._first.d/s.mp4"));
        timing.accepted_frame(1_000_000, origin + Duration::from_millis(100));
        timing.accepted_frame(1_166_667, origin + Duration::from_millis(117));
        timing.accepted_frame(1_100_000, origin + Duration::from_millis(133));
        timing.close_callback(origin + Duration::from_millis(150));
        timing.encoder_finished(origin + Duration::from_millis(160), true);
        timing.control_stopped(origin + Duration::from_millis(165), true);
        timing.persist().unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&timing.sidecar).unwrap()).unwrap();
        assert_eq!(value["acceptedFrames"], 3);
        assert_eq!(value["first"]["native_timestamp_100ns"], 1_000_000);
        assert_eq!(value["last"]["capture_clock_elapsed_ns"], 133_000_000);
        assert_eq!(value["nativeTimestampRegressions"], 1);
        assert_eq!(value["maxNativeGap100ns"], 166_667);
        assert_eq!(value["controlStopReturnElapsedNs"], 165_000_000);
        assert!(std::fs::metadata(&timing.sidecar).unwrap().len() < 1024);
        assert!(
            timing.persist().is_err(),
            "a segment diagnostic is never overwritten"
        );
    }

    #[test]
    fn separate_staging_paths_keep_rollover_observations_isolated() {
        let root = tempfile::tempdir().unwrap();
        let origin = Instant::now();
        let first = WgcTimingRecorder::new(origin, 60, &root.path().join("._first.d/s.mp4"));
        let second = WgcTimingRecorder::new(origin, 60, &root.path().join("._second.d/s.mp4"));
        first.accepted_frame(10, origin);
        first.control_stopped(origin, false);
        first.persist().unwrap();
        second.accepted_frame(20, origin);
        second.persist().unwrap();
        let a: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&first.sidecar).unwrap()).unwrap();
        let b: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&second.sidecar).unwrap()).unwrap();
        assert_eq!(a["first"]["native_timestamp_100ns"], 10);
        assert_eq!(b["first"]["native_timestamp_100ns"], 20);
        assert_eq!(a["controlStopOk"], false);
        assert!(b["controlStopOk"].is_null());
    }
}
