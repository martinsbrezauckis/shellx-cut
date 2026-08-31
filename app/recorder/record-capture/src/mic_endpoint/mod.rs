//! Private microphone endpoint identity and resolution.
//!
//! Endpoint identifiers must never leave this module except into Cut's private
//! app-local preference file. The server maps an endpoint to a short-lived
//! capability token and a sanitized friendly label before it reaches any public
//! verb or UI surface.

use record_core::{error_codes, RecordError, Result};
use serde::{Deserialize, Serialize};
#[cfg(feature = "mic")]
use std::sync::atomic::AtomicBool;
#[cfg(feature = "mic")]
use std::sync::Arc;
#[cfg(feature = "mic")]
use std::thread::{self, JoinHandle};
#[cfg(feature = "mic")]
use std::time::Instant;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// A native input identity. This is a private-storage type, not a public API
/// payload: Windows uses the MMDevice endpoint id and macOS a Core Audio device
/// UID. Linux deliberately has no selected-input mode in this slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "platform", rename_all = "snake_case")]
pub enum MicrophoneEndpointRef {
    Windows { endpoint_id: String },
    Macos { device_uid: String },
}

/// The source fixed for one capture. A selected reference is resolved before
/// starting the stream and is never changed while the capture is running.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MicrophoneSource {
    #[default]
    SystemDefault,
    Selected(MicrophoneEndpointRef),
}

/// Native data returned only to the server's private microphone owner. `label`
/// is an OS-friendly display candidate; the server sanitizes it before public
/// projection. `reference` is deliberately not `Serialize`d by any verb type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrophoneEndpoint {
    pub reference: MicrophoneEndpointRef,
    pub label: String,
}

/// List active input endpoints on the hosts with a stable native identity.
/// Linux remains system-default-only and consequently returns no selectable
/// endpoints even when CPAL can observe input devices.
pub fn list_microphone_endpoints() -> Vec<MicrophoneEndpoint> {
    #[cfg(windows)]
    {
        return windows::list();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::list();
    }
    #[allow(unreachable_code)]
    Vec::new()
}

/// Resolve a fixed selected source at admission time. The stored endpoint id / UID
/// is compared against a fresh native enumeration, so a disconnected saved input
/// fails closed instead of switching to the current system default.
pub fn resolve_microphone_source(source: &MicrophoneSource) -> Result<()> {
    match source {
        MicrophoneSource::SystemDefault => Ok(()),
        MicrophoneSource::Selected(reference) => {
            let present = list_microphone_endpoints()
                .into_iter()
                .any(|endpoint| endpoint.reference == *reference);
            if present {
                Ok(())
            } else {
                Err(RecordError::new(
                    error_codes::CAPTURE,
                    "selected microphone is unavailable",
                    "the privately saved native microphone endpoint is absent",
                )
                .with_action("choose another microphone or switch to System Default"))
            }
        }
    }
}

/// Open the source already fixed for this recording. A selected endpoint is
/// re-checked before resolving its CPAL stream and never falls back to the
/// system default. The native identifier remains private throughout.
#[cfg(feature = "mic")]
pub(crate) fn spawn_microphone_capture(
    path: String,
    source: MicrophoneSource,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
) -> JoinHandle<Result<crate::mic::CapturedMicrophone>> {
    match cpal_device_for_source(&source) {
        Ok(device) => crate::mic::spawn_device_mic(path, device, stop, ready, capture_started),
        Err(error) => thread::spawn(move || Err(error)),
    }
}

/// A resolved native microphone plus the process-local claim that keeps another
/// Cut capture from opening the same device while this session is live.
#[cfg(feature = "mic")]
pub(crate) struct ReservedMicrophoneCapture {
    device: cpal::Device,
    reservation: crate::mic::MicrophoneCaptureReservation,
}

/// Resolve the exact selected/default source before a standalone voiceover
/// session starts. This is deliberately separate from worker spawning so the
/// caller can refuse a busy or missing device before it creates any artifact.
#[cfg(feature = "mic")]
pub(crate) fn reserve_microphone_capture(
    source: &MicrophoneSource,
) -> Result<ReservedMicrophoneCapture> {
    resolve_microphone_source(source)?;
    let reservation = crate::mic::reserve_microphone_capture()?;
    match cpal_device_for_source(source) {
        Ok(device) => Ok(ReservedMicrophoneCapture {
            device,
            reservation,
        }),
        Err(error) => Err(error),
    }
}

