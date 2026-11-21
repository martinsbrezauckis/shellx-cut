//! Checkpoint publication without interrupting screen-sample delivery.
//! The next private writer accepts samples before the previous writer is
//! finished, probed or published. The journal still owns one open reservation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use record_core::{error_codes, RecordError, Result};
use screencapturekit::prelude::*;

use crate::checkpoint::Checkpoints;
use crate::input::{InputListener, InputSnapshot};
use crate::macos_finalization::stop_audio_at_video_boundary;
use crate::macos_screen_writer::{private_stage, NativeSegment, ScreenWriter};
use crate::macos_segment_frames::{keep_output_open, segment_start_ms, unfinished, Unfinished};
use crate::macos_segment_prepare::PrepareWorker;
use crate::macos_system_audio_spool::SystemAudioSpool;
use crate::macos_system_tap::{SystemAudioResult, SystemAudioTap};
use crate::CaptureConfig;

const SPOOL_EVERY: Duration = Duration::from_secs(1);

pub(crate) struct SegmentRun<'a> {
    pub(crate) stream: &'a mut SCStream,
    pub(crate) writer: ScreenWriter,
    pub(crate) writer_stage: Option<record_recovery::PrivateStaging>,
    pub(crate) writer_size: (u32, u32, u32),
    pub(crate) segment: Option<(u64, PathBuf)>,
    pub(crate) checkpoints: &'a mut Option<Checkpoints>,
    pub(crate) start: Instant,
    pub(crate) cfg: &'a CaptureConfig,
    pub(crate) stop: &'a AtomicBool,
    pub(crate) sys_tap: &'a mut Option<SystemAudioTap>,
    pub(crate) input: InputListener,
    pub(crate) out_dir: &'a Path,
    pub(crate) clear_preview: &'a mut dyn FnMut(),
    pub(crate) ffmpeg: String,
    pub(crate) ffprobe: String,
}

pub(crate) struct SegmentsDone {
    pub(crate) duration_ms: u64,
    pub(crate) sealed_input: InputSnapshot,
    pub(crate) stopped_system_audio: Option<SystemAudioResult>,
    pub(crate) spool: SystemAudioSpool,
    pub(crate) notes: Vec<String>,
}

pub(crate) fn finish_error(context: &str, cause: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, context, cause.to_string()).with_action(
        "start the recording again; if it fails the same way, quit and reopen ShellX Cut, then record once more",
    )
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// An encoder or publication error must stop every capture-side producer too.
struct StopOnError<'a> {
    stream: &'a SCStream,
    stop: &'a AtomicBool,
    armed: bool,
}
impl Drop for StopOnError<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.stop.store(true, Ordering::Release);
            let _ = self.stream.stop_capture();
        }
    }
}

