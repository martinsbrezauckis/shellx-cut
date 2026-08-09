//! Serializable recovery contract shared by the append journal and restart scanner.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub(crate) const SCHEMA: &str = "shellx-cut/record-checkpoints/1";

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("checkpoint manifest is corrupt: {0}")]
    Corrupt(String),
    #[error("checkpoint I/O at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("checkpoint serialization: {0}")]
    Json(#[from] serde_json::Error),
    #[error("checkpoint input is invalid: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CaptureStart {
    pub schema: String,
    pub capture_id: String,
    pub owner_pid: u32,
    /// OS process-start identity, not merely a reusable PID.
    pub owner_identity: String,
    /// Per-capture nonce recorded with the owner identity and journal. It makes a
    /// stale owner record distinguishable even when paths and PIDs are recycled.
    #[serde(default)]
    pub owner_nonce: String,
    pub started_unix_ms: u64,
    pub checkpoint_interval_ms: u64,
}

impl CaptureStart {
    pub fn new(capture_id: impl Into<String>, checkpoint_interval_ms: u64) -> Self {
        Self {
            schema: SCHEMA.into(),
            capture_id: capture_id.into(),
            owner_pid: std::process::id(),
            owner_identity: match owner_probe(std::process::id()) {
                OwnerProbe::Identity(identity) => identity,
                OwnerProbe::Dead | OwnerProbe::Ambiguous => "unavailable".into(),
            },
            owner_nonce: owner_nonce(),
            started_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
                .unwrap_or(0),
            checkpoint_interval_ms,
        }
    }
}

