//! Path-safe, read-only capture-recovery status projection.

use std::path::Path;

use serde::Serialize;

use crate::{read_manifest, RecoveryReceipt, RecoveryState};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureRecoveryState {
    Complete,
    Recovered,
    Quarantined,
    Interrupted,
    OwnerAmbiguous,
    TornJournal,
    Corrupt,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReceiptStatus {
    pub state: RecoveryState,
    pub recovered_segments: u64,
    pub lost_tail_ms: Option<u64>,
    pub lost_tail_lower_bound_ms: u64,
    pub lost_tail_upper_bound_ms: Option<u64>,
    pub audio_first_packet_offset_ms: Option<u64>,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CaptureRecoveryStatus {
    pub state: CaptureRecoveryState,
    pub checkpoints: u64,
    pub has_open_segment: bool,
    pub receipt: Option<ReceiptStatus>,
}

/// This reads only the local capture journal. It never probes, remuxes, renames,
/// signals, or otherwise changes a capture; callers must run recovery separately.
pub fn recovery_status(root: &Path) -> CaptureRecoveryStatus {
    let manifest = match read_manifest(root) {
        Ok(manifest) => manifest,
        Err(_) => {
            return CaptureRecoveryStatus {
                state: CaptureRecoveryState::Corrupt,
                checkpoints: 0,
                has_open_segment: false,
                receipt: None,
            }
        }
    };
    let receipt = manifest.receipt.as_ref().map(receipt_status);
    let state = if manifest.has_torn_tail() {
        CaptureRecoveryState::TornJournal
    } else if let Some(receipt) = manifest.receipt.as_ref() {
        match receipt.state {
            RecoveryState::Complete => CaptureRecoveryState::Complete,
            RecoveryState::Recovered => CaptureRecoveryState::Recovered,
            RecoveryState::Quarantined => CaptureRecoveryState::Quarantined,
            RecoveryState::Interrupted => CaptureRecoveryState::Interrupted,
        }
    } else {
        // Status is contractually process-free. Startup recovery may make a
        // bounded owner observation before mutating, but this read verb never
        // invokes ps/sysctl (or platform-equivalent process APIs).
        CaptureRecoveryState::OwnerAmbiguous
    };
    CaptureRecoveryStatus {
        state,
        checkpoints: manifest.checkpoints.len() as u64,
        has_open_segment: manifest.has_open_segment(),
        receipt,
    }
}

fn receipt_status(receipt: &RecoveryReceipt) -> ReceiptStatus {
    ReceiptStatus {
        state: receipt.state.clone(),
        recovered_segments: receipt.recovered_segments,
        lost_tail_ms: receipt.lost_tail_ms,
        lost_tail_lower_bound_ms: receipt.lost_tail_lower_bound_ms,
        lost_tail_upper_bound_ms: receipt.lost_tail_upper_bound_ms,
        audio_first_packet_offset_ms: receipt.audio_first_packet_offset_ms,
        source: receipt.source.as_deref().and_then(safe_file_name),
    }
}

fn safe_file_name(value: &str) -> Option<String> {
    let path = Path::new(value);
    (path.file_name().and_then(|name| name.to_str()) == Some(value) && !value.contains(['/', '\\']))
        .then(|| value.to_string())
}
