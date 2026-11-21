//! Prepare independent constant-rate parts concurrently at the Stop boundary.

use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

/// One part to make: capture root, checkpoints so far, index, next first frame.
type PartJob = (PathBuf, Vec<record_recovery::Checkpoint>, usize, u64);

/// Makes constant-rate parts one at a time on its own thread while the
/// recording runs. Best effort: a part it could not make is made at Stop.
pub(crate) struct PrepareWorker {
    ffmpeg: String,
    ffprobe: String,
    jobs: Option<mpsc::Sender<PartJob>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl PrepareWorker {
    pub(crate) fn start(ffmpeg: String, ffprobe: String) -> Self {
        let (jobs, queue) = mpsc::channel::<PartJob>();
        let (worker_ffmpeg, worker_ffprobe) = (ffmpeg.clone(), ffprobe.clone());
        let thread = thread::Builder::new()
            .name("cut-capture-parts".into())
            .spawn(move || {
                for (root, published, index, next_start_ms) in queue {
                    let _ = record_recovery::prepare_cfr_segment(
                        &root,
                        &published,
                        index,
                        Some(next_start_ms),
                        &worker_ffmpeg,
                        &worker_ffprobe,
                    );
                }
            })
            .ok();
        Self {
            ffmpeg,
            ffprobe,
            jobs: Some(jobs),
            thread,
        }
    }

    pub(crate) fn submit(
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

    /// Prepare the final short part while the preceding background part still
    /// runs. Stop must not serialize those two independent encodes.
    pub(crate) fn finish(
        mut self,
        final_parts: Option<(PathBuf, Vec<record_recovery::Checkpoint>)>,
    ) {
        self.jobs.take();
        if let Some((root, published)) = final_parts {
            if published.len() > 1 {
                let _ = record_recovery::prepare_cfr_segment(
                    &root,
                    &published,
                    published.len() - 1,
                    None,
                    &self.ffmpeg,
                    &self.ffprobe,
                );
            }
        }
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
