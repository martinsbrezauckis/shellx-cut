//! Validated immutable public Recording Scenes replay handoff.

use record_core::scene_projection::{
    SceneFixtureDescriptor, SceneFixtureRenderer, SceneTimerFixture,
};
use record_core::{replay_scene_events, AcceptedStartSnapshot, SceneEvent};
use serde::{Deserialize, Serialize};

use crate::recording_scenes_engine::{
    require_public_snapshot, RecordingSceneEngineError, RECORDING_SCENE_PROJECTION_SCHEMA,
};

/// Complete immutable validated replay for a later projection worker. It
/// carries no mutable journal location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordingSceneProjection {
    schema: String,
    logical_media_time_ms: u64,
    journal_sha256: String,
    snapshot: AcceptedStartSnapshot,
    events: Vec<SceneEvent>,
    scene: SceneFixtureDescriptor,
}

impl RecordingSceneProjection {
    pub(crate) fn from_validated_replay(
        logical_media_time_ms: u64,
        journal_sha256: String,
        snapshot: AcceptedStartSnapshot,
        events: Vec<SceneEvent>,
        scene: SceneFixtureDescriptor,
    ) -> Self {
        Self {
            schema: RECORDING_SCENE_PROJECTION_SCHEMA.into(),
            logical_media_time_ms,
            journal_sha256,
            snapshot,
            events,
            scene,
        }
    }

    pub fn schema(&self) -> &str {
        &self.schema
    }

    pub fn logical_media_time_ms(&self) -> u64 {
        self.logical_media_time_ms
    }

    pub fn journal_sha256(&self) -> &str {
        &self.journal_sha256
    }

    pub fn snapshot(&self) -> &AcceptedStartSnapshot {
        &self.snapshot
    }

    pub fn events(&self) -> &[SceneEvent] {
        &self.events
    }

    pub fn scene(&self) -> &SceneFixtureDescriptor {
        &self.scene
    }

    /// Revalidate untrusted receipt JSON before a projection/export consumer
    /// uses it. This replays the frozen snapshot/events and requires the
    /// supplied terminal fixture and journal digest shape to agree exactly.
    pub fn validate(&self) -> Result<(), RecordingSceneEngineError> {
        if self.schema != RECORDING_SCENE_PROJECTION_SCHEMA {
            return Err(RecordingSceneEngineError::invalid(
                "recording scene projection schema is unsupported",
            ));
        }
        if self.journal_sha256.len() != 64
            || !self
                .journal_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(RecordingSceneEngineError::invalid(
                "recording scene projection journal digest is invalid",
            ));
        }
        require_public_snapshot(&self.snapshot)?;
        let state = replay_scene_events(&self.snapshot, &self.events)
            .map_err(RecordingSceneEngineError::scene)?;
        let projected =
            SceneFixtureRenderer::render(&self.snapshot, &state, self.logical_media_time_ms)
                .map_err(RecordingSceneEngineError::scene)?;
        if projected != self.scene {
            return Err(RecordingSceneEngineError::invalid(
                "recording scene projection terminal fixture does not match replay",
            ));
        }
        if !terminal_timer(&projected.timer) {
            return Err(RecordingSceneEngineError::invalid(
                "recording scene projection is not terminal",
            ));
        }
        Ok(())
    }
}

pub(crate) fn terminal_timer(timer: &SceneTimerFixture) -> bool {
    matches!(
        timer,
        SceneTimerFixture::Off
            | SceneTimerFixture::Elapsed {
                phase: record_core::TimerPhase::Ended,
                ..
            }
            | SceneTimerFixture::Countdown {
                phase: record_core::TimerPhase::Ended,
                ..
            }
    )
}
