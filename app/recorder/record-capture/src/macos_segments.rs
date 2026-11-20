//! The macOS checkpoint rotation of one live ScreenCaptureKit stream.
//!
//! Every 15 s the current `SCRecordingOutput` is detached, finished and
//! published as a durable checkpoint, and a new output is attached; the
//! stream itself stays live (the SCK counterpart of WGC encoder rotation).
//!
//! Rules that keep a take whole (each one was a lost take before):
//! - An output that has not received a picture yet is never detached: SCK
//!   cannot finish it ("failure to process first sample buffer"). On a still
//!   screen the current output simply runs on until the screen changes.
//! - A segment that ends without a picture, or whose output cannot be
//!   finished, is closed by holding the previous segment's last frame over its
//!   time, the way a recorder repeats the last picture of a still screen. Only
//!   a failure the person should know about is added to the take's notes.
//! - A segment's place on the capture clock is its first picture, not the
//!   moment its output was attached, so picture and sound stay in sync after
//!   a long still stretch.
//! - Publication reads container facts only (no full decode), so the next
//!   output is attached quickly; the decode happens when the constant-rate
//!   part is made, in the background while the recording runs.
//! - System sound is copied to a crash-safe file once a second.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use record_core::{error_codes, RecordError, Result};
use screencapturekit::prelude::*;

use crate::checkpoint::Checkpoints;
use crate::input::{InputListener, InputSnapshot};
use crate::macos_checkpoint::SegmentOutput;
use crate::macos_finalization::stop_audio_at_video_boundary;
use crate::macos_segment_frames::{
    keep_output_open, segment_start_ms, unfinished, SegmentFrames, Unfinished,
};
use crate::macos_system_audio_spool::SystemAudioSpool;
use crate::macos_system_tap::{SystemAudioResult, SystemAudioTap};
use crate::CaptureConfig;

const SPOOL_EVERY: Duration = Duration::from_secs(1);

pub(crate) struct SegmentRun<'a> {
    pub(crate) stream: &'a mut SCStream,
    pub(crate) recording: SegmentOutput,
    pub(crate) segment: Option<(u64, PathBuf)>,
    /// Capture-clock time the first output was attached.
    pub(crate) attached_ms: u64,
    pub(crate) checkpoints: &'a mut Option<Checkpoints>,
    pub(crate) frames: Arc<SegmentFrames>,
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

/// An output failure after the stream is running is not a permission problem:
/// the grant was already proven by the frames this capture received.
pub(crate) fn finish_error(context: &str, cause: impl std::fmt::Display) -> RecordError {
    RecordError::new(error_codes::CAPTURE, context, cause.to_string()).with_action(
        "start the recording again; if it fails the same way, quit and reopen ShellX Cut, then record once more",
    )
}

fn seconds(ms: u64) -> String {
    format!("{:.1} s", ms as f64 / 1000.0)
}

