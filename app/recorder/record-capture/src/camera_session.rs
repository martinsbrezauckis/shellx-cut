//! Private camera-session lifecycle built around the screen-owned capture clock.
//!
//! This is deliberately a test-injected spine, not a native adapter or a server
//! start surface. A later capture owner may append the resulting typed evidence
//! to its durable session journal, but this module never owns the screen take.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use record_core::{
    error_codes, CameraArtifact, CameraClockRange, CameraMediaFacts, CameraTerminalState,
    FrameRate, RecordError, Result,
};
use record_recovery::{RecordingStream, StreamFragment, StreamFragmentFacts};

use crate::{camera::validate_request_part, CameraReadiness, CameraRequest, CaptureClock};

/// Test-injected proof returned after a backend has stopped its camera source.
///
/// A native implementation must derive all fields from its sealed local media;
/// no caller supplies the first or final frame offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CameraMediaSeal {
    pub(crate) artifact_id: String,
    pub(crate) video: String,
    pub(crate) media: CameraMediaFacts,
    pub(crate) bytes: u64,
}

/// Truthful terminal result from a camera backend. `NoMedia` means that no
/// camera prefix exists; it is not a malformed empty `CameraMediaSeal`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CameraStopOutcome {
    NoMedia,
    Sealed(CameraMediaSeal),
}

/// Minimal backend seam for the camera-session spine.
///
/// `readiness` is the pre-start probe. `start` receives only the screen-owned
/// origin. `stop` returns `NoMedia` for a truthful zero-frame terminal or a
/// `Sealed` local prefix with independently measured media facts. If `start`
/// returns an error after it may have acquired resources, the caller invokes one
/// `stop(Cancelled)` cleanup and returns the original start error. This trait
/// intentionally has no native implementation in this slice.
pub(crate) trait CameraSessionBackend {
    fn readiness(&self, request: &CameraRequest) -> CameraReadiness;
    fn start(&mut self, request: &CameraRequest, screen_origin: Instant) -> Result<()>;
    fn stop(&mut self, terminal_state: CameraTerminalState) -> Result<CameraStopOutcome>;
}

/// An enabled camera sidecar that has passed its probe and joined the shared
/// screen clock. Frames are observed explicitly so setup or browser timestamps
/// can never become camera timing evidence.
#[derive(Debug)]
pub(crate) struct CameraSession<B: CameraSessionBackend> {
    backend: B,
    request: CameraRequest,
    screen_origin: Instant,
    first_frame_offset_ms: Option<u64>,
    last_frame_offset_ms: Option<u64>,
    stop_state: CameraStopState,
}

#[derive(Debug)]
enum CameraStopState {
    Live,
    Stopped {
        terminal_state: CameraTerminalState,
        outcome: CameraStopOutcome,
    },
    Failed(RecordError),
}

impl<B: CameraSessionBackend> CameraSession<B> {
    /// Return the backend's current pre-start probe without beginning a session.
    pub(crate) fn probe(backend: &B, request: &CameraRequest) -> CameraReadiness {
        backend.readiness(request)
    }

    /// Start only after a ready probe and the screen backend's real clock origin.
    /// A pre-start refusal and cancellation both leave the backend unstarted.
    pub(crate) fn start(
        mut backend: B,
        request: CameraRequest,
        screen_clock: &CaptureClock,
        stop: &AtomicBool,
    ) -> Result<Self> {
        validate_request(&request)?;
        match Self::probe(&backend, &request) {
            CameraReadiness::Ready { device, .. } if device.id == request.device_id => {}
            CameraReadiness::Ready { .. } => {
                return Err(capture_error(
                    "selected camera probe returned a different device",
                    "camera capture requires Ready for the exact requested device_id",
                ));
            }
            _ => {
                return Err(capture_error(
                    "selected camera did not pass the pre-start probe",
                    "camera capture requires a Ready probe before session start",
                ));
            }
        }
        let Some(screen_origin) = screen_clock.wait_started(stop) else {
            return Err(capture_error(
                "camera session was cancelled before screen capture started",
                "the screen-owned CaptureClock never opened",
            ));
        };
        if let Err(start_error) = backend.start(&request, screen_origin) {
            let _ = backend.stop(CameraTerminalState::Cancelled);
            return Err(start_error);
        }
        Ok(Self {
            backend,
            request,
            screen_origin,
            first_frame_offset_ms: None,
            last_frame_offset_ms: None,
            stop_state: CameraStopState::Live,
        })
    }

