//! Close, verify and durably publish retired Windows recording writers.
//!
//! One physical writer is journaled at a time. Later writers use independent
//! private stages while the prior writer closes and passes full verification.
//! The worker then publishes the old part and reserves the next journal slot.
//! A private stage is moved into that reserved slot only after its writer is
//! closed. A failed stage is never called a checkpoint.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use record_core::{error_codes, RecordError, Result};
use record_recovery::{CheckpointFacts, PrivateStaging};

use crate::checkpoint::Checkpoints;
use crate::windows_wgc_timing::WgcTimingRecorder;

type CloseWriter = Box<dyn FnOnce() -> Result<()> + Send>;

pub(crate) enum WriterStage {
    /// The first writer uses the journal's initial reservation directly.
    Journal,
    /// Subsequent writers use a separately reserved private stage. This
    /// cannot be published until the preceding checkpoint has passed decode.
    Private(PrivateStaging),
}

pub(crate) struct RetiredPart {
    pub(crate) stage: WriterStage,
    pub(crate) reserved_start_ms: u64,
    /// Computed by the callback owner from the first accepted native frame
    /// with QpcCaptureOrigin.place_first_frame. None means no frame arrived.
    pub(crate) media_start_ms: Option<u64>,
    pub(crate) end_ms: u64,
    /// Capture-clock boundary sampled by the callback before it swaps writers.
    pub(crate) boundary_at: Instant,
    /// The next writer's reserved capture-clock start, if there is one.
    pub(crate) next_start_ms: Option<u64>,
    /// Read the next writer's first real picture after this part is published.
    /// It must be nonblocking; None means the next frame has not arrived yet.
    pub(crate) next_media_start: Option<Box<dyn FnOnce() -> Option<u64> + Send>>,
    pub(crate) timing: WgcTimingRecorder,
    /// Closes only this encoder. It must not stop the whole WGC session.
    pub(crate) close_writer: CloseWriter,
}

/// On a full queue the caller gets its encoder back. It must stop capture,
/// close the writer and report a capture failure; it must never drop a part.
pub(crate) type SubmissionError = (RecordError, Box<RetiredPart>);

pub(crate) struct BackgroundWindowsPublisher {
    sender: Option<SyncSender<RetiredPart>>,
    worker: Option<JoinHandle<Result<Checkpoints>>>,
    first_failure: Arc<Mutex<Option<RecordError>>>,
    preparer: Option<PrepareWorker>,
}

impl BackgroundWindowsPublisher {
    pub(crate) fn start(
        checkpoints: Checkpoints,
        initial_reserved: (u64, PathBuf),
        ffmpeg: String,
        ffprobe: String,
    ) -> Result<(Self, PathBuf)> {
        let (root, _) = checkpoints.published();
        if initial_reserved.0 != 0 {
            return Err(error("the first Windows checkpoint is not sequence zero"));
        }
        let first_path = initial_reserved.1;
        let initial_path = first_path.clone();
        let preparer = PrepareWorker::start(root.clone(), ffmpeg.clone(), ffprobe.clone());
        let prepare_sender = preparer.sender.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let first_failure = Arc::new(Mutex::new(None));
        let failure_for_thread = first_failure.clone();
        let worker = thread::Builder::new()
            .name("cut-wgc-checkpoints".into())
            .spawn(move || {
                let result = run_publication(checkpoints, &receiver, first_path, prepare_sender);
                if let Err(failure) = &result {
                    *failure_for_thread
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(failure.clone());
                    // After a publication failure, close any already queued
                    // writers. The caller sees `first_failure`, closes its
                    // sender, then joins. None of these parts is published.
                    for part in receiver {
                        let _ = (part.close_writer)();
                    }
                }
                result
            })
            .map_err(|cause| {
                RecordError::new(
                    error_codes::CAPTURE,
                    "start Windows checkpoint worker",
                    cause.to_string(),
                )
            })?;
        Ok((
            Self {
                sender: Some(sender),
                worker: Some(worker),
                first_failure,
                preparer: Some(preparer),
            },
            initial_path,
        ))
    }

    pub(crate) fn try_retire(&self, part: RetiredPart) -> std::result::Result<(), SubmissionError> {
        if let Some(failure) = self.failure() {
            return Err((failure, Box::new(part)));
        }
        let Some(sender) = &self.sender else {
            return Err((error("Windows checkpoint worker is closed"), Box::new(part)));
        };
        match sender.try_send(part) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(part)) => Err((
                error("Windows checkpoint writer queue is full"),
                Box::new(part),
            )),
            Err(TrySendError::Disconnected(part)) => {
                Err((error("Windows checkpoint worker ended"), Box::new(part)))
            }
        }
    }

    pub(crate) fn failure(&self) -> Option<RecordError> {
        self.first_failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Called after the terminal writer has been submitted. Waiting here is
    /// required before stitching; it does not belong in the frame callback.
    pub(crate) fn finish(mut self) -> Result<Checkpoints> {
        self.sender.take();
        let checkpoints = self
            .worker
            .take()
            .ok_or_else(|| error("Windows checkpoint worker missing"))?
            .join()
            .map_err(|_| error("Windows checkpoint worker panicked"))??;
        if let Some(preparer) = self.preparer.take() {
            preparer.finish();
        }
        Ok(checkpoints)
    }
}

