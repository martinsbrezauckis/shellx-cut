use super::super::pause_projection::SealedLegacyProjectionRun;
use super::super::windows_pause_evidence::RecordingInputDraft;
use record_capture::StreamBoundary;
use record_recovery::{
    RecordingSessionJournal, RecordingSessionJournalEntry, RecordingSessionJournalFile,
    RecordingStream, SealedRun, SessionJournalError,
};
use std::error::Error as StdError;
use std::fmt;
use std::time::Instant;

/// A sink for the one private session journal. The read-only journal snapshot is
/// intentionally included so every compound operation can be preflighted before
/// it mutates the durable prefix. Production uses `RecordingSessionJournalFile`;
/// tests inject a deterministic in-memory failure sink.
pub(crate) trait RecordingSessionJournalSink {
    fn journal(&self) -> &RecordingSessionJournal;

    fn append_entry(
        &mut self,
        entry: RecordingSessionJournalEntry,
    ) -> Result<(), SessionJournalError>;
}

impl RecordingSessionJournalSink for RecordingSessionJournalFile {
    fn journal(&self) -> &RecordingSessionJournal {
        RecordingSessionJournalFile::journal(self)
    }

    fn append_entry(
        &mut self,
        entry: RecordingSessionJournalEntry,
    ) -> Result<(), SessionJournalError> {
        RecordingSessionJournalFile::append_entry(self, entry)
    }
}

/// A paired monotonic and Unix observation of the real shared backend clock.
///
/// Callers create this only after the backend has opened `CaptureClock`; the
/// coordinator refuses to write `Started` until it owns one. Every later Unix
/// timestamp is checked rather than saturated, so an invalid clock ordering or
/// an unrepresentable conversion cannot fabricate durable chronology.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SessionTimeOrigin {
    pub(super) monotonic: Instant,
    pub(super) unix_ms: u64,
}

impl SessionTimeOrigin {
    pub(crate) fn observed(monotonic: Instant, unix_ms: u64) -> Self {
        Self { monotonic, unix_ms }
    }

    pub(crate) fn elapsed_ms(self, at: Instant) -> Result<u64, RunSealCoordinatorError> {
        let elapsed = at
            .checked_duration_since(self.monotonic)
            .ok_or_else(|| invalid("monotonic fact predates the backend clock origin"))?;
        u64::try_from(elapsed.as_millis())
            .map_err(|_| invalid("monotonic elapsed time does not fit in milliseconds"))
    }

    pub(crate) fn matches(self, monotonic: Instant, unix_ms: u64) -> bool {
        self.monotonic == monotonic && self.unix_ms == unix_ms
    }

    pub(super) fn unix_at(self, at: Instant) -> Result<u64, RunSealCoordinatorError> {
        self.unix_ms
            .checked_add(self.elapsed_ms(at)?)
            .ok_or_else(|| invalid("Unix timestamp overflows the durable journal domain"))
    }
}

/// Immutable evidence for one active run. The evidence deliberately contains no
/// caller-provided path: artifact identity remains inside the validated
/// `SealedRun` journal contract.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SealedRunEvidence {
    pub(super) generation: u64,
    pub(super) run: SealedRun,
    pub(super) legacy_projection_run: SealedLegacyProjectionRun,
    pub(super) sealed_streams: Vec<RecordingStream>,
    pub(super) recording_input: Option<RecordingInputDraft>,
}

impl SealedRunEvidence {
    pub(crate) fn new(
        generation: u64,
        run: SealedRun,
        legacy_projection_run: SealedLegacyProjectionRun,
        sealed_streams: Vec<RecordingStream>,
    ) -> Self {
        Self {
            generation,
            run,
            legacy_projection_run,
            sealed_streams,
            recording_input: None,
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn with_recording_input(mut self, recording_input: RecordingInputDraft) -> Self {
        self.recording_input = Some(recording_input);
        self
    }

    pub(crate) fn recording_input(&self) -> Option<&RecordingInputDraft> {
        self.recording_input.as_ref()
    }

    #[allow(dead_code)] // The exact run accessor is exercised by private evidence tests.
    pub(crate) fn run(&self) -> &SealedRun {
        &self.run
    }

    /// Clone the exact sealed event input that was validated with this durable
    /// run. The projection executor receives it only after terminal completion.
    pub(crate) fn legacy_projection_run(&self) -> SealedLegacyProjectionRun {
        self.legacy_projection_run.clone()
    }
}

/// Exact readiness facts for a resume boundary. This is intentionally stream
/// identity only: a path is not readiness evidence, and a native registry must
/// report its own generation-tagged readiness before this type is constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResumeReadinessEvidence {
    pub(super) generation: u64,
    pub(super) ready_streams: Vec<RecordingStream>,
}

impl ResumeReadinessEvidence {
    pub(crate) fn new(generation: u64, ready_streams: Vec<RecordingStream>) -> Self {
        Self {
            generation,
            ready_streams,
        }
    }
}

/// A generation-tagged pause command correlation. It intentionally contains no
/// time endpoint: command issue disables timestamp issuance, but only the
/// later post-close sealed evidence may author the durable run boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PauseSealRequest {
    pub(super) boundary: StreamBoundary,
}

