//! Explicit-use admission and recoverable start transition for a camera session.
//!
//! Kept separate from terminal sealing so native permission ownership stays
//! bounded and reviewable: passive probes never receive a Use camera intent.

use std::sync::atomic::AtomicBool;

use record_core::{error_codes, CameraTerminalState, RecordError, Result};

use crate::camera::validate_request_part;
use crate::camera_session::{CameraSession, CameraSessionBackend, CameraStopState};
use crate::{CameraReadiness, CameraRequest, CameraUseIntent, CaptureClock};

/// A recoverable rejected start returns the backend to its registry. This lets
/// a later explicit retry observe a truthful denied/busy/no-frame state instead
/// of silently discarding a native adapter after a prompt or probe failure.
#[derive(Debug)]
pub(crate) struct CameraSessionStartFailure<B: CameraSessionBackend> {
    backend: B,
    error: RecordError,
}

impl<B: CameraSessionBackend> CameraSessionStartFailure<B> {
    pub(crate) fn into_parts(self) -> (B, RecordError) {
        (self.backend, self.error)
    }
}

impl<B: CameraSessionBackend> CameraSession<B> {
    /// Return the backend's current passive pre-start probe without beginning a
    /// session or handing it an explicit permission-opening intent.
    pub(crate) fn probe(backend: &B, request: &CameraRequest) -> CameraReadiness {
        backend.readiness(request)
    }

    /// Compatibility wrapper for call sites that do not need to retain a
    /// backend after a rejected start. New runtime ownership should use
    /// `start_recoverable` so a native adapter can report a truthful state on a
    /// later explicit retry.
    #[allow(
        dead_code,
        reason = "existing deterministic lifecycle tests exercise this wrapper while production ownership uses start_recoverable"
    )]
    pub(crate) fn start(
        backend: B,
        intent: CameraUseIntent,
        screen_clock: &CaptureClock,
        stop: &AtomicBool,
    ) -> Result<Self> {
        Self::start_recoverable(backend, intent, screen_clock, stop)
            .map_err(|failure| failure.error)
    }

    /// Start only after a passive ready/permission-required probe, the explicit
    /// Use camera intent, and the screen backend's real clock origin. A
    /// pre-start refusal and cancellation both leave the backend unstarted.
    pub(crate) fn start_recoverable(
        mut backend: B,
        intent: CameraUseIntent,
        screen_clock: &CaptureClock,
        stop: &AtomicBool,
    ) -> std::result::Result<Self, CameraSessionStartFailure<B>> {
        let request = intent.request().clone();
        if let Err(error) = validate_request(&request) {
            return Err(CameraSessionStartFailure { backend, error });
        }
        match Self::probe(&backend, &request) {
            CameraReadiness::Ready { device, .. }
            | CameraReadiness::PermissionRequired { device, .. }
                if device.id == request.device_id => {}
            CameraReadiness::Ready { .. } | CameraReadiness::PermissionRequired { .. } => {
                return Err(CameraSessionStartFailure {
                    backend,
                    error: capture_error(
                        "selected camera probe returned a different device",
                        "camera capture requires a startable probe for the exact requested device_id",
                    ),
                });
            }
            _ => {
                return Err(CameraSessionStartFailure {
                    backend,
                    error: capture_error(
                        "selected camera did not pass the pre-start probe",
                        "camera capture requires Ready or PermissionRequired before an explicit start",
                    ),
                });
            }
        }
        let Some(screen_origin) = screen_clock.wait_started(stop) else {
            return Err(CameraSessionStartFailure {
                backend,
                error: capture_error(
                    "camera session was cancelled before screen capture started",
                    "the screen-owned CaptureClock never opened",
                ),
            });
        };
        if let Err(start_error) = backend.start(&intent, screen_origin) {
            let _ = backend.stop(CameraTerminalState::Cancelled);
            return Err(CameraSessionStartFailure {
                backend,
                error: start_error,
            });
        }
        Ok(Self {
            backend: Some(backend),
            request,
            screen_origin,
            first_frame_offset_ms: None,
            last_frame_end_offset_ms: None,
            stop_state: CameraStopState::Live,
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
