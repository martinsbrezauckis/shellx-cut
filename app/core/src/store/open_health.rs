//! Open-time recovery facts for the currently materialized project.
//!
//! The journal remains the only durable truth. These facts record what the
//! most recent strict open had to do, so bounded read surfaces can disclose
//! recovery without inspecting project-local paths or treating a cache as
//! authoritative.

use crate::JournalRecovery;

/// Whether the disposable `project.json` cache matched journal replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectCacheHealth {
    Matched,
    Rebuilt,
}

impl ProjectCacheHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Matched => "matched",
            Self::Rebuilt => "rebuilt",
        }
    }
}

/// Validation result for the nearest disposable history snapshot at open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectSnapshotHealth {
    NotPresent,
    Verified { prefix_ops: usize },
    Rejected,
}

impl ProjectSnapshotHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotPresent => "not_present",
            Self::Verified { .. } => "verified",
            Self::Rejected => "rejected",
        }
    }

    pub fn prefix_ops(self) -> Option<usize> {
        match self {
            Self::Verified { prefix_ops } => Some(prefix_ops),
            Self::NotPresent | Self::Rejected => None,
        }
    }
}

/// Immutable recovery outcome from project creation/open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectOpenHealth {
    pub cache: ProjectCacheHealth,
    pub snapshot: ProjectSnapshotHealth,
    /// Present only when opening repaired a malformed final JSONL record. The
    /// quarantine paths remain local implementation detail; callers expose
    /// only the discarded byte range.
    pub journal_tail_recovery: Option<JournalRecovery>,
}

impl ProjectOpenHealth {
    pub(crate) fn new_project() -> Self {
        Self {
            cache: ProjectCacheHealth::Matched,
            snapshot: ProjectSnapshotHealth::NotPresent,
            journal_tail_recovery: None,
        }
    }
}
