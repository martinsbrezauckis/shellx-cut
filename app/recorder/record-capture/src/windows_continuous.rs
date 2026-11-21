//! One capture session with privately prepared, independently sealed writers.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use record_core::{error_codes, RecordError, Result, Settings};
use record_recovery::PrivateStaging;

use crate::checkpoint::Checkpoints;
use crate::windows_checkpoint_worker::{BackgroundWindowsPublisher, RetiredPart, WriterStage};
use crate::windows_wgc_clock_origin::{FirstFramePlacement, QpcCaptureOrigin};
use crate::windows_wgc_encoder::WgcVideoEncoder;
use crate::windows_wgc_handler::LiveWgcControl;
use crate::windows_wgc_run::WgcNativeControl;
use crate::windows_wgc_timing::WgcTimingRecorder;
use crate::windows_writer_rotation::WriterSwap;
use crate::CaptureConfig;

// Field order keeps the native encoder closed before its private path drops.
struct PreparedWriter {
    encoder: WgcVideoEncoder,
    stage: PrivateStaging,
    timing: WgcTimingRecorder,
}

struct ActiveWriter {
    stage: WriterStage,
    timing: WgcTimingRecorder,
    reserved_start_ms: u64,
}

pub(crate) fn capture(
    cfg: &CaptureConfig,
    stop: &Arc<AtomicBool>,
    start: Instant,
    settings: &Settings,
    origin: QpcCaptureOrigin,
    mut checkpoints: Checkpoints,
    mut start_control: impl FnMut(&Path, u64) -> Result<(LiveWgcControl, WgcTimingRecorder)>,
) -> Result<(u64, Checkpoints)> {
    let duration_ms = cfg.duration_ms.unwrap_or(u64::MAX / 4);
    let (width, height, fps) = (settings.width, settings.height, settings.fps as u32);
    let root = PathBuf::from(&cfg.out_dir);
    let interval = checkpoints.interval_ms();
    let initial = checkpoints.begin_windows_wgc(0)?;
    let (publisher, initial_path) = BackgroundWindowsPublisher::start(
        checkpoints,
        initial,
        tool("SHELLX_RECORD_FFMPEG", "ffmpeg"),
        tool("SHELLX_RECORD_FFPROBE", "ffprobe"),
    )?;
    let (mut control, timing) = start_control(&initial_path, 0)?;
    let mut active = ActiveWriter {
        stage: WriterStage::Journal,
        timing,
        reserved_start_ms: 0,
    };
    let mut prepared = None;
    let result = (|| {
        let mut next_boundary_ms = interval;
        loop {
            let end_at = next_boundary_ms.min(duration_ms);
            while !stop.load(Ordering::Acquire) && elapsed_ms(start) < end_at {
                if let Some(failure) = publisher.failure() {
                    return Err(failure);
                }
                // Preparing a Media Foundation sink never owns the callback
                // lock: frames continue into the current writer meanwhile.
                if end_at < duration_ms
                    && prepared.is_none()
                    && end_at.saturating_sub(elapsed_ms(start)) <= 1_000
                {
                    let stage = PrivateStaging::create_windows_wgc(&root)
                        .map_err(|cause| error(&cause.to_string()))?;
                    let timing = WgcTimingRecorder::new(start, fps, stage.path());
                    // This is the same native session. A callback queued while
                    // the writer swaps may carry a real pre-planned-cut frame.
                    let native_boundary = origin.reserved_boundary_hns(0).map_err(error)?;
                    let encoder = WgcVideoEncoder::new_with_clock(
                        width,
                        height,
                        fps,
                        stage.path(),
                        Some((start, origin, native_boundary)),
                    )
                    .map_err(|cause| error(&cause.to_string()))?;
                    prepared = Some(PreparedWriter {
                        encoder,
                        stage,
                        timing,
                    });
                }
                thread::sleep(Duration::from_millis(5));
            }
            if stop.load(Ordering::Acquire) || elapsed_ms(start) >= duration_ms {
                if let Some(lifecycle) = cfg.source_lifecycle.as_ref() {
                    lifecycle.expect_terminal_close();
                }
                let boundary_at = Instant::now();
                stop.store(true, Ordering::Release);
                control.close_at(boundary_at)?;
                let end_ms = elapsed_at(start, boundary_at).max(active.reserved_start_ms + 1);
                let media_start_ms = media_start(&active.timing, origin, active.reserved_start_ms)?;
                let part = RetiredPart {
                    stage: std::mem::replace(&mut active.stage, WriterStage::Journal),
                    reserved_start_ms: active.reserved_start_ms,
                    media_start_ms,
                    end_ms,
                    boundary_at,
                    next_start_ms: None,
                    next_media_start: None,
                    timing: active.timing.clone(),
                    close_writer: Box::new(|| Ok(())),
                };
                submit(&publisher, part)?;
                return Ok(end_ms);
            }
            let next = prepared
                .take()
                .ok_or_else(|| error("next writer was not prepared"))?;
            let PreparedWriter {
                encoder,
                stage,
                timing,
            } = next;
            let retired = control.rotate_writer(WriterSwap {
                encoder,
                timing: timing.clone(),
            })?;
            let boundary_at = retired.boundary_at;
            // A queued callback may predate the wall-clock swap. Wait outside
            // its callback lock for the first new native frame and use that
            // one cut for both journal ranges and the old pixel hold. On a
            // still screen, retain the wall cut after this bounded wait.
            let first_new_native = await_first_native(&timing, Duration::from_millis(100));
            let (end_ms, media_cut_at) = match first_new_native {
                Some(native) => origin.native_cut(start, native).map_err(error)?,
                None => (elapsed_at(start, boundary_at), boundary_at),
            };
            if end_ms <= active.reserved_start_ms {
                return Err(error("next native writer cut precedes its reservation"));
            }
            let media_start_ms = media_start(&active.timing, origin, active.reserved_start_ms)?;
            let old_stage = std::mem::replace(&mut active.stage, WriterStage::Private(stage));
            let old_start_ms = active.reserved_start_ms;
            active.reserved_start_ms = end_ms;
            active.timing = timing.clone();
            let next_media_start =
                Box::new(move || media_start(&timing, origin, end_ms).ok().flatten());
            let sealed_timing = retired.timing.clone();
            let part = RetiredPart {
                stage: old_stage,
                reserved_start_ms: old_start_ms,
                media_start_ms,
                end_ms,
                boundary_at,
                next_start_ms: Some(end_ms),
                next_media_start: Some(next_media_start),
                timing: retired.timing,
                close_writer: Box::new(move || {
                    let finished = retired.encoder.finish_after_control_stop(media_cut_at);
                    sealed_timing.encoder_finished(Instant::now(), finished.is_ok());
                    if let Ok(Some(held_ms)) = &finished {
                        sealed_timing.held_pixel_duration(*held_ms);
                    }
                    finished
                        .map(|_| ())
                        .map_err(|cause| error(&cause.to_string()))
                }),
            };
            submit(&publisher, part)?;
            next_boundary_ms = next_boundary_ms.saturating_add(interval);
        }
    })();
    if result.is_err() {
        stop.store(true, Ordering::Release);
        let _ = control.close();
    }
    // Join outside the capture callback. The immutable published prefix is
    // already recoverable while this waits for the final writer and cache.
    let published = publisher.finish();
    result.and_then(|duration| published.map(|checkpoints| (duration, checkpoints)))
}

