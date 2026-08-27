//! Deterministic acknowledgement coordination for future pause/resume workers.
//!
//! This module owns no native stream, thread, or server registry. It produces a
//! boundary command for every accepted pause or resume request and advances the
//! logical clock only after the caller reports the required acknowledgements.

use crate::{
    AcknowledgementRejection, AcknowledgementResult, BoundaryRequestResult, LogicalSessionClock,
    PauseStreamCoordinatorStatus, PendingStreamBoundary, SelectedCaptureStreams, SessionPhase,
    SessionTransition, SessionTransitionResult, StreamAcknowledgement, StreamBoundary,
    StreamBoundaryKind, StreamRefusal,
};
use record_recovery::RecordingStream;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
struct PendingBoundary {
    boundary: StreamBoundary,
    acknowledged: BTreeSet<RecordingStream>,
}

/// A logical pause/resume coordinator with no media or worker ownership.
///
/// A successful pause request freezes [`LogicalSessionClock`] immediately, then
/// waits for every selected stream to seal. A successful resume request remains
/// timestamp-silent until every selected stream reports readiness. `Stop` clears
/// any pending boundary and is terminal in every phase.
#[derive(Debug, Clone)]
pub struct PauseStreamCoordinator {
    streams: SelectedCaptureStreams,
    clock: LogicalSessionClock,
    latest_generation: u64,
    pending: Option<PendingBoundary>,
    last_resume_refusal: Option<StreamRefusal>,
}

impl PauseStreamCoordinator {
    /// Create a coordinator in `Preparing` with its immutable stream selection.
    pub fn new(streams: SelectedCaptureStreams, active_duration_budget: Option<Duration>) -> Self {
        Self {
            streams,
            clock: LogicalSessionClock::new(active_duration_budget),
            latest_generation: 0,
            pending: None,
            last_resume_refusal: None,
        }
    }

    /// Start logical timestamp issuance after capture setup has independently
    /// succeeded. This does not fabricate any worker readiness acknowledgement.
    pub fn start_at(&mut self, at: Instant) -> SessionTransitionResult {
        self.clock.apply_at(SessionTransition::Start, at)
    }

    /// Start logical timestamp issuance using the local monotonic clock.
    pub fn start(&mut self) -> SessionTransitionResult {
        self.start_at(Instant::now())
    }

    /// Request a pause boundary and immediately disable timestamp issuance.
    pub fn request_pause_at(&mut self, at: Instant) -> BoundaryRequestResult {
        self.request_boundary_at(StreamBoundaryKind::Pause, at)
    }

    /// Request a pause boundary using the local monotonic clock.
    pub fn request_pause(&mut self) -> BoundaryRequestResult {
        self.request_pause_at(Instant::now())
    }

    /// Request a resume boundary. Timestamp issuance remains disabled until all
    /// selected streams explicitly report readiness for its generation.
    pub fn request_resume_at(&mut self, at: Instant) -> BoundaryRequestResult {
        self.request_boundary_at(StreamBoundaryKind::Resume, at)
    }

    /// Request a resume boundary using the local monotonic clock.
    pub fn request_resume(&mut self) -> BoundaryRequestResult {
        self.request_resume_at(Instant::now())
    }

