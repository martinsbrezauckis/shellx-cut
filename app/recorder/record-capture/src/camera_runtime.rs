//! Private native-camera adapter registry.
//!
//! This is deliberately not a Cut verb, UI surface, or device advertisement.
//! It gives a reviewed platform adapter one narrow path from permission-neutral
//! enumeration through an explicit Use camera intent to the screen owner's Stop
//! hand-off. The default registry has no adapter, so it cannot fabricate native
//! availability on any host.

use std::sync::atomic::AtomicBool;

use record_core::{error_codes, CameraTerminalState, RecordError, Result};

use crate::camera_session::{CameraSession, CameraSessionBackend, CameraSessionEvidence};
use crate::{
    CameraDevice, CameraFrameObservation, CameraReadiness, CameraRequest, CameraUseIntent,
    CaptureClock,
};

/// One future native adapter. Enumeration and readiness are permission-neutral;
/// `CameraSessionBackend::start` is the only method that receives a Use camera
/// intent and may therefore open a system permission prompt.
pub(crate) trait CameraRuntimeAdapter: CameraSessionBackend {
    fn enumerate(&self) -> Result<Vec<CameraDevice>>;
}

/// A missing adapter is intentionally distinct from an empty device list. The
/// latter can only be returned by a real adapter after a real enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CameraEnumeration {
    Unavailable { detail: &'static str },
    Devices(Vec<CameraDevice>),
}

/// Result of an explicit Use camera request. Refusal preserves the adapter for
/// recovery and carries the adapter's truthful readiness classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CameraUseOutcome {
    Unavailable { detail: &'static str },
    Refused(CameraReadiness),
    Started,
}

/// Terminal output owned by the screen recording's Stop action. `NoMedia` is a
/// valid zero-frame cancellation/device-loss result and never becomes a camera
/// artifact. A sealed value is already an independently editable artifact with
/// exact measured timing and complete-file hash facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CameraStopResult {
    NoMedia,
    Sealed(CameraSessionEvidence),
}

/// One per-screen-recording owner. There can be at most one active camera
/// session, and the caller must present the same capture id to observe frames or
/// Stop it. This prevents one recording from stopping or adopting another
/// recording's camera stream.
pub(crate) struct CameraRuntime {
    adapter: Option<Box<dyn CameraRuntimeAdapter>>,
    active: Option<ActiveCamera>,
    unavailable_detail: &'static str,
}

struct ActiveCamera {
    request: CameraRequest,
    session: CameraSession<Box<dyn CameraRuntimeAdapter>>,
}

impl CameraRuntime {
    /// Construct the safe production default. It deliberately does not inspect
    /// devices or permission state: no platform adapter has been admitted yet.
    pub(crate) fn unavailable_for_build() -> Self {
        Self {
            adapter: None,
            active: None,
            unavailable_detail: native_camera_blocker(),
        }
    }

    /// Register a platform adapter only after its native API, maintained source,
    /// and license have been reviewed. This constructor remains crate-private so
    /// it cannot become an accidental public camera surface before server wiring.
    pub(crate) fn with_adapter(adapter: Box<dyn CameraRuntimeAdapter>) -> Self {
        Self {
            adapter: Some(adapter),
            active: None,
            unavailable_detail: native_camera_blocker(),
        }
    }

    /// Enumerate through a real registered adapter only. The adapter contract
    /// forbids permission-opening work here; an unavailable build returns its
    /// blocker rather than a misleading empty list.
    pub(crate) fn enumerate(&self) -> Result<CameraEnumeration> {
        match self.adapter.as_ref() {
            Some(adapter) => Ok(CameraEnumeration::Devices(adapter.enumerate()?)),
            None => Ok(CameraEnumeration::Unavailable {
                detail: self.unavailable_detail,
            }),
        }
    }

    /// Return a passive readiness state. A running session is always Busy and
    /// cannot be implicitly replaced by another device request.
    pub(crate) fn readiness(&self, request: &CameraRequest) -> CameraReadiness {
        if self.active.is_some() {
            return CameraReadiness::Busy {
                detail: "another camera session is owned by the active screen recording".into(),
            };
        }
        match self.adapter.as_ref() {
            Some(adapter) => adapter.readiness(request),
            None => CameraReadiness::Missing {
                detail: self.unavailable_detail.into(),
            },
        }
    }

