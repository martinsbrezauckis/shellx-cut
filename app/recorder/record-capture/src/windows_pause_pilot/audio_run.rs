//! Required microphone and process-loopback ownership for one logical WGC run.
//!
//! The owner is deliberately created only after WGC accepted the exact target.
//! It never publishes an open WAV: a Pause or Stop first publishes the paired
//! WGC checkpoint, then asks every selected owner to stop, flush, join, and
//! re-open its immutable audio leaf before a native event can be emitted.

use super::{WindowsPausePilotProfile, WindowsPausePilotStarted, WindowsSealedAudioRun};
use record_recovery::RecordingStream;

#[cfg(all(windows, feature = "capture-windows"))]
mod native;
#[cfg(all(windows, feature = "capture-windows"))]
mod sealed_wav;

pub(crate) trait WindowsPauseAudioFactory: Send {
    fn start(
        &mut self,
        profile: &WindowsPausePilotProfile,
        started: &WindowsPausePilotStarted,
    ) -> Result<Vec<Box<dyn WindowsPauseAudioOwner>>, ()>;
}

pub(crate) trait WindowsPauseAudioOwner: Send {
    fn stream(&self) -> RecordingStream;

    /// Stop, flush, join, and prove the exact immutable source leaf. The
    /// screen raw endpoint is supplied only after WGC has sealed/published its
    /// run, so a sidecar can never acknowledge an audio tail past screen.
    fn seal_after_screen(
        self: Box<Self>,
        screen_raw_end_ms: u64,
    ) -> Result<WindowsSealedAudioRun, ()>;
}

pub(crate) fn start_for_profile(
    profile: &WindowsPausePilotProfile,
    factory: &mut dyn WindowsPauseAudioFactory,
    started: &WindowsPausePilotStarted,
) -> Result<Vec<Box<dyn WindowsPauseAudioOwner>>, ()> {
    let expected = profile.selected_audio_streams();
    let owners = factory.start(profile, started)?;
    if owners.len() != expected.len()
        || owners
            .iter()
            .map(|owner| owner.stream())
            .collect::<Vec<_>>()
            != expected
    {
        return Err(());
    }
    Ok(owners)
}

pub(crate) fn seal_selected_after_screen(
    owners: Vec<Box<dyn WindowsPauseAudioOwner>>,
    expected: &[RecordingStream],
    screen_raw_end_ms: u64,
) -> Result<Vec<WindowsSealedAudioRun>, ()> {
    let mut sealed = Vec::with_capacity(owners.len());
    for owner in owners {
        let declared_stream = owner.stream();
        let item = owner.seal_after_screen(screen_raw_end_ms)?;
        if item.stream != declared_stream {
            return Err(());
        }
        sealed.push(item);
    }
    sealed.sort_by_key(|item| item.stream);
    if sealed.len() != expected.len()
        || sealed.iter().map(|item| item.stream).collect::<Vec<_>>() != expected
        || sealed.iter().any(|item| {
            item.source_generation == 0
                || item.artifact.is_empty()
                || item.bytes == 0
                || item.sha256.len() != 64
                || item.native_ready_unix_ms == 0
                || item.raw_end_ms != screen_raw_end_ms
                || item.raw_end_ms <= item.raw_start_ms
                || item.native_ready_raw_ms < item.raw_start_ms
                || item.native_ready_raw_ms > item.raw_end_ms
                || item.media_duration_ms == 0
                || item.media_duration_ms > item.raw_end_ms - item.raw_start_ms
        })
    {
        return Err(());
    }
    Ok(sealed)
}

/// Screen-only production construction. A real selected-audio profile replaces
/// this with the Windows factory below; it cannot silently return an empty list.
pub(crate) struct DisabledWindowsPauseAudioFactory;

impl WindowsPauseAudioFactory for DisabledWindowsPauseAudioFactory {
    fn start(
        &mut self,
        profile: &WindowsPausePilotProfile,
        _started: &WindowsPausePilotStarted,
    ) -> Result<Vec<Box<dyn WindowsPauseAudioOwner>>, ()> {
        profile
            .selected_audio_streams()
            .is_empty()
            .then(Vec::new)
            .ok_or(())
    }
}

#[cfg(all(windows, feature = "capture-windows"))]
pub(crate) use native::RequiredWindowsPauseAudioFactory;
