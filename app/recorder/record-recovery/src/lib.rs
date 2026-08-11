//! Durable, append-only capture checkpoint ownership and fail-closed recovery.
//!
//! A live encoder's open container is deliberately never a recovery input. Backends
//! publish a checkpoint only after closing its own MP4; recovery re-hashes and probes
//! those immutable files before a concat/remux operation can use them.

mod atomic;
mod containment;
mod contract;
mod integrity;
mod journal;
#[cfg(target_os = "macos")]
mod macos_owner;
mod manifest;
mod media;
mod recovery;
mod segment;
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
mod stitch_tests;
#[cfg(test)]
mod tests;

pub use atomic::{publish_new_synced, replace_file_synced, replace_synced};
pub use containment::CaptureRoot;
pub use contract::{
    CaptureManifest, CaptureStart, Checkpoint, CheckpointFacts, ManifestError, MediaFacts,
    RecoveryReceipt, RecoveryState,
};
pub use manifest::{
    is_plain_dir, is_plain_regular_file, read_manifest, ManifestOwner, MANIFEST_FILE,
};
pub use media::verify_media;
pub use recovery::{owner_state, recover_interrupted, OwnerState, RecoveryResult};
pub use staging::{
    create_staging_file, windows_wgc_path_budget, PrivateStaging, WindowsWgcPathBudget,
};
pub use status::{recovery_status, CaptureRecoveryState, CaptureRecoveryStatus, ReceiptStatus};
pub use stitch::stitch_complete;
pub use torn_repair::seal_torn_receipt;
