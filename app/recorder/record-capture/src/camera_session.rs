//! Private camera-session lifecycle built around the screen-owned capture clock.
//!
//! This is not a server start surface. A private native adapter may feed its
//! typed evidence through the spine, while a later capture owner may append it
//! to a durable session journal; this module never owns the screen take.

use std::time::Instant;

use record_core::{
    error_codes, CameraArtifact, CameraClockRange, CameraTerminalState, RecordError, Result,
};

pub(crate) use crate::camera_finalization::CameraMediaSeal;
pub(crate) use crate::camera_session_evidence::CameraSessionEvidence;
use crate::{CameraFrameObservation, CameraReadiness, CameraRequest, CameraUseIntent};

/// Truthful terminal result from a camera backend. `NoMedia` means that no
/// camera prefix exists; it is not a malformed empty `CameraMediaSeal`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CameraStopOutcome {
    NoMedia,
    Sealed(CameraMediaSeal),
}

/// Minimal backend seam for the camera-session spine.
///
/// `readiness` is a passive pre-start probe and must not enumerate through a
/// permission-opening API. `start` receives the private explicit-use intent as
/// well as the screen-owned origin; it is the only operation allowed to ask the
/// OS for camera permission. `stop` returns `NoMedia` for a truthful zero-frame
/// terminal or a `Sealed` local prefix with independently measured media facts.
/// If `start` returns an error after it may have acquired resources, the caller
/// invokes one `stop(Cancelled)` cleanup and returns the original start error.
pub(crate) trait CameraSessionBackend {
    fn readiness(&self, request: &CameraRequest) -> CameraReadiness;
    fn start(&mut self, intent: &CameraUseIntent, screen_origin: Instant) -> Result<()>;
    fn stop(&mut self, terminal_state: CameraTerminalState) -> Result<CameraStopOutcome>;

    /// Drain only native samples that were delivered after the backend admitted
    /// its record path. The session applies them after native Stop has completed
    /// so a callback cannot race a fabricated final-frame boundary.
    fn take_frame_observations(&mut self) -> Result<Vec<CameraFrameObservation>> {
        Ok(Vec::new())
    }

    /// A native backend may strengthen the requested terminal state. In
    /// particular, Media Foundation device-loss events must never become a
    /// misleading `Complete` camera artifact while the screen take continues.
    fn terminal_state(&self, requested: CameraTerminalState) -> CameraTerminalState {
        requested
    }
}

impl<T: CameraSessionBackend + ?Sized> CameraSessionBackend for Box<T> {
    fn readiness(&self, request: &CameraRequest) -> CameraReadiness {
        (**self).readiness(request)
    }

    fn start(&mut self, intent: &CameraUseIntent, screen_origin: Instant) -> Result<()> {
        (**self).start(intent, screen_origin)
    }

    fn stop(&mut self, terminal_state: CameraTerminalState) -> Result<CameraStopOutcome> {
        (**self).stop(terminal_state)
    }

    fn take_frame_observations(&mut self) -> Result<Vec<CameraFrameObservation>> {
        (**self).take_frame_observations()
    }

    fn terminal_state(&self, requested: CameraTerminalState) -> CameraTerminalState {
        (**self).terminal_state(requested)
    }
}

/// An enabled camera sidecar that has passed its probe and joined the shared
/// screen clock. Frames are observed explicitly so setup or browser timestamps
/// can never become camera timing evidence.
#[derive(Debug)]
pub(crate) struct CameraSession<B: CameraSessionBackend> {
    pub(super) backend: Option<B>,
    pub(super) request: CameraRequest,
    pub(super) screen_origin: Instant,
    pub(super) first_frame_offset_ms: Option<u64>,
    pub(super) last_frame_end_offset_ms: Option<u64>,
    pub(super) stop_state: CameraStopState,
}

#[derive(Debug)]
pub(super) enum CameraStopState {
    Live,
    Stopped {
        terminal_state: CameraTerminalState,
        outcome: CameraStopOutcome,
    },
    Failed(RecordError),
}

