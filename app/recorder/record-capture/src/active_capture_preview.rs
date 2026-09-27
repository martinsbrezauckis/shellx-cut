//! Optional, memory-only pixels from the recording's own native screen stream.
//! Preview has no authority over capture: readback/conversion failures only
//! change this mailbox and can never fail the encoder or checkpoint owner.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::source_preview::SourcePreviewFrame;

const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);
const STALE_AFTER: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Default)]
pub struct ActiveCapturePreview(Arc<Mutex<State>>);

#[derive(Debug, Default)]
struct State {
    generation: u64,
    mode: Mode,
    frame: Option<SourcePreviewFrame>,
    received_at: Option<Instant>,
    next_sample: Option<Instant>,
    last_sample_cost_ms: Option<u64>,
    max_sample_cost_ms: u64,
    recursion: RecursionStatus,
    controller_exclusion: ControllerExclusionStatus,
    sample_interval: Duration,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RecursionStatus {
    None,
    #[default]
    Possible,
    Unavoidable,
}

impl RecursionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Possible => "possible",
            Self::Unavoidable => "unavoidable",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ControllerExclusionStatus {
    ConfirmedExcluded,
    #[default]
    NotConfirmed,
    NotApplicable,
}

impl ControllerExclusionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmedExcluded => "confirmed_excluded",
            Self::NotConfirmed => "not_confirmed",
            Self::NotApplicable => "not_applicable",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Pending,
    Enabled,
    BackendUnavailable,
    #[cfg(any(
        test,
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos"),
        all(target_os = "linux", feature = "capture-linux")
    ))]
    ReadbackUnavailable,
    Terminal,
}

#[cfg(any(
    test,
    all(windows, feature = "capture-windows"),
    all(target_os = "macos", feature = "capture-macos"),
    all(target_os = "linux", feature = "capture-linux")
))]
pub(crate) struct NativePreviewPixels<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub offset: usize,
    pub format: crate::source_preview_bitmap::SourcePreviewPixelFormat,
    pub pixels: &'a [u8],
}

#[derive(Debug, Clone)]
pub struct ActiveCapturePreviewSnapshot {
    pub generation: u64,
    pub state: &'static str,
    pub reason: Option<&'static str>,
    pub frame: Option<SourcePreviewFrame>,
    pub frame_age_ms: Option<u64>,
    pub last_sample_cost_ms: Option<u64>,
    pub max_sample_cost_ms: u64,
    pub recursion: RecursionStatus,
    pub controller_exclusion: ControllerExclusionStatus,
}

impl ActiveCapturePreview {
    pub fn enable(&self) {
        let mut state = self.lock();
        if state.mode != Mode::Terminal {
            state.mode = Mode::Enabled;
        }
    }

    pub fn set_controller_safety(
        &self,
        recursion: RecursionStatus,
        controller_exclusion: ControllerExclusionStatus,
    ) {
        let mut state = self.lock();
        if state.mode != Mode::Terminal {
            state.recursion = recursion;
            state.controller_exclusion = controller_exclusion;
        }
    }

    pub fn refuse_backend_unavailable(&self) {
        self.disable(Mode::BackendUnavailable);
    }

