//! Sealing a known terminal receipt after a final JSONL record tore.

use std::path::Path;

use crate::contract::valid_receipt;
use crate::{read_manifest, ManifestError, RecoveryReceipt};

/// Archive the exact unsynced tail and atomically replace it with `receipt`.
/// This is deliberately distinct from interrupted media recovery: callers that
/// already proved a normal `source.mp4` may retain that source rather than
/// remuxing it to a recovery artifact.
pub fn seal_torn_receipt(root: &Path, receipt: &RecoveryReceipt) -> Result<(), ManifestError> {
    let manifest = read_manifest(root)?;
    if !manifest.has_torn_tail() {
        return Err(ManifestError::Invalid(
            "manifest has no torn final entry to seal".into(),
        ));
    }
    if !valid_receipt(receipt, &manifest.checkpoints, manifest.has_open_segment()) {
        return Err(ManifestError::Invalid(
            "recovery receipt is inconsistent with the capture journal".into(),
        ));
    }
    crate::journal::repair_torn(root, &manifest, receipt)?;
    Ok(())
}
