//! Ordinary WGC checkpoint rollover within one unpaused logical run.

use std::time::Instant;

use super::logical_run::ActiveLogicalRun;
use super::worker_facts::pilot_started;
use super::WindowsPausePilotProfile;
use crate::windows_wgc_run::{
    WgcCheckpointPublisher, WgcControlFactory, WgcRunOwner, WgcStartObservation,
};

pub(super) fn rollover_checkpoint<T, F, P>(
    owner: &mut WgcRunOwner<T, F, P>,
    profile: &WindowsPausePilotProfile,
    logical_run: &mut Option<ActiveLogicalRun>,
    reserve_start_ms: &mut impl FnMut() -> u64,
    observe_started: &mut impl FnMut() -> record_core::Result<WgcStartObservation>,
    observe_boundary: &mut impl FnMut() -> (u64, Instant),
) -> Result<(), ()>
where
    F: WgcControlFactory<T>,
    P: WgcCheckpointPublisher,
{
    let reserved_start_ms = reserve_start_ms();
    let (sealed, next) = owner
        .rollover_checkpoint(|| observe_boundary().0, reserved_start_ms, observe_started)
        .map_err(|_| ())?;
    let sealed_end_ms = sealed.boundary.end_ms;
    let started = pilot_started(profile, next.clone())?;
    let logical = logical_run.as_mut().ok_or(())?;
    logical.append(sealed)?;
    if !logical.accepts(&started.accepted) || next.start_ms < sealed_end_ms {
        return Err(());
    }
    Ok(())
}
