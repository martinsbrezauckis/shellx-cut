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

/// Bound real interleaved PCM to the screen interval after its first packet.
/// This never pads a missing packet or counts the packet offset twice.
#[cfg(any(all(target_os = "macos", feature = "capture-macos"), test))]
pub(super) fn bounded_audio_samples(
    raw_start_ms: u64,
    raw_end_ms: u64,
    first_packet_offset_ms: u64,
    sample_rate: u32,
    channels: u16,
    available_samples: usize,
) -> Result<usize, ()> {
    if sample_rate == 0 || channels == 0 || !available_samples.is_multiple_of(usize::from(channels))
    {
        return Err(());
    }
    let packet_start = raw_start_ms.checked_add(first_packet_offset_ms).ok_or(())?;
    let duration = raw_end_ms
        .checked_sub(packet_start)
        .filter(|value| *value > 0)
        .ok_or(())?;
    let frames = duration.checked_mul(u64::from(sample_rate)).ok_or(())? / 1_000;
    let available_frames = available_samples / usize::from(channels);
    let frames = usize::try_from(frames)
        .map_err(|_| ())?
        .min(available_frames);
    let samples = frames
        .checked_mul(usize::from(channels))
        .filter(|value| *value > 0)
        .ok_or(())?;
    Ok(samples)
}

#[cfg(test)]
mod boundary_tests {
    use super::bounded_audio_samples;

    #[test]
    fn trims_the_retained_native_publication_tail_and_preserves_packet_offset() {
        // Actual r12: 603648 stereo frames / 48000 = 12576ms, screen 15..10321.
        let raw_samples = 603_648 * 2;
        assert_eq!(
            bounded_audio_samples(15, 10_321, 0, 48_000, 2, raw_samples),
            Ok(494_688 * 2)
        );
        assert_eq!(
            bounded_audio_samples(15, 10_321, 37, 48_000, 2, raw_samples),
            Ok(492_912 * 2)
        );
    }

    #[test]
    fn never_pads_short_audio_and_keeps_complete_multichannel_frames() {
        assert_eq!(bounded_audio_samples(0, 1000, 20, 48_000, 2, 10), Ok(10));
        assert_eq!(bounded_audio_samples(0, 1000, 0, 44_100, 3, 12), Ok(12));
    }

    #[test]
    fn refuses_invalid_or_unrepresentable_packet_intervals() {
        for args in [
            (0, 0, 0, 48000, 2, 10),
            (0, 10, 10, 48000, 2, 10),
            (0, 10, 11, 48000, 2, 10),
            (u64::MAX, u64::MAX, 1, 48000, 2, 10),
            (0, u64::MAX, 0, 48000, 2, 10),
            (0, 1000, 0, 0, 2, 10),
            (0, 1000, 0, 48000, 0, 10),
            (0, 1000, 0, 48000, 2, 1),
            (0, 1000, 0, 48000, 2, 11),
        ] {
            assert!(bounded_audio_samples(args.0, args.1, args.2, args.3, args.4, args.5).is_err());
        }
    }
}
