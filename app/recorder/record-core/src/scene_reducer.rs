//! Pure transition and logical-media-time projection for accepted recorder scenes.

use crate::scene::{
    AcceptedStartSnapshot, PresetRevision, SceneError, SceneEvent, SceneEventKind, SceneId,
    SceneResult, SceneSnapshotRevision, TimerConfig,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimerPhase {
    Off,
    Ready,
    Running,
    Paused,
    TimeUp,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TimerReading {
    pub phase: TimerPhase,
    pub display_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TimerState {
    config: TimerConfig,
    phase: TimerPhase,
    elapsed_ms: u64,
    running_since_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SceneState {
    snapshot_revision: SceneSnapshotRevision,
    active_scene_id: SceneId,
    last_sequence: u64,
    last_logical_time_ms: Option<u64>,
    timer: TimerState,
}

impl SceneState {
    pub fn initial(snapshot: &AcceptedStartSnapshot) -> SceneResult<Self> {
        let config = snapshot.timer_config()?;
        Ok(Self {
            snapshot_revision: snapshot.revision(),
            active_scene_id: snapshot.initial_scene_id().clone(),
            last_sequence: 0,
            last_logical_time_ms: None,
            timer: TimerState {
                config,
                phase: if matches!(config, TimerConfig::Off) {
                    TimerPhase::Off
                } else {
                    TimerPhase::Ready
                },
                elapsed_ms: 0,
                running_since_ms: None,
            },
        })
    }

    pub fn active_scene_id(&self) -> &SceneId {
        &self.active_scene_id
    }

    pub(crate) fn snapshot_revision(&self) -> SceneSnapshotRevision {
        self.snapshot_revision
    }

    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub fn timer_at(&self, logical_media_time_ms: u64) -> SceneResult<TimerReading> {
        if self
            .last_logical_time_ms
            .is_some_and(|last| logical_media_time_ms < last)
        {
            return Err(SceneError::InvalidLogicalTime);
        }
        self.timer.read_at(logical_media_time_ms)
    }
}

impl TimerState {
    fn elapsed_at(&self, logical_media_time_ms: u64) -> SceneResult<u64> {
        match self.running_since_ms {
            Some(started) => self
                .elapsed_ms
                .checked_add(
                    logical_media_time_ms
                        .checked_sub(started)
                        .ok_or(SceneError::InvalidLogicalTime)?,
                )
                .ok_or(SceneError::TimerOverflow),
            None => Ok(self.elapsed_ms),
        }
    }

    fn read_at(&self, logical_media_time_ms: u64) -> SceneResult<TimerReading> {
        let elapsed_ms = self.elapsed_at(logical_media_time_ms)?;
        match self.config {
            TimerConfig::Off => Ok(TimerReading {
                phase: TimerPhase::Off,
                display_ms: None,
            }),
            TimerConfig::Elapsed => Ok(TimerReading {
                phase: self.phase,
                display_ms: Some(elapsed_ms),
            }),
            TimerConfig::Countdown { duration_ms }
                if self.phase != TimerPhase::Ended && elapsed_ms >= duration_ms.get() =>
            {
                Ok(TimerReading {
                    phase: TimerPhase::TimeUp,
                    display_ms: Some(0),
                })
            }
            TimerConfig::Countdown { duration_ms } => Ok(TimerReading {
                phase: self.phase,
                display_ms: Some(duration_ms.get().saturating_sub(elapsed_ms)),
            }),
        }
    }
}

pub fn reduce_scene_event(
    snapshot: &AcceptedStartSnapshot,
    state: &SceneState,
    event: &SceneEvent,
) -> SceneResult<SceneState> {
    if state.snapshot_revision != snapshot.revision()
        || event.snapshot_revision != snapshot.revision()
    {
        return Err(SceneError::StaleSnapshot);
    }
    if event.sequence
        != state
            .last_sequence
            .checked_add(1)
            .ok_or(SceneError::InvalidSequence)?
    {
        return Err(SceneError::InvalidSequence);
    }
    if state
        .last_logical_time_ms
        .is_some_and(|last| event.logical_media_time_ms < last)
    {
        return Err(SceneError::InvalidLogicalTime);
    }

    let mut next = state.clone();
    match &event.kind {
        SceneEventKind::ActivateScene {
            scene_id,
            preset_revision,
        } => {
            activate_scene(&mut next, snapshot, scene_id, *preset_revision)?;
        }
        kind => apply_timer_transition(&mut next, event.logical_media_time_ms, kind)?,
    }
    next.last_sequence = event.sequence;
    next.last_logical_time_ms = Some(event.logical_media_time_ms);
    Ok(next)
}

fn activate_scene(
    state: &mut SceneState,
    snapshot: &AcceptedStartSnapshot,
    scene_id: &SceneId,
    preset_revision: PresetRevision,
) -> SceneResult<()> {
    let preset = snapshot
        .presets()
        .iter()
        .find(|preset| preset.id() == scene_id)
        .ok_or(SceneError::SceneMissing)?;
    if preset.revision() != preset_revision {
        return Err(SceneError::SceneRevisionMismatch);
    }
    if state.active_scene_id == *scene_id {
        return Err(SceneError::SceneAlreadyActive);
    }
    state.active_scene_id = scene_id.clone();
    Ok(())
}

fn apply_timer_transition(
    state: &mut SceneState,
    logical_media_time_ms: u64,
    kind: &SceneEventKind,
) -> SceneResult<()> {
    if matches!(state.timer.config, TimerConfig::Off) {
        return Err(SceneError::TimerDisabled);
    }
    let reading = state.timer.read_at(logical_media_time_ms)?;
    let elapsed_ms = state.timer.elapsed_at(logical_media_time_ms)?;
    match kind {
        SceneEventKind::TimerStart if reading.phase == TimerPhase::Ready => {
            state.timer.phase = TimerPhase::Running;
            state.timer.running_since_ms = Some(logical_media_time_ms);
        }
        SceneEventKind::TimerPause if reading.phase == TimerPhase::Running => {
            state.timer.elapsed_ms = elapsed_ms;
            state.timer.phase = TimerPhase::Paused;
            state.timer.running_since_ms = None;
        }
        SceneEventKind::TimerResume if reading.phase == TimerPhase::Paused => {
            state.timer.phase = TimerPhase::Running;
            state.timer.running_since_ms = Some(logical_media_time_ms);
        }
        SceneEventKind::TimerReset if reading.phase != TimerPhase::Off => {
            state.timer.elapsed_ms = 0;
            state.timer.phase = TimerPhase::Ready;
            state.timer.running_since_ms = None;
        }
        SceneEventKind::TimerEnd
            if matches!(
                reading.phase,
                TimerPhase::Running | TimerPhase::Paused | TimerPhase::TimeUp
            ) =>
        {
            state.timer.elapsed_ms = elapsed_ms;
            state.timer.phase = TimerPhase::Ended;
            state.timer.running_since_ms = None;
        }
        _ => return Err(SceneError::InvalidTimerTransition),
    }
    Ok(())
}

pub fn replay_scene_events(
    snapshot: &AcceptedStartSnapshot,
    events: &[SceneEvent],
) -> SceneResult<SceneState> {
    events
        .iter()
        .try_fold(SceneState::initial(snapshot)?, |state, event| {
            reduce_scene_event(snapshot, &state, event)
        })
}