fn submit(publisher: &BackgroundWindowsPublisher, part: RetiredPart) -> Result<()> {
    publisher.try_retire(part).map_err(|(failure, part)| {
        // Queue backpressure is a terminal capture failure, never permission
        // to discard a writer or silently omit its samples.
        let _ = (part.close_writer)();
        failure
    })
}

fn media_start(
    timing: &WgcTimingRecorder,
    origin: QpcCaptureOrigin,
    reserved: u64,
) -> Result<Option<u64>> {
    timing
        .first_accepted_native_timestamp_100ns()
        .map(|native| {
            origin
                .place_first_frame(native, reserved)
                .and_then(|placement| match placement {
                    FirstFramePlacement::AtOrAfterStart { start_ms } => Ok(start_ms),
                    FirstFramePlacement::BeforeStart { lead_hns }
                        if reserved == 0 || lead_hns < 10_000 =>
                    {
                        Ok(reserved)
                    }
                    FirstFramePlacement::BeforeStart { .. } => {
                        Err("first native frame predates the writer's fallback cut")
                    }
                })
                .map_err(error)
        })
        .transpose()
}

fn await_first_native(timing: &WgcTimingRecorder, timeout: Duration) -> Option<i64> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(native) = timing.first_accepted_native_timestamp_100ns() {
            return Some(native);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn elapsed_at(start: Instant, at: Instant) -> u64 {
    u64::try_from(at.saturating_duration_since(start).as_millis()).unwrap_or(u64::MAX)
}

fn elapsed_ms(start: Instant) -> u64 {
    elapsed_at(start, Instant::now())
}

fn tool(variable: &str, fallback: &str) -> String {
    std::env::var(variable).unwrap_or_else(|_| fallback.to_owned())
}

fn error(cause: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, "rotate recording writer", cause)
}