    /// Record one delivered frame against the screen origin, never against wall
    /// clock, setup, or browser event time.
    pub(crate) fn observe_frame_at(&mut self, frame_at: Instant) -> Result<()> {
        if !matches!(&self.stop_state, CameraStopState::Live) {
            return Err(invalid("camera session cannot accept frames after stop"));
        }
        let offset = frame_at
            .checked_duration_since(self.screen_origin)
            .ok_or_else(|| invalid("camera frame precedes the screen CaptureClock origin"))?;
        let offset_ms = u64::try_from(offset.as_millis())
            .map_err(|_| invalid("camera frame offset exceeds the supported range"))?;
        if self
            .last_frame_offset_ms
            .is_some_and(|last| offset_ms < last)
        {
            return Err(invalid("camera frame offsets must be monotonic"));
        }
        self.first_frame_offset_ms.get_or_insert(offset_ms);
        self.last_frame_offset_ms = Some(offset_ms);
        Ok(())
    }

    /// Stop the camera independently. This does not signal, stop, or otherwise
    /// alter the screen capture; a `DeviceLost` terminal therefore preserves its
    /// separately owned screen take.
    pub(crate) fn stop(&mut self, terminal_state: CameraTerminalState) -> Result<()> {
        if !matches!(&self.stop_state, CameraStopState::Live) {
            return Err(invalid("camera session is already stopped"));
        }
        match self.backend.stop(terminal_state) {
            Ok(CameraStopOutcome::NoMedia) => {
                self.stop_state = CameraStopState::Stopped {
                    terminal_state,
                    outcome: CameraStopOutcome::NoMedia,
                };
                Ok(())
            }
            Ok(CameraStopOutcome::Sealed(seal)) if seal.bytes != 0 => {
                self.stop_state = CameraStopState::Stopped {
                    terminal_state,
                    outcome: CameraStopOutcome::Sealed(seal),
                };
                Ok(())
            }
            Ok(CameraStopOutcome::Sealed(_)) => {
                let error = invalid("sealed camera evidence must have non-zero bytes");
                self.stop_state = CameraStopState::Failed(error.clone());
                Err(error)
            }
            Err(error) => {
                self.stop_state = CameraStopState::Failed(error.clone());
                Err(error)
            }
        }
    }

    /// Validate and expose the independent camera prefix for later durable
    /// journal ownership. A session with no observed frames cannot invent one.
    pub(crate) fn seal(self) -> Result<CameraSessionEvidence> {
        let (terminal_state, seal) = match &self.stop_state {
            CameraStopState::Live => {
                return Err(invalid("camera session must stop before sealing"));
            }
            CameraStopState::Stopped {
                terminal_state: _,
                outcome: CameraStopOutcome::NoMedia,
            } => return Err(invalid("camera session terminal has no media to seal")),
            CameraStopState::Stopped {
                terminal_state,
                outcome: CameraStopOutcome::Sealed(seal),
            } => (*terminal_state, seal),
            CameraStopState::Failed(error) => return Err(error.clone()),
        };
        let frame_range = self.frame_range()?;
        let artifact = CameraArtifact::new(
            self.request.capture_id.clone(),
            seal.artifact_id.clone(),
            seal.video.clone(),
            frame_range,
            seal.media.clone(),
            terminal_state,
        )?;
        Ok(CameraSessionEvidence {
            artifact,
            bytes: seal.bytes,
        })
    }

    fn frame_range(&self) -> Result<CameraClockRange> {
        let first_frame_offset_ms = self
            .first_frame_offset_ms
            .ok_or_else(|| invalid("camera session has no observed first frame"))?;
        let end_frame_offset_ms = self
            .last_frame_offset_ms
            .ok_or_else(|| invalid("camera session has no observed final frame"))?;
        if end_frame_offset_ms <= first_frame_offset_ms {
            return Err(invalid(
                "camera session needs a non-empty measured frame interval before sealing",
            ));
        }
        Ok(CameraClockRange {
            first_frame_offset_ms,
            end_frame_offset_ms,
        })
    }
}