fn owner_nonce() -> String {
    let ticks = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}", std::process::id(), ticks)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointFacts {
    /// First captured video time on the global EventTrack clock. Stitching uses
    /// this, rather than a reservation timestamp, when materializing restart
    /// gaps into the playable source timeline.
    pub start_ms: u64,
    pub end_ms: u64,
    /// The event-sidecar position of that same first frame. It is checked against
    /// `start_ms` so a stale/rebased segment can never silently shift events.
    pub event_offset_ms: u64,
    /// Global first-packet offset for an independently finalized audio sidecar,
    /// when the backend can prove one. Unknown is represented by `None`; never
    /// write a guessed zero merely because audio was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_offset_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    pub sequence: u64,
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
    /// Facts measured from a completed container, before this record is published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaFacts>,
    #[serde(flatten)]
    pub facts: CheckpointFacts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaFacts {
    pub duration_ms: u64,
    pub decoded_video_frames: u64,
    pub has_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct OpenCheckpoint {
    pub sequence: u64,
    pub staging: String,
    pub start_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
    Complete,
    Recovered,
    Interrupted,
    Quarantined,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryReceipt {
    pub state: RecoveryState,
    pub recovered_segments: u64,
    /// Exact loss is unknown while a final open segment did not publish.
    pub lost_tail_ms: Option<u64>,
    pub lost_tail_lower_bound_ms: u64,
    pub lost_tail_upper_bound_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_first_packet_offset_ms: Option<u64>,
    pub source: Option<String>,
    pub note: String,
}

/// A sealed receipt is part of the journal's integrity boundary, not merely a
/// status annotation. Keep these checks shared by the reader and writer so a
/// syntactically valid, impossible terminal state cannot hide an unsealed
/// capture or project a path back through the public status API.
pub(crate) fn valid_receipt(
    receipt: &RecoveryReceipt,
    checkpoints: &[Checkpoint],
    has_open_segment: bool,
) -> bool {
    let count = checkpoints.len() as u64;
    if receipt.recovered_segments > count {
        return false;
    }
    let kept_end_ms = receipt
        .recovered_segments
        .checked_sub(1)
        .and_then(|index| checkpoints.get(index as usize))
        .map(|checkpoint| checkpoint.facts.end_ms)
        .unwrap_or(0);
    let committed_end_ms = checkpoints
        .last()
        .map(|checkpoint| checkpoint.facts.end_ms)
        .unwrap_or(0);
    let known_loss = committed_end_ms.saturating_sub(kept_end_ms);
    let loss_is_coherent = if receipt.state == RecoveryState::Complete {
        receipt.lost_tail_ms == Some(0)
            && receipt.lost_tail_lower_bound_ms == 0
            && receipt.lost_tail_upper_bound_ms == Some(0)
    } else if has_open_segment || receipt.recovered_segments == count {
        receipt.lost_tail_ms.is_none()
            && receipt.lost_tail_lower_bound_ms == known_loss
            && receipt.lost_tail_upper_bound_ms.is_none()
    } else {
        receipt.lost_tail_ms == Some(known_loss)
            && receipt.lost_tail_lower_bound_ms == known_loss
            && receipt.lost_tail_upper_bound_ms == Some(known_loss)
    };
    if !loss_is_coherent {
        return false;
    }
    match receipt.state {
        RecoveryState::Complete => {
            !has_open_segment
                && receipt.recovered_segments == count
                && receipt.lost_tail_ms == Some(0)
                && receipt.lost_tail_lower_bound_ms == 0
                && receipt.lost_tail_upper_bound_ms == Some(0)
                && receipt.source.as_deref() == Some("source.mp4")
        }
        RecoveryState::Recovered => {
            receipt.recovered_segments > 0 && receipt.source.as_deref() == Some("recovered.mp4")
        }
        RecoveryState::Quarantined => {
            receipt.recovered_segments < count
                && ((receipt.recovered_segments == 0 && receipt.source.is_none())
                    || (receipt.recovered_segments > 0
                        && receipt.source.as_deref() == Some("recovered.mp4")))
        }
        RecoveryState::Interrupted => receipt.recovered_segments == 0 && receipt.source.is_none(),
    }
}

#[derive(Debug, Clone)]
pub struct CaptureManifest {
    pub start: CaptureStart,
    pub checkpoints: Vec<Checkpoint>,
    pub receipt: Option<RecoveryReceipt>,
    pub(crate) openings: Vec<OpenCheckpoint>,
    pub(crate) torn_tail: bool,
    /// Bytes through the last synced newline; kept only while a final line tore.
    pub(crate) valid_prefix: Vec<u8>,
    /// Exact unsynced final bytes. Recovery quarantines these, never parses them.
    pub(crate) torn_tail_bytes: Option<Vec<u8>>,
}

impl CaptureManifest {
    pub fn next_sequence(&self) -> u64 {
        self.checkpoints.len() as u64
    }

    pub fn has_torn_tail(&self) -> bool {
        self.torn_tail
    }

    pub fn has_open_segment(&self) -> bool {
        !self.openings.is_empty()
    }
}

pub(crate) enum OwnerProbe {
    Identity(String),
    Dead,
    Ambiguous,
}

#[cfg(target_os = "linux")]
pub(crate) fn owner_probe(pid: u32) -> OwnerProbe {
    let path = format!("/proc/{pid}/stat");
    if !std::path::Path::new(&path).exists() {
        return OwnerProbe::Dead;
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(") ")?
                .1
                .split_whitespace()
                .nth(19)
                .map(str::to_string)
        })
        .and_then(|start| {
            std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .ok()
                .map(|boot| format!("{}:{}", boot.trim(), start))
        })
        .map(OwnerProbe::Identity)
        .unwrap_or(OwnerProbe::Ambiguous)
}
#[cfg(target_os = "macos")]
pub(crate) fn owner_probe(pid: u32) -> OwnerProbe {
    crate::macos_owner::owner_probe(pid)
}
#[cfg(windows)]
pub(crate) fn owner_probe(pid: u32) -> OwnerProbe {
    windows_owner_probe(pid)
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(crate) fn owner_probe(_pid: u32) -> OwnerProbe {
    OwnerProbe::Ambiguous
}

#[cfg(windows)]
fn windows_owner_probe(pid: u32) -> OwnerProbe {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_INVALID_PARAMETER};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return if GetLastError() == ERROR_INVALID_PARAMETER {
                OwnerProbe::Dead
            } else {
                OwnerProbe::Ambiguous
            };
        }
        let mut created = std::mem::zeroed();
        let mut exited = std::mem::zeroed();
        let mut kernel = std::mem::zeroed();
        let mut user = std::mem::zeroed();
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        if ok {
            OwnerProbe::Identity(format!(
                "{}:{}",
                created.dwLowDateTime, created.dwHighDateTime
            ))
        } else {
            OwnerProbe::Ambiguous
        }
    }
}
