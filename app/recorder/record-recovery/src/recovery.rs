//! Owner liveness and idempotent interrupted recovery orchestration.

use std::path::{Path, PathBuf};

use crate::contract::{owner_probe, OwnerProbe};
use crate::integrity::{quarantine, quarantine_path, verified_prefix, PrefixIssue};
use crate::manifest::{read_manifest, ManifestOwner};
use crate::{CaptureManifest, ManifestError, RecoveryReceipt, RecoveryState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerState {
    Alive,
    Dead,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryResult {
    pub receipt: RecoveryReceipt,
    pub quarantined: Option<PathBuf>,
}

/// Recover exactly the contiguous verified prefix of a dead capture. A live or
/// ambiguous owner is never touched; this operation never sends a signal.
pub fn recover_interrupted(
    root: &Path,
    ffmpeg: &str,
    ffprobe: &str,
    owner: OwnerState,
) -> Result<Option<RecoveryResult>, ManifestError> {
    let manifest = read_manifest(root)?;
    if manifest.receipt.is_some() || owner != OwnerState::Dead {
        return Ok(None);
    }
    let (usable, issue) = verified_prefix(root, &manifest.checkpoints, ffmpeg, ffprobe)?;
    // Persist proved-corrupt evidence before any torn-journal repair.  A crash
    // between these two independent publications must retry as
    // `AlreadyQuarantined`, never reinterpret the absent checkpoint as merely
    // missing and seal a different terminal state.
    let checkpoint_quarantine = match issue {
        Some(PrefixIssue::Corrupt(sequence)) => Some(quarantine(root, sequence)?),
        Some(PrefixIssue::AlreadyQuarantined(sequence)) => Some(quarantine_path(root, sequence)),
        _ => None,
    };
    let recovered = (!usable.is_empty())
        .then(|| crate::stitch::stitch_complete(root, &usable, ffmpeg, ffprobe, "recovered.mp4"))
        .transpose()?;
    let (lost_tail_ms, lost_tail_lower_bound_ms, lost_tail_upper_bound_ms) =
        lost_tail(&manifest, usable.len());
    let state = match (issue, recovered.is_some()) {
        (Some(PrefixIssue::Corrupt(_) | PrefixIssue::AlreadyQuarantined(_)), _) => {
            RecoveryState::Quarantined
        }
        (_, true) => RecoveryState::Recovered,
        _ => RecoveryState::Interrupted,
    };
    let receipt = RecoveryReceipt {
        state,
        recovered_segments: usable.len() as u64,
        lost_tail_ms,
        lost_tail_lower_bound_ms,
        lost_tail_upper_bound_ms,
        audio_first_packet_offset_ms: None,
        source: recovered.as_ref().map(|path| file_name(path)),
        note: receipt_note(issue, recovered.is_some()),
    };
    let torn_tail_quarantine = if manifest.torn_tail {
        Some(crate::journal::repair_torn(root, &manifest, &receipt)?)
    } else {
        None
    };
    if !manifest.torn_tail {
        let mut owner = ManifestOwner::open(root)?;
        owner.publish_receipt(receipt.clone())?;
    }
    Ok(Some(RecoveryResult {
        receipt,
        quarantined: checkpoint_quarantine.or(torn_tail_quarantine),
    }))
}

pub fn owner_state(start: &crate::CaptureStart) -> OwnerState {
    match owner_probe(start.owner_pid) {
        OwnerProbe::Identity(identity) if identity == start.owner_identity => OwnerState::Alive,
        OwnerProbe::Identity(_) => OwnerState::Ambiguous,
        OwnerProbe::Dead => OwnerState::Dead,
        OwnerProbe::Ambiguous => OwnerState::Ambiguous,
    }
}

fn receipt_note(issue: Option<PrefixIssue>, recovered: bool) -> String {
    if matches!(
        issue,
        Some(PrefixIssue::Corrupt(_) | PrefixIssue::AlreadyQuarantined(_))
    ) {
        "interrupted capture: corrupt checkpoint quarantined; only verified prefix recovered".into()
    } else if matches!(issue, Some(PrefixIssue::MissingEvidence)) {
        "interrupted capture: missing or unsafe checkpoint was not moved; verified prefix recovered"
            .into()
    } else if !recovered {
        "interrupted capture: no independently playable checkpoint was available".into()
    } else {
        "interrupted capture: independently finalized checkpoints recovered".into()
    }
}

fn lost_tail(manifest: &CaptureManifest, used: usize) -> (Option<u64>, u64, Option<u64>) {
    let end = manifest
        .checkpoints
        .last()
        .map(|checkpoint| checkpoint.facts.end_ms)
        .unwrap_or(0);
    let kept = used
        .checked_sub(1)
        .and_then(|index| manifest.checkpoints.get(index))
        .map(|checkpoint| checkpoint.facts.end_ms)
        .unwrap_or(0);
    let known = end.saturating_sub(kept);
    if !manifest.openings.is_empty() || used == manifest.checkpoints.len() {
        (None, known, None)
    } else {
        (Some(known), known, Some(known))
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_string()
}