impl<B: CameraSessionBackend> CameraSession<B> {
    /// Record one real delivered frame interval against the screen origin, never
    /// against wall clock, setup, or browser event time. A sample start alone
    /// is intentionally insufficient: it cannot establish a final media end.
    pub(crate) fn observe_frame(&mut self, observation: CameraFrameObservation) -> Result<()> {
        if !matches!(&self.stop_state, CameraStopState::Live) {
            return Err(invalid("camera session cannot accept frames after stop"));
        }
        let start_offset = observation
            .started_at
            .checked_duration_since(self.screen_origin)
            .ok_or_else(|| invalid("camera frame precedes the screen CaptureClock origin"))?;
        let end_offset = observation
            .ended_at
            .checked_duration_since(self.screen_origin)
            .ok_or_else(|| invalid("camera frame precedes the screen CaptureClock origin"))?;
        let start_offset_ms = u64::try_from(start_offset.as_millis())
            .map_err(|_| invalid("camera frame offset exceeds the supported range"))?;
        let end_offset_ms = u64::try_from(end_offset.as_millis())
            .map_err(|_| invalid("camera frame offset exceeds the supported range"))?;
        if end_offset_ms <= start_offset_ms {
            return Err(invalid(
                "camera frame observation must have a non-empty interval",
            ));
        }
        if self
            .last_frame_end_offset_ms
            .is_some_and(|last_end| start_offset_ms < last_end || end_offset_ms <= last_end)
        {
            return Err(invalid(
                "camera frame observation intervals must be monotonic and non-overlapping",
            ));
        }
        self.first_frame_offset_ms.get_or_insert(start_offset_ms);
        self.last_frame_end_offset_ms = Some(end_offset_ms);
        Ok(())
    }

    /// Stop the camera independently. This does not signal, stop, or otherwise
    /// alter the screen capture; a `DeviceLost` terminal therefore preserves its
    /// separately owned screen take.
    pub(crate) fn stop(&mut self, terminal_state: CameraTerminalState) -> Result<()> {
        if !matches!(&self.stop_state, CameraStopState::Live) {
            return Err(invalid("camera session is already stopped"));
        }
        let (outcome, terminal_state, observations) = {
            let backend = self.backend.as_mut().ok_or_else(|| {
                invalid("camera session backend is unavailable after terminal cleanup")
            })?;
            let outcome = match backend.stop(terminal_state) {
                Ok(outcome) => outcome,
                Err(error) => {
                    self.stop_state = CameraStopState::Failed(error.clone());
                    return Err(error);
                }
            };
            let terminal_state = backend.terminal_state(terminal_state);
            let observations = match backend.take_frame_observations() {
                Ok(observations) => observations,
                Err(error) => {
                    self.stop_state = CameraStopState::Failed(error.clone());
                    return Err(error);
                }
            };
            (outcome, terminal_state, observations)
        };
        for observation in observations {
            if let Err(error) = self.observe_frame(observation) {
                self.stop_state = CameraStopState::Failed(error.clone());
                return Err(error);
            }
        }
        match outcome {
            CameraStopOutcome::NoMedia => {
                self.stop_state = CameraStopState::Stopped {
                    terminal_state,
                    outcome: CameraStopOutcome::NoMedia,
                };
                Ok(())
            }
            CameraStopOutcome::Sealed(seal) if seal.bytes() != 0 => {
                if let Err(error) = self.ensure_duration_projection_matches_observed_clock(&seal) {
                    self.stop_state = CameraStopState::Failed(error.clone());
                    return Err(error);
                }
                self.stop_state = CameraStopState::Stopped {
                    terminal_state,
                    outcome: CameraStopOutcome::Sealed(seal),
                };
                Ok(())
            }
            CameraStopOutcome::Sealed(_) => {
                let error = invalid("sealed camera evidence must have non-zero bytes");
                self.stop_state = CameraStopState::Failed(error.clone());
                Err(error)
            }
        }
    }

