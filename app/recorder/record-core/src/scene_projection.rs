//! Pure deterministic scene fixture projection.
//!
//! This deliberately describes the accepted scene at a caller-supplied logical
//! media time. It does not open a camera, drive a device, mutate a UI, or make a
//! live scene switch; a renderer can only consume the resulting descriptor.

use serde::{Deserialize, Serialize};

use crate::{
    AcceptedStartSnapshot, SceneComposition, SceneError, SceneId, SceneResult, SceneState,
    TimerConfig, TimerPhase,
};

pub use crate::scene_editable_timeline::{
    EditableSceneTimeline, SceneCameraSegment, SceneScreenSegment, SceneTimerSegment,
    EDITABLE_SCENE_TIMELINE_SCHEMA,
};

/// A render-ready fixture description for one accepted recorder-scene state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneFixtureDescriptor {
    pub active_scene_id: SceneId,
    pub composition: SceneComposition,
    pub timer: SceneTimerFixture,
}

/// The timer reading that accompanies a fixture descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneTimerFixture {
    Off,
    Elapsed {
        phase: TimerPhase,
        elapsed_ms: u64,
    },
    Countdown {
        phase: TimerPhase,
        remaining_ms: u64,
    },
}

/// Stateless fixture renderer for the immutable scene snapshot and reducer state.
#[derive(Debug, Default, Clone, Copy)]
pub struct SceneFixtureRenderer;

impl SceneFixtureRenderer {
    pub fn render(
        snapshot: &AcceptedStartSnapshot,
        state: &SceneState,
        logical_media_time_ms: u64,
    ) -> SceneResult<SceneFixtureDescriptor> {
        project_scene_fixture(snapshot, state, logical_media_time_ms)
    }
}

/// Project an accepted scene without consulting wall time or any native source.
pub fn project_scene_fixture(
    snapshot: &AcceptedStartSnapshot,
    state: &SceneState,
    logical_media_time_ms: u64,
) -> SceneResult<SceneFixtureDescriptor> {
    if state.snapshot_revision() != snapshot.revision() {
        return Err(SceneError::StaleSnapshot);
    }
    let preset = snapshot
        .presets()
        .iter()
        .find(|preset| preset.id() == state.active_scene_id())
        .ok_or(SceneError::SceneMissing)?;
    let reading = state.timer_at(logical_media_time_ms)?;
    let timer = match snapshot.timer_config()? {
        TimerConfig::Off => SceneTimerFixture::Off,
        TimerConfig::Elapsed => SceneTimerFixture::Elapsed {
            phase: reading.phase,
            elapsed_ms: reading
                .display_ms
                .ok_or(SceneError::InvalidScenePersistence)?,
        },
        TimerConfig::Countdown { .. } => SceneTimerFixture::Countdown {
            phase: reading.phase,
            remaining_ms: reading
                .display_ms
                .ok_or(SceneError::InvalidScenePersistence)?,
        },
    };
    Ok(SceneFixtureDescriptor {
        active_scene_id: preset.id().clone(),
        composition: preset.composition(),
        timer,
    })
}