impl PauseSealRequest {
    pub(crate) fn boundary(&self) -> &StreamBoundary {
        &self.boundary
    }
}

/// A terminal stop command correlation. A stop with
/// `requires_sealed_run == false` follows a previously durable pause or an
/// uncompleted resume and therefore has no active run to seal. Like pause, it
/// never pre-authorizes a final logical or observed endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StopSealRequest {
    /// The generation the server-side sealed evidence must carry. Initial
    /// direct Stop is generation zero because it has no Pause boundary.
    pub(super) generation: Option<u64>,
}

impl StopSealRequest {
    pub(crate) fn requires_sealed_run(&self) -> bool {
        self.generation.is_some()
    }

    pub(crate) fn generation(&self) -> Option<u64> {
        self.generation
    }
}

/// Fail-closed private coordinator error. None of these errors is exposed as a
/// verb response; a future owner must map it only after it has a real lifecycle
/// and receipt contract.
#[derive(Debug)]
pub(crate) enum RunSealCoordinatorError {
    Invalid(String),
    Journal(SessionJournalError),
    FailedAfterDurableAppend,
    Stopped,
    BoundaryPending,
}

impl fmt::Display for RunSealCoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(detail) => {
                write!(formatter, "invalid run-seal coordinator fact: {detail}")
            }
            Self::Journal(error) => {
                write!(
                    formatter,
                    "recording session journal append failed: {error}"
                )
            }
            Self::FailedAfterDurableAppend => {
                formatter.write_str("run-seal coordinator is closed after a durable append failure")
            }
            Self::Stopped => {
                formatter.write_str("stop is terminal and wins over late boundary facts")
            }
            Self::BoundaryPending => {
                formatter.write_str("a pause or resume boundary is already pending")
            }
        }
    }
}

impl StdError for RunSealCoordinatorError {}

impl From<SessionJournalError> for RunSealCoordinatorError {
    fn from(error: SessionJournalError) -> Self {
        Self::Journal(error)
    }
}

#[derive(Debug, Clone)]
pub(super) struct ActiveRun {
    pub(super) generation: u64,
    pub(super) sequence: u64,
    pub(super) observed_start_ms: u64,
    pub(super) logical_start_ms: u64,
}

impl ActiveRun {
    pub(super) fn for_generation(&self, generation: u64) -> ExpectedRun {
        ExpectedRun {
            generation: self.generation,
            sequence: self.sequence,
            observed_start_ms: self.observed_start_ms,
            logical_start_ms: self.logical_start_ms,
        }
        .with_generation(generation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExpectedRun {
    pub(super) generation: u64,
    pub(super) sequence: u64,
    pub(super) observed_start_ms: u64,
    pub(super) logical_start_ms: u64,
}

impl ExpectedRun {
    fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }
}

pub(super) struct PendingPause {
    pub(super) boundary: StreamBoundary,
    pub(super) expected: ExpectedRun,
    pub(super) floor: RunBoundaryFloor,
}

pub(super) struct PendingResume {
    pub(super) boundary: StreamBoundary,
}

pub(super) enum PendingBoundary {
    Pause(PendingPause),
    Resume(PendingResume),
}

pub(super) struct PendingStop {
    pub(super) expected: Option<ExpectedRun>,
    pub(super) floor: Option<RunBoundaryFloor>,
    pub(super) terminal_logical_end_ms: u64,
}

/// Command-time lower bounds used only to reject a stale sealed artifact. They
/// are not an accepted run endpoint and never appear in the durable journal.
#[derive(Debug, Clone, Copy)]
pub(super) struct RunBoundaryFloor {
    pub(super) logical_ms: u64,
    pub(super) observed_ms: u64,
}

pub(super) fn invalid(detail: impl Into<String>) -> RunSealCoordinatorError {
    RunSealCoordinatorError::Invalid(detail.into())
}
