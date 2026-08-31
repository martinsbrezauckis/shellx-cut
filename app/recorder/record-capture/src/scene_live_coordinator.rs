//! Private live ownership for the Screen-only recorder-scene subset.
//!
//! This converts the accepted pure scene snapshot into one durable capture
//! receipt. It has no native camera, renderer, server verb, or UI dependency:
//! a caller supplies already-safe logical media time from its live coordinator.

use std::fmt;

use record_core::scene::ScenePreset;
use record_core::scene_projection::{SceneFixtureDescriptor, SceneFixtureRenderer};
use record_core::{
    AcceptedStartSnapshot, CatalogRevision, PresetRevision, SceneCatalog, SceneComposition,
    SceneEvent, SceneEventKind, SceneId, TimerConfig,
};
use record_recovery::CaptureRoot;
use serde::Serialize;

use crate::scene_journal::SceneJournalOwner;

/// Private scene-owner failure. This stays below every server/public contract.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateSceneCoordinatorError {
    detail: String,
}

/// The sealed private Screen-only projection that a recorder owner may bind to
/// its ordinary screen output. It remains deliberately below the server schema
/// and is never a UI, camera, or device capability.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivateSceneProjection {
    schema: String,
    logical_media_time_ms: u64,
    journal_sha256: String,
    scene: SceneFixtureDescriptor,
}

impl fmt::Display for PrivateSceneCoordinatorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for PrivateSceneCoordinatorError {}

impl PrivateSceneCoordinatorError {
    /// Construct a private cross-crate admission error without widening a
    /// server or public recorder contract.
    pub fn from_detail(detail: String) -> Self {
        Self { detail }
    }
}

/// One capture's journaled Screen-only scene state.
///
/// Event time is logical media time, never wall time. The caller must obtain it
/// from the same pause-safe clock that owns the selected capture streams.
#[doc(hidden)]
#[derive(Debug)]
pub struct PrivateSceneCoordinator {
    journal: SceneJournalOwner,
}

impl PrivateSceneCoordinator {
    /// Create the deliberately minimal production subset: one Screen-only scene
    /// with one private elapsed timer and no visible timer control. The durable
    /// header freezes that decision before native capture starts.
    pub fn create_default_screen_only(
        root: &CaptureRoot,
        capture_id: &str,
    ) -> Result<Self, PrivateSceneCoordinatorError> {
        Self::create(root, capture_id, default_screen_only_snapshot())
    }

    /// Create a private coordinator from a pre-accepted Screen-only snapshot.
    /// This is used by deterministic owner tests and future private selection
    /// admission; presenter layouts are rejected before a journal is created.
    pub fn create(
        root: &CaptureRoot,
        capture_id: &str,
        snapshot: AcceptedStartSnapshot,
    ) -> Result<Self, PrivateSceneCoordinatorError> {
        require_screen_only(&snapshot)?;
        let journal = SceneJournalOwner::create_new(root, capture_id, snapshot).map_err(journal)?;
        Ok(Self { journal })
    }

    /// Reopen a prior durable scene receipt after a process interruption.
    ///
    /// The journal owner repairs only its narrowly-defined torn final record and
    /// then replays the reducer before this coordinator accepts another event.
    pub fn reopen(
        root: &CaptureRoot,
        capture_id: &str,
    ) -> Result<Self, PrivateSceneCoordinatorError> {
        let journal = SceneJournalOwner::open(root, capture_id).map_err(journal)?;
        require_screen_only(journal.replayed().snapshot())?;
        Ok(Self { journal })
    }

    /// Project the current accepted scene at caller-owned logical media time.
    pub fn project_at(
        &self,
        logical_media_time_ms: u64,
    ) -> Result<SceneFixtureDescriptor, PrivateSceneCoordinatorError> {
        SceneFixtureRenderer::render(
            self.journal.replayed().snapshot(),
            self.journal.replayed().state(),
            logical_media_time_ms,
        )
        .map_err(scene)
    }

