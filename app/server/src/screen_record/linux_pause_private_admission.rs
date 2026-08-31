//! Linux admission boundary for a future private pause-aware capture owner.
//!
//! Linux ordinary capture is deliberately *not* a pause owner. Its ScreenCast
//! session is created inside `record_capture::linux::LinuxCapture`, where the
//! portal chooses the source and owns consent. The present API exposes one
//! capture-wide stop flag, not an independently sealed/reopenable portal run.
//!
//! The optional audio paths are likewise capture-wide only: CPAL publishes one
//! `mic.wav`, while PipeWire system audio publishes one `system.wav` plus one
//! first-packet timing sidecar. Neither supplies the immutable, run-indexed
//! audio leaf and timing fact required by the pause projection. A private pause
//! path must therefore refuse them explicitly rather than silently dropping
//! them, reusing a capture-wide artifact, or synthesizing silence.
//!
//! This is intentionally a pure admission gate. It opens no portal, never
//! reads a restore token, creates no capture files, and is not wired to a verb
//! or the normal recording route. The shared pause coordinator and projection
//! remain unavailable to Linux until a native owner can retain one consented
//! ScreenCast session across exact sealed runs and provide matching no-replace
//! microphone/system-audio sidecars.

use record_capture::SelectedCaptureStreams;

/// A requested stream that has no current Linux pause-safe owner.
///
/// These values are private, bounded diagnostics. They intentionally exclude
/// portal node ids, restore tokens, device names, filesystem paths, and any
/// user-selected screen identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LinuxPauseUnavailableStream {
    /// The current portal capture does not expose a pause/resume lifecycle that
    /// can prove the same consented source survived a sealed boundary.
    ScreenVideo,
    /// CPAL's ordinary microphone capture produces one capture-wide leaf, not
    /// a sealed per-run sidecar that pause projection can bind to ScreenVideo.
    MicrophoneAudio,
    /// PipeWire system audio produces one capture-wide WAV and first-packet
    /// timing fact, not one immutable timing fact for every sealed screen run.
    SystemAudio,
}

/// Why a Linux private pause-owner admission was refused before native work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LinuxPausePrivateRefusal {
    /// Passive input has no Linux pause-run sidecar owner. Wayland's input
    /// route also has its own portal/evdev permission boundary.
    PassiveInput,
    /// Key content cannot be selected without the unavailable input run owner.
    KeyInput,
    /// A portal source is selected by the user, not by a stable window id.
    Window,
    /// Linux has no pause-safe portal crop owner.
    Region,
    /// The ordinary Linux cursor path is capture-wide, not a sealed run fact.
    Cursor,
    /// One exact selected stream cannot be sealed and projected safely.
    StreamUnavailable {
        selected_streams: SelectedCaptureStreams,
        stream: LinuxPauseUnavailableStream,
    },
}

/// Return the immutable selection a future Linux owner would have to honor.
///
/// This is deliberately kept next to refusal admission so a caller cannot use
/// a failed optional stream request as permission to substitute screen-only
/// capture. `SelectedCaptureStreams` supplies the shared coordinator's
/// canonical Screen -> Microphone -> System Audio order.
pub(super) fn selected_streams(microphone: bool, system_audio: bool) -> SelectedCaptureStreams {
    SelectedCaptureStreams::new(microphone, system_audio, false, false)
}

/// Refuse every profile that would need Linux pause/resume media ownership.
///
/// A future implementation may replace this with an admitted owner only after
/// all selected streams can return independently sealed evidence. It must
/// preserve the exact `selected_streams` value instead of narrowing an audio
/// request to screen-only, and must leave portal consent user-owned.
#[allow(clippy::too_many_arguments)]
pub(super) fn admit(
    microphone: bool,
    system_audio: bool,
    passive_input: bool,
    key_input: bool,
    has_window: bool,
    has_region: bool,
    capture_cursor: bool,
) -> Result<(), LinuxPausePrivateRefusal> {
    if passive_input {
        return Err(LinuxPausePrivateRefusal::PassiveInput);
    }
    if key_input {
        return Err(LinuxPausePrivateRefusal::KeyInput);
    }
    if has_window {
        return Err(LinuxPausePrivateRefusal::Window);
    }
    if has_region {
        return Err(LinuxPausePrivateRefusal::Region);
    }
    if capture_cursor {
        return Err(LinuxPausePrivateRefusal::Cursor);
    }

    let selected_streams = selected_streams(microphone, system_audio);
    // Check requested optional streams first. This reports the actual missing
    // owner rather than pretending the screen-only portal path accepted an
    // audio request. Screen remains unavailable even with no audio selected.
    let stream = if microphone {
        LinuxPauseUnavailableStream::MicrophoneAudio
    } else if system_audio {
        LinuxPauseUnavailableStream::SystemAudio
    } else {
        LinuxPauseUnavailableStream::ScreenVideo
    };
    Err(LinuxPausePrivateRefusal::StreamUnavailable {
        selected_streams,
        stream,
    })
}

