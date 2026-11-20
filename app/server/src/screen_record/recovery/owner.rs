//! Who owns an unsealed capture, as seen from this engine.

use record_recovery::{owner_state, CaptureStart, OwnerState};

/// The recovery crate's process observation, with one addition only this
/// engine can make, used when the person asks to recover a capture: a capture
/// whose recorded owner is this very process, but which this process no
/// longer holds as an active recording, has been abandoned by its worker and
/// can be recovered now. (A failing worker seals its own capture; this covers
/// a capture whose seal could not be written.) The automatic scan at project
/// open keeps the plain observation.
pub(crate) fn effective_owner(start: &CaptureStart, capture_id: &str) -> OwnerState {
    let observed = owner_state(start);
    if observed == OwnerState::Alive
        && start.owner_pid == std::process::id()
        && !crate::screen_record::capture_registry::is_capture_active(capture_id)
    {
        return OwnerState::Dead;
    }
    observed
}