pub(crate) fn record(run: SegmentRun<'_>) -> Result<SegmentsDone> {
    let SegmentRun {
        stream,
        writer,
        mut writer_stage,
        writer_size,
        mut segment,
        checkpoints,
        start,
        cfg,
        stop,
        sys_tap,
        input,
        out_dir,
        clear_preview,
        ffmpeg,
        ffprobe,
    } = run;
    let mut stop_on_error = StopOnError {
        stream,
        stop,
        armed: true,
    };
    let mut input = Some(input);
    let mut spool = SystemAudioSpool::new(out_dir);
    let mut spooled_at = Instant::now();
    let mut notes = Vec::new();
    let prepare = PrepareWorker::start(ffmpeg, ffprobe);
    let mut pending_prepare: Option<(usize, u64)> = None;
    let full_end = cfg.duration_ms.unwrap_or(u64::MAX / 4);
    let interval = checkpoints.as_ref().map(Checkpoints::interval_ms);
    let mut checkpoint_end = interval.unwrap_or(full_end);
    let mut attached_ms = 0;
    loop {
        checkpoint_end = checkpoint_end.min(full_end);
        while !stop.load(Ordering::Relaxed) && elapsed_ms(start) < checkpoint_end {
            thread::sleep(Duration::from_millis(20));
            if spooled_at.elapsed() >= SPOOL_EVERY {
                spooled_at = Instant::now();
                spool_new_sound(sys_tap.as_ref(), &mut spool);
            }
            let window = writer
                .window()
                .map_err(|e| finish_error("write screen sample", e))?;
            if let (Some((index, previous_end)), Some(first)) = (pending_prepare, window.first_ms) {
                if let Some(owner) = checkpoints.as_ref() {
                    let (root, published) = owner.published();
                    prepare.submit(root, published, index, first.max(previous_end));
                }
                pending_prepare = None;
            }
        }
        let final_segment = stop.load(Ordering::Relaxed) || elapsed_ms(start) >= full_end;
        let window = writer
            .window()
            .map_err(|e| finish_error("write screen sample", e))?;
        if keep_output_open(final_segment, window) {
            checkpoint_end = checkpoint_end.saturating_add(interval.unwrap_or(full_end));
            continue;
        }
        if final_segment {
            if let (Some((index, previous_end)), Some(owner)) =
                (pending_prepare.take(), checkpoints.as_ref())
            {
                let (root, published) = owner.published();
                prepare.submit(
                    root,
                    published,
                    index,
                    window.first_ms.unwrap_or(previous_end).max(previous_end),
                );
            }
        }
        // Prepare the next file while the old writer still accepts samples.
        // Its private stage is not recovery proof until the preceding segment
        // has been published and the journal can reserve its single next slot.
        let next_stage = if final_segment {
            None
        } else {
            Some(
                private_stage(cfg.checkpoint.as_ref().expect("checkpoint configured"))
                    .map_err(|e| finish_error("reserve next screen writer", e))?,
            )
        };
        let next = next_stage
            .as_ref()
            .map(|stage| {
                NativeSegment::new(stage.path(), writer_size.0, writer_size.1, writer_size.2)
            })
            .transpose()
            .map_err(|e| finish_error("create next screen writer", e))?;
        let stopped_system_audio = if final_segment {
            if let Some(lifecycle) = cfg.source_lifecycle.as_ref() {
                lifecycle.expect_terminal_close();
            }
            stop.store(true, Ordering::Relaxed);
            clear_preview();
            writer.request_stop();

            let mut stop_error = None;
            let sound = stop_audio_at_video_boundary(
                || {
                    stop_error = stream.stop_capture().err();
                },
                sys_tap,
                SystemAudioTap::finish,
            );
            crate::release_system_audio_lease(cfg);
            if let Some(error) = stop_error {
                return Err(finish_error("stop screen capture", error));
            }
            writer
                .drain()
                .map_err(|e| finish_error("drain screen samples", e))?;
            sound
        } else {
            None
        };
        let (old, boundary) = writer
            .switch(next)
            .map_err(|e| finish_error("switch screen writer", e))?;
        let capture_end_ms = boundary.elapsed_ms;
        let sealed_input = if final_segment {
            Some(
                input
                    .take()
                    .expect("input remains owned until Stop")
                    .seal(capture_end_ms)?,
            )
        } else {
            None
        };
        // Capture is already writing the next file while this completion and
        // publication run. No sample delivery depends on their duration.

        let completion = old.finish(boundary.pts_ns);

        match (checkpoints.as_mut(), segment.take()) {
            (Some(owner), Some((sequence, staging))) => {
                let (_, published) = owner.published();
                let previous_end = published.last().map(|checkpoint| checkpoint.facts.end_ms);
                let start_ms =
                    segment_start_ms(boundary.window, attached_ms, previous_end, capture_end_ms);
                let facts = |start_ms| record_recovery::CheckpointFacts {
                    start_ms,
                    end_ms: capture_end_ms,
                    event_offset_ms: start_ms,
                    audio_offset_ms: None,
                };
                let published = completion.and_then(|()| {
                    let stage = writer_stage
                        .as_ref()
                        .ok_or("screen writer stage is missing")?;
                    record_recovery::publish_new_synced(stage.path(), &staging)
                        .map_err(|e| e.to_string())?;
                    owner
                        .publish_without_decode(sequence, &staging, facts(start_ms))
                        .map(|_| ())
                        .map_err(|e| e.cause)
                });
                if let Err(reason) = published {
                    match unfinished(previous_end, boundary.window) {
                        Unfinished::Fail => {
                            return Err(finish_error("finalize screen checkpoint", reason))
                        }
                        Unfinished::Hold { start_ms, tell } => {
                            owner.publish_held_frame(sequence, &staging, facts(start_ms))?;
                            if tell {
                                notes.push(format!(
                                    "The screen from {:.1} s to {:.1} s could not be saved ({reason}), so the picture before it is shown for that time.",
                                    start_ms as f64 / 1000.0, capture_end_ms as f64 / 1000.0,
                                ));
                            }
                        }
                    }
                }
            }
            _ => completion.map_err(|e| finish_error("finalize screen recording", e))?,
        }
        // Release the native owner before cleaning its private stage.

        drop(old);
        writer_stage = next_stage;
        if final_segment {
            prepare.finish(checkpoints.as_ref().map(Checkpoints::published));

            stop_on_error.armed = false;
            return Ok(SegmentsDone {
                duration_ms: capture_end_ms,
                sealed_input: sealed_input.expect("Stop seals input"),
                stopped_system_audio,
                spool,
                notes,
            });
        }
        let owner = checkpoints
            .as_mut()
            .expect("checkpoint rotation configured");
        let (_, published) = owner.published();
        if let Some(last) = published.last() {
            let index = published.len() - 1;
            if let Some(first) = writer
                .window()
                .map_err(|e| finish_error("read next screen sample", e))?
                .first_ms
            {
                prepare.submit(
                    owner.published().0,
                    published.clone(),
                    index,
                    first.max(last.facts.end_ms),
                );
                pending_prepare = None;
            } else {
                pending_prepare = Some((index, last.facts.end_ms));
            }
        }
        attached_ms = capture_end_ms;
        segment = Some(owner.begin(capture_end_ms)?);
        checkpoint_end = checkpoint_end.saturating_add(interval.unwrap_or(full_end));
    }
}

fn spool_new_sound(tap: Option<&SystemAudioTap>, spool: &mut SystemAudioSpool) {
    let Some(new) = tap.and_then(SystemAudioTap::copy_new) else {
        return;
    };
    if let Some(samples) = new.samples.as_ref() {
        spool.append(
            samples.as_slice(),
            new.channels,
            new.rate,
            new.first_packet_offset_ms,
        );
    }
}
