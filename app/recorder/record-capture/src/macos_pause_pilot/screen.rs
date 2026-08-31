//! Exact-target ScreenCaptureKit owner for a private macOS pause run.
//!
//! The owner re-enumerates the opaque monitor at every start. It never accepts
//! an ordinal, title, primary-display flag, or geometry fallback. A capture
//! generation is only reported after SCK accepts its recording output; terminal
//! close waits for that output to finish before publishing its checkpoint.

use super::{
    close::NativeCloseState, MacosPauseAcceptedScreen, MacosPausePilotProfile,
    MacosPausePilotStarted, MacosPauseScreenOwner, MacosPauseScreenRange, MacosPauseStartError,
    MacosSealedScreenRun,
};
use crate::{checkpoint::Checkpoints, macos_checkpoint::SegmentOutput, CheckpointConfig};
use record_core::Settings;
use record_recovery::CheckpointFacts;
use screencapturekit::{prelude::*, shareable_content::SCShareableContentInfo};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// One synchronous ScreenCaptureKit generation. There is no detached native
/// thread: `seal_active` and `abort_and_join` wait for the recording-output
/// completion callback before they return.
pub(crate) struct RequiredMacosPauseScreenOwner {
    checkpoints: Checkpoints,
    origin: Instant,
    next_physical_generation: u64,
    active: Option<ActiveScreen>,
}

struct ActiveScreen {
    stream: SCStream,
    output: SegmentOutput,
    checkpoint_sequence: u64,
    staging: std::path::PathBuf,
    started: MacosPausePilotStarted,
    close: NativeCloseState,
}

impl ActiveScreen {
    fn close_native(&mut self) -> Result<(), ()> {
        let Self {
            stream,
            output,
            close,
            ..
        } = self;
        close
            .close(
                || stream.stop_capture().map_err(|_| ()),
                || {
                    stream
                        .remove_recording_output(output.output())
                        .map_err(|_| ())
                },
                || output.wait_complete().map_err(|_| ()),
            )
            .map_err(|_| ())
    }
}

/// A failed native start has no retained `ActiveScreen` to hand back to the
/// owner. Still use the identical close state machine while this local stream
/// exists, and surface an exhausted close instead of ignoring it.
fn close_failed_start(stream: &SCStream, output: &SegmentOutput) {
    let mut close = NativeCloseState::default();
    let first = close.close(
        || stream.stop_capture().map_err(|_| ()),
        || {
            stream
                .remove_recording_output(output.output())
                .map_err(|_| ())
        },
        || output.wait_complete().map_err(|_| ()),
    );
    let result = if first.is_err() {
        close.close(
            || stream.stop_capture().map_err(|_| ()),
            || {
                stream
                    .remove_recording_output(output.output())
                    .map_err(|_| ())
            },
            || output.wait_complete().map_err(|_| ()),
        )
    } else {
        first
    };
    if result.is_err() {
        eprintln!("private macOS pause start failed with unresolved native close state");
    }
}

impl RequiredMacosPauseScreenOwner {
    pub(crate) fn new(checkpoint: CheckpointConfig) -> Result<Self, MacosPauseStartError> {
        let checkpoints = Checkpoints::open(Some(&checkpoint))
            .map_err(|_| MacosPauseStartError::NativeStartFailed)?
            .ok_or(MacosPauseStartError::NativeStartFailed)?;
        Ok(Self {
            checkpoints,
            origin: Instant::now(),
            next_physical_generation: 1,
            active: None,
        })
    }

    fn elapsed_ms(&self, at: Instant) -> Result<u64, MacosPauseStartError> {
        u64::try_from(
            at.checked_duration_since(self.origin)
                .ok_or(MacosPauseStartError::NativeStartFailed)?
                .as_millis(),
        )
        .map_err(|_| MacosPauseStartError::NativeStartFailed)
    }
}

