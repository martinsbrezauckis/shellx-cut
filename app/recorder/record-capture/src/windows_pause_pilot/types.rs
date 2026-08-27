//! Neutral private protocol for the future Windows pause-ready recorder.
//!
//! These values intentionally contain no Cut server, journal, verb, UI, or
//! artifact-root type. A later server adapter translates the independently
//! observed native facts into its private durable evidence.

use record_core::Settings;
use record_recovery::Checkpoint;
use std::time::Instant;

const WINDOWS_MONITOR_ID_PREFIX: &str = "shellx-monitor-v1:windows:";
const MONITOR_ID_DIGEST_LEN: usize = 64;

/// The one accepted native profile. Every other live-recording surface is a
/// refusal, not a setting that the pilot silently drops.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowsPausePilotProfile {
    exact_monitor_id: String,
    fps: u32,
}

impl WindowsPausePilotProfile {
    /// Admit only one exact native monitor and an integer encoder cadence.
    pub fn admit(request: WindowsPausePilotRequest) -> Result<Self, WindowsPausePilotRefusal> {
        request.validate()
    }

    pub fn exact_monitor_id(&self) -> &str {
        &self.exact_monitor_id
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }
}

/// Internal admission input for a future pause-ready start path.
///
/// This deliberately mirrors every currently unsupported recording surface so
/// the caller cannot accidentally turn one off and claim the requested mode
/// was captured. It is not a public request schema.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowsPausePilotRequest {
    pub exact_monitor_id: Option<String>,
    pub fps: f64,
    pub audio: bool,
    pub microphone: bool,
    pub system_audio: bool,
    pub input: bool,
    pub window: bool,
    pub camera: bool,
    pub studio: bool,
    pub autoedit: bool,
    pub polish: bool,
}

impl WindowsPausePilotRequest {
    pub fn screen_video_only(exact_monitor_id: String, fps: f64) -> Self {
        Self {
            exact_monitor_id: Some(exact_monitor_id),
            fps,
            audio: false,
            microphone: false,
            system_audio: false,
            input: false,
            window: false,
            camera: false,
            studio: false,
            autoedit: false,
            polish: false,
        }
    }

    fn validate(self) -> Result<WindowsPausePilotProfile, WindowsPausePilotRefusal> {
        let exact_monitor_id = self
            .exact_monitor_id
            .filter(|id| valid_exact_monitor_id(id))
            .ok_or(WindowsPausePilotRefusal::ExactMonitorRequired)?;
        let unsupported = [
            (self.audio, WindowsPausePilotRefusal::Audio),
            (self.microphone, WindowsPausePilotRefusal::Microphone),
            (self.system_audio, WindowsPausePilotRefusal::SystemAudio),
            (self.input, WindowsPausePilotRefusal::Input),
            (self.window, WindowsPausePilotRefusal::Window),
            (self.camera, WindowsPausePilotRefusal::Camera),
            (self.studio, WindowsPausePilotRefusal::Studio),
            (self.autoedit, WindowsPausePilotRefusal::Autoedit),
            (self.polish, WindowsPausePilotRefusal::Polish),
        ];
        if let Some((_, refusal)) = unsupported.into_iter().find(|(requested, _)| *requested) {
            return Err(refusal);
        }
        if !self.fps.is_finite() || !(1.0..=240.0).contains(&self.fps) || self.fps.fract() != 0.0 {
            return Err(WindowsPausePilotRefusal::IntegerFpsRequired);
        }
        Ok(WindowsPausePilotProfile {
            exact_monitor_id,
            fps: self.fps as u32,
        })
    }
}

fn valid_exact_monitor_id(id: &str) -> bool {
    id.strip_prefix(WINDOWS_MONITOR_ID_PREFIX)
        .is_some_and(|digest| {
            digest.len() == MONITOR_ID_DIGEST_LEN
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

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
