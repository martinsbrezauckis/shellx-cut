//! Readiness of the exact video writer, separate from screen-frame delivery.

use record_core::{RecordError, Result};
use serde::Serialize;

pub const VIDEO_ENCODER_UNAVAILABLE: &str = "video_encoder_unavailable";
pub const VIDEO_ENCODER_MESSAGE: &str =
    "This PC cannot encode a screen recording at this size and frame rate.";
pub const VIDEO_ENCODER_ACTION: &str =
    "Try a smaller display or lower frame rate, or update the graphics driver before recording again.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoderPurpose {
    Doctor,
    RecordingStart,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoEncoderRequest {
    pub fps: f64,
    pub monitor: Option<u32>,
    pub monitor_id: Option<String>,
    pub window: Option<String>,
}

impl Default for VideoEncoderRequest {
    fn default() -> Self {
        Self {
            fps: 30.0,
            monitor: None,
            monitor_id: None,
            window: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VideoEncoderReadiness {
    pub checked: bool,
    pub ready: bool,
    pub reason: Option<String>,
    pub message: String,
    pub detail: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: u32,
}

impl VideoEncoderReadiness {
    #[cfg(any(test, all(windows, feature = "capture-windows")))]
    pub(crate) fn unchecked(fps: u32) -> Self {
        Self {
            checked: false,
            ready: false,
            reason: Some("video_encoder_not_checked".into()),
            message: "Video recording has not been checked for this source and frame rate.".into(),
            detail: None,
            width: None,
            height: None,
            fps,
        }
    }

    #[cfg(all(windows, feature = "capture-windows"))]
    pub(crate) fn minimized_window(fps: u32) -> Self {
        let mut result = Self::unchecked(fps);
        result.reason = Some("window_minimized".into());
        result.message = "The window is minimized. Restore it, then check again.".into();
        result
    }

    #[cfg(any(test, all(windows, feature = "capture-windows")))]
    pub(crate) fn from_setup(
        width: u32,
        height: u32,
        fps: u32,
        setup: std::result::Result<(), String>,
    ) -> Self {
        let (ready, reason, message, detail) = match setup {
            Ok(()) => (
                true,
                None,
                "Video recording is available at this size and frame rate.".into(),
                None,
            ),
            Err(detail) => (
                false,
                Some(VIDEO_ENCODER_UNAVAILABLE.into()),
                VIDEO_ENCODER_MESSAGE.into(),
                Some(detail),
            ),
        };
        Self {
            checked: true,
            ready,
            reason,
            message,
            detail,
            width: Some(width),
            height: Some(height),
            fps,
        }
    }

    pub fn ensure_ready(&self) -> Result<()> {
        if self.ready {
            return Ok(());
        }
        if self.reason.as_deref() == Some("video_encoder_source_unavailable") {
            return Err(RecordError::new(
                "capture",
                &self.message,
                self.detail.as_deref().unwrap_or(&self.message),
            )
            .with_action(
                "Choose a current display or window, then check the recording setup again.",
            ));
        }
        Err(encoder_error(
            self.detail.as_deref().unwrap_or(&self.message),
        ))
    }
}

pub fn encoder_error(detail: impl Into<String>) -> RecordError {
    RecordError::new(VIDEO_ENCODER_UNAVAILABLE, VIDEO_ENCODER_MESSAGE, detail)
        .with_action(VIDEO_ENCODER_ACTION)
}

/// Windows prepares its actual native writer. Other platforms retain their
/// existing readiness and never acquire a Windows encoder requirement.
pub fn video_encoder_readiness(
    request: VideoEncoderRequest,
    check: bool,
    purpose: VideoEncoderPurpose,
) -> Option<VideoEncoderReadiness> {
    #[cfg(all(windows, feature = "capture-windows"))]
    {
        Some(crate::windows_encoder_probe::readiness(
            request, check, purpose,
        ))
    }
    #[cfg(not(all(windows, feature = "capture-windows")))]
    {
        let _ = (request, check, purpose);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_decision_preserves_ready_and_reports_native_refusal_without_hiding_it() {
        for (width, height, fps) in [(1920, 1080, 30), (2560, 1440, 60)] {
            let usable = VideoEncoderReadiness::from_setup(width, height, fps, Ok(()));
            assert!(usable.ready && usable.checked);
            assert!(usable.ensure_ready().is_ok());
            assert!(usable.reason.is_none() && usable.detail.is_none());
            let raw = "A valid type has not been set (0xC00D6D60)";
            let unavailable =
                VideoEncoderReadiness::from_setup(width, height, fps, Err(raw.into()));
            assert!(!unavailable.ready && unavailable.checked);
            assert_eq!(
                unavailable.reason.as_deref(),
                Some(VIDEO_ENCODER_UNAVAILABLE)
            );
            assert_eq!(unavailable.message, VIDEO_ENCODER_MESSAGE);
            assert_eq!(unavailable.detail.as_deref(), Some(raw));
            let error = unavailable.ensure_ready().unwrap_err();
            assert_eq!(error.code, VIDEO_ENCODER_UNAVAILABLE);
            assert_eq!(error.message, unavailable.message);
            assert_eq!(error.cause, raw);
        }
    }

    #[test]
    fn an_unchecked_encoder_is_never_ready() {
        let status = VideoEncoderReadiness::unchecked(60);
        assert!(!status.checked && !status.ready);
        assert!(status.ensure_ready().is_err());
    }
}