    /// Apply one stream acknowledgement at a caller-supplied monotonic instant.
    pub fn acknowledge_at(
        &mut self,
        acknowledgement: StreamAcknowledgement,
        at: Instant,
    ) -> AcknowledgementResult {
        if self.clock.phase().is_terminal() {
            return AcknowledgementResult::Rejected(AcknowledgementRejection::Stopped);
        }

        let stream = acknowledgement.stream();
        if !self.streams.contains(stream) {
            return AcknowledgementResult::Rejected(AcknowledgementRejection::UnknownStream {
                stream,
            });
        }

        let Some(pending) = self.pending.as_ref() else {
            return AcknowledgementResult::Rejected(
                self.no_pending_rejection(acknowledgement.generation()),
            );
        };
        let expected_generation = pending.boundary.generation;
        let received_generation = acknowledgement.generation();
        if received_generation != expected_generation {
            return AcknowledgementResult::Rejected(generation_rejection(
                expected_generation,
                received_generation,
            ));
        }

        let expected_kind = pending.boundary.kind;
        let received_kind = acknowledgement.kind();
        if received_kind != expected_kind {
            return AcknowledgementResult::Rejected(AcknowledgementRejection::WrongBoundary {
                expected: expected_kind,
                received: received_kind,
            });
        }

        if acknowledgement.is_refusal() {
            if pending.acknowledged.contains(&stream) {
                return AcknowledgementResult::Rejected(
                    AcknowledgementRejection::DuplicateStream {
                        stream,
                        generation: received_generation,
                    },
                );
            }
            return self.handle_resume_refusal(stream, received_generation, at);
        }

        let completed_boundary = match self.record_acknowledgement(stream, received_generation) {
            Ok(completed_boundary) => completed_boundary,
            Err(rejection) => return AcknowledgementResult::Rejected(rejection),
        };
        if completed_boundary {
            self.complete_boundary(expected_kind, at);
        }
        AcknowledgementResult::Accepted {
            phase: self.clock.phase(),
            completed_boundary,
        }
    }

    /// Apply one acknowledgement using the local monotonic clock.
    pub fn acknowledge(&mut self, acknowledgement: StreamAcknowledgement) -> AcknowledgementResult {
        self.acknowledge_at(acknowledgement, Instant::now())
    }

    /// Stop the logical session and discard the outstanding boundary, if any.
    ///
    /// This never waits for workers and does not claim they stopped; a future
    /// registry must own physical shutdown and report it separately.
    pub fn stop_at(&mut self, at: Instant) -> SessionTransitionResult {
        self.pending = None;
        self.clock.apply_at(SessionTransition::Stop, at)
    }

    /// Stop the logical session using the local monotonic clock.
    pub fn stop(&mut self) -> SessionTransitionResult {
        self.stop_at(Instant::now())
    }

    /// Return the coordinator's immutable stream registration.
    pub fn selected_streams(&self) -> &SelectedCaptureStreams {
        &self.streams
    }

    /// Return the current logical phase.
    pub fn phase(&self) -> SessionPhase {
        self.clock.phase()
    }

    /// Return a logical timestamp only when the coordinator is actively recording.
    pub fn timestamp_at(&self, at: Instant) -> Option<Duration> {
        self.clock.timestamp_at(at)
    }

    /// Return whether the configured active-duration budget has been exhausted.
    /// This observation does not stop the coordinator or a native worker.
    pub fn active_duration_exhausted_at(&self, at: Instant) -> bool {
        self.clock.active_duration_exhausted_at(at)
    }

    /// Commit a post-close logical endpoint after a native owner has sealed its
    /// artifact. This is intentionally only valid after Pause or Stop: command
    /// issue remains timestamp-silent, but does not itself author the durable
    /// run endpoint.
    pub fn accept_post_close_elapsed(&mut self, elapsed: Duration) -> bool {
        self.clock.accept_post_close_elapsed(elapsed)
    }

    /// Snapshot registry-safe state without driving workers or transitions.
    pub fn status_at(&self, at: Instant) -> PauseStreamCoordinatorStatus {
        let logical_elapsed = self.clock.logical_elapsed_at(at);
        PauseStreamCoordinatorStatus {
            phase: self.clock.phase(),
            selected_streams: self.streams.clone(),
            pending_boundary: self.pending.as_ref().map(|pending| PendingStreamBoundary {
                boundary: pending.boundary.clone(),
                awaiting_streams: self.awaiting_streams(pending),
            }),
            latest_generation: (self.latest_generation != 0).then_some(self.latest_generation),
            timestamp_issuance_enabled: self.clock.timestamp_at(at).is_some(),
            logical_elapsed_ms: duration_millis(logical_elapsed),
            remaining_active_duration_ms: self
                .clock
                .remaining_active_duration_at(at)
                .map(duration_millis),
            active_duration_exhausted: self.clock.active_duration_exhausted_at(at),
            last_resume_refusal: self.last_resume_refusal,
        }
    }

