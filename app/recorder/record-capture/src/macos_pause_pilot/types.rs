//! Private, platform-bound values for a pause-safe macOS ScreenCaptureKit run.

use record_core::Settings;
use record_recovery::{Checkpoint, RecordingStream};
use std::time::Instant;

const MACOS_MONITOR_ID_PREFIX: &str = "shellx-monitor-v1:macos:";
const MONITOR_ID_DIGEST_LEN: usize = 64;

/// A bounded refusal that never exposes a native display number, device UID,
/// output path, or TCC/provider detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacosPausePilotRefusal {
    ExactMonitorRequired,
    IntegerFpsRequired,
    Audio,
    Input,
    Window,
    Region,
    Camera,
    Studio,
    Autoedit,
    Polish,
}

/// Startup failure remains distinct from an admission refusal: ScreenCaptureKit
/// can refuse an exact selected display after a valid request was admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacosPauseStartError {
    Refused(MacosPausePilotRefusal),
    NativeStartFailed,
}

/// The only native profile this private owner may carry: one opaque macOS
/// display, integer cadence, and optional independently owned audio streams.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MacosPausePilotProfile {
    exact_monitor_id: String,
    fps: u32,
    microphone: bool,
    system_audio: bool,
}

impl MacosPausePilotProfile {
    pub(crate) fn admit(request: MacosPausePilotRequest) -> Result<Self, MacosPausePilotRefusal> {
        request.validate()
    }

    pub(crate) fn exact_monitor_id(&self) -> &str {
        &self.exact_monitor_id
    }

    pub(crate) fn fps(&self) -> u32 {
        self.fps
    }

    pub(crate) fn selected_audio_streams(&self) -> Vec<RecordingStream> {
        let mut streams = Vec::with_capacity(2);
        if self.microphone {
            streams.push(RecordingStream::MicrophoneAudio);
        }
        if self.system_audio {
            streams.push(RecordingStream::SystemAudio);
        }
        streams
    }
}

/// Private source values mirroring every ordinary surface the pause pilot
/// refuses. It is never deserialized from a public verb or UI model.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MacosPausePilotRequest {
    pub(crate) exact_monitor_id: Option<String>,
    pub(crate) fps: f64,
    pub(crate) audio: bool,
    pub(crate) microphone: bool,
    pub(crate) system_audio: bool,
    pub(crate) input: bool,
    pub(crate) window: bool,
    pub(crate) region: bool,
    pub(crate) camera: bool,
    pub(crate) studio: bool,
    pub(crate) autoedit: bool,
    pub(crate) polish: bool,
}

impl MacosPausePilotRequest {
    pub(crate) fn screen_video_only(exact_monitor_id: String, fps: f64) -> Self {
        Self::screen_with_audio(exact_monitor_id, fps, false, false)
    }

    pub(crate) fn screen_with_audio(
        exact_monitor_id: String,
        fps: f64,
        microphone: bool,
        system_audio: bool,
    ) -> Self {
        Self {
            exact_monitor_id: Some(exact_monitor_id),
            fps,
            audio: false,
            microphone,
            system_audio,
            input: false,
            window: false,
            region: false,
            camera: false,
            studio: false,
            autoedit: false,
            polish: false,
        }
    }

    fn validate(self) -> Result<MacosPausePilotProfile, MacosPausePilotRefusal> {
        let exact_monitor_id = self
            .exact_monitor_id
            .filter(|id| valid_exact_monitor_id(id))
            .ok_or(MacosPausePilotRefusal::ExactMonitorRequired)?;
        let unsupported = [
            (self.audio, MacosPausePilotRefusal::Audio),
            (self.input, MacosPausePilotRefusal::Input),
            (self.window, MacosPausePilotRefusal::Window),
            (self.region, MacosPausePilotRefusal::Region),
            (self.camera, MacosPausePilotRefusal::Camera),
            (self.studio, MacosPausePilotRefusal::Studio),
            (self.autoedit, MacosPausePilotRefusal::Autoedit),
            (self.polish, MacosPausePilotRefusal::Polish),
        ];
        if let Some((_, refusal)) = unsupported.into_iter().find(|(requested, _)| *requested) {
            return Err(refusal);
        }
        if !self.fps.is_finite() || !(1.0..=240.0).contains(&self.fps) || self.fps.fract() != 0.0 {
            return Err(MacosPausePilotRefusal::IntegerFpsRequired);
        }
        Ok(MacosPausePilotProfile {
            exact_monitor_id,
            fps: self.fps as u32,
            microphone: self.microphone,
            system_audio: self.system_audio,
        })
    }
}

/// A native screen pixel range accepted after ScreenCaptureKit opened the exact
/// opaque display. Neither an ordinal nor a friendly display name is allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MacosPauseScreenRange {
    pub(crate) origin_x: i32,
    pub(crate) origin_y: i32,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// Encoder settings and the exact physical screen range accepted by one native
/// ScreenCaptureKit generation.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MacosPauseAcceptedScreen {
    pub(crate) settings: Settings,
    pub(crate) range: MacosPauseScreenRange,
}

