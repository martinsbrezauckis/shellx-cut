//! Ordinary Windows WGC checkpoint publication and its native timing owner.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use record_core::{error_codes, RecordError, Result};
use record_recovery::{Checkpoint, CheckpointFacts};

use crate::checkpoint::Checkpoints;
use crate::windows_wgc_clock_origin::{FirstFramePlacement, QpcCaptureOrigin};
use crate::windows_wgc_run::WgcCheckpointPublisher;
use crate::windows_wgc_timing::WgcTimingRecorder;

pub(crate) struct WindowsCheckpointPublisher {
    pub(crate) checkpoints: Checkpoints,
    /// Ordinary WGC can encode frames during start_free_threaded. The private
    /// pause pilot still uses the separate post-open observed-start contract.
    pub(crate) include_native_startup: bool,
    pub(crate) timing: Option<Arc<Mutex<Option<WgcTimingRecorder>>>>,
    pub(crate) qpc_origin: Option<QpcCaptureOrigin>,
}

impl WgcCheckpointPublisher for WindowsCheckpointPublisher {
    fn reserve(&mut self, start_ms: u64) -> Result<(u64, PathBuf)> {
        self.checkpoints.begin_windows_wgc(start_ms)
    }

    fn capture_start_ms(&self, reserved_start_ms: u64, observed_start_ms: u64) -> u64 {
        if self.include_native_startup {
            reserved_start_ms
        } else {
            observed_start_ms
        }
    }

    fn sealed_media_start_ms(
        &self,
        sequence: u64,
        reserved_start_ms: u64,
        _end_ms: u64,
    ) -> Result<u64> {
        let Some(origin) = self.qpc_origin else {
            return Ok(reserved_start_ms);
        };
        let slot = self
            .timing
            .as_ref()
            .ok_or_else(|| clock_error("WGC timing owner missing"))?;
        let guard = slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let timing = guard
            .as_ref()
            .ok_or_else(|| clock_error("WGC timing observation missing"))?;
        let Some(native) = timing.first_accepted_native_timestamp_100ns() else {
            // The maintained checkpoint verifier alone may supply a held prior
            // picture for a later zero-frame segment after successful Stop.
            if sequence > 0 && timing.control_stop_succeeded() {
                return Ok(reserved_start_ms);
            }
            return Err(clock_error(
                "initial WGC segment has no accepted native frame",
            ));
        };
        match origin
            .place_first_frame(native, reserved_start_ms)
            .map_err(clock_error)?
        {
            FirstFramePlacement::AtOrAfterStart { start_ms } => Ok(start_ms),
            FirstFramePlacement::BeforeStart { .. } => Ok(reserved_start_ms),
        }
    }

    fn verify_and_publish_new(
        &mut self,
        sequence: u64,
        staging: &Path,
        facts: CheckpointFacts,
    ) -> Result<Checkpoint> {
        let timing = self.timing.as_ref().and_then(|slot| {
            slot.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
        });
        self.checkpoints
            .publish_windows_wgc(sequence, staging, facts, timing.as_ref())
    }
}

fn clock_error(detail: &str) -> RecordError {
    RecordError::new(error_codes::CAPTURE, "invalid WGC media clock", detail)
        .with_action("stop the recording and start a fresh capture")
}