    /// Validate and expose the independent camera prefix for later durable
    /// journal ownership. A session with no observed frames cannot invent one.
    pub(crate) fn seal(self) -> Result<CameraSessionEvidence> {
        let (_backend, evidence) = self.finish();
        evidence?.ok_or_else(|| invalid("camera session terminal has no media to seal"))
    }

    /// Return the retained adapter with one terminal result. `finish` is the
    /// registry hand-off used by the shared screen Stop owner: it never creates
    /// a camera artifact for `NoMedia`, and it returns the backend even after a
    /// failed seal so a future explicit retry can recover deliberately.
    pub(crate) fn finish(mut self) -> (B, Result<Option<CameraSessionEvidence>>) {
        if matches!(&self.stop_state, CameraStopState::Live) {
            let _ = self.stop(CameraTerminalState::Cancelled);
        }
        let evidence = self.evidence();
        let backend = self
            .backend
            .take()
            .expect("camera session retains its backend until terminal hand-off");
        (backend, evidence)
    }

    /// Return a backend after a terminal stop failure. A live session is first
    /// cancelled exactly once, so moving it out cannot leak a native stream.
    pub(crate) fn into_backend(mut self) -> B {
        if matches!(&self.stop_state, CameraStopState::Live) {
            let _ = self.stop(CameraTerminalState::Cancelled);
        }
        self.backend
            .take()
            .expect("camera session retains its backend until terminal hand-off")
    }

    fn evidence(&self) -> Result<Option<CameraSessionEvidence>> {
        let (terminal_state, seal) = match &self.stop_state {
            CameraStopState::Live => {
                return Err(invalid("camera session must stop before sealing"));
            }
            CameraStopState::Stopped {
                terminal_state: _,
                outcome: CameraStopOutcome::NoMedia,
            } => return Ok(None),
            CameraStopState::Stopped {
                terminal_state,
                outcome: CameraStopOutcome::Sealed(seal),
            } => (*terminal_state, seal),
            CameraStopState::Failed(error) => return Err(error.clone()),
        };
        let frame_range = self.frame_range()?;
        let artifact = CameraArtifact::new(
            self.request.capture_id.clone(),
            seal.artifact_id().to_owned(),
            seal.video().to_owned(),
            frame_range,
            seal.media().clone(),
            terminal_state,
        )?;
        Ok(Some(CameraSessionEvidence {
            artifact,
            bytes: seal.bytes(),
        }))
    }

    fn frame_range(&self) -> Result<CameraClockRange> {
        let first_frame_offset_ms = self
            .first_frame_offset_ms
            .ok_or_else(|| invalid("camera session has no observed first frame"))?;
        let end_frame_offset_ms = self
            .last_frame_end_offset_ms
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

    /// The finalizer's native adapter proves its own exact sample-time contract.
    /// This shared session boundary compares only the deterministic millisecond
    /// floor projection carried by `CameraMediaFacts` and `CaptureClock`.
    fn ensure_duration_projection_matches_observed_clock(
        &self,
        seal: &CameraMediaSeal,
    ) -> Result<()> {
        let range = self.frame_range()?;
        let observed_duration_ms = range
            .end_frame_offset_ms
            .checked_sub(range.first_frame_offset_ms)
            .ok_or_else(|| invalid("camera session observed frame range underflowed"))?;
        if seal.media().duration_ms != observed_duration_ms {
            return Err(invalid(
                "finalized camera duration projection does not match the observed CaptureClock interval",
            ));
        }
        Ok(())
    }
}

impl<B: CameraSessionBackend> Drop for CameraSession<B> {
    fn drop(&mut self) {
        if matches!(&self.stop_state, CameraStopState::Live) {
            // Reuse the normal terminal path so callback evidence is drained
            // only after native Stop and final media can never bypass the
            // native-clock evidence rule during cleanup.
            let _ = self.stop(CameraTerminalState::Cancelled);
        }
    }
}

pub(super) fn invalid(message: &str) -> RecordError {
    RecordError::new(
        error_codes::INVALID_ARGS,
        message,
        "invalid private camera-session evidence",
    )
}
