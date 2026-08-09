//! replay.rs — `ReplayCapture`: a `Capture` backed by files on disk.
//!
//! Reads an existing source video + `EventTrack` JSON and presents them as a
//! capture result. This is (1) the deterministic test/CI path for the whole
//! pipeline on any platform, and (2) the real "import an existing screen
//! recording + its event log" entry point. No platform deps — builds everywhere.

use record_core::{error_codes, EventTrack, RecordError, Result, Settings};

use crate::{Capture, CaptureConfig, CaptureOutput};

/// A capture that replays a recorded track + video.
pub struct ReplayCapture {
    pub track_path: String,
    pub video_path: String,
}

impl ReplayCapture {
    pub fn new(track_path: impl Into<String>, video_path: impl Into<String>) -> Self {
        Self {
            track_path: track_path.into(),
            video_path: video_path.into(),
        }
    }
}

impl Capture for ReplayCapture {
    /// `stop` is accepted to satisfy the trait but IGNORED — a replay reads
    /// finished files off disk, so there is no live capture loop to interrupt.
    fn capture(
        &self,
        _cfg: &CaptureConfig,
        _stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<CaptureOutput> {
        let bytes = std::fs::read(&self.track_path)
            .map_err(|e| RecordError::new(error_codes::IO, "read event track", e.to_string()))?;
        let events: EventTrack = serde_json::from_slice(&bytes).map_err(|e| {
            RecordError::new(
                error_codes::INVALID_ARGS,
                "parse event track JSON",
                e.to_string(),
            )
            .with_action("pass a valid EventTrack JSON (see gen-fixture)")
        })?;
        let settings = Settings {
            width: events.screen_w,
            height: events.screen_h,
            fps: 30.0,
            audio_rate: 48_000,
        };
        Ok(CaptureOutput {
            source_video: self.video_path.clone(),
            events,
            webcam_video: None,
            audio: None,
            settings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_reads_track_and_builds_project() {
        // Write a fixture track to a temp file, replay it.
        let track = record_core::fixtures::generate("click-walkthrough").unwrap();
        let dir = std::env::temp_dir();
        let tp = dir.join("shellx_record_replay_test.track.json");
        std::fs::write(&tp, serde_json::to_vec(&track).unwrap()).unwrap();

        let cap = ReplayCapture::new(tp.to_string_lossy().to_string(), "fake.mp4");
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let out = cap.capture(&CaptureConfig::default(), stop).unwrap();
        assert_eq!(out.source_video, "fake.mp4");
        assert_eq!(out.events.screen_w, track.screen_w);
        let proj = out.into_project();
        assert_eq!(proj.source_video, "fake.mp4");
        assert_eq!(proj.events.duration_ms, track.duration_ms);

        let _ = std::fs::remove_file(&tp);
    }

    #[test]
    fn replay_bad_track_errors_with_cause() {
        let dir = std::env::temp_dir();
        let tp = dir.join("shellx_record_replay_bad.json");
        std::fs::write(&tp, b"not json").unwrap();
        let cap = ReplayCapture::new(tp.to_string_lossy().to_string(), "x.mp4");
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let err = cap.capture(&CaptureConfig::default(), stop).unwrap_err();
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(err.suggested_action.is_some());
        let _ = std::fs::remove_file(&tp);
    }
}
