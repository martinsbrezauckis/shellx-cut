//! Immutable stream registration for pause/resume coordination.

use record_recovery::RecordingStream;
use serde::Serialize;

/// The immutable, canonical capture-stream selection for one session.
///
/// `ScreenVideo` is mandatory. The remaining durable stream identities are opt-in
/// at registration time and come directly from `record_recovery` so coordination,
/// recovery, and future journal entries use one taxonomy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelectedCaptureStreams {
    streams: Vec<RecordingStream>,
}

impl SelectedCaptureStreams {
    /// Register screen plus the explicitly selected optional streams.
    pub fn new(microphone: bool, system_audio: bool, camera: bool, input: bool) -> Self {
        let mut streams = vec![RecordingStream::ScreenVideo];
        if microphone {
            streams.push(RecordingStream::MicrophoneAudio);
        }
        if system_audio {
            streams.push(RecordingStream::SystemAudio);
        }
        if camera {
            streams.push(RecordingStream::CameraVideo);
        }
        if input {
            streams.push(RecordingStream::InputEvents);
        }
        streams.sort_unstable();
        Self { streams }
    }

    /// Register a screen-only session.
    pub fn screen_only() -> Self {
        Self::new(false, false, false, false)
    }

    /// Return the canonical, immutable stream order used for every boundary.
    pub fn streams(&self) -> &[RecordingStream] {
        &self.streams
    }

    pub(crate) fn contains(&self, stream: RecordingStream) -> bool {
        self.streams.contains(&stream)
    }
}