impl MacosPauseScreenOwner for RequiredMacosPauseScreenOwner {
    fn start(
        &mut self,
        profile: &MacosPausePilotProfile,
    ) -> Result<MacosPausePilotStarted, MacosPauseStartError> {
        if self.active.is_some() || self.next_physical_generation == 0 {
            return Err(MacosPauseStartError::NativeStartFailed);
        }
        crate::macos::sck_init_cg();
        let display = crate::macos_monitor_target::resolve_monitor(profile.exact_monitor_id())
            .ok_or(MacosPauseStartError::NativeStartFailed)?;
        // `resolve_monitor` already verifies the opaque digest against a fresh
        // SCK enumeration. Re-checking the returned value keeps this boundary
        // fail-closed if that helper changes.
        if crate::macos_monitor_target::monitor_id(&display).as_deref()
            != Some(profile.exact_monitor_id())
        {
            return Err(MacosPauseStartError::NativeStartFailed);
        }
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build();
        let (pixel_width, pixel_height) = SCShareableContentInfo::for_filter(&filter)
            .map(|info| info.pixel_size())
            .filter(|(width, height)| *width >= 2 && *height >= 2)
            .ok_or(MacosPauseStartError::NativeStartFailed)?;
        let width = pixel_width & !1;
        let height = pixel_height & !1;
        if width == 0 || height == 0 {
            return Err(MacosPauseStartError::NativeStartFailed);
        }
        let started_at = Instant::now();
        let observed_start_ms = self.elapsed_ms(started_at)?;
        let (checkpoint_sequence, staging) = self
            .checkpoints
            .begin(observed_start_ms)
            .map_err(|_| MacosPauseStartError::NativeStartFailed)?;
        let output =
            SegmentOutput::new(&staging).map_err(|_| MacosPauseStartError::NativeStartFailed)?;
        let stream = SCStream::new(
            &filter,
            &crate::macos::recording_stream_config(width, height, profile.fps(), false),
        );
        if stream.add_recording_output(output.output()).is_err() || stream.start_capture().is_err()
        {
            close_failed_start(&stream, &output);
            return Err(MacosPauseStartError::NativeStartFailed);
        }
        let monotonic_at = Instant::now();
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .ok_or(MacosPauseStartError::NativeStartFailed)?;
        let physical_generation = self.next_physical_generation;
        self.next_physical_generation = self
            .next_physical_generation
            .checked_add(1)
            .ok_or(MacosPauseStartError::NativeStartFailed)?;
        let started = MacosPausePilotStarted {
            exact_monitor_id: profile.exact_monitor_id().to_owned(),
            physical_generation,
            observed_start_ms,
            monotonic_at,
            unix_ms,
            accepted: MacosPauseAcceptedScreen {
                settings: Settings {
                    width,
                    height,
                    fps: profile.fps() as f32,
                    audio_rate: 48_000,
                },
                // Pause capture refuses all input/cursor mapping. The range is
                // therefore only the negotiated SCK output extent, not a
                // fabricated global desktop transform.
                range: MacosPauseScreenRange {
                    origin_x: 0,
                    origin_y: 0,
                    width,
                    height,
                },
            },
        };
        self.active = Some(ActiveScreen {
            stream,
            output,
            checkpoint_sequence,
            staging,
            started: started.clone(),
            close: NativeCloseState::default(),
        });
        Ok(started)
    }

    fn seal_active(&mut self) -> Result<MacosSealedScreenRun, ()> {
        // Closing the output callback alone is not terminal evidence. The
        // stream must acknowledge both Stop and output detachment, and the
        // callback must complete, before the checkpoint can become durable.
        // `close_native` retains its exact partial phase/error, so retries do
        // not repeat a successful Stop or output detach.
        self.active.as_mut().ok_or(())?.close_native()?;
        let observed_end_ms = self.elapsed_ms(Instant::now()).map_err(|_| ())?;
        let active = self.active.take().ok_or(())?;
        if observed_end_ms <= active.started.observed_start_ms {
            return Err(());
        }
        let checkpoint = self
            .checkpoints
            .publish(
                active.checkpoint_sequence,
                &active.staging,
                CheckpointFacts {
                    start_ms: active.started.observed_start_ms,
                    end_ms: observed_end_ms,
                    event_offset_ms: active.started.observed_start_ms,
                    audio_offset_ms: None,
                },
            )
            .map_err(|_| ())?;
        Ok(MacosSealedScreenRun {
            exact_monitor_id: active.started.exact_monitor_id,
            physical_generation: active.started.physical_generation,
            observed_start_ms: active.started.observed_start_ms,
            observed_end_ms,
            observed_at: Instant::now(),
            accepted: active.started.accepted,
            checkpoints: vec![checkpoint],
        })
    }

    fn abort_and_join(&mut self) -> Result<(), ()> {
        // Keep this close state until every native acknowledgement has
        // completed. A caller may retry `abort_and_join` after a transient
        // error; completed operations are never repeated and an exhausted
        // bounded retry remains observable to the owning run owner.
        let Some(active) = self.active.as_mut() else {
            return Ok(());
        };
        active.close_native()?;
        let _ = self.active.take();
        // The active checkpoint intentionally remains un-published. Recovery
        // sees that torn/open reservation rather than a plausible sealed run.
        Ok(())
    }
}