fn elapsed_ms(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub(crate) fn record(run: SegmentRun<'_>) -> Result<SegmentsDone> {
    let SegmentRun {
        stream,
        mut recording,
        mut segment,
        mut attached_ms,
        checkpoints,
        frames,
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
    let mut input = Some(input);
    let mut spool = SystemAudioSpool::new(out_dir);
    let mut spooled_at = Instant::now();
    let mut notes = Vec::new();
    let mut stopped_system_audio = None;
    let prepare = PrepareWorker::start(ffmpeg, ffprobe);
    // (index of the last published checkpoint, its end) until the next
    // segment's first picture says how long that checkpoint's last frame holds.
    let mut pending_prepare: Option<(usize, u64)> = None;
    let full_end = cfg.duration_ms.unwrap_or(u64::MAX / 4);
    let interval = checkpoints.as_ref().map(Checkpoints::interval_ms);
    let mut checkpoint_end = interval.map_or(full_end, |every| attached_ms.saturating_add(every));
    loop {
        checkpoint_end = checkpoint_end.min(full_end);
        while !stop.load(Ordering::Relaxed) && elapsed_ms(start) < checkpoint_end {
            thread::sleep(Duration::from_millis(50));
            if spooled_at.elapsed() >= SPOOL_EVERY {
                spooled_at = Instant::now();
                spool_new_sound(sys_tap.as_ref(), &mut spool);
            }
            if let (Some((index, previous_end)), Some(first)) =
                (pending_prepare, frames.window().first_ms)
            {
                if let Some(owner) = checkpoints.as_ref() {
                    let (root, published) = owner.published();
                    let next_start = first.max(attached_ms).max(previous_end);
                    prepare.submit(root, published, index, next_start);
                }
                pending_prepare = None;
            }
        }
        // The last elapsed instant the current output can contain.
        let capture_end_ms = elapsed_ms(start);
        let final_segment = stop.load(Ordering::Relaxed) || capture_end_ms >= full_end;
        let window = frames.window();
        if keep_output_open(final_segment, window) {
            // Nothing reached this output yet (a still screen). Detaching it now
            // would fail it; let it run on until a picture arrives or Stop.
            checkpoint_end = checkpoint_end.saturating_add(interval.unwrap_or(full_end));
            continue;
        }
        let sealed_input = if final_segment {
            // Stop native video and Core Audio as one capture-end boundary.
            // Output completion, part making and stitching may take seconds and
            // must not become recorded audio.
            if let Some(lifecycle) = cfg.source_lifecycle.as_ref() {
                lifecycle.expect_terminal_close();
            }
            stop.store(true, Ordering::Relaxed); // end mic + input at this boundary
            clear_preview();
            stopped_system_audio = stop_audio_at_video_boundary(
                || {
                    let _ = stream.stop_capture();
                },
                sys_tap,
                SystemAudioTap::finish,
            );
            // The tap is torn down: the native stream is free for the next
            // recording or a system-audio test while this take is written.
            crate::release_system_audio_lease(cfg);
            // Seal the listener before output completion or any stitch/audio
            // work can add late sidecar samples.
            Some(
                input
                    .take()
                    .expect("input listener remains owned until the final segment")
                    .seal(capture_end_ms)?,
            )
        } else {
            None
        };
        let _ = stream.remove_recording_output(recording.output());
        let completion = recording.wait_complete();
        match (checkpoints.as_mut(), segment.take()) {
            (Some(owner), Some((sequence, staging))) => {
                let (_, published) = owner.published();
                let previous_end = published.last().map(|checkpoint| checkpoint.facts.end_ms);
                let start_ms = segment_start_ms(window, attached_ms, previous_end, capture_end_ms);
                let facts = |start_ms| record_recovery::CheckpointFacts {
                    start_ms,
                    end_ms: capture_end_ms,
                    event_offset_ms: start_ms,
                    // Neither mic nor Core Audio tap exposes a proven
                    // first-packet offset at this video-finalize boundary.
                    audio_offset_ms: None,
                };
                let published = completion.and_then(|()| {
                    owner
                        .publish_without_decode(sequence, &staging, facts(start_ms))
                        .map(|_| ())
                        .map_err(|error| error.cause)
                });
                if let Err(reason) = published {
                    match unfinished(previous_end, window) {
                        Unfinished::Fail => {
                            return Err(finish_error(
                                "finalize ScreenCaptureKit checkpoint",
                                reason,
                            ))
                        }
                        Unfinished::Hold { start_ms, tell } => {
                            owner.publish_held_frame(sequence, &staging, facts(start_ms))?;
                            if tell {
                                notes.push(format!(
                                    "The screen from {} to {} could not be saved ({reason}), so the picture before it is shown for that time.",
                                    seconds(start_ms),
                                    seconds(capture_end_ms),
                                ));
                            }
                        }
                    }
                }
            }
            _ => completion
                .map_err(|reason| finish_error("finalize ScreenCaptureKit recording", reason))?,
        }
        if final_segment {
            prepare.finish();
            return Ok(SegmentsDone {
                duration_ms: capture_end_ms,
                sealed_input: sealed_input.expect("final segment seals its input listener"),
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
            pending_prepare = Some((published.len() - 1, last.facts.end_ms));
        }
        let next = owner.begin(elapsed_ms(start))?;
        let next_recording = SegmentOutput::new(&next.1)
            .map_err(|e| finish_error("create rotated recording output", e))?;
        frames.arm();
        stream
            .add_recording_output(next_recording.output())
            .map_err(|e| finish_error("attach rotated recording output", format!("{e:?}")))?;
        // `add_recording_output` returning is the SCK encoder-start boundary.
        attached_ms = elapsed_ms(start);
        segment = Some(next);
        recording = next_recording;
        checkpoint_end = attached_ms.saturating_add(interval.unwrap_or(full_end));
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

/// One part to make: capture root, checkpoints so far, index, next first frame.
type PartJob = (PathBuf, Vec<record_recovery::Checkpoint>, usize, u64);

/// Makes constant-rate parts one at a time on its own thread while the
/// recording runs. Best effort: a part it could not make is made at Stop.
struct PrepareWorker {
    jobs: Option<mpsc::Sender<PartJob>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl PrepareWorker {
    fn start(ffmpeg: String, ffprobe: String) -> Self {
        let (jobs, queue) = mpsc::channel::<PartJob>();
        let thread = thread::Builder::new()
            .name("cut-capture-parts".into())
            .spawn(move || {
                for (root, published, index, next_start_ms) in queue {
                    let _ = record_recovery::prepare_cfr_segment(
                        &root,
                        &published,
                        index,
                        Some(next_start_ms),
                        &ffmpeg,
                        &ffprobe,
                    );
                }
            })
            .ok();
        Self {
            jobs: Some(jobs),
            thread,
        }
    }

    fn submit(
        &self,
        root: PathBuf,
        published: Vec<record_recovery::Checkpoint>,
        index: usize,
        next_start_ms: u64,
    ) {
        if let Some(jobs) = self.jobs.as_ref() {
            let _ = jobs.send((root, published, index, next_start_ms));
        }
    }

    /// Wait for the part in progress; the stitch reuses whatever is finished.
    fn finish(mut self) {
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PrepareWorker {
    fn drop(&mut self) {
        // An error return leaves the worker to finish its current part alone.
        self.jobs.take();
    }
}