    /// End the private timer at a caller-owned logical media boundary. A
    /// duplicate Stop is harmless: the already-ended scene is only projected,
    /// never given a second terminal event.
    pub fn finish_at(
        &mut self,
        logical_media_time_ms: u64,
    ) -> Result<SceneFixtureDescriptor, PrivateSceneCoordinatorError> {
        let projection = self.project_at(logical_media_time_ms)?;
        let running = match projection.timer {
            record_core::scene_projection::SceneTimerFixture::Off => false,
            record_core::scene_projection::SceneTimerFixture::Elapsed { phase, .. }
            | record_core::scene_projection::SceneTimerFixture::Countdown { phase, .. } => {
                matches!(
                    phase,
                    record_core::TimerPhase::Running
                        | record_core::TimerPhase::Paused
                        | record_core::TimerPhase::TimeUp
                )
            }
        };
        if running {
            self.append_at(logical_media_time_ms, SceneEventKind::TimerEnd)
        } else {
            Ok(projection)
        }
    }

    /// Produce one receipt-ready projection from the durable journal. This
    /// requires the timer to have reached its terminal state so a recovered
    /// editable screen output never evaluates a live wall clock later.
    pub fn completed_projection_at(
        &self,
        logical_media_time_ms: u64,
    ) -> Result<PrivateSceneProjection, PrivateSceneCoordinatorError> {
        let scene = self.project_at(logical_media_time_ms)?;
        let terminal = match scene.timer {
            record_core::scene_projection::SceneTimerFixture::Off => true,
            record_core::scene_projection::SceneTimerFixture::Elapsed { phase, .. }
            | record_core::scene_projection::SceneTimerFixture::Countdown { phase, .. } => {
                matches!(phase, record_core::TimerPhase::Ended)
            }
        };
        if !terminal {
            return Err(PrivateSceneCoordinatorError {
                detail: "private scene projection requires a terminal timer state".into(),
            });
        }
        Ok(PrivateSceneProjection {
            schema: "shellx-record/private-screen-scene-projection@1".into(),
            logical_media_time_ms,
            journal_sha256: self.journal.durable_sha256().map_err(journal)?,
            scene,
        })
    }

    /// Durably append one private scene action and project the resulting state.
    ///
    /// The next sequence is derived from the replayed receipt on every append,
    /// so reopening never guesses or duplicates a sequence number.
    pub fn append_at(
        &mut self,
        logical_media_time_ms: u64,
        kind: SceneEventKind,
    ) -> Result<SceneFixtureDescriptor, PrivateSceneCoordinatorError> {
        let replay = self.journal.replayed();
        let sequence = replay
            .state()
            .last_sequence()
            .checked_add(1)
            .ok_or_else(|| PrivateSceneCoordinatorError {
                detail: "private scene event sequence is exhausted".into(),
            })?;
        let event = SceneEvent {
            sequence,
            logical_media_time_ms,
            snapshot_revision: replay.snapshot().revision(),
            kind,
        };
        self.journal.append_event(event).map_err(journal)?;
        self.project_at(logical_media_time_ms)
    }
}

fn default_screen_only_snapshot() -> AcceptedStartSnapshot {
    let id = SceneId::parse("screen-only").expect("literal private scene id is valid");
    let preset = ScenePreset::new(
        id.clone(),
        "Screen",
        PresetRevision::new(1).expect("literal private preset revision is non-zero"),
        SceneComposition::ScreenOnly,
        TimerConfig::Elapsed,
    )
    .expect("literal private Screen-only scene is valid");
    let catalog = SceneCatalog::new(
        CatalogRevision::new(1).expect("literal private catalog revision is non-zero"),
        vec![preset],
    )
    .expect("literal private Screen-only catalog is valid");
    AcceptedStartSnapshot::accept(&catalog, &id)
        .expect("literal private Screen-only scene is present in its catalog")
}

fn require_screen_only(
    snapshot: &AcceptedStartSnapshot,
) -> Result<(), PrivateSceneCoordinatorError> {
    snapshot
        .presets()
        .iter()
        .all(|preset| matches!(preset.composition(), SceneComposition::ScreenOnly))
        .then_some(())
        .ok_or_else(|| PrivateSceneCoordinatorError {
            detail: "private live scene coordination accepts only Screen-only layouts".into(),
        })
}

fn journal(error: impl fmt::Display) -> PrivateSceneCoordinatorError {
    PrivateSceneCoordinatorError {
        detail: format!("private scene journal rejected the operation: {error}"),
    }
}

fn scene(error: record_core::SceneError) -> PrivateSceneCoordinatorError {
    PrivateSceneCoordinatorError {
        detail: format!("private scene projection rejected the operation: {error}"),
    }
}
