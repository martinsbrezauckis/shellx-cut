//! Neutral private protocol for the future Windows pause-ready recorder.
//!
//! These values intentionally contain no Cut server, journal, verb, UI, or
//! artifact-root type. A later server adapter translates the independently
//! observed native facts into its private durable evidence.

use record_core::{ClickSample, CursorSample, KeySample, ScrollSample, Settings};
use record_recovery::{Checkpoint, RecordingStream};
use std::time::Instant;

mod admission;
pub use admission::{WindowsPausePilotProfile, WindowsPausePilotRequest};

/// A bounded refusal category. It intentionally excludes source paths, device
/// labels, native handles, or provider details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPausePilotRefusal {
    ExactMonitorRequired,
    IntegerFpsRequired,
    Audio,
    Microphone,
    SystemAudio,
    Input,
    Window,
    Camera,
    Studio,
    Autoedit,
    Polish,
    TargetUnavailable,
}

/// Startup has two intentionally different failure classes: an exact target
/// may be absent, while reservation or native WGC acceptance may fail despite
/// an exact target being present. Neither exposes native details or paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPausePilotStartError {
    Refused(WindowsPausePilotRefusal),
    NativeStartFailed,
}

/// A command the server may enqueue after it has durably correlated a logical
/// worker epoch. Sending it is never physical completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPausePilotCommand {
    Pause { generation: u64, epoch: u64 },
    Resume { generation: u64, epoch: u64 },
    Stop { epoch: u64 },
}

impl WindowsPausePilotCommand {
    pub fn epoch(self) -> u64 {
        match self {
            Self::Pause { epoch, .. } | Self::Resume { epoch, .. } | Self::Stop { epoch } => epoch,
        }
    }
}

/// The operation that produced an independently observed pilot outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPausePilotOperation {
    Start,
    Pause,
    Resume,
    Stop,
}

/// The exact monitor rectangle WGC accepted when it opened one pilot segment.
/// It has no window fallback, title, ordinal, or primary-display identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsPausePilotCaptureRange {
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
}

/// Native encoder settings and monitor range accepted for one physical WGC
/// segment. These are carried from the post-open native owner, never supplied
/// by a server request or inferred from a logical pause generation.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowsPausePilotAcceptedCapture {
    pub settings: Settings,
    pub range: WindowsPausePilotCaptureRange,
}

/// Paired native observation emitted only after WGC accepted the segment. The
/// two clocks are captured independently; Unix time is never reconstructed
/// from a monotonic instant. `observed_start_ms` is the raw WGC worker-clock
/// tick; it is carried so a future server evidence owner can calibrate it
/// against `monotonic_at` without treating it as an already-normalized session
/// offset.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowsPausePilotStarted {
    pub physical_generation: u64,
    pub observed_start_ms: u64,
    pub monotonic_at: Instant,
    pub unix_ms: u64,
    pub accepted: WindowsPausePilotAcceptedCapture,
}

/// One immutable physical WGC checkpoint contained by a later logical run.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowsSealedWgcCheckpoint {
    pub physical_generation: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    pub checkpoint: Checkpoint,
}

/// One passive input snapshot produced by the same logical WGC run. Samples
/// are half-open against the WGC run span and are converted into an immutable
/// per-run sidecar by a future server evidence owner. The current translator
/// rejects this payload, so it cannot widen the admitted private profile.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowsSealedInputRun {
    pub cursor: Vec<CursorSample>,
    pub clicks: Vec<ClickSample>,
    pub scrolls: Vec<ScrollSample>,
    pub keys: Vec<KeySample>,
}

/// One required microphone or system-audio owner after it has stopped, flushed
/// its WAV, and re-read the exact published leaf. `source_generation` is the
/// WGC physical generation that admitted this owner, not a server-requested
/// pause generation. The relative artifact is verified again by the private
/// server evidence owner before it reaches a journal or sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsSealedAudioRun {
    pub stream: RecordingStream,
    pub source_generation: u64,
    pub artifact: String,
    pub bytes: u64,
    pub sha256: String,
    pub media_duration_ms: u64,
    pub native_ready_unix_ms: u64,
    pub native_ready_raw_ms: u64,
    pub raw_start_ms: u64,
    pub raw_end_ms: u64,
}

/// The non-empty contiguous physical checkpoint range owned by one logical
/// screen-only run. Generation and checkpoint sequences are independent but
/// each must advance by exactly one within this bounded result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowsPausePilotCheckpointRange {
    pub first_physical_generation: u64,
    pub last_physical_generation: u64,
    pub first_checkpoint_sequence: u64,
    pub last_checkpoint_sequence: u64,
}

/// One immutable logical screen-only run. It may consist of several ordinary
/// WGC checkpoint rotations before a Pause or Stop boundary seals the range.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowsSealedScreenRun {
    /// The raw WGC worker-clock start. It exactly matches the `Started` or
    /// `ResumeReady` fact that opened this logical run.
    pub observed_start_ms: u64,
    /// The raw WGC worker-clock close tick. The containing `PauseSealed` or
    /// `StopSealed` event samples `observed_at` in the same boundary closure.
    pub observed_end_ms: u64,
    pub accepted: WindowsPausePilotAcceptedCapture,
    pub range: WindowsPausePilotCheckpointRange,
    pub checkpoints: Vec<WindowsSealedWgcCheckpoint>,
}

/// A native worker event. `PauseSealed` and `StopSealed` occur only after WGC
/// close, checkpoint verification, and immutable publication have completed.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowsPausePilotEvent {
    Started {
        started: WindowsPausePilotStarted,
    },
    PauseSealed {
        generation: u64,
        epoch: u64,
        run: WindowsSealedScreenRun,
        /// Present exactly when the immutable profile selected passive input.
        /// The snapshot is accepted only after the matching WGC run has
        /// closed, verified, and published.
        input: Option<WindowsSealedInputRun>,
        /// Every selected audio owner is present exactly once only after it
        /// stopped and flushed after the paired WGC publication.
        audio: Vec<WindowsSealedAudioRun>,
        /// Paired with `run.observed_end_ms` by the same post-close native
        /// boundary observation; it is never reconstructed from that tick.
        observed_at: Instant,
    },
    ResumeReady {
        generation: u64,
        epoch: u64,
        started: WindowsPausePilotStarted,
    },
    ResumeRefused {
        generation: u64,
        epoch: u64,
        refusal: WindowsPausePilotRefusal,
        observed_at: Instant,
    },
    StopSealed {
        epoch: u64,
        run: Option<WindowsSealedScreenRun>,
        /// Paired with `run` when an active selected-input run was sealed.
        input: Option<WindowsSealedInputRun>,
        audio: Vec<WindowsSealedAudioRun>,
        /// When `run` is present, this is paired with its `observed_end_ms` by
        /// the same post-close native boundary observation.
        observed_at: Instant,
    },
    Failed {
        operation: WindowsPausePilotOperation,
        generation: Option<u64>,
        epoch: u64,
        observed_at: Instant,
    },
}
