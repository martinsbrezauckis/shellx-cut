//! Reservation-aware adapter for ordinary WGC physical checkpoints.

use std::path::Path;

use record_core::Result;

use crate::windows_wgc_run::{state_error, WgcControlFactory, WgcNativeControl, WgcStartedControl};

/// Only ordinary Windows capture uses this explicit reservation-aware adapter.
pub(crate) struct TimedWgcFactory<F>(pub(crate) F);

impl<T, F, C> WgcControlFactory<T> for TimedWgcFactory<F>
where
    F: FnMut(&T, &Path, u64) -> Result<WgcStartedControl<C>>,
    C: WgcNativeControl,
{
    type Control = C;

    fn start(&mut self, _target: &T, _staging: &Path) -> Result<WgcStartedControl<Self::Control>> {
        Err(state_error(
            "timed WGC factory requires its reserved boundary",
        ))
    }

    fn start_at(
        &mut self,
        target: &T,
        staging: &Path,
        reserved_start_ms: u64,
    ) -> Result<WgcStartedControl<Self::Control>> {
        self.0(target, staging, reserved_start_ms)
    }
}

impl<T, F, C> WgcControlFactory<T> for F
where
    F: FnMut(&T, &Path) -> Result<WgcStartedControl<C>>,
    C: WgcNativeControl,
{
    type Control = C;

    fn start(&mut self, target: &T, staging: &Path) -> Result<WgcStartedControl<Self::Control>> {
        self(target, staging)
    }
}