    /// Snapshot status using the local monotonic clock.
    pub fn status(&self) -> PauseStreamCoordinatorStatus {
        self.status_at(Instant::now())
    }

    fn request_boundary_at(
        &mut self,
        kind: StreamBoundaryKind,
        at: Instant,
    ) -> BoundaryRequestResult {
        let (transition, expected_phase) = match kind {
            StreamBoundaryKind::Pause => {
                (SessionTransition::PauseRequested, SessionPhase::Recording)
            }
            StreamBoundaryKind::Resume => {
                (SessionTransition::ResumeRequested, SessionPhase::Paused)
            }
        };
        if self.clock.phase() != expected_phase {
            return ignored_request(self.clock.apply_at(transition, at));
        }

        let Some(generation) = self.latest_generation.checked_add(1) else {
            return BoundaryRequestResult::GenerationExhausted {
                phase: self.clock.phase(),
            };
        };
        let result = self.clock.apply_at(transition, at);
        debug_assert!(result.changed());
        self.latest_generation = generation;
        let boundary = StreamBoundary::new(kind, generation, self.streams.clone());
        self.pending = Some(PendingBoundary {
            boundary: boundary.clone(),
            acknowledged: BTreeSet::new(),
        });
        BoundaryRequestResult::Issued(boundary)
    }

    fn record_acknowledgement(
        &mut self,
        stream: RecordingStream,
        generation: u64,
    ) -> Result<bool, AcknowledgementRejection> {
        let pending = self
            .pending
            .as_mut()
            .expect("pending boundary checked before recording acknowledgement");
        if !pending.acknowledged.insert(stream) {
            return Err(AcknowledgementRejection::DuplicateStream { stream, generation });
        }
        Ok(pending.acknowledged.len() == self.streams.streams().len())
    }

    fn complete_boundary(&mut self, kind: StreamBoundaryKind, at: Instant) {
        let transition = match kind {
            StreamBoundaryKind::Pause => SessionTransition::PauseCompleted,
            StreamBoundaryKind::Resume => SessionTransition::ResumeCompleted,
        };
        self.pending = None;
        let result = self.clock.apply_at(transition, at);
        debug_assert!(result.changed());
        if kind == StreamBoundaryKind::Resume {
            self.last_resume_refusal = None;
        }
    }

    fn handle_resume_refusal(
        &mut self,
        stream: RecordingStream,
        generation: u64,
        at: Instant,
    ) -> AcknowledgementResult {
        let refusal = StreamRefusal { stream, generation };
        self.pending = None;
        self.last_resume_refusal = Some(refusal);
        let result = self.clock.apply_at(SessionTransition::ResumeAborted, at);
        debug_assert!(matches!(
            result,
            SessionTransitionResult::Applied {
                to: SessionPhase::Paused,
                ..
            }
        ));
        AcknowledgementResult::ResumeRefused(refusal)
    }

    fn awaiting_streams(&self, pending: &PendingBoundary) -> Vec<RecordingStream> {
        self.streams
            .streams()
            .iter()
            .copied()
            .filter(|stream| !pending.acknowledged.contains(stream))
            .collect()
    }

    fn no_pending_rejection(&self, received: u64) -> AcknowledgementRejection {
        if self.latest_generation == 0 {
            AcknowledgementRejection::NoBoundary { received }
        } else {
            generation_rejection(self.latest_generation, received)
        }
    }
}

fn generation_rejection(expected: u64, received: u64) -> AcknowledgementRejection {
    if received <= expected {
        AcknowledgementRejection::StaleGeneration { expected, received }
    } else {
        AcknowledgementRejection::FutureGeneration { expected, received }
    }
}

fn ignored_request(result: SessionTransitionResult) -> BoundaryRequestResult {
    match result {
        SessionTransitionResult::Ignored { phase, reason } => {
            BoundaryRequestResult::Ignored { phase, reason }
        }
        SessionTransitionResult::Applied { .. } => {
            unreachable!("request precondition prevents an already-applicable transition")
        }
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
