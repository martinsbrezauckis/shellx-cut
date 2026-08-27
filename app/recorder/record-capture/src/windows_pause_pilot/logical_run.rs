//! Immutable logical-run accumulation for ordinary WGC checkpoint rotation.

use super::{
    WindowsPausePilotAcceptedCapture, WindowsPausePilotCaptureRange,
    WindowsPausePilotCheckpointRange, WindowsPausePilotStarted, WindowsSealedScreenRun,
    WindowsSealedWgcCheckpoint,
};
use crate::windows_wgc_run::SealedScreenRun;

/// Private accumulation for exactly one continuous logical pause-pilot run.
/// Pause or terminal Stop consumes it after sealing the active checkpoint;
/// Resume deliberately starts an empty, fresh logical run.
pub(super) struct ActiveLogicalRun {
    observed_start_ms: u64,
    accepted: WindowsPausePilotAcceptedCapture,
    checkpoints: Vec<WindowsSealedWgcCheckpoint>,
}

impl ActiveLogicalRun {
    pub(super) fn with_observed_start(started: &WindowsPausePilotStarted) -> Self {
        Self {
            observed_start_ms: started.observed_start_ms,
            accepted: started.accepted.clone(),
            checkpoints: Vec::new(),
        }
    }

    pub(super) fn append(&mut self, run: SealedScreenRun) -> Result<(), ()> {
        let (accepted, checkpoint) = physical_checkpoint(run)?;
        if accepted != self.accepted || checkpoint.start_ms < self.observed_start_ms {
            return Err(());
        }
        if let Some(previous) = self.checkpoints.last() {
            if Some(checkpoint.physical_generation) != previous.physical_generation.checked_add(1)
                || Some(checkpoint.checkpoint.sequence)
                    != previous.checkpoint.sequence.checked_add(1)
                || checkpoint.start_ms < previous.end_ms
            {
                return Err(());
            }
        }
        self.checkpoints.push(checkpoint);
        Ok(())
    }

    pub(super) fn seal(self) -> Result<WindowsSealedScreenRun, ()> {
        let first = self.checkpoints.first().ok_or(())?;
        let last = self.checkpoints.last().ok_or(())?;
        if first.start_ms != self.observed_start_ms || last.end_ms <= self.observed_start_ms {
            return Err(());
        }
        Ok(WindowsSealedScreenRun {
            observed_start_ms: self.observed_start_ms,
            observed_end_ms: last.end_ms,
            accepted: self.accepted,
            range: WindowsPausePilotCheckpointRange {
                first_physical_generation: first.physical_generation,
                last_physical_generation: last.physical_generation,
                first_checkpoint_sequence: first.checkpoint.sequence,
                last_checkpoint_sequence: last.checkpoint.sequence,
            },
            checkpoints: self.checkpoints,
        })
    }

    pub(super) fn accepts(&self, accepted: &WindowsPausePilotAcceptedCapture) -> bool {
        self.accepted == *accepted
    }
}

fn physical_checkpoint(
    run: SealedScreenRun,
) -> Result<(WindowsPausePilotAcceptedCapture, WindowsSealedWgcCheckpoint), ()> {
    let range = run.accepted.range.ok_or(())?;
    if run.accepted.settings.width != range.width
        || run.accepted.settings.height != range.height
        || run.boundary.end_ms <= run.boundary.start_ms
        || run.checkpoint.facts.start_ms != run.boundary.start_ms
        || run.checkpoint.facts.end_ms != run.boundary.end_ms
        || run.checkpoint.facts.event_offset_ms != run.boundary.event_offset_ms
        || run.checkpoint.facts.audio_offset_ms.is_some()
    {
        return Err(());
    }
    Ok((
        WindowsPausePilotAcceptedCapture {
            settings: run.accepted.settings,
            range: WindowsPausePilotCaptureRange {
                origin_x: range.origin_x,
                origin_y: range.origin_y,
                width: range.width,
                height: range.height,
            },
        },
        WindowsSealedWgcCheckpoint {
            physical_generation: run.boundary.physical_generation,
            start_ms: run.boundary.start_ms,
            end_ms: run.boundary.end_ms,
            checkpoint: run.checkpoint,
        },
    ))
}
