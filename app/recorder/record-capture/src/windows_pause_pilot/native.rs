//! Windows-only construction for the private owned pause-pilot worker.
//!
//! This is deliberately outside the ordinary one-shot capture backend. It has
//! no server/UI caller: a later durable server pause-session must retain the
//! returned lifecycle owner beside the existing private dispatch adapter and
//! event translator.

use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::{Duration, Instant};

use record_core::{Result, Settings};
use windows_capture::{
    capture::GraphicsCaptureApiHandler,
    monitor::Monitor as WcMonitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings as WcSettings,
    },
};

use super::audio_run::RequiredWindowsPauseAudioFactory;
use super::input_run::RequiredWindowsPauseInputFactory;
use super::{
    lifecycle::spawn_owned, WindowsPausePilotProfile, WindowsPausePilotStartError,
    WindowsPausePilotThread,
};
use crate::{
    checkpoint::Checkpoints,
    windows::{
        cap_err, monitor_surface, observe_wgc_start, wgc_monitor_range, WindowsCheckpointPublisher,
    },
    windows_wgc_handler::{EncFlags, Handler, LiveWgcControl},
    windows_wgc_run::{WgcAcceptedCapture, WgcControlFactory, WgcRunOwner, WgcStartedControl},
    CheckpointConfig,
};

/// Actual WGC factory for the private pause pilot. It has the same native
/// `Handler::start_free_threaded` encoder/control path as ordinary monitor
/// capture, but requires an exact monitor rectangle because the pilot must not
/// fabricate a later pointer/event transform from a missing range.
struct WindowsPausePilotWgcFactory {
    fps: u32,
}

impl WgcControlFactory<WcMonitor> for WindowsPausePilotWgcFactory {
    type Control = LiveWgcControl;

    fn start(
        &mut self,
        monitor: &WcMonitor,
        destination: &Path,
    ) -> Result<WgcStartedControl<Self::Control>> {
        let width = monitor
            .width()
            .map_err(|error| cap_err("pause pilot monitor width", error))?;
        let height = monitor
            .height()
            .map_err(|error| cap_err("pause pilot monitor height", error))?;
        let range = monitor_surface(monitor)
            .and_then(wgc_monitor_range)
            .filter(|range| range.width == width && range.height == height)
            .ok_or_else(|| {
                cap_err(
                    "start pause pilot",
                    "the exact monitor has no stable native capture range",
                )
            })?;
        let accepted = WgcAcceptedCapture::new(
            Settings {
                width,
                height,
                fps: self.fps as f32,
                audio_rate: 48_000,
            },
            Some(range),
        )?;
        let flags = EncFlags {
            w: width,
            h: height,
            fps: self.fps,
            path: destination.display().to_string(),
            crop: None,
            readiness: None,
            source_lifecycle: None,
            // This private monitor-only pilot has no outer one-shot wait loop
            // to wake on `on_closed`; retain a local handler stop flag to
            // satisfy the shared WGC callback contract without inventing a
            // public lifecycle projection.
            stop: Arc::new(AtomicBool::new(false)),
            timing: None,
        };
        let control = Handler::start_free_threaded(WcSettings::new(
            *monitor,
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Rgba8,
            flags,
        ))
        .map_err(|error| cap_err("start pause pilot", error))?;
        Ok(WgcStartedControl::new(
            LiveWgcControl {
                close: Some(Box::new(move || {
                    control
                        .stop()
                        .map_err(|error| cap_err("finalize WGC checkpoint", error))
                })),
                source_lifecycle: None,
            },
            accepted,
        ))
    }
}

/// Start a private, owned WGC pause-pilot thread from real Windows capture
/// primitives. This is intentionally not called by the ordinary one-shot
/// `Capture::capture` path and cannot be selected by the current server/UI.
pub(super) fn start_private(
    profile: WindowsPausePilotProfile,
    checkpoint: CheckpointConfig,
) -> std::result::Result<WindowsPausePilotThread, WindowsPausePilotStartError> {
    crate::windows_runtime::pin_process_mta()
        .map_err(|_| WindowsPausePilotStartError::NativeStartFailed)?;
    if checkpoint.interval_ms == 0 {
        return Err(WindowsPausePilotStartError::NativeStartFailed);
    }
    // Match the admitted private checkpoint cadence. It is a recovery rollover
    // interval, never a short command-polling loop: each elapsed interval seals
    // and publishes an immutable WGC checkpoint before reopening.
    let rollover_interval = Duration::from_millis(checkpoint.interval_ms);
    let audio_capture_dir = PathBuf::from(&checkpoint.manifest_dir);
    // These WGC facts use a fresh worker-local recording clock. They are not
    // yet `RunSealCoordinator`-normalized: the later durable server owner must
    // establish its session origin from the returned `Started.monotonic_at`
    // rather than treating `Started.observed_start_ms` as already-zero.
    let started_at = Instant::now();
    let profile_for_factory = profile.clone();
    spawn_owned(
        profile,
        |id| crate::windows_monitor_target::resolve_monitor(id).ok(),
        move |monitor| {
            let checkpoints = Checkpoints::open(Some(&checkpoint))?
                .ok_or_else(|| cap_err("start pause pilot", "a checkpoint manifest is required"))?;
            Ok(WgcRunOwner::new(
                monitor,
                WindowsPausePilotWgcFactory {
                    fps: profile_for_factory.fps(),
                },
                WindowsCheckpointPublisher {
                    checkpoints,
                    include_native_startup: false,
                    timing: None,
                },
            ))
        },
        Box::new(RequiredWindowsPauseInputFactory),
        Box::new(RequiredWindowsPauseAudioFactory::new(
            audio_capture_dir,
            crate::MicrophoneSource::SystemDefault,
        )),
        move || u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
        move || observe_wgc_start(started_at),
        move || {
            (
                u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
                Instant::now(),
            )
        },
        rollover_interval,
    )
}
