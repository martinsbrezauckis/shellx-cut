//! Future per-logical-run ownership for selected passive input.
//!
//! Input is created only after WGC has accepted the exact target, and it is
//! sealed only after the corresponding WGC run has closed and published. A
//! failed native input close is therefore a failed run, never an empty input
//! acknowledgement. Production admission remains screen-video-only because the
//! current server translator rejects this payload until it has an immutable
//! input-sidecar verifier and matching journal fragment.

use super::{WindowsPausePilotProfile, WindowsPausePilotStarted, WindowsSealedInputRun};
#[cfg(all(windows, feature = "capture-windows"))]
use crate::input::InputListener;

/// Object-safe seam so worker tests can prove the WGC→input ordering without
/// opening a process-global native hook. Production always uses
/// [`RequiredWindowsPauseInputFactory`].
pub(crate) trait WindowsPauseInputFactory: Send {
    fn start(
        &mut self,
        started: &WindowsPausePilotStarted,
    ) -> Result<Box<dyn WindowsPauseInputOwner>, ()>;
}

/// One selected physical input run. Its seal is consumed exactly once.
pub(crate) trait WindowsPauseInputOwner: Send {
    fn seal_after_screen(
        self: Box<Self>,
        screen_duration_ms: u64,
    ) -> Result<WindowsSealedInputRun, ()>;
}

pub(crate) fn start_for_profile(
    profile: &WindowsPausePilotProfile,
    factory: &mut dyn WindowsPauseInputFactory,
    started: &WindowsPausePilotStarted,
) -> Result<Option<Box<dyn WindowsPauseInputOwner>>, ()> {
    profile
        .captures_input()
        .then(|| factory.start(started))
        .transpose()
}

/// The real private Windows owner. It refuses startup unless the native passive
/// hook exists, unlike the ordinary terminal capture path where input is
/// optional.
#[cfg(all(windows, feature = "capture-windows"))]
pub(crate) struct RequiredWindowsPauseInputFactory;

#[cfg(all(windows, feature = "capture-windows"))]
impl WindowsPauseInputFactory for RequiredWindowsPauseInputFactory {
    fn start(
        &mut self,
        started: &WindowsPausePilotStarted,
    ) -> Result<Box<dyn WindowsPauseInputOwner>, ()> {
        InputListener::start_required(started.monotonic_at, false)
            .map(|listener| Box::new(RequiredWindowsPauseInputOwner { listener }) as _)
            .map_err(|_| ())
    }
}

#[cfg(all(windows, feature = "capture-windows"))]
struct RequiredWindowsPauseInputOwner {
    listener: InputListener,
}

#[cfg(all(windows, feature = "capture-windows"))]
impl WindowsPauseInputOwner for RequiredWindowsPauseInputOwner {
    fn seal_after_screen(
        self: Box<Self>,
        screen_duration_ms: u64,
    ) -> Result<WindowsSealedInputRun, ()> {
        let (cursor, clicks, scrolls, keys) =
            self.listener.seal(screen_duration_ms).map_err(|_| ())?;
        Ok(WindowsSealedInputRun {
            cursor,
            clicks,
            scrolls,
            keys,
        })
    }
}

/// The screen-only profile must never instantiate a listener. Keeping this as
/// an explicit factory makes an accidental selected-input profile fail closed.
#[cfg(test)]
pub(crate) struct DisabledWindowsPauseInputFactory;

#[cfg(test)]
impl WindowsPauseInputFactory for DisabledWindowsPauseInputFactory {
    fn start(
        &mut self,
        _started: &WindowsPausePilotStarted,
    ) -> Result<Box<dyn WindowsPauseInputOwner>, ()> {
        Err(())
    }
}
