//! Selected macOS audio ownership for one sealed ScreenCaptureKit run.

use super::{
    MacosPauseAudioLayout, MacosPausePilotProfile, MacosPausePilotStarted, MacosSealedAudioRun,
};
use record_recovery::RecordingStream;

/// Starts exactly the optional audio streams admitted for one native screen
/// generation. It must not silently omit a selected stream.
pub(crate) trait MacosPauseAudioFactory {
    fn start(
        &mut self,
        profile: &MacosPausePilotProfile,
        started: &MacosPausePilotStarted,
    ) -> Result<Vec<Box<dyn MacosPauseAudioOwner>>, ()>;
}

/// Owns one Mic or Core Audio leaf until ScreenCaptureKit has closed and
/// published its paired screen run.
pub(crate) trait MacosPauseAudioOwner {
    fn stream(&self) -> RecordingStream;

    /// Stop, join/reap, flush, publish without replacement, and re-open the
    /// exact WAV. This method must complete terminal native cleanup on both
    /// success and error; `screen_raw_end_ms` is supplied only after the
    /// screen run has sealed, preventing a selected input from acknowledging
    /// a fabricated tail.
    fn seal_after_screen(
        self: Box<Self>,
        screen_raw_end_ms: u64,
    ) -> Result<MacosSealedAudioRun, ()>;

    /// Terminal cleanup for an active but unpublished sidecar. Implementations
    /// must stop their native producer and join/reap it before returning.
    fn abort_and_join(self: Box<Self>);
}

pub(crate) fn start_for_profile(
    profile: &MacosPausePilotProfile,
    factory: &mut dyn MacosPauseAudioFactory,
    started: &MacosPausePilotStarted,
) -> Result<Vec<Box<dyn MacosPauseAudioOwner>>, ()> {
    let expected = profile.selected_audio_streams();
    let owners = factory.start(profile, started)?;
    if owners.len() != expected.len()
        || owners
            .iter()
            .map(|owner| owner.stream())
            .collect::<Vec<_>>()
            != expected
    {
        abort_selected(owners);
        return Err(());
    }
    Ok(owners)
}

pub(crate) fn seal_selected_after_screen(
    owners: Vec<Box<dyn MacosPauseAudioOwner>>,
    expected: &[RecordingStream],
    screen_raw_end_ms: u64,
) -> Result<Vec<MacosSealedAudioRun>, ()> {
    let mut owners = owners.into_iter();
    let mut sealed = Vec::with_capacity(expected.len());
    while let Some(owner) = owners.next() {
        let declared_stream = owner.stream();
        let item = match owner.seal_after_screen(screen_raw_end_ms) {
            Ok(item) => item,
            Err(()) => {
                abort_selected(owners);
                return Err(());
            }
        };
        if item.stream != declared_stream {
            abort_selected(owners);
            return Err(());
        }
        sealed.push(item);
    }
    sealed.sort_by_key(|item| item.stream);
    if sealed.len() != expected.len()
        || sealed.iter().map(|item| item.stream).collect::<Vec<_>>() != expected
        || sealed.iter().any(|item| {
            item.layout != MacosPauseAudioLayout::PacketStart
                || item.source_generation == 0
                || item.artifact.is_empty()
                || item.bytes == 0
                || item.sha256.len() != 64
                || item.media_duration_ms == 0
                || item.native_ready_unix_ms == 0
                || item.raw_end_ms != screen_raw_end_ms
                || item.raw_end_ms <= item.raw_start_ms
                || item.native_ready_raw_ms < item.raw_start_ms
                || item.native_ready_raw_ms > item.raw_end_ms
                || item.media_duration_ms > item.raw_end_ms - item.raw_start_ms
        })
    {
        return Err(());
    }
    Ok(sealed)
}

pub(crate) fn abort_selected(owners: impl IntoIterator<Item = Box<dyn MacosPauseAudioOwner>>) {
    for owner in owners {
        owner.abort_and_join();
    }
}