    #[cfg(any(
        test,
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos")
    ))]
    pub(crate) fn mark_readback_unavailable(&self, generation: u64) {
        let mut state = self.lock();
        if state.mode == Mode::Enabled && state.generation == generation {
            state.mode = Mode::ReadbackUnavailable;
            state.frame = None;
            state.received_at = None;
        }
    }

    fn disable(&self, mode: Mode) {
        let mut state = self.lock();
        if state.mode != Mode::Terminal {
            state.mode = mode;
            state.frame = None;
            state.received_at = None;
        }
    }

    /// Each physical WGC segment gets a fresh generation. A late callback from
    /// the preceding segment cannot repopulate the mailbox after rollover.
    pub fn begin_segment(&self) -> Option<u64> {
        let mut state = self.lock();
        if state.mode != Mode::Enabled {
            return None;
        }
        state.generation = state.generation.checked_add(1)?;
        state.frame = None;
        state.received_at = None;
        state.next_sample = None;
        state.sample_interval = SAMPLE_INTERVAL;
        Some(state.generation)
    }

    /// Clear pixels from a paused or sealed stream without ending this capture.
    /// A resumed physical stream must call begin_segment before publishing.
    pub fn clear_current_generation(&self) {
        let mut state = self.lock();
        state.frame = None;
        state.received_at = None;
        state.next_sample = None;
        state.generation = state.generation.saturating_add(1);
    }

    pub fn snapshot(&self) -> ActiveCapturePreviewSnapshot {
        let state = self.lock();
        let frame_age_ms = state
            .received_at
            .map(|at| u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX));
        let (name, reason) = match state.mode {
            Mode::Pending => ("awaiting_source", None),
            Mode::Enabled if state.frame.is_some() && frame_age_ms.is_some_and(|age| age >= STALE_AFTER.as_millis() as u64) => ("stale", Some("The latest real source frame is older than five seconds.")),
            Mode::Enabled if state.frame.is_some() => ("ready", None),
            Mode::Enabled => ("awaiting_frame", None),
            Mode::BackendUnavailable => ("unavailable", Some("This native recording backend does not yet provide live source pixels.")),
            #[cfg(any(
                test,
                all(windows, feature = "capture-windows"),
                all(target_os = "macos", feature = "capture-macos"),
                all(target_os = "linux", feature = "capture-linux")
            ))]
            Mode::ReadbackUnavailable => ("unavailable", Some("The native recording frame could not be read for preview; recording continues.")),
            Mode::Terminal => ("terminal", None),
        };
        ActiveCapturePreviewSnapshot {
            generation: state.generation,
            state: name,
            reason,
            frame: state.frame.clone(),
            frame_age_ms,
            last_sample_cost_ms: state.last_sample_cost_ms,
            max_sample_cost_ms: state.max_sample_cost_ms,
            recursion: state.recursion,
            controller_exclusion: state.controller_exclusion,
        }
    }

    pub fn terminate(&self) {
        let mut state = self.lock();
        state.mode = Mode::Terminal;
        state.frame = None;
        state.received_at = None;
        state.next_sample = None;
        state.generation = state.generation.saturating_add(1);
    }

    #[cfg(any(
        test,
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos"),
        all(target_os = "linux", feature = "capture-linux")
    ))]
    pub(crate) fn claim_sample(&self, generation: u64, now: Instant) -> bool {
        let mut state = self.lock();
        if state.mode != Mode::Enabled || state.generation != generation {
            return false;
        }
        if state.next_sample.is_some_and(|next| now < next) {
            return false;
        }
        state.next_sample = Some(now + state.sample_interval.max(SAMPLE_INTERVAL));
        true
    }

    #[cfg(any(
        test,
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos")
    ))]
    pub(crate) fn publish_bgra(
        &self,
        generation: u64,
        at_ms: u64,
        width: u32,
        height: u32,
        stride: usize,
        pixels: &[u8],
    ) {
        self.publish_native(
            generation,
            at_ms,
            NativePreviewPixels {
                width,
                height,
                stride,
                offset: 0,
                format: crate::source_preview_bitmap::SourcePreviewPixelFormat::Bgra,
                pixels,
            },
        );
    }

    #[cfg(any(
        test,
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos"),
        all(target_os = "linux", feature = "capture-linux")
    ))]
    pub(crate) fn publish_native(
        &self,
        generation: u64,
        at_ms: u64,
        source: NativePreviewPixels<'_>,
    ) {
        // Conversion writes directly into a bounded BMP. The native source
        // buffer is borrowed only for this call and is never retained.
        let frame = crate::source_preview_bitmap::active_native_bmp(
            source.width,
            source.height,
            source.format,
            source.stride,
            source.pixels,
            source.offset,
        )
        .and_then(|bytes| SourcePreviewFrame::new(at_ms, bytes));
        let mut state = self.lock();
        if state.mode != Mode::Enabled || state.generation != generation {
            return;
        }
        match frame {
            Ok(frame) => {
                state.frame = Some(frame);
                state.received_at = Some(Instant::now());
            }
            Err(_) => {
                state.frame = None;
                state.received_at = None;
                state.mode = Mode::ReadbackUnavailable;
            }
        }
    }

    #[cfg(any(
        test,
        all(windows, feature = "capture-windows"),
        all(target_os = "macos", feature = "capture-macos"),
        all(target_os = "linux", feature = "capture-linux")
    ))]
    pub(crate) fn record_sample_cost(&self, generation: u64, cost: Duration) {
        let mut state = self.lock();
        if state.mode != Mode::Enabled || state.generation != generation {
            return;
        }
        let ms = u64::try_from(cost.as_millis()).unwrap_or(u64::MAX);
        state.last_sample_cost_ms = Some(ms);
        state.max_sample_cost_ms = state.max_sample_cost_ms.max(ms);
        // A costly optional readback lowers sampling pressure. It never
        // disables capture, and native qualification still measures the cost.
        state.sample_interval = if cost > Duration::from_millis(80) {
            Duration::from_millis(400)
        } else if cost > Duration::from_millis(40) {
            Duration::from_millis(200)
        } else {
            SAMPLE_INTERVAL
        };
        state.next_sample = Some(Instant::now() + state.sample_interval);
    }

    #[cfg(all(windows, feature = "capture-windows"))]
    pub(crate) fn observe_wgc_frame(
        &self,
        generation: u64,
        frame: &mut windows_capture::frame::Frame,
        started: Instant,
    ) {
        let now = Instant::now();
        if !self.claim_sample(generation, now) {
            return;
        }
        // `frame.buffer()` maps the 4K GPU source to CPU memory. Latest-only
        // sampling starts at up to 10 Hz and adapts downward if expensive.
        // Native 4K qualification must still measure callback/encoder impact.
        match frame.buffer() {
            Ok(mut buffer) => self.publish_bgra(
                generation,
                u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                buffer.width(),
                buffer.height(),
                usize::try_from(buffer.row_pitch()).unwrap_or(usize::MAX),
                buffer.as_raw_buffer(),
            ),
            Err(_) => self.mark_readback_unavailable(generation),
        }
        self.record_sample_cost(generation, now.elapsed());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::{ActiveCapturePreview, ControllerExclusionStatus, RecursionStatus};
    use std::time::{Duration, Instant};

    #[test]
    fn only_current_segment_retains_real_bounded_pixels() {
        let preview = ActiveCapturePreview::default();
        preview.enable();
        let first = preview.begin_segment().unwrap();
        assert_eq!(preview.snapshot().state, "awaiting_frame");
        assert!(preview.claim_sample(first, Instant::now()));
        preview.publish_bgra(first, 0, 2, 2, 8, &[255; 16]);
        let ready = preview.snapshot();
        assert_eq!(ready.state, "ready");
        assert!(ready.frame.unwrap().encoded().starts_with(b"BM"));
        let second = preview.begin_segment().unwrap();
        assert_eq!(preview.snapshot().state, "awaiting_frame");
        preview.publish_bgra(first, 1, 2, 2, 8, &[255; 16]);
        assert!(preview.snapshot().frame.is_none());
        preview.publish_bgra(second, 2, 2, 2, 8, &[255; 16]);
        assert!(preview.snapshot().frame.is_some());
        preview.terminate();
        assert_eq!(preview.snapshot().state, "terminal");
        assert!(preview.snapshot().frame.is_none());
    }

    #[test]
    fn preview_refusal_and_bad_pixels_do_not_escape_to_capture() {
        let preview = ActiveCapturePreview::default();
        preview.refuse_backend_unavailable();
        assert_eq!(preview.snapshot().state, "unavailable");
        assert!(preview.begin_segment().is_none());
        let preview = ActiveCapturePreview::default();
        preview.enable();
        let generation = preview.begin_segment().unwrap();
        preview.publish_bgra(generation, 0, 2, 2, 8, &[]);
        assert_eq!(preview.snapshot().state, "unavailable");
        assert!(preview.snapshot().frame.is_none());
    }

    #[test]
    fn four_k_frame_is_sampled_at_ten_hertz_and_downscaled_under_memory_cap() {
        let preview = ActiveCapturePreview::default();
        preview.enable();
        let generation = preview.begin_segment().unwrap();
        let now = Instant::now();
        assert!(preview.claim_sample(generation, now));
        assert!(!preview.claim_sample(generation, now + Duration::from_millis(99)));
        assert!(preview.claim_sample(generation, now + Duration::from_millis(100)));
        let pixels = vec![64_u8; 3840 * 2160 * 4];
        preview.publish_bgra(generation, 1000, 3840, 2160, 3840 * 4, &pixels);
        let frame = preview.snapshot().frame.unwrap();
        assert!(frame.encoded().starts_with(b"BM"));
        assert!(frame.encoded().len() <= 54 + 640 * 360 * 4);
    }

    #[test]
    fn slow_preview_work_adapts_sampling_without_losing_real_pixels() {
        let preview = ActiveCapturePreview::default();
        preview.enable();
        let generation = preview.begin_segment().unwrap();
        preview.publish_bgra(generation, 0, 2, 2, 8, &[255; 16]);
        preview.record_sample_cost(generation, Duration::from_millis(81));
        let snapshot = preview.snapshot();
        assert_eq!(snapshot.state, "ready");
        assert_eq!(snapshot.max_sample_cost_ms, 81);
        assert!(!preview.claim_sample(generation, Instant::now() + Duration::from_millis(100)));
        assert!(preview.begin_segment().is_some());
    }

    #[test]
    fn ready_pixels_can_report_possible_recursion_without_being_hidden() {
        let preview = ActiveCapturePreview::default();
        preview.set_controller_safety(
            RecursionStatus::Possible,
            ControllerExclusionStatus::NotConfirmed,
        );
        preview.enable();
        let generation = preview.begin_segment().unwrap();
        preview.publish_bgra(generation, 0, 2, 2, 8, &[255; 16]);
        let snapshot = preview.snapshot();
        assert_eq!(snapshot.state, "ready");
        assert_eq!(snapshot.recursion, RecursionStatus::Possible);
        assert_eq!(
            snapshot.controller_exclusion,
            ControllerExclusionStatus::NotConfirmed
        );
        assert!(snapshot.frame.is_some());
        preview.clear_current_generation();
        assert!(preview.snapshot().frame.is_none());
        preview.publish_bgra(generation, 1, 2, 2, 8, &[255; 16]);
        assert!(preview.snapshot().frame.is_none());
    }

    #[test]
    fn readback_failure_is_bound_to_current_generation_only() {
        let preview = ActiveCapturePreview::default();
        preview.enable();
        let old = preview.begin_segment().unwrap();
        let current = preview.begin_segment().unwrap();
        preview.mark_readback_unavailable(old);
        assert_eq!(preview.snapshot().state, "awaiting_frame");
        preview.mark_readback_unavailable(current);
        assert_eq!(preview.snapshot().state, "unavailable");
        assert!(preview.snapshot().frame.is_none());
    }
}
