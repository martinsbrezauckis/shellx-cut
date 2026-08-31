//! Durable, append-only capture checkpoint ownership and fail-closed recovery.
//!
//! A live encoder's open container is deliberately never a recovery input. Backends
//! publish a checkpoint only after closing its own MP4; recovery re-hashes and probes
//! those immutable files before a concat/remux operation can use them.

mod atomic;
mod containment;
mod contract;
mod frame_grid;
mod integrity;
mod journal;
#[cfg(target_os = "macos")]
mod macos_owner;
mod manifest;
mod media;
mod recovery;
mod run_stitch;
mod segment;
mod session_contract;
mod session_journal;
mod session_journal_io;
mod session_validation;
mod staging;
mod status;
mod stitch;
mod torn_repair;

#[cfg(test)]
mod manifest_tests;
#[cfg(test)]
mod publication_tests;
#[cfg(test)]
mod receipt_tests;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod session_input_sidecar_tests;
#[cfg(test)]
mod session_journal_io_tests;
#[cfg(test)]
mod session_journal_tests;
#[cfg(test)]
mod stitch_tests;
#[cfg(test)]
mod tests;

pub use atomic::{publish_new_synced, replace_file_synced, replace_synced};
pub use containment::CaptureRoot;
pub use contract::{
    CaptureManifest, CaptureStart, Checkpoint, CheckpointFacts, ManifestError, MediaFacts,
    RecoveryReceipt, RecoveryState,
};
pub use frame_grid::{
    quantize_run_aware_stitch, ExactFrameGridDuration, FrameGridError, FrameGridStitchPlan,
    FrameGridStitchSpan,
};
pub use manifest::{
    is_plain_dir, is_plain_regular_file, read_manifest, ManifestOwner, MANIFEST_FILE,
};
pub use media::verify_media;
pub use recovery::{owner_state, recover_interrupted, OwnerState, RecoveryResult};
pub use run_stitch::{plan_run_aware_stitch, RunAwareStitchPlan, RunAwareStitchSpan};
pub use session_contract::{
    CheckpointSequenceRange, DurableStateTransition, RecordingInputSidecarPin,
    RecordingProjectBinding, RecordingProjectIdentity, RecordingSessionIntent,
    RecordingSessionJournalEntry, RecordingSessionState, RecordingStream, SealedRun,
    SessionJournalError, SessionTerminal, StreamFragment, StreamFragmentFacts, TerminalDisposition,
    RECORDING_PROJECT_ID_SCHEMA, RECORDING_SESSION_JOURNAL_SCHEMA,
};
pub use session_journal::RecordingSessionJournal;
pub use session_journal_io::{RecordingSessionJournalFile, RECORDING_SESSION_JOURNAL_FILE};
pub use staging::{
    create_staging_file, windows_wgc_path_budget, PrivateStaging, WindowsWgcPathBudget,
};
pub use status::{recovery_status, CaptureRecoveryState, CaptureRecoveryStatus, ReceiptStatus};
pub use stitch::{stitch_complete, stitch_complete_with_media, StitchedMedia};
pub use torn_repair::seal_torn_receipt;
