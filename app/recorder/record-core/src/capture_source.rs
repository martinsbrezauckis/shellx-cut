//! Verified final-source facts reserved for Recorder quality resolution.
//!
//! REC-QUALITY-01 deliberately does not invent a cross-platform quality picker:
//! the current native backends can capture a selected source, but they do not
//! share a truthful output-size or encoder-profile admission path. This private
//! foundation records only facts that the sealed final-media verifier actually
//! measured. A future visible picker must pair its requested choice with these
//! resolved facts or return a typed refusal.

use serde::{Deserialize, Serialize};

pub const CAPTURE_SOURCE_FACTS_SCHEMA: &str = "shellx-record/capture-source-facts/1";

/// Final screen-source facts from one verified playable container.
///
/// `codec_name` identifies the container's video codec, not an encoder
/// implementation. It belongs behind Advanced evidence; no UI may relabel it
/// as an encoder selection or infer a bitrate/profile from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureSourceFacts {
    pub schema: String,
    pub width: u32,
    pub height: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec_name: Option<String>,
}

impl CaptureSourceFacts {
    pub fn new(width: u32, height: u32, codec_name: Option<String>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        Some(Self {
            schema: CAPTURE_SOURCE_FACTS_SCHEMA.into(),
            width,
            height,
            codec_name: codec_name
                .map(|codec| codec.trim().to_owned())
                .filter(|codec| !codec.is_empty()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_facts_keep_only_real_dimensions_and_never_upgrade_a_codec_to_an_encoder() {
        let facts = CaptureSourceFacts::new(1_920, 1_080, Some("h264".into())).unwrap();
        assert_eq!(facts.schema, CAPTURE_SOURCE_FACTS_SCHEMA);
        assert_eq!(facts.codec_name.as_deref(), Some("h264"));
        assert!(CaptureSourceFacts::new(0, 1_080, Some("h264".into())).is_none());
    }
}
