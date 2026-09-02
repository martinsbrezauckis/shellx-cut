//! Process-local audio-meter ownership and read-only status projection.
//!
//! A handle exists only for audio that the accepted capture already owns. The
//! status path never enumerates or opens an input, and terminalization makes a
//! retained packet stale before a caller can mistake it for live delivery.

use record_capture::{AudioLevelLifecycle, AudioLevelSnapshot, RollingAudioLevel};
use serde::Serialize;
use std::sync::Arc;

#[cfg(target_os = "macos")]
const MACOS_SYSTEM_AUDIO_METER_DETAIL: &str =
    "macOS system-audio meter is unavailable: screencapturekit 8 exposes Audio callbacks, but this active SCRecordingOutput stream is deliberately video-only because capturesAudio caused video stalls and zero audio buffers; enabling it would change source output. The current Core Audio tap transfers PCM to Rust only at Stop.";

#[derive(Debug, Clone)]
pub(crate) struct CaptureAudioMeters {
    microphone: MeterSource,
    system_audio: MeterSource,
}

#[derive(Debug, Clone)]
enum MeterSource {
    NotRequested,
    Native(Arc<RollingAudioLevel>),
    #[cfg(not(any(windows, target_os = "linux")))]
    Unavailable {
        detail: &'static str,
    },
}

/// Bounded read-only source status returned from `screen_record.status`.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AudioMetersStatus {
    pub(crate) microphone: AudioMeterStatus,
    pub(crate) system_audio: AudioMeterStatus,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AudioMeterStatus {
    pub(crate) state: &'static str,
    pub(crate) peak_dbfs: Option<f32>,
    pub(crate) rms_dbfs: Option<f32>,
    pub(crate) decayed_peak_dbfs: Option<f32>,
    pub(crate) sample_age_ms: Option<u64>,
    pub(crate) stale: bool,
    pub(crate) clipping: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<&'static str>,
}

impl CaptureAudioMeters {
    pub(crate) fn new(microphone_requested: bool, system_audio_requested: bool) -> Self {
        Self {
            microphone: MeterSource::from_requested(microphone_requested),
            system_audio: system_audio_source(system_audio_requested),
        }
    }

    pub(crate) fn microphone_meter(&self) -> Option<Arc<RollingAudioLevel>> {
        self.microphone.meter()
    }

    pub(crate) fn system_audio_meter(&self) -> Option<Arc<RollingAudioLevel>> {
        self.system_audio.meter()
    }

    pub(crate) fn status(&self) -> AudioMetersStatus {
        AudioMetersStatus {
            microphone: self.microphone.status(),
            system_audio: self.system_audio.status(),
        }
    }

    pub(crate) fn mark_terminal(&self) {
        self.microphone.mark_terminal();
        self.system_audio.mark_terminal();
    }
}

impl MeterSource {
    fn from_requested(requested: bool) -> Self {
        if requested {
            Self::Native(Arc::new(RollingAudioLevel::new()))
        } else {
            Self::NotRequested
        }
    }

    fn meter(&self) -> Option<Arc<RollingAudioLevel>> {
        match self {
            Self::Native(meter) => Some(Arc::clone(meter)),
            Self::NotRequested => None,
            #[cfg(not(any(windows, target_os = "linux")))]
            Self::Unavailable { .. } => None,
        }
    }

    fn status(&self) -> AudioMeterStatus {
        match self {
            Self::NotRequested => AudioMeterStatus::not_requested(),
            #[cfg(not(any(windows, target_os = "linux")))]
            Self::Unavailable { detail } => AudioMeterStatus::unavailable(detail),
            Self::Native(meter) => AudioMeterStatus::from_snapshot(meter.snapshot()),
        }
    }

    fn mark_terminal(&self) {
        if let Self::Native(meter) = self {
            meter.mark_stopped();
        }
    }
}

fn system_audio_source(requested: bool) -> MeterSource {
    if !requested {
        return MeterSource::NotRequested;
    }
    #[cfg(target_os = "macos")]
    {
        MeterSource::Unavailable {
            detail: MACOS_SYSTEM_AUDIO_METER_DETAIL,
        }
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        MeterSource::from_requested(true)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        MeterSource::Unavailable {
            detail: "system-audio meter is unavailable: the current backend does not deliver PCM packets to Rust callbacks.",
        }
    }
}

impl AudioMeterStatus {
    fn not_requested() -> Self {
        Self {
            state: "not_requested",
            peak_dbfs: None,
            rms_dbfs: None,
            decayed_peak_dbfs: None,
            sample_age_ms: None,
            stale: true,
            clipping: false,
            detail: None,
        }
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    fn unavailable(detail: &'static str) -> Self {
        Self {
            state: "unavailable",
            detail: Some(detail),
            ..Self::not_requested()
        }
    }

    fn from_snapshot(snapshot: AudioLevelSnapshot) -> Self {
        let state = match snapshot.lifecycle {
            AudioLevelLifecycle::DeviceLost => "device_lost",
            AudioLevelLifecycle::Stopped => "stopped",
            AudioLevelLifecycle::Active if snapshot.sample_age_ms.is_none() => "awaiting_samples",
            AudioLevelLifecycle::Active if snapshot.stale => "stale",
            AudioLevelLifecycle::Active => "live",
        };
        Self {
            state,
            peak_dbfs: snapshot.peak_dbfs,
            rms_dbfs: snapshot.rms_dbfs,
            decayed_peak_dbfs: snapshot.decayed_peak_dbfs,
            sample_age_ms: snapshot.sample_age_ms,
            stale: snapshot.stale,
            clipping: state == "live" && snapshot.clipping,
            detail: None,
        }
    }
}