#[cfg(feature = "mic")]
pub(crate) fn spawn_reserved_microphone_capture(
    path: String,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
    reserved: ReservedMicrophoneCapture,
    recording_gate: Option<Arc<crate::mic::MicRecordingGate>>,
) -> JoinHandle<Result<crate::mic::CapturedMicrophone>> {
    crate::mic::spawn_device_mic_reserved(
        path,
        reserved.device,
        stop,
        ready,
        capture_started,
        reserved.reservation,
        recording_gate,
    )
}

/// Private pause-owner variant. The returned WAV starts at the first received
/// microphone packet; its offset remains a separately sealed fact consumed by
/// the common pause projection exactly once.
#[cfg(feature = "mic")]
#[cfg_attr(
    not(all(windows, feature = "capture-windows")),
    allow(dead_code, reason = "private Windows pause-sidecar capture path")
)]
pub(crate) fn spawn_reserved_microphone_capture_unpadded(
    path: String,
    stop: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    capture_started: Instant,
    reserved: ReservedMicrophoneCapture,
    recording_gate: Option<Arc<crate::mic::MicRecordingGate>>,
) -> JoinHandle<Result<crate::mic::CapturedMicrophone>> {
    crate::mic::spawn_device_mic_reserved_unpadded(
        path,
        reserved.device,
        stop,
        ready,
        capture_started,
        reserved.reservation,
        recording_gate,
    )
}

/// Run the bounded warm/test against the same source resolution used by capture.
/// No device name is part of the public result projection.
pub fn warm_microphone(source: &MicrophoneSource, max_ms: u64) -> crate::MicWarm {
    #[cfg(feature = "mic")]
    {
        let warm = crate::mic::warm_device(cpal_device_for_source(source).ok(), max_ms);
        return crate::MicWarm {
            live: warm.live,
            peak_dbfs: warm.peak_dbfs,
            supported: true,
        };
    }
    #[allow(unreachable_code)]
    {
        let _ = (source, max_ms);
        crate::MicWarm {
            live: false,
            peak_dbfs: None,
            supported: false,
        }
    }
}

#[cfg(feature = "mic")]
fn cpal_device_for_source(source: &MicrophoneSource) -> Result<cpal::Device> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    match source {
        MicrophoneSource::SystemDefault => host.default_input_device().ok_or_else(|| {
            RecordError::new(
                error_codes::CAPTURE,
                "no input device",
                "no default microphone found",
            )
            .with_action("connect/enable a microphone and grant mic permission")
        }),
        MicrophoneSource::Selected(reference) => {
            let endpoint = list_microphone_endpoints()
                .into_iter()
                .find(|endpoint| endpoint.reference == *reference)
                .ok_or_else(|| {
                    RecordError::new(
                        error_codes::CAPTURE,
                        "selected microphone is unavailable",
                        "the privately saved native microphone endpoint is absent",
                    )
                    .with_action("choose another microphone or switch to System Default")
                })?;
            let mut matching = host
                .input_devices()
                .map_err(|_| {
                    RecordError::new(
                        error_codes::CAPTURE,
                        "enumerate microphone inputs",
                        "the active capture backend could not enumerate microphone inputs",
                    )
                })?
                .filter_map(|device| {
                    device
                        .name()
                        .ok()
                        .filter(|name| name == &endpoint.label)
                        .map(|_| device)
                })
                .collect::<Vec<_>>();
            if matching.len() == 1 {
                Ok(matching.remove(0))
            } else {
                Err(RecordError::new(
                    error_codes::CAPTURE,
                    "selected microphone cannot be opened",
                    "the selected native endpoint does not map uniquely to an active capture device",
                )
                .with_action("choose another microphone or switch to System Default"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MicrophoneEndpointRef, MicrophoneSource};

    #[test]
    fn selected_source_is_private_identity_not_a_display_label() {
        let source = MicrophoneSource::Selected(MicrophoneEndpointRef::Windows {
            endpoint_id: "{0.0.1.00000000}.private-mmdevice-id".into(),
        });
        assert!(matches!(source, MicrophoneSource::Selected(_)));
    }

    #[test]
    fn default_source_preserves_existing_callers() {
        assert_eq!(MicrophoneSource::default(), MicrophoneSource::SystemDefault);
    }
}
