//! Durable public scene owner built on the existing generic journal/reducer.

use std::fmt;

use record_core::scene_projection::{
    SceneFixtureDescriptor, SceneFixtureRenderer, SceneTimerFixture,
};
use record_core::{
    AcceptedStartSnapshot, PresetRevision, SceneComposition, SceneEvent, SceneEventKind, SceneId,
    TimerConfig,
};
use record_recovery::CaptureRoot;

use crate::recording_scenes_projection::{terminal_timer, RecordingSceneProjection};
use crate::scene_journal::SceneJournalOwner;

pub const RECORDING_SCENE_PROJECTION_SCHEMA: &str = "shellx-record/recording-scene-projection@1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingSceneEngineError {
    detail: String,
}

impl fmt::Display for RecordingSceneEngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for RecordingSceneEngineError {}

impl RecordingSceneEngineError {
    pub(crate) fn invalid(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }

    pub(crate) fn scene(error: record_core::SceneError) -> Self {
        scene(error)
    }
}

/// Bounded public timer transitions. A live reset/restart appends reset then
/// start at one CaptureClock timestamp so its acknowledged state is truthful
/// (`running`, never a reducer-internal unstarted Ready state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingSceneTimerAction {
    Pause,
    Resume,
    Reset,
    Restart,
    End,
}

#[derive(Debug)]
pub struct RecordingSceneEngine {
    journal: SceneJournalOwner,
}

impl RecordingSceneEngine {
    pub fn create(
        root: &CaptureRoot,
        capture_id: &str,
        snapshot: AcceptedStartSnapshot,
    ) -> Result<Self, RecordingSceneEngineError> {
        require_public_snapshot(&snapshot)?;
        Ok(Self {
            journal: SceneJournalOwner::create_new(root, capture_id, snapshot).map_err(journal)?,
        })
    }

    /// Reopen verifies canonical header/events; only the existing exclusive
    /// owner torn-tail repair rule can change the file.
    pub fn reopen(root: &CaptureRoot, capture_id: &str) -> Result<Self, RecordingSceneEngineError> {
        let journal = SceneJournalOwner::open(root, capture_id).map_err(journal)?;
        require_public_snapshot(journal.replayed().snapshot())?;
        Ok(Self { journal })
    }

    pub fn start_at(
        &mut self,
        logical_media_time_ms: u64,
    ) -> Result<SceneFixtureDescriptor, RecordingSceneEngineError> {
        if self.journal.replayed().state().last_sequence() != 0 {
            return self.project_at(logical_media_time_ms);
        }
        if logical_media_time_ms != 0 {
            return Err(RecordingSceneEngineError {
                detail: "recording scene timer must start at CaptureClock origin".into(),
            });
        }
        if matches!(self.timer_config()?, TimerConfig::Off) {
            self.project_at(0)
        } else {
            self.append_at(0, SceneEventKind::TimerStart)
        }
    }

    pub fn activate_at(
        &mut self,
        logical_media_time_ms: u64,
        scene_id: SceneId,
        preset_revision: PresetRevision,
    ) -> Result<SceneFixtureDescriptor, RecordingSceneEngineError> {
        self.append_at(
            logical_media_time_ms,
            SceneEventKind::ActivateScene {
                scene_id,
                preset_revision,
            },
        )
    }

    /// Check frozen composition before caller appends an activation event.
    pub fn activation_requires_camera(
        &self,
        scene_id: &SceneId,
        preset_revision: PresetRevision,
    ) -> Result<bool, RecordingSceneEngineError> {
        let preset = self
            .journal
            .replayed()
            .snapshot()
            .preset(scene_id, preset_revision)
            .ok_or_else(|| RecordingSceneEngineError {
                detail: "recording scene id or preset revision is not in the frozen catalog".into(),
            })?;
        Ok(matches!(
            preset.composition(),
            SceneComposition::PresenterPip(_)
        ))
    }

