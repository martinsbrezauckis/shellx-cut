//! Exact native screen-fact checks shared by owner transitions.

use super::{
    MacosPausePilotProfile, MacosPausePilotStarted, MacosPauseScreenRange, MacosSealedScreenRun,
};

pub(super) fn valid_started(
    profile: &MacosPausePilotProfile,
    started: &MacosPausePilotStarted,
) -> bool {
    let accepted = &started.accepted;
    started.exact_monitor_id == profile.exact_monitor_id()
        && started.physical_generation != 0
        && accepted.settings.fps == profile.fps() as f32
        && accepted.settings.width == accepted.range.width
        && accepted.settings.height == accepted.range.height
        && valid_range(accepted.range)
}

pub(super) fn valid_sealed_run(
    started: &MacosPausePilotStarted,
    run: &MacosSealedScreenRun,
) -> bool {
    let Some(first) = run.checkpoints.first() else {
        return false;
    };
    let Some(last) = run.checkpoints.last() else {
        return false;
    };
    run.exact_monitor_id == started.exact_monitor_id
        && run.physical_generation == started.physical_generation
        && run.observed_start_ms == started.observed_start_ms
        && run.observed_end_ms > run.observed_start_ms
        && run.accepted == started.accepted
        && valid_range(run.accepted.range)
        && first.facts.start_ms == run.observed_start_ms
        && last.facts.end_ms == run.observed_end_ms
        && run.checkpoints.iter().all(|checkpoint| {
            checkpoint.facts.end_ms > checkpoint.facts.start_ms
                && checkpoint.facts.event_offset_ms == checkpoint.facts.start_ms
                && checkpoint.facts.audio_offset_ms.is_none()
                && checkpoint.facts.start_ms >= run.observed_start_ms
                && checkpoint.facts.end_ms <= run.observed_end_ms
        })
        && run.checkpoints.windows(2).all(|pair| {
            pair[0]
                .sequence
                .checked_add(1)
                .is_some_and(|next| next == pair[1].sequence)
                && pair[0].facts.end_ms <= pair[1].facts.start_ms
        })
}

fn valid_range(range: MacosPauseScreenRange) -> bool {
    range.width != 0 && range.height != 0
}
