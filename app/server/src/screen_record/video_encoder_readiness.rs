//! Selected-source encoder readiness, before any capture reservation or folder.

use super::doctor_projection::{ready_rollup, RecordCard, RecordDoctor, RecordStartAdmission};
use super::start_readiness;
use cut_core::CutError;
use record_capture::{VideoEncoderReadiness, VideoEncoderRequest};

#[derive(Default, serde::Deserialize)]
pub(super) struct DoctorArgs {
    #[serde(default)]
    pub warm_mic: bool,
    pub mode: Option<String>,
    #[serde(flatten)]
    pub encoder: Args,
}

#[derive(Default, serde::Deserialize)]
pub(super) struct Args {
    fps: Option<f64>,
    monitor: Option<u32>,
    monitor_id: Option<String>,
    window: Option<String>,
}

impl Args {
    pub(super) fn request(self) -> Result<VideoEncoderRequest, CutError> {
        let fps = self.fps.unwrap_or(30.0);
        super::validate_capture_settings(None, fps)?;
        if self.window.is_some() && self.monitor_id.is_some() {
            return Err(CutError::new(
                cut_core::error_codes::INVALID_ARGS,
                "Choose one screen or window for the recording check.",
                "window cannot be combined with monitor_id",
            ));
        }
        Ok(VideoEncoderRequest {
            fps,
            monitor: self.monitor,
            monitor_id: self.monitor_id,
            window: self.window,
        })
    }
}

pub(super) fn apply(
    doctor: &mut RecordDoctor,
    request: VideoEncoderRequest,
    check: bool,
) -> Option<VideoEncoderReadiness> {
    let encoder = record_capture::video_encoder_readiness(
        request,
        check,
        record_capture::VideoEncoderPurpose::Doctor,
    )?;
    let unchecked = encoder.reason.as_deref() == Some("video_encoder_not_checked");
    doctor.cards.push(RecordCard {
        name: "video_encoder".into(),
        status: if encoder.ready {
            "ok"
        } else if unchecked {
            "unknown"
        } else {
            "degraded"
        }
        .into(),
        detail: encoder.message.clone(),
        start_admission: if unchecked {
            RecordStartAdmission::CheckedAtStart
        } else {
            RecordStartAdmission::Strict
        },
    });
    doctor.ready = ready_rollup(&doctor.cards);
    doctor.start_allowed = start_readiness::start_allowed(&doctor.cards);
    Some(encoder)
}

pub(super) fn admit(request: VideoEncoderRequest) -> Result<(), CutError> {
    if let Some(encoder) = record_capture::video_encoder_readiness(
        request,
        true,
        record_capture::VideoEncoderPurpose::RecordingStart,
    ) {
        encoder.ensure_ready().map_err(super::record_err)?;
    }
    Ok(())
}

pub(super) fn projection(
    mut body: serde_json::Value,
    encoder: Option<VideoEncoderReadiness>,
) -> serde_json::Value {
    if let Some(encoder) = encoder {
        body["video_encoder"] = serde_json::json!(encoder);
    }
    body
}
