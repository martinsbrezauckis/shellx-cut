//! Physical microphone-WAV layout selection.

/// Whether a microphone WAV encodes its first-packet offset as physical
/// silence or carries that offset only as a separate native timing fact.
///
/// Ordinary one-shot capture retains the historic padded layout. The private
/// pause owner uses `PacketStart`: the shared projection already positions each
/// sealed sidecar from its timing fact and must never count the offset twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MicrophoneWavLayout {
    PaddedToCaptureClock,
    #[cfg_attr(
        not(all(target_os = "macos", feature = "capture-macos")),
        allow(dead_code, reason = "private macOS pause-sidecar capture layout")
    )]
    PacketStart,
}

pub(super) const fn pads_first_packet_offset(
    layout: MicrophoneWavLayout,
    first_packet_offset_ms: u64,
) -> bool {
    matches!(layout, MicrophoneWavLayout::PaddedToCaptureClock)
        && first_packet_offset_ms != u64::MAX
}