#[cfg(test)]
mod tests {
    use super::{admit, selected_streams, LinuxPausePrivateRefusal, LinuxPauseUnavailableStream};
    use record_capture::RecordingStream;

    fn unavailable(
        result: Result<(), LinuxPausePrivateRefusal>,
    ) -> (
        record_capture::SelectedCaptureStreams,
        LinuxPauseUnavailableStream,
    ) {
        match result {
            Err(LinuxPausePrivateRefusal::StreamUnavailable {
                selected_streams,
                stream,
            }) => (selected_streams, stream),
            other => panic!("expected a stream refusal, got {other:?}"),
        }
    }

    #[test]
    fn canonical_selection_keeps_screen_and_optional_audio_in_shared_order() {
        assert_eq!(
            selected_streams(true, true).streams(),
            &[
                RecordingStream::ScreenVideo,
                RecordingStream::MicrophoneAudio,
                RecordingStream::SystemAudio,
            ]
        );
        assert_eq!(
            selected_streams(false, true).streams(),
            &[RecordingStream::ScreenVideo, RecordingStream::SystemAudio]
        );
    }

    #[test]
    fn each_currently_unsealed_stream_is_explicitly_unavailable() {
        let (screen_selection, screen) =
            unavailable(admit(false, false, false, false, false, false, false));
        assert_eq!(screen, LinuxPauseUnavailableStream::ScreenVideo);
        assert_eq!(screen_selection.streams(), &[RecordingStream::ScreenVideo]);

        let (microphone_selection, microphone) =
            unavailable(admit(true, false, false, false, false, false, false));
        assert_eq!(microphone, LinuxPauseUnavailableStream::MicrophoneAudio);
        assert_eq!(
            microphone_selection.streams(),
            &[
                RecordingStream::ScreenVideo,
                RecordingStream::MicrophoneAudio
            ]
        );

        let (system_selection, system) =
            unavailable(admit(false, true, false, false, false, false, false));
        assert_eq!(system, LinuxPauseUnavailableStream::SystemAudio);
        assert_eq!(
            system_selection.streams(),
            &[RecordingStream::ScreenVideo, RecordingStream::SystemAudio]
        );
    }

    #[test]
    fn combined_audio_request_is_never_narrowed_to_screen_only() {
        let (selected, stream) = unavailable(admit(true, true, false, false, false, false, false));
        assert_eq!(stream, LinuxPauseUnavailableStream::MicrophoneAudio);
        assert_eq!(
            selected.streams(),
            &[
                RecordingStream::ScreenVideo,
                RecordingStream::MicrophoneAudio,
                RecordingStream::SystemAudio,
            ]
        );
    }

    #[test]
    fn unsupported_normal_streams_fail_before_pause_owner_admission() {
        assert_eq!(
            admit(false, false, true, false, false, false, false),
            Err(LinuxPausePrivateRefusal::PassiveInput)
        );
        assert_eq!(
            admit(false, false, false, true, false, false, false),
            Err(LinuxPausePrivateRefusal::KeyInput)
        );
        assert_eq!(
            admit(false, false, false, false, true, false, false),
            Err(LinuxPausePrivateRefusal::Window)
        );
        assert_eq!(
            admit(false, false, false, false, false, true, false),
            Err(LinuxPausePrivateRefusal::Region)
        );
        assert_eq!(
            admit(false, false, false, false, false, false, true),
            Err(LinuxPausePrivateRefusal::Cursor)
        );
    }
}
