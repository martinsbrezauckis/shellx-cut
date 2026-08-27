//! Unit coverage for private metadata-review admission and refusal boundaries.

use super::super::{Candidate, OfflineAssetSnapshot};
use super::*;
use serde_json::json;
use std::path::PathBuf;

fn stored(
    kind: &str,
    size: u64,
    duration_ms: Option<u64>,
    dimensions: Option<(u32, u32)>,
) -> StoredMetadata {
    let mut probe = json!({
        "kind": kind,
        "format": "mov,mp4 / h264+aac",
        "raw": {
            "format": {"size": size.to_string(), "format_name": "mov,mp4"},
            "streams": [{"codec_name": "h264"}, {"codec_name": "aac"}]
        }
    });
    if let Some(duration_ms) = duration_ms {
        probe["duration_ms"] = json!(duration_ms);
    }
    if let Some((width, height)) = dimensions {
        probe["width"] = json!(width);
        probe["height"] = json!(height);
    }
    stored_metadata(Some(&probe))
}

fn candidate(
    name: &str,
    bytes: u64,
    kind: &str,
    duration_ms: Option<u64>,
    dimensions: Option<(u32, u32)>,
    format: &str,
    codecs: &str,
) -> Candidate {
    let mut probe = json!({"kind": kind, "format": format, "raw": {
        "format": {"format_name": format.split('/').next().unwrap_or_default().trim()},
        "streams": codecs.split('+').map(|codec| json!({"codec_name": codec.trim()})).collect::<Vec<_>>()
    }});
    if let Some(duration_ms) = duration_ms {
        probe["duration_ms"] = json!(duration_ms);
    }
    if let Some((width, height)) = dimensions {
        probe["width"] = json!(width);
        probe["height"] = json!(height);
    }
    Candidate {
        path: format!("/private/{name}"),
        display_name: name.into(),
        hash: "sha256:candidate".into(),
        metadata: CandidateMetadata {
            bytes,
            facts: Some(ProbeFacts::from_json(&probe)),
        },
    }
}

fn asset(metadata: StoredMetadata) -> OfflineAssetSnapshot {
    OfflineAssetSnapshot {
        asset_id: "a1".into(),
        expected_hash: "sha256s:sampled".into(),
        old_path: "/gone/clip.mov".into(),
        source_path: PathBuf::from("/gone/clip.mov"),
        display_name: "clip.mov".into(),
        metadata,
    }
}

fn matching_video() -> Candidate {
    candidate(
        "clip.mov",
        42,
        "video",
        Some(1_000),
        Some((1920, 1080)),
        "mov,mp4 / h264+aac",
        "h264+aac",
    )
}

#[test]
fn unique_strong_candidate_is_review_only() {
    let asset = asset(stored("video", 42, Some(1_000), Some((1920, 1080))));
    let assessment = assess(&asset, &[matching_video()]);
    assert_eq!(assessment.disposition, "metadata_review");
    assert_eq!(
        assessment.diagnostics,
        vec![
            FACT_BASENAME,
            FACT_KIND,
            FACT_SIZE,
            FACT_DURATION,
            FACT_DIMENSIONS,
            FACT_FORMAT,
            FACT_CODECS
        ]
    );
}

#[test]
fn unique_strong_still_uses_dimensions_not_duration() {
    let asset = asset(stored("image", 42, None, Some((1920, 1080))));
    let candidates = vec![candidate(
        "clip.mov",
        42,
        "image",
        None,
        Some((1920, 1080)),
        "mov,mp4 / h264+aac",
        "h264+aac",
    )];
    let assessment = assess(&asset, &candidates);
    assert_eq!(assessment.disposition, "metadata_review");
    assert!(assessment.diagnostics.contains(&FACT_DIMENSIONS.to_owned()));
    assert!(!assessment.diagnostics.contains(&FACT_DURATION.to_owned()));
}

#[test]
fn missing_stored_probe_or_size_is_insufficient() {
    let mut asset = asset(stored("audio", 42, Some(1_000), None));
    asset.metadata = stored_metadata(None);
    assert_eq!(assess(&asset, &[]).disposition, "metadata_insufficient");
    asset.metadata = StoredMetadata {
        probe_available: true,
        raw_size: None,
        facts: ProbeFacts::default(),
    };
    assert_eq!(assess(&asset, &[]).disposition, "metadata_insufficient");
}

#[test]
fn size_mismatch_never_admits_metadata_review() {
    let asset = asset(stored("video", 42, Some(1_000), Some((1920, 1080))));
    let candidate = candidate(
        "clip.mov",
        41,
        "video",
        Some(1_000),
        Some((1920, 1080)),
        "mov,mp4 / h264+aac",
        "h264+aac",
    );
    let assessment = assess(&asset, &[candidate]);
    assert_eq!(assessment.disposition, "no_match");
    assert!(!assessment.diagnostics.contains(&FACT_SIZE.to_owned()));
}

#[test]
fn duration_mismatch_never_admits_metadata_review() {
    let asset = asset(stored("video", 42, Some(1_000), Some((1920, 1080))));
    let candidate = candidate(
        "clip.mov",
        42,
        "video",
        Some(999),
        Some((1920, 1080)),
        "mov,mp4 / h264+aac",
        "h264+aac",
    );
    let assessment = assess(&asset, &[candidate]);
    assert_eq!(assessment.disposition, "no_match");
    assert!(!assessment.diagnostics.contains(&FACT_DURATION.to_owned()));
}

#[test]
fn codec_only_mismatch_refuses_metadata_review() {
    let asset = asset(stored("video", 42, Some(1_000), Some((1920, 1080))));
    let candidate = candidate(
        "clip.mov",
        42,
        "video",
        Some(1_000),
        Some((1920, 1080)),
        "mov,mp4 / h264+aac",
        "hevc+aac",
    );
    let assessment = assess(&asset, &[candidate]);
    assert_eq!(assessment.disposition, "metadata_mismatch");
    assert!(!assessment.diagnostics.contains(&FACT_CODECS.to_owned()));
}

#[test]
fn dimension_and_format_mismatch_refuse_metadata_review() {
    let asset = asset(stored("video", 42, Some(1_000), Some((1920, 1080))));
    let candidates = vec![candidate(
        "clip.mov",
        42,
        "video",
        Some(1_000),
        Some((1280, 720)),
        "matroska / vp9+opus",
        "vp9+opus",
    )];
    let assessment = assess(&asset, &candidates);
    assert_eq!(assessment.disposition, "metadata_mismatch");
    assert!(!assessment.diagnostics.contains(&FACT_DIMENSIONS.to_owned()));
    assert!(!assessment.diagnostics.contains(&FACT_FORMAT.to_owned()));
}

#[test]
fn two_strong_candidates_remain_ambiguous() {
    let asset = asset(stored("video", 42, Some(1_000), Some((1920, 1080))));
    let first = matching_video();
    let mut second = matching_video();
    second.path = "/private/other/clip.mov".into();
    let assessment = assess(&asset, &[first, second]);
    assert_eq!(assessment.disposition, "ambiguous_metadata");
    assert_eq!(
        assessment.diagnostics,
        vec![
            FACT_BASENAME,
            FACT_KIND,
            FACT_SIZE,
            FACT_DURATION,
            FACT_DIMENSIONS,
            FACT_FORMAT,
            FACT_CODECS
        ]
    );
}
