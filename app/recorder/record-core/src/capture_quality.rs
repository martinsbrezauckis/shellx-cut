//! Truthful output-quality request and final-resolution evidence.
//!
//! A visible recorder control may use these values only when its native backend
//! admits the request and writes a final container that the verifier measured.
//! `encoder` is a named encoder implementation selected by that backend, never
//! a guess derived from the final container codec.

use serde::{Deserialize, Serialize};

use crate::{CaptureSourceFacts, CAPTURE_SOURCE_FACTS_SCHEMA};

pub const CAPTURE_QUALITY_SCHEMA: &str = "shellx-record/capture-quality/1";

/// A simple output-height policy. The target is an upper bound: a smaller
/// source is retained rather than upscaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaptureOutputSize {
    #[serde(rename = "source")]
    Source,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "720p")]
    P720,
}

impl CaptureOutputSize {
    pub const fn max_height(self) -> Option<u32> {
        match self {
            Self::Source => None,
            Self::P1080 => Some(1_080),
            Self::P720 => Some(720),
        }
    }
}

/// A novice-facing quality preset. Numeric codec controls belong in an
/// Advanced surface and are intentionally absent from this transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureQualityProfile {
    Standard,
    High,
}

/// The exact simple request accepted by a supporting backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureQualityRequest {
    pub output_size: CaptureOutputSize,
    pub profile: CaptureQualityProfile,
}

impl CaptureQualityRequest {
    pub const fn new(output_size: CaptureOutputSize, profile: CaptureQualityProfile) -> Self {
        Self {
            output_size,
            profile,
        }
    }
}

/// Final evidence for one accepted quality request. `width`/`height` and the
/// H.264 result were measured from the final source container; `encoder` names
/// the backend's actual selected encoder, which is not inferred from `codec`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureQualityResolution {
    pub schema: String,
    pub requested: CaptureQualityRequest,
    pub width: u32,
    pub height: u32,
    pub encoder: String,
}

impl CaptureQualityResolution {
    pub fn new(
        requested: CaptureQualityRequest,
        width: u32,
        height: u32,
        encoder: impl Into<String>,
    ) -> Option<Self> {
        let encoder = encoder.into().trim().to_owned();
        (width > 0 && height > 0 && !encoder.is_empty()).then_some(Self {
            schema: CAPTURE_QUALITY_SCHEMA.into(),
            requested,
            width,
            height,
            encoder,
        })
    }

    /// Revalidate deserialized quality evidence before exposing it from a
    /// persisted RecordingProject. Constructors do not protect serde input, so
    /// every request invariant and the paired final-source facts are checked
    /// again at the projection boundary.
    pub fn matches_verified_source(&self, source: &CaptureSourceFacts) -> bool {
        self.schema == CAPTURE_QUALITY_SCHEMA
            && source.schema == CAPTURE_SOURCE_FACTS_SCHEMA
            && self.width > 0
            && self.height > 0
            && self.encoder == "libx264"
            && self
                .requested
                .output_size
                .max_height()
                .is_none_or(|height| self.height <= height)
            && (source.width, source.height) == (self.width, self.height)
            && source.codec_name.as_deref() == Some("h264")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_quality_never_carries_numeric_codec_controls() {
        let request =
            CaptureQualityRequest::new(CaptureOutputSize::P1080, CaptureQualityProfile::High);
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["output_size"], "1080p");
        assert_eq!(json["profile"], "high");
        assert!(json.get("crf").is_none());
        assert!(
            serde_json::from_value::<CaptureQualityRequest>(serde_json::json!({
                "output_size": "1080p", "profile": "high", "crf": 20
            }))
            .is_err()
        );
        assert_eq!(CaptureOutputSize::Source.max_height(), None);
        assert_eq!(CaptureOutputSize::P720.max_height(), Some(720));
    }

    #[test]
    fn resolution_requires_real_dimensions_and_a_named_encoder() {
        let request =
            CaptureQualityRequest::new(CaptureOutputSize::Source, CaptureQualityProfile::Standard);
        let resolution =
            CaptureQualityResolution::new(request.clone(), 1_920, 1_080, "libx264").unwrap();
        assert_eq!(resolution.schema, CAPTURE_QUALITY_SCHEMA);
        assert_eq!(resolution.requested, request);
        assert!(CaptureQualityResolution::new(request, 0, 1_080, "libx264").is_none());
    }

    #[test]
    fn persisted_resolution_rechecks_request_encoder_and_source_facts() {
        let request =
            CaptureQualityRequest::new(CaptureOutputSize::P720, CaptureQualityProfile::High);
        let resolution = CaptureQualityResolution::new(request, 1_280, 720, "libx264").unwrap();
        let source = CaptureSourceFacts::new(1_280, 720, Some("h264".into())).unwrap();
        assert!(resolution.matches_verified_source(&source));

        let mut oversized = resolution.clone();
        oversized.height = 1_080;
        let oversized_source = CaptureSourceFacts::new(1_920, 1_080, Some("h264".into())).unwrap();
        oversized.width = 1_920;
        assert!(!oversized.matches_verified_source(&oversized_source));

        let mut false_encoder = resolution;
        false_encoder.encoder = "anything".into();
        assert!(!false_encoder.matches_verified_source(&source));
    }
}