    /// Start only with an explicit Use camera intent. `PermissionRequired` is
    /// admitted to the backend only here; Ready is also admitted. Denied, Busy,
    /// NoFrame, Missing, and merely Enumerated states return without invoking a
    /// permission-capable backend start.
    pub(crate) fn use_camera(
        &mut self,
        intent: CameraUseIntent,
        screen_clock: &CaptureClock,
        screen_stop: &AtomicBool,
    ) -> Result<CameraUseOutcome> {
        if self.active.is_some() {
            return Ok(CameraUseOutcome::Refused(CameraReadiness::Busy {
                detail: "another camera session is owned by the active screen recording".into(),
            }));
        }
        let Some(adapter) = self.adapter.take() else {
            return Ok(CameraUseOutcome::Unavailable {
                detail: self.unavailable_detail,
            });
        };
        let request = intent.request().clone();
        let readiness = adapter.readiness(&request);
        if !allows_explicit_start(&readiness, &request) {
            self.adapter = Some(adapter);
            return Ok(CameraUseOutcome::Refused(readiness));
        }

        match CameraSession::start_recoverable(adapter, intent, screen_clock, screen_stop) {
            Ok(session) => {
                self.active = Some(ActiveCamera { request, session });
                Ok(CameraUseOutcome::Started)
            }
            Err(failure) => {
                let (adapter, error) = failure.into_parts();
                // An adapter that prompted or probed during start must update
                // its passive state before returning. That yields a typed denial,
                // busy, or no-frame result instead of a generic retry loop.
                let recovered = adapter.readiness(&request);
                self.adapter = Some(adapter);
                if matches!(
                    recovered,
                    CameraReadiness::PermissionDenied { .. }
                        | CameraReadiness::Busy { .. }
                        | CameraReadiness::NoFrame { .. }
                ) {
                    Ok(CameraUseOutcome::Refused(recovered))
                } else {
                    Err(error)
                }
            }
        }
    }

    /// Record an actual delivered frame interval under the same screen owner.
    /// Browser, setup, or permission timestamps never enter the CameraArtifact
    /// clock, and a sample start never substitutes for its end boundary.
    pub(crate) fn observe_frame(
        &mut self,
        capture_id: &str,
        observation: CameraFrameObservation,
    ) -> Result<()> {
        let active = self.active.as_mut().ok_or_else(|| {
            invalid_runtime("camera frame arrived without an active camera session")
        })?;
        ensure_capture_owner(&active.request, capture_id)?;
        active.session.observe_frame(observation)
    }

    /// The shared screen Stop owner must pass the exact capture id. It receives
    /// one sealed independent artifact, or NoMedia, and the adapter is returned
    /// to the registry for a later explicit request. A failed Stop is terminal
    /// for this session and is never retried implicitly.
    pub(crate) fn stop_for_screen_owner(
        &mut self,
        capture_id: &str,
        terminal_state: CameraTerminalState,
    ) -> Result<CameraStopResult> {
        let Some(mut active) = self.active.take() else {
            return Err(invalid_runtime(
                "camera Stop arrived without an active camera session",
            ));
        };
        if let Err(error) = ensure_capture_owner(&active.request, capture_id) {
            self.active = Some(active);
            return Err(error);
        }
        if let Err(error) = active.session.stop(terminal_state) {
            self.adapter = Some(active.session.into_backend());
            return Err(error);
        }
        let (adapter, evidence) = active.session.finish();
        self.adapter = Some(adapter);
        match evidence? {
            None => Ok(CameraStopResult::NoMedia),
            Some(evidence) => Ok(CameraStopResult::Sealed(evidence)),
        }
    }
}

fn allows_explicit_start(readiness: &CameraReadiness, request: &CameraRequest) -> bool {
    match readiness {
        CameraReadiness::Ready { device, .. }
        | CameraReadiness::PermissionRequired { device, .. } => device.id == request.device_id,
        CameraReadiness::Missing { .. }
        | CameraReadiness::Enumerated { .. }
        | CameraReadiness::PermissionDenied { .. }
        | CameraReadiness::Busy { .. }
        | CameraReadiness::NoFrame { .. } => false,
    }
}

fn ensure_capture_owner(request: &CameraRequest, capture_id: &str) -> Result<()> {
    if request.capture_id == capture_id {
        return Ok(());
    }
    Err(invalid_runtime(
        "camera operation capture id does not match the active screen owner",
    ))
}

fn invalid_runtime(message: &str) -> RecordError {
    RecordError::new(
        error_codes::INVALID_ARGS,
        message,
        "invalid private camera runtime ownership",
    )
}

/// Specific admission blocker for the fallback registry. These are not
/// installation instructions: supported platforms construct their private
/// adapters through their camera owner; this fallback never advertises one.
pub(crate) fn native_camera_blocker() -> &'static str {
    if cfg!(all(windows, feature = "capture-windows")) {
        "Windows camera capture is unavailable through the fallback runtime; the private Media Foundation owner supplies the supported path"
    } else if cfg!(all(target_os = "macos", feature = "capture-macos")) {
        "macOS camera capture is unavailable through the fallback runtime; the private AVFoundation owner supplies the supported path"
    } else if cfg!(all(target_os = "linux", feature = "capture-linux")) {
        "Linux camera capture is not registered: the reviewed ashpd/PipeWire path is ScreenCast capture, not a Camera portal or V4L2 adapter"
    } else {
        "camera capture adapter is not compiled for this build"
    }
}
