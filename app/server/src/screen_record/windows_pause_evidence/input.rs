//! Native-only provenance carried from calibrated WGC evidence to sidecar sealing.

use record_capture::windows_pause_pilot::WindowsSealedAudioRun;
use record_recovery::RecordingStream;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordingInputDraft {
    display_id: String,
    display_origin_x: i32,
    display_origin_y: i32,
    display_width: u32,
    display_height: u32,
    output_width: u32,
    output_height: u32,
    fps: u32,
    native_ready_unix_ms: u64,
    native_ready_raw_ms: u64,
    raw_start_ms: u64,
    raw_end_ms: u64,
    audio: Vec<RecordingAudioDraft>,
}

impl RecordingInputDraft {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_native(
        display_id: String,
        display_origin_x: i32,
        display_origin_y: i32,
        display_width: u32,
        display_height: u32,
        output_width: u32,
        output_height: u32,
        fps: u32,
        native_ready_unix_ms: u64,
        native_ready_raw_ms: u64,
        raw_start_ms: u64,
        raw_end_ms: u64,
        audio: Vec<RecordingAudioDraft>,
    ) -> Self {
        Self {
            display_id,
            display_origin_x,
            display_origin_y,
            display_width,
            display_height,
            output_width,
            output_height,
            fps,
            native_ready_unix_ms,
            native_ready_raw_ms,
            raw_start_ms,
            raw_end_ms,
            audio,
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn test_screen_only(
        display_id: String,
        display_origin_x: i32,
        display_origin_y: i32,
        display_width: u32,
        display_height: u32,
        output_width: u32,
        output_height: u32,
        fps: u32,
        ready_unix_ms: u64,
        raw_start_ms: u64,
        raw_end_ms: u64,
    ) -> Self {
        Self::test_screen_only_with_native_ready(
            display_id,
            display_origin_x,
            display_origin_y,
            display_width,
            display_height,
            output_width,
            output_height,
            fps,
            ready_unix_ms,
            raw_start_ms,
            raw_end_ms,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn test_screen_only_with_native_ready(
        display_id: String,
        display_origin_x: i32,
        display_origin_y: i32,
        display_width: u32,
        display_height: u32,
        output_width: u32,
        output_height: u32,
        fps: u32,
        native_ready_unix_ms: u64,
        raw_start_ms: u64,
        raw_end_ms: u64,
    ) -> Self {
        Self::from_native(
            display_id,
            display_origin_x,
            display_origin_y,
            display_width,
            display_height,
            output_width,
            output_height,
            fps,
            native_ready_unix_ms,
            raw_start_ms,
            raw_start_ms,
            raw_end_ms,
            Vec::new(),
        )
    }

    pub(crate) fn display_id(&self) -> &str {
        &self.display_id
    }
    pub(crate) fn display_origin_x(&self) -> i32 {
        self.display_origin_x
    }
    pub(crate) fn display_origin_y(&self) -> i32 {
        self.display_origin_y
    }
    pub(crate) fn display_width(&self) -> u32 {
        self.display_width
    }
    pub(crate) fn display_height(&self) -> u32 {
        self.display_height
    }
    pub(crate) fn output_width(&self) -> u32 {
        self.output_width
    }
    pub(crate) fn output_height(&self) -> u32 {
        self.output_height
    }
    pub(crate) fn fps(&self) -> u32 {
        self.fps
    }
    pub(crate) fn native_ready_unix_ms(&self) -> u64 {
        self.native_ready_unix_ms
    }
    pub(crate) fn native_ready_raw_ms(&self) -> u64 {
        self.native_ready_raw_ms
    }
    pub(crate) fn raw_start_ms(&self) -> u64 {
        self.raw_start_ms
    }
    pub(crate) fn raw_end_ms(&self) -> u64 {
        self.raw_end_ms
    }
    pub(crate) fn audio(&self) -> &[RecordingAudioDraft] {
        &self.audio
    }

    #[cfg(test)]
    pub(crate) fn with_test_audio(mut self, audio: Vec<RecordingAudioDraft>) -> Self {
        self.audio = audio;
        self
    }
}

/// A sealed non-screen source from the same logical WGC run. The server keeps
/// this private until it re-verifies the exact local WAV and embeds it in the
/// journal-pinned no-replace sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecordingAudioDraft {
    stream: RecordingStream,
    source_generation: u64,
    artifact: String,
    bytes: u64,
    sha256: String,
    media_duration_ms: u64,
    native_ready_unix_ms: u64,
    native_ready_raw_ms: u64,
    raw_start_ms: u64,
    raw_end_ms: u64,
}

impl RecordingAudioDraft {
    pub(crate) fn from_native(value: &WindowsSealedAudioRun) -> Result<Self, ()> {
        if !matches!(
            value.stream,
            RecordingStream::MicrophoneAudio | RecordingStream::SystemAudio
        ) || value.source_generation == 0
            || value.artifact.is_empty()
            || value.bytes == 0
            || value.sha256.len() != 64
            || value.media_duration_ms == 0
            || value.native_ready_unix_ms == 0
            || value.raw_end_ms <= value.raw_start_ms
            || value.native_ready_raw_ms < value.raw_start_ms
            || value.native_ready_raw_ms > value.raw_end_ms
            || value.media_duration_ms > value.raw_end_ms - value.raw_start_ms
        {
            return Err(());
        }
        Ok(Self {
            stream: value.stream,
            source_generation: value.source_generation,
            artifact: value.artifact.clone(),
            bytes: value.bytes,
            sha256: value.sha256.clone(),
            media_duration_ms: value.media_duration_ms,
            native_ready_unix_ms: value.native_ready_unix_ms,
            native_ready_raw_ms: value.native_ready_raw_ms,
            raw_start_ms: value.raw_start_ms,
            raw_end_ms: value.raw_end_ms,
        })
    }

    pub(crate) fn stream(&self) -> RecordingStream {
        self.stream
    }
    pub(crate) fn source_generation(&self) -> u64 {
        self.source_generation
    }
    pub(crate) fn artifact(&self) -> &str {
        &self.artifact
    }
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }
    pub(crate) fn media_duration_ms(&self) -> u64 {
        self.media_duration_ms
    }
    pub(crate) fn native_ready_unix_ms(&self) -> u64 {
        self.native_ready_unix_ms
    }
    pub(crate) fn native_ready_raw_ms(&self) -> u64 {
        self.native_ready_raw_ms
    }
    pub(crate) fn raw_start_ms(&self) -> u64 {
        self.raw_start_ms
    }
    pub(crate) fn raw_end_ms(&self) -> u64 {
        self.raw_end_ms
    }

    #[cfg(test)]
    pub(crate) fn test_audio(
        stream: RecordingStream,
        source_generation: u64,
        artifact: String,
        bytes: u64,
        sha256: String,
        raw_start_ms: u64,
        raw_end_ms: u64,
    ) -> Self {
        Self {
            stream,
            source_generation,
            artifact,
            bytes,
            sha256,
            media_duration_ms: raw_end_ms - raw_start_ms,
            native_ready_unix_ms: 1_000,
            native_ready_raw_ms: raw_start_ms,
            raw_start_ms,
            raw_end_ms,
        }
    }
}