    pub fn timer_action_at(
        &mut self,
        logical_media_time_ms: u64,
        action: RecordingSceneTimerAction,
    ) -> Result<SceneFixtureDescriptor, RecordingSceneEngineError> {
        match action {
            RecordingSceneTimerAction::Pause => {
                self.append_at(logical_media_time_ms, SceneEventKind::TimerPause)
            }
            RecordingSceneTimerAction::Resume => {
                self.append_at(logical_media_time_ms, SceneEventKind::TimerResume)
            }
            RecordingSceneTimerAction::Reset => {
                self.append_at(logical_media_time_ms, SceneEventKind::TimerReset)?;
                self.append_at(logical_media_time_ms, SceneEventKind::TimerStart)
            }
            RecordingSceneTimerAction::Restart => {
                self.append_at(logical_media_time_ms, SceneEventKind::TimerReset)?;
                self.append_at(logical_media_time_ms, SceneEventKind::TimerStart)
            }
            RecordingSceneTimerAction::End => {
                self.append_at(logical_media_time_ms, SceneEventKind::TimerEnd)
            }
        }
    }

    pub fn project_at(
        &self,
        logical_media_time_ms: u64,
    ) -> Result<SceneFixtureDescriptor, RecordingSceneEngineError> {
        SceneFixtureRenderer::render(
            self.journal.replayed().snapshot(),
            self.journal.replayed().state(),
            logical_media_time_ms,
        )
        .map_err(scene)
    }

    pub fn finish_at(
        &mut self,
        logical_media_time_ms: u64,
    ) -> Result<SceneFixtureDescriptor, RecordingSceneEngineError> {
        let projection = self.project_at(logical_media_time_ms)?;
        if matches!(
            projection.timer,
            SceneTimerFixture::Elapsed {
                phase: record_core::TimerPhase::Running
                    | record_core::TimerPhase::Paused
                    | record_core::TimerPhase::TimeUp,
                ..
            } | SceneTimerFixture::Countdown {
                phase: record_core::TimerPhase::Running
                    | record_core::TimerPhase::Paused
                    | record_core::TimerPhase::TimeUp,
                ..
            }
        ) {
            self.append_at(logical_media_time_ms, SceneEventKind::TimerEnd)
        } else {
            Ok(projection)
        }
    }

    pub fn completed_projection_at(
        &self,
        logical_media_time_ms: u64,
    ) -> Result<RecordingSceneProjection, RecordingSceneEngineError> {
        let scene = self.project_at(logical_media_time_ms)?;
        if !terminal_timer(&scene.timer) {
            return Err(RecordingSceneEngineError {
                detail: "recording scene projection requires a terminal timer state".into(),
            });
        }
        Ok(RecordingSceneProjection::from_validated_replay(
            logical_media_time_ms,
            self.journal.durable_sha256().map_err(journal)?,
            self.journal.replayed().snapshot().clone(),
            self.journal.replayed().events().to_vec(),
            scene,
        ))
    }

    fn append_at(
        &mut self,
        logical_media_time_ms: u64,
        kind: SceneEventKind,
    ) -> Result<SceneFixtureDescriptor, RecordingSceneEngineError> {
        let replay = self.journal.replayed();
        let sequence = replay
            .state()
            .last_sequence()
            .checked_add(1)
            .ok_or_else(|| RecordingSceneEngineError {
                detail: "recording scene event sequence is exhausted".into(),
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

    fn timer_config(&self) -> Result<TimerConfig, RecordingSceneEngineError> {
        self.journal
            .replayed()
            .snapshot()
            .timer_config()
            .map_err(scene)
    }
}

pub(crate) fn require_public_snapshot(
    snapshot: &AcceptedStartSnapshot,
) -> Result<(), RecordingSceneEngineError> {
    let timer = snapshot.timer_config().map_err(scene)?;
    if snapshot
        .presets()
        .iter()
        .any(|preset| preset.timer() != timer)
    {
        return Err(RecordingSceneEngineError {
            detail: "public recording scenes require one capture-wide timer".into(),
        });
    }
    if snapshot.presets().iter().any(|preset| {
        !matches!(
            preset.composition(),
            SceneComposition::ScreenOnly | SceneComposition::PresenterPip(_)
        )
    }) {
        return Err(RecordingSceneEngineError {
            detail: "recording scene layout is not representable by the public engine".into(),
        });
    }
    Ok(())
}

fn journal(error: impl fmt::Display) -> RecordingSceneEngineError {
    RecordingSceneEngineError {
        detail: format!("recording scene journal rejected the operation: {error}"),
    }
}

fn scene(error: record_core::SceneError) -> RecordingSceneEngineError {
    RecordingSceneEngineError {
        detail: format!("recording scene projection rejected the operation: {error}"),
    }
}
