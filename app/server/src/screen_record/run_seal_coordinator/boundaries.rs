//! Command-time correlation for a pause-aware run seal.
//!
//! These requests stop timestamp issuance immediately, but retain only floors
//! that reject stale evidence. They never decide a durable run endpoint.

use super::coordinator::RunSealCoordinator;
use super::types::{
    invalid, PauseSealRequest, PendingBoundary, PendingPause, PendingResume, PendingStop,
    RecordingSessionJournalSink, RunBoundaryFloor, RunSealCoordinatorError, StopSealRequest,
};
use record_capture::{BoundaryRequestResult, StreamBoundary};
use std::time::Instant;

impl<J: RecordingSessionJournalSink> RunSealCoordinator<J> {
    /// Suppress timestamp issuance and create a generation-correlated pause
    /// request. A later sealed artifact must supply the actual accepted end.
    pub(crate) fn request_pause_at(
        &mut self,
        at: Instant,
    ) -> Result<PauseSealRequest, RunSealCoordinatorError> {
        self.ensure_mutable()?;
        let origin = self.origin()?;
        let active = self
            .active_run
            .as_ref()
            .ok_or_else(|| invalid("pause requires an active recording run"))?
            .clone();
        let floor = RunBoundaryFloor {
            logical_ms: self.logical_timestamp_ms(at)?,
            observed_ms: origin.elapsed_ms(at)?,
        };
        if floor.logical_ms <= active.logical_start_ms
            || floor.observed_ms <= active.observed_start_ms
        {
            return Err(invalid(
                "pause requires a positive active run before command issue",
            ));
        }
        let expected_generation = active
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("pause generation overflows the durable run identity"))?;
        let BoundaryRequestResult::Issued(boundary) = self.logical.request_pause_at(at) else {
            return Err(invalid(
                "pause boundary is not valid from the current logical phase",
            ));
        };
        if boundary.generation != expected_generation {
            self.failed_after_append = true;
            return Err(invalid(
                "pause boundary generation did not advance the active run",
            ));
        }
        self.pending_boundary = Some(PendingBoundary::Pause(PendingPause {
            expected: active.for_generation(boundary.generation),
            boundary: boundary.clone(),
            floor,
        }));
        Ok(PauseSealRequest { boundary })
    }

    /// Return a generation-tagged resume command. Timestamp issuance remains
    /// disabled until the durable readiness path completes.
    pub(crate) fn request_resume_at(
        &mut self,
        at: Instant,
    ) -> Result<StreamBoundary, RunSealCoordinatorError> {
        self.ensure_mutable()?;
        let _ = self.origin()?;
        let BoundaryRequestResult::Issued(boundary) = self.logical.request_resume_at(at) else {
            return Err(invalid(
                "resume boundary is not valid from the current logical phase",
            ));
        };
        self.pending_boundary = Some(PendingBoundary::Resume(PendingResume {
            boundary: boundary.clone(),
        }));
        Ok(boundary)
    }

    /// Stop wins immediately. The returned request identifies an active run if
    /// one exists, but contains no chosen endpoint; close evidence owns that.
    pub(crate) fn request_stop_at(
        &mut self,
        at: Instant,
    ) -> Result<StopSealRequest, RunSealCoordinatorError> {
        if self.failed_after_append {
            return Err(RunSealCoordinatorError::FailedAfterDurableAppend);
        }
        if self.pending_stop.is_some() || self.logical.phase().is_terminal() {
            return Err(RunSealCoordinatorError::Stopped);
        }
        let origin = self.origin()?;
        let (expected, floor) = match self.pending_boundary.as_ref() {
            Some(PendingBoundary::Pause(pending)) => {
                (Some(pending.expected.clone()), Some(pending.floor))
            }
            Some(PendingBoundary::Resume(_)) => (None, None),
            None => match self.active_run.as_ref() {
                Some(active) => {
                    let floor = RunBoundaryFloor {
                        logical_ms: self.logical_timestamp_ms(at)?,
                        observed_ms: origin.elapsed_ms(at)?,
                    };
                    (floor.logical_ms > active.logical_start_ms
                        && floor.observed_ms > active.observed_start_ms)
                        .then(|| (active.for_generation(active.generation), floor))
                        .map_or((None, None), |(expected, floor)| {
                            (Some(expected), Some(floor))
                        })
                }
                None => (None, None),
            },
        };
        let request = StopSealRequest {
            generation: expected.as_ref().map(|run| run.generation),
        };

        let result = self.logical.stop_at(at);
        debug_assert!(result.changed());
        self.pending_boundary = None;
        self.pending_stop = Some(PendingStop {
            expected,
            floor,
            terminal_logical_end_ms: self.last_logical_end_ms,
        });
        Ok(request)
    }
}