impl<B: CameraSessionBackend> Drop for CameraSession<B> {
    fn drop(&mut self) {
        if !matches!(&self.stop_state, CameraStopState::Live) {
            return;
        }
        self.stop_state = match self.backend.stop(CameraTerminalState::Cancelled) {
            Ok(CameraStopOutcome::NoMedia) => CameraStopState::Stopped {
                terminal_state: CameraTerminalState::Cancelled,
                outcome: CameraStopOutcome::NoMedia,
            },
            Ok(CameraStopOutcome::Sealed(seal)) if seal.bytes != 0 => CameraStopState::Stopped {
                terminal_state: CameraTerminalState::Cancelled,
                outcome: CameraStopOutcome::Sealed(seal),
            },
            Ok(CameraStopOutcome::Sealed(_)) => {
                CameraStopState::Failed(invalid("sealed camera evidence must have non-zero bytes"))
            }
            Err(error) => CameraStopState::Failed(error),
        };
    }
}

/// Validated private evidence ready for a future durable recording-session
/// owner. It does not write a journal itself, so only that owner decides which
/// screen run contains the camera prefix and when the append is durable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CameraSessionEvidence {
    artifact: CameraArtifact,
    bytes: u64,
}

impl CameraSessionEvidence {
    pub(crate) fn artifact(&self) -> &CameraArtifact {
        &self.artifact
    }

    /// Bind this prefix to one physical screen run measured on the same
    /// `CaptureClock`. This does not compact paused time; a durable owner must
    /// separately project physical runs into its logical session timeline.
    pub(crate) fn stream_fragment(
        &self,
        run_capture_clock_start_ms: u64,
        run_capture_clock_end_ms: u64,
        stream_sequence: u64,
    ) -> Result<StreamFragment> {
        self.artifact.validate()?;
        if run_capture_clock_end_ms <= run_capture_clock_start_ms {
            return Err(invalid("containing physical screen run is empty"));
        }
        if self.artifact.clock.first_frame_offset_ms < run_capture_clock_start_ms
            || self.artifact.clock.end_frame_offset_ms > run_capture_clock_end_ms
        {
            return Err(invalid(
                "camera prefix is not fully contained by the physical screen run",
            ));
        }
        let start_offset_ms = self
            .artifact
            .clock
            .first_frame_offset_ms
            .checked_sub(run_capture_clock_start_ms)
            .ok_or_else(|| invalid("camera prefix begins before the containing screen run"))?;
        let end_offset_ms = self
            .artifact
            .clock
            .end_frame_offset_ms
            .checked_sub(run_capture_clock_start_ms)
            .ok_or_else(|| invalid("camera prefix ends before the containing screen run"))?;
        if end_offset_ms <= start_offset_ms {
            return Err(invalid(
                "camera prefix is empty within the containing screen run",
            ));
        }
        let frame_rate = FrameRate::new(
            u64::from(self.artifact.media.fps_num),
            u64::from(self.artifact.media.fps_den),
        )
        .map_err(|_| invalid("camera media FPS cannot form journal frame-rate evidence"))?;
        Ok(StreamFragment {
            stream: RecordingStream::CameraVideo,
            checkpoint_sequence: None,
            stream_sequence,
            artifact: self.artifact.video.clone(),
            bytes: self.bytes,
            sha256: self.artifact.media.sha256.clone(),
            facts: StreamFragmentFacts {
                start_offset_ms,
                end_offset_ms,
                media_duration_ms: self.artifact.media.duration_ms,
                decoded_video_frames: Some(self.artifact.media.frame_count),
                avg_frame_rate: Some(frame_rate),
                r_frame_rate: Some(frame_rate),
            },
        })
    }
}

fn validate_request(request: &CameraRequest) -> Result<()> {
    validate_request_part("capture_id", &request.capture_id)?;
    validate_request_part("device_id", &request.device_id)
}

fn capture_error(message: &str, cause: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, message, cause)
}

fn invalid(message: &str) -> RecordError {
    RecordError::new(
        error_codes::INVALID_ARGS,
        message,
        "invalid private camera-session evidence",
    )
}