/// Independent native start clocks. `observed_start_ms` is raw native-run
/// provenance; a future journal-bound owner must calibrate it against
/// `monotonic_at` rather than treating it as a logical offset.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MacosPausePilotStarted {
    /// The exact opaque display identity freshly revalidated by the native
    /// start boundary. It must equal the private profile's selected target.
    pub(crate) exact_monitor_id: String,
    pub(crate) physical_generation: u64,
    pub(crate) observed_start_ms: u64,
    pub(crate) monotonic_at: Instant,
    pub(crate) unix_ms: u64,
    pub(crate) accepted: MacosPauseAcceptedScreen,
}

/// A future journal-bound owner issues a generation/epoch-correlated command
/// only after its shared logical clock and durable owner accept the control edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacosPauseCommand {
    Pause {
        generation: u64,
        epoch: u64,
        physical_generation: u64,
    },
    Resume {
        generation: u64,
        epoch: u64,
        resume_from_physical_generation: u64,
    },
    Stop {
        epoch: u64,
        physical_generation: u64,
    },
}

impl MacosPauseCommand {
    pub(crate) fn epoch(self) -> u64 {
        match self {
            Self::Pause { epoch, .. } | Self::Resume { epoch, .. } | Self::Stop { epoch, .. } => {
                epoch
            }
        }
    }

    pub(crate) fn generation(self) -> Option<u64> {
        match self {
            Self::Pause { generation, .. } | Self::Resume { generation, .. } => Some(generation),
            Self::Stop { .. } => None,
        }
    }
}

/// A delayed or stale private command is ignored without altering the active
/// native run. These categories intentionally carry only counters, never a
/// display, device, path, or provider detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacosPauseCommandRejection {
    Stopped,
    EpochOutOfOrder { expected: u64, received: u64 },
    GenerationOutOfOrder { expected: u64, received: u64 },
    PhysicalGenerationMismatch { expected: u64, received: u64 },
    WrongPhase,
}

/// Physical layout of every selected pause sidecar. The private owner accepts
/// only a first-packet WAV: neither Mic nor Core Audio may encode its native
/// first-packet offset as leading silence. The common projection consumes the
/// paired native timing fact once when it positions the sidecar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MacosPauseAudioLayout {
    PacketStart,
}

/// One immutable ScreenCaptureKit run after every included recording output
/// has closed and each checkpoint is published. The `observed_at` timestamp is
/// taken after close/publication; it is not reconstructed from raw ticks.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MacosSealedScreenRun {
    pub(crate) exact_monitor_id: String,
    pub(crate) physical_generation: u64,
    pub(crate) observed_start_ms: u64,
    pub(crate) observed_end_ms: u64,
    pub(crate) observed_at: Instant,
    pub(crate) accepted: MacosPauseAcceptedScreen,
    pub(crate) checkpoints: Vec<Checkpoint>,
}

/// One required Mic or Core Audio leaf, published without replacement and
/// reopened before any future server-side sidecar or projection may consume it.
/// Both leaves use `PacketStart`, so their raw readiness facts have identical
/// placement semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MacosSealedAudioRun {
    pub(crate) stream: RecordingStream,
    pub(crate) layout: MacosPauseAudioLayout,
    pub(crate) source_generation: u64,
    pub(crate) artifact: String,
    pub(crate) bytes: u64,
    pub(crate) sha256: String,
    pub(crate) media_duration_ms: u64,
    pub(crate) native_ready_unix_ms: u64,
    pub(crate) native_ready_raw_ms: u64,
    pub(crate) raw_start_ms: u64,
    pub(crate) raw_end_ms: u64,
}

/// A private event emitted only after all selected sources for its boundary are
/// sealed. It is intentionally not a public status or capture receipt.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MacosPausePilotEvent {
    Started {
        started: MacosPausePilotStarted,
    },
    PauseSealed {
        generation: u64,
        epoch: u64,
        run: MacosSealedScreenRun,
        audio: Vec<MacosSealedAudioRun>,
    },
    ResumeReady {
        generation: u64,
        epoch: u64,
        started: MacosPausePilotStarted,
    },
    ResumeRefused {
        generation: u64,
        epoch: u64,
    },
    StopSealed {
        epoch: u64,
        run: Option<MacosSealedScreenRun>,
        audio: Vec<MacosSealedAudioRun>,
        observed_at: Instant,
    },
    Failed {
        command: MacosPauseCommand,
        observed_at: Instant,
        /// `true` means the bounded native close retry was exhausted or still
        /// pending. Any future server owner must retain recovery evidence and
        /// must not treat this event as a completed terminal close.
        teardown_unresolved: bool,
    },
    Rejected {
        command: MacosPauseCommand,
        rejection: MacosPauseCommandRejection,
    },
}

fn valid_exact_monitor_id(id: &str) -> bool {
    id.strip_prefix(MACOS_MONITOR_ID_PREFIX)
        .is_some_and(|digest| {
            digest.len() == MONITOR_ID_DIGEST_LEN
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}
