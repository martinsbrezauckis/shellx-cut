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
    last_native_gap_100ns: i64,
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
                last_native_gap_100ns: 0,
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
                let gap = native_timestamp_100ns.saturating_sub(previous.native_timestamp_100ns);
                summary.last_native_gap_100ns = gap;
                summary.max_native_gap_100ns = summary.max_native_gap_100ns.max(gap);
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

    /// WGC's encoder makes a constant-rate file from sparse native frames. At
    /// Stop it can extend the last held frame by the preceding native gap, so
    /// the file can outlive the capture clock even though every real frame was
    /// delivered before Stop. The retained PC4 case matched the largest earlier
    /// native gap rather than the last gap. Permit clipping only when both
    /// independent clocks agree on the real frame span and one observed gap
    /// explains the excess. The caller still verifies the clipped video.
    pub(crate) fn explains_extrapolated_tail(
        &self,
        start_ms: u64,
        end_ms: u64,
        media_duration_ms: u64,
        decoded_frames: u64,
    ) -> bool {
        const CLOCK_SLOP_MS: u64 = 100;
        const ENCODER_SLOP_MS: u64 = 100;
        let summary = self
            .summary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (Some(first), Some(last), Some(span_ms)) =
            (summary.first, summary.last, end_ms.checked_sub(start_ms))
        else {
            return false;
        };
        let Some(native_ticks) = last
            .native_timestamp_100ns
            .checked_sub(first.native_timestamp_100ns)
        else {
            return false;
        };
        let Ok(native_ticks) = u64::try_from(native_ticks) else {
            return false;
        };
        let native_ms = native_ticks / 10_000;
        let callback_ms = last
            .capture_clock_elapsed_ns
            .saturating_sub(first.capture_clock_elapsed_ns)
            / 1_000_000;
        let last_callback_ms = last.capture_clock_elapsed_ns / 1_000_000;
        let stop_return_ms = summary.control_stop_return_elapsed_ns.unwrap_or(0) / 1_000_000;
        let last_gap_ms = u64::try_from(summary.last_native_gap_100ns).unwrap_or(0) / 10_000;
        let max_gap_ms = u64::try_from(summary.max_native_gap_100ns).unwrap_or(0) / 10_000;
        let explains_tail = [last_gap_ms, max_gap_ms].into_iter().any(|gap_ms| {
            gap_ms > 0
                && media_duration_ms.abs_diff(native_ms.saturating_add(gap_ms)) <= ENCODER_SLOP_MS
        });
        summary.control_stop_ok == Some(true)
            && stop_return_ms.abs_diff(end_ms) <= CLOCK_SLOP_MS
            && summary.native_timestamp_regressions == 0
            && summary.accepted_frames >= 2
            && decoded_frames > summary.accepted_frames
            && media_duration_ms > span_ms.saturating_add(200)
            && native_ms.abs_diff(callback_ms) <= CLOCK_SLOP_MS
            && last_callback_ms <= end_ms.saturating_add(CLOCK_SLOP_MS)
            && native_ms <= span_ms.saturating_add(CLOCK_SLOP_MS)
            && explains_tail
    }

    pub(crate) fn accepted_frames(&self) -> u64 {
        self.summary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .accepted_frames
    }

    /// A zero-frame tail can be replaced only after the native control has
    /// joined and reported a successful Stop. A failed close remains terminal.
    pub(crate) fn control_stop_succeeded(&self) -> bool {
        self.summary
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .control_stop_ok
            == Some(true)
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

    #[test]
    fn only_clock_proven_sparse_frame_extrapolation_can_clip() {
        let root = tempfile::tempdir().unwrap();
        let origin = Instant::now();
        let timing = WgcTimingRecorder::new(origin, 60, &root.path().join("._tail.d/s.mp4"));
        let first = 5_978_568_688_251i64;
        timing.accepted_frame(first, origin + Duration::from_millis(477));
        timing.accepted_frame(first + 11_000_000, origin + Duration::from_millis(1_577));
        timing.accepted_frame(first + 44_334_192, origin + Duration::from_millis(4_910));
        timing.control_stopped(origin + Duration::from_millis(7_337), true);
        assert!(timing.explains_extrapolated_tail(0, 7_338, 7_750, 465));
        assert!(!timing.explains_extrapolated_tail(0, 7_338, 7_750, 3));
        assert!(!timing.explains_extrapolated_tail(0, 7_338, 8_500, 465));

        let unsynchronized =
            WgcTimingRecorder::new(origin, 60, &root.path().join("._skew.d/s.mp4"));
        unsynchronized.accepted_frame(first, origin + Duration::from_millis(477));
        unsynchronized.accepted_frame(first + 44_334_192, origin + Duration::from_millis(3_000));
        unsynchronized.control_stopped(origin + Duration::from_millis(7_337), true);
        assert!(!unsynchronized.explains_extrapolated_tail(0, 7_338, 7_750, 465));
    }

    #[test]
    fn earlier_max_gap_explains_measured_pc4_cfr_tail_without_moving_stop() {
        // Retained 839 PC4 segment: 69 native frames, 476 decoded CFR frames,
        // 7,933 ms media and a separately observed 7,499 ms Stop boundary.
        let root = tempfile::tempdir().unwrap();
        let origin = Instant::now();
        let timing = WgcTimingRecorder::new(origin, 60, &root.path().join("._pc4.d/s.mp4"));
        let first = 6_146_209_642_756i64;
        let last = 6_146_255_143_426i64;
        timing.accepted_frame(first, origin + Duration::from_nanos(479_874_400));
        let mut timestamp = first + 34_000_076; // 3,400.008 ms earlier gap.
        timing.accepted_frame(timestamp, origin + Duration::from_nanos(3_879_882_000));
        for _ in 0..66 {
            timestamp += 171_726;
            timing.accepted_frame(
                timestamp,
                origin + Duration::from_nanos(479_874_400 + ((timestamp - first) as u64) * 100),
            );
        }
        assert!(timestamp < last - 166_655);
        timing.accepted_frame(last, origin + Duration::from_nanos(5_028_166_500));
        timing.control_stopped(origin + Duration::from_nanos(7_497_007_500), true);
        assert_eq!(timing.accepted_frames(), 69);
        assert!(timing.explains_extrapolated_tail(0, 7_499, 7_933, 476));
        assert!(!timing.explains_extrapolated_tail(0, 7_499, 8_500, 476));
        assert!(!timing.explains_extrapolated_tail(0, 7_499, 7_933, 68));
        assert!(!timing.explains_extrapolated_tail(0, 7_600, 7_933, 476));
        timing.control_stopped(origin + Duration::from_nanos(7_497_007_500), false);
        assert!(!timing.explains_extrapolated_tail(0, 7_499, 7_933, 476));

        let regressed =
            WgcTimingRecorder::new(origin, 60, &root.path().join("._regressed.d/s.mp4"));
        regressed.accepted_frame(first, origin + Duration::from_nanos(479_874_400));
        regressed.accepted_frame(
            first + 34_000_076,
            origin + Duration::from_nanos(3_879_882_000),
        );
        regressed.accepted_frame(
            first + 45_000_000,
            origin + Duration::from_nanos(4_979_874_400),
        );
        regressed.accepted_frame(
            first + 44_000_000,
            origin + Duration::from_nanos(4_879_874_400),
        );
        regressed.control_stopped(origin + Duration::from_millis(7_399), true);
        assert!(!regressed.explains_extrapolated_tail(0, 7_400, 7_800, 476));
    }
}
