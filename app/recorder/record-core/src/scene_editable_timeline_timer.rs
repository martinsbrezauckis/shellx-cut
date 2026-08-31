//! Logical timer labels and range validation for editable scene timelines.

use crate::scene_editable_timeline::{EditableSceneTimeline, SceneTimerSegment};
use crate::scene_projection::SceneTimerFixture;
use crate::{SceneError, SceneResult, TimerPhase};

impl EditableSceneTimeline {
    /// Resolve a timer label solely from a persisted logical export timestamp.
    pub fn timer_label_at(&self, t_ms: u64) -> SceneResult<Option<String>> {
        self.validate()?;
        if t_ms > self.logical_duration_ms {
            return Err(SceneError::InvalidLogicalTime);
        }
        if t_ms == self.logical_duration_ms {
            return Ok(timer_label(self.terminal.timer));
        }
        let segment = self
            .timer
            .iter()
            .find(|segment| segment.start_ms <= t_ms && t_ms < segment.end_ms)
            .ok_or(SceneError::InvalidScenePersistence)?;
        timer_label_within(segment, t_ms)
    }
}

fn timer_label_within(segment: &SceneTimerSegment, t_ms: u64) -> SceneResult<Option<String>> {
    let elapsed = t_ms
        .checked_sub(segment.start_ms)
        .ok_or(SceneError::InvalidLogicalTime)?;
    let fixture = match segment.at_start {
        SceneTimerFixture::Off => return Ok(None),
        SceneTimerFixture::Elapsed { phase, elapsed_ms } => SceneTimerFixture::Elapsed {
            phase,
            elapsed_ms: if phase == TimerPhase::Running {
                elapsed_ms.saturating_add(elapsed)
            } else {
                elapsed_ms
            },
        },
        SceneTimerFixture::Countdown {
            phase,
            remaining_ms,
        } => SceneTimerFixture::Countdown {
            phase,
            remaining_ms: if phase == TimerPhase::Running {
                remaining_ms.saturating_sub(elapsed)
            } else {
                remaining_ms
            },
        },
    };
    Ok(timer_label(fixture))
}

fn timer_label(timer: SceneTimerFixture) -> Option<String> {
    let milliseconds = match timer {
        SceneTimerFixture::Off => return None,
        SceneTimerFixture::Elapsed { elapsed_ms, .. } => elapsed_ms,
        SceneTimerFixture::Countdown { remaining_ms, .. } => remaining_ms,
    };
    let seconds = milliseconds / 1_000;
    Some(format!("{:02}:{:02}", seconds / 60, seconds % 60))
}

pub(super) fn valid_timer_range(segment: &SceneTimerSegment) -> bool {
    let duration = segment.end_ms - segment.start_ms;
    match (segment.at_start, segment.at_end) {
        (SceneTimerFixture::Off, SceneTimerFixture::Off) => true,
        (
            SceneTimerFixture::Elapsed {
                phase: TimerPhase::Running,
                elapsed_ms: start,
            },
            SceneTimerFixture::Elapsed {
                phase: TimerPhase::Running,
                elapsed_ms: end,
            },
        ) => start.checked_add(duration) == Some(end),
        (
            SceneTimerFixture::Countdown {
                phase: TimerPhase::Running,
                remaining_ms: start,
            },
            SceneTimerFixture::Countdown {
                phase: TimerPhase::Running,
                remaining_ms: end,
            },
        ) if start > duration => start - duration == end,
        (
            SceneTimerFixture::Countdown {
                phase: TimerPhase::Running,
                remaining_ms: start,
            },
            SceneTimerFixture::Countdown {
                phase: TimerPhase::TimeUp,
                remaining_ms: 0,
            },
        ) => start <= duration,
        (
            SceneTimerFixture::Elapsed {
                phase: start_phase,
                elapsed_ms: start,
            },
            SceneTimerFixture::Elapsed {
                phase: end_phase,
                elapsed_ms: end,
            },
        ) => start_phase == end_phase && start_phase != TimerPhase::Running && start == end,
        (
            SceneTimerFixture::Countdown {
                phase: start_phase,
                remaining_ms: start,
            },
            SceneTimerFixture::Countdown {
                phase: end_phase,
                remaining_ms: end,
            },
        ) => start_phase == end_phase && start_phase != TimerPhase::Running && start == end,
        _ => false,
    }
}