impl Drop for BackgroundWindowsPublisher {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(preparer) = self.preparer.take() {
            preparer.finish();
        }
    }
}

fn run_publication(
    mut checkpoints: Checkpoints,
    receiver: &Receiver<RetiredPart>,
    first_path: PathBuf,
    prepare: Option<SyncSender<PreparePart>>,
) -> Result<Checkpoints> {
    let mut logical = (0_u64, first_path);
    let mut expected_start = 0_u64;
    for part in receiver {
        if part.reserved_start_ms != expected_start
            || !valid_boundaries(part.reserved_start_ms, part.end_ms, part.next_start_ms)
        {
            let _ = (part.close_writer)();
            return Err(error("Windows writer boundaries are out of order"));
        }
        let RetiredPart {
            mut stage,
            reserved_start_ms,
            media_start_ms,
            end_ms,
            boundary_at,
            next_start_ms,
            next_media_start,
            timing,
            close_writer,
        } = part;
        if matches!(&stage, WriterStage::Journal) != (logical.0 == 0) {
            let _ = close_writer();
            return Err(error(
                "Windows writer stage does not match its journal sequence",
            ));
        }
        let close_result = close_writer();
        // Retain the captured writer boundary rather than verifier completion.
        timing.writer_sealed(boundary_at, close_result.is_ok());
        close_result?;
        install_closed_stage(&mut stage, &logical.1)?;
        let start_ms =
            placed_media_start_ms(media_start_ms, reserved_start_ms, logical.0, &timing)?;
        if start_ms >= end_ms {
            return Err(error("Windows writer has no positive media span"));
        }
        let facts = CheckpointFacts {
            start_ms,
            end_ms,
            event_offset_ms: start_ms,
            audio_offset_ms: None,
        };
        // Full decode, no-replace move, hash and append-only journal sync all
        // complete before the next logical opening is allowed.
        checkpoints.publish_windows_wgc(logical.0, &logical.1, facts, Some(&timing))?;
        let _ = timing.persist();
        if let Some(next_start_ms) = next_start_ms {
            let prepare_start_ms = preparation_start(next_start_ms, next_media_start);
            let (root, published) = checkpoints.published();
            logical = checkpoints.begin_windows_wgc(next_start_ms)?;
            expected_start = next_start_ms;
            if let Some(preparer) = &prepare {
                let _ = preparer.try_send(PreparePart {
                    root,
                    published: published.clone(),
                    index: published.len() - 1,
                    next_media_start_ms: prepare_start_ms,
                });
            }
        } else {
            return Ok(checkpoints);
        }
    }
    Err(error(
        "Windows checkpoint worker ended before the final writer",
    ))
}

fn valid_boundaries(start_ms: u64, end_ms: u64, next_start_ms: Option<u64>) -> bool {
    end_ms > start_ms && next_start_ms.is_none_or(|next| next >= end_ms)
}

fn preparation_start(
    reserved_start_ms: u64,
    next_media_start: Option<Box<dyn FnOnce() -> Option<u64> + Send>>,
) -> u64 {
    next_media_start
        .and_then(|read| read())
        .filter(|first| *first >= reserved_start_ms)
        .unwrap_or(reserved_start_ms)
}

fn install_closed_stage(stage: &mut WriterStage, logical: &Path) -> Result<()> {
    match stage {
        WriterStage::Journal => Ok(()),
        WriterStage::Private(private) => {
            record_recovery::publish_new_synced(private.path(), logical).map_err(|cause| {
                RecordError::new(
                    error_codes::IO,
                    "install closed Windows writer",
                    cause.to_string(),
                )
            })
        }
    }
}

fn placed_media_start_ms(
    first: Option<u64>,
    reserved_start_ms: u64,
    sequence: u64,
    timing: &WgcTimingRecorder,
) -> Result<u64> {
    let Some(first) = first else {
        if sequence > 0 && timing.successful_segment_seal() {
            return Ok(reserved_start_ms);
        }
        return Err(error("initial Windows writer has no accepted picture"));
    };
    if first < reserved_start_ms {
        return Err(error(
            "the first Windows picture predates its writer reservation",
        ));
    }
    Ok(first)
}

fn error(cause: &str) -> RecordError {
    RecordError::new(
        error_codes::CAPTURE,
        "save Windows recording checkpoint",
        cause,
    )
}

#[derive(Clone)]
struct PreparePart {
    root: PathBuf,
    published: Vec<record_recovery::Checkpoint>,
    index: usize,
    next_media_start_ms: u64,
}

struct PrepareWorker {
    sender: Option<SyncSender<PreparePart>>,
    worker: Option<JoinHandle<()>>,
}

impl PrepareWorker {
    fn start(root: PathBuf, ffmpeg: String, ffprobe: String) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<PreparePart>(1);
        let worker = thread::Builder::new()
            .name("cut-wgc-cfr-parts".into())
            .spawn(move || {
                for job in receiver {
                    debug_assert_eq!(job.root, root);
                    let _ = record_recovery::prepare_cfr_segment(
                        &job.root,
                        &job.published,
                        job.index,
                        Some(job.next_media_start_ms),
                        &ffmpeg,
                        &ffprobe,
                    );
                }
            })
            .ok();
        Self {
            sender: worker.as_ref().map(|_| sender),
            worker,
        }
    }

    fn finish(mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
#[path = "windows_checkpoint_worker_tests.rs"]
mod tests;
