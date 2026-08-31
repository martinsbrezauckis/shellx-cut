//! Durable scene replay projected into three bounded editable tracks.

use serde::{Deserialize, Serialize};

use crate::scene_projection::{SceneFixtureDescriptor, SceneFixtureRenderer, SceneTimerFixture};
use crate::{
    reduce_scene_event, AcceptedStartSnapshot, PresenterPip, SceneComposition, SceneError,
    SceneEvent, SceneId, SceneResult, SceneState,
};

/// Schema for the bounded, editable scene tracks stored in an EditPlan.
pub const EDITABLE_SCENE_TIMELINE_SCHEMA: &str = "shellx-record/editable-scene-timeline@1";

/// One contiguous source-screen span. The screen source remains the existing
/// editable capture; scene changes never synthesize or split source media.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneScreenSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub scene_id: SceneId,
}

/// One contiguous camera presentation span. `presenter: None` is Screen-only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneCameraSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub presenter: Option<PresenterPip>,
}

/// A timer range contains both logical endpoints for reproducible export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneTimerSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub at_start: SceneTimerFixture,
    pub at_end: SceneTimerFixture,
}

/// The small, deterministic timeline projection of one sealed scene journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditableSceneTimeline {
    pub schema: String,
    pub logical_duration_ms: u64,
    pub journal_sha256: String,
    pub screen: Vec<SceneScreenSegment>,
    pub camera: Vec<SceneCameraSegment>,
    pub timer: Vec<SceneTimerSegment>,
    pub terminal: SceneFixtureDescriptor,
}

impl EditableSceneTimeline {
    /// Derive tracks from replay-validated events. Every boundary is logical
    /// media time, never an inferred keyframe or a wall-clock sample.
    pub fn from_replay(
        snapshot: &AcceptedStartSnapshot,
        events: &[SceneEvent],
        logical_duration_ms: u64,
        journal_sha256: impl Into<String>,
    ) -> SceneResult<Self> {
        let journal_sha256 = journal_sha256.into();
        if !is_sha256(&journal_sha256) {
            return Err(SceneError::InvalidScenePersistence);
        }
        let mut state = SceneState::initial(snapshot)?;
        let mut event_index = 0usize;
        let mut cursor = 0u64;
        let mut screen = Vec::new();
        let mut camera = Vec::new();
        let mut timer = Vec::new();
        while event_index < events.len() {
            let event = &events[event_index];
            if event.logical_media_time_ms > logical_duration_ms {
                return Err(SceneError::InvalidLogicalTime);
            }
            while event_index < events.len() && events[event_index].logical_media_time_ms == cursor
            {
                state = reduce_scene_event(snapshot, &state, &events[event_index])?;
                event_index += 1;
            }
            let next = events
                .get(event_index)
                .map(|next| next.logical_media_time_ms)
                .unwrap_or(logical_duration_ms);
            if next < cursor || next > logical_duration_ms {
                return Err(SceneError::InvalidLogicalTime);
            }
            if next > cursor {
                append_segment(
                    &mut screen,
                    &mut camera,
                    &mut timer,
                    cursor,
                    next,
                    &SceneFixtureRenderer::render(snapshot, &state, cursor)?,
                    &SceneFixtureRenderer::render(snapshot, &state, next)?,
                );
                cursor = next;
            } else if event_index == events.len() {
                break;
            }
        }
        while event_index < events.len() && events[event_index].logical_media_time_ms == cursor {
            state = reduce_scene_event(snapshot, &state, &events[event_index])?;
            event_index += 1;
        }
        if event_index != events.len() || cursor > logical_duration_ms {
            return Err(SceneError::InvalidLogicalTime);
        }
        if cursor < logical_duration_ms {
            append_segment(
                &mut screen,
                &mut camera,
                &mut timer,
                cursor,
                logical_duration_ms,
                &SceneFixtureRenderer::render(snapshot, &state, cursor)?,
                &SceneFixtureRenderer::render(snapshot, &state, logical_duration_ms)?,
            );
        }
        let timeline = Self {
            schema: EDITABLE_SCENE_TIMELINE_SCHEMA.into(),
            logical_duration_ms,
            journal_sha256,
            screen,
            camera,
            timer,
            terminal: SceneFixtureRenderer::render(snapshot, &state, logical_duration_ms)?,
        };
        timeline.validate()?;
        Ok(timeline)
    }

    /// Verify the standalone persisted form before it becomes part of a plan.
    pub fn validate(&self) -> SceneResult<()> {
        if self.schema != EDITABLE_SCENE_TIMELINE_SCHEMA || !is_sha256(&self.journal_sha256) {
            return Err(SceneError::InvalidScenePersistence);
        }
        if self.screen.len() != self.camera.len() || self.screen.len() != self.timer.len() {
            return Err(SceneError::InvalidScenePersistence);
        }
        let mut expected_start = 0u64;
        for ((screen, camera), timer) in self.screen.iter().zip(&self.camera).zip(&self.timer) {
            if screen.start_ms != expected_start
                || screen.end_ms <= screen.start_ms
                || camera.start_ms != screen.start_ms
                || camera.end_ms != screen.end_ms
                || timer.start_ms != screen.start_ms
                || timer.end_ms != screen.end_ms
                || !crate::scene_editable_timeline_timer::valid_timer_range(timer)
            {
                return Err(SceneError::InvalidScenePersistence);
            }
            expected_start = screen.end_ms;
        }
        (expected_start == self.logical_duration_ms)
            .then_some(())
            .ok_or(SceneError::InvalidScenePersistence)
    }
}

fn append_segment(
    screen: &mut Vec<SceneScreenSegment>,
    camera: &mut Vec<SceneCameraSegment>,
    timer: &mut Vec<SceneTimerSegment>,
    start_ms: u64,
    end_ms: u64,
    at_start: &SceneFixtureDescriptor,
    at_end: &SceneFixtureDescriptor,
) {
    screen.push(SceneScreenSegment {
        start_ms,
        end_ms,
        scene_id: at_start.active_scene_id.clone(),
    });
    camera.push(SceneCameraSegment {
        start_ms,
        end_ms,
        presenter: match at_start.composition {
            SceneComposition::ScreenOnly => None,
            SceneComposition::PresenterPip(presenter) => Some(presenter),
        },
    });
    timer.push(SceneTimerSegment {
        start_ms,
        end_ms,
        at_start: at_start.timer,
        at_end: at_end.timer,
    });
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
