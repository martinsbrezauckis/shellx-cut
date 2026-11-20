//! applicability.rs — which quality checks apply to THIS export.
//!
//! A check that has nothing to check is "not applicable": never a failure and
//! never a warning (no large editor warns about missing subtitles on an export
//! that has none by design). This is a different state from "unmeasured",
//! which means the check applies but could not be measured here (for example
//! the captions tools are not installed); that stays visible with its reason.
//!
//! When each check applies (decided from the timeline the render was made
//! from, never from the rendered bytes):
//! - `caption_presence`: the export burns in captions, i.e. a VISIBLE caption
//!   track carries at least one caption clip. render.final has no separate
//!   burn-in or sidecar option; a hidden caption track is not burned in.
//! - `cut_on_word`: a clip on the timeline whose sound is program audio has a
//!   transcript (without one, no cut can be compared with words).
//! - `lufs`, `silence_at_edges`: the timeline has sound: an unmuted
//!   program-audio clip whose source has an audio stream.
//! - `black_or_frozen_frames`, `uniform_border`: the timeline has picture (a
//!   clip on a video track). The frozen half of `black_or_frozen_frames` does
//!   not apply when every picture is a still image (a still is frozen by
//!   design); black stretches are still checked.
//!
//! A check is waived ONLY on facts this engine measured: what the timeline
//! placed (the edit journal) and what the engine's own probe of each placed
//! source said for THIS render (`MeasuredSources`), never the probe facts
//! cached in `project.json` (a received project can state them falsely). A
//! source the engine could not probe is unknown, and an unknown never waives:
//! the check runs, or reads "not measured".
//! - `duration_matches_edl`, `sources_decodable`: always apply (they say
//!   whether the file is the timeline).
//!
//! Delivery standards apply only when asked (`apply_request`): like Premiere
//! Pro / Media Encoder (Loudness Normalization is an export option, off by
//! default; no warning without a target), a plain export is not measured
//! against a loudness target nobody set, nor against an edge-silence budget.
//! - `lufs`: a loudness target is set (`normalize_loudness`, then measured
//!   against THAT target) or a footage profile is chosen explicitly (its
//!   -16 LUFS spoken-content target), e.g. by `autopilot.run`.
//! - `silence_at_edges`: a footage profile is chosen explicitly.

use crate::types::Transcript;
use cut_core::{check_names as cn, not_applicable_check, CheckResult, Edl, Project, TrackKind};
use serde_json::json;
use std::collections::BTreeMap;

/// What this engine's own probe of one placed source said, for this render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFacts {
    /// The source has an audio stream.
    pub has_audio: bool,
    /// The source is a still image (not moving video).
    pub still: bool,
}

/// Asset id -> the engine's measurement of that source. An asset missing here
/// is UNKNOWN: nothing is waived on it.
pub type MeasuredSources = BTreeMap<String, SourceFacts>;

const CAPTIONS_APPLY: &str =
    "the export burns in captions (caption clips on a visible caption track)";
const WORDS_APPLY: &str = "a clip on the timeline has a transcript";
const SOUND_APPLIES: &str = "the timeline has sound";
const PICTURE_APPLIES: &str = "the timeline has picture";

/// What the timeline offers the quality checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineScope {
    /// Caption clips are burned into this export.
    pub burns_captions: bool,
    /// A caption track carries clips but is hidden, so nothing is burned.
    pub hidden_captions: bool,
    /// A program-audio clip has a transcript.
    pub has_transcript: bool,
    /// An unmuted program-audio clip's source has sound: `Some(false)` only
    /// when the timeline places none, or every placed one was measured silent;
    /// `None` when a placed source could not be measured (never waives).
    pub sound: Option<bool>,
    /// A clip sits on a video track.
    pub has_picture: bool,
    /// Every media clip on a video track was MEASURED a still image.
    pub all_picture_still: bool,
}

/// Read the scope from the edit (journal-backed project and EDL) and the
/// engine's own measurement of the placed sources (see the module note).
pub fn timeline_scope(
    project: &Project,
    edl: &Edl,
    transcripts: &[&Transcript],
    measured: &MeasuredSources,
) -> TimelineScope {
    let muted = |track: &str| project.tracks.iter().any(|t| t.id == track && t.muted);
    let program_audio = || {
        edl.segments
            .iter()
            .filter(|s| s.asset.is_some() && edl.is_audio_bearing_track(&s.track))
    };
    let placed_audio: Vec<Option<bool>> = program_audio()
        .filter(|s| !muted(&s.track))
        .map(|s| {
            measured
                .get(s.asset.as_deref().unwrap_or_default())
                .map(|f| f.has_audio)
        })
        .collect();
    let sound = if placed_audio.contains(&Some(true)) {
        Some(true)
    } else if placed_audio.iter().all(|a| *a == Some(false)) {
        Some(false) // none placed, or every placed source measured silent
    } else {
        None
    };
    let has_transcript = program_audio().any(|s| {
        let asset = s.asset.as_deref().unwrap_or_default();
        transcripts.iter().any(|t| t.asset == asset)
    });
    let pictures: Vec<_> = edl
        .segments
        .iter()
        .filter(|s| s.track_kind == TrackKind::Video && s.clip_id.is_some())
        .collect();
    let all_picture_still = !pictures.is_empty()
        && pictures.iter().all(|s| {
            s.asset
                .as_deref()
                .and_then(|asset| measured.get(asset))
                .is_some_and(|f| f.still)
        });
    let burns_captions = edl
        .segments
        .iter()
        .any(|s| s.track_kind == TrackKind::Caption && s.caption_text.is_some());
    let hidden_captions = !burns_captions
        && project
            .tracks
            .iter()
            .any(|t| t.kind == TrackKind::Caption && !t.visible && !t.clips.is_empty());
    TimelineScope {
        burns_captions,
        hidden_captions,
        has_transcript,
        sound,
        has_picture: !pictures.is_empty(),
        all_picture_still,
    }
}

/// Replace each check that does not apply with its not-applicable form, and
/// drop the frozen half of `black_or_frozen_frames` on an all-still timeline.
pub fn apply_scope(checks: &mut [CheckResult], scope: &TimelineScope) {
    for check in checks.iter_mut() {
        let not_applicable = match check.name.as_str() {
            cn::CAPTION_PRESENCE if !scope.burns_captions => Some((
                CAPTIONS_APPLY,
                if scope.hidden_captions {
                    "The caption track is hidden, so this export carries no captions."
                } else {
                    "This export has no captions."
                },
            )),
            cn::CUT_ON_WORD if !scope.has_transcript => Some((
                WORDS_APPLY,
                "No clip on the timeline has a transcript, so cuts cannot be compared with words.",
            )),
            cn::LUFS | cn::SILENCE_AT_EDGES if scope.sound == Some(false) => {
                Some((SOUND_APPLIES, "The timeline has no sound."))
            }
            cn::BLACK_OR_FROZEN_FRAMES | cn::UNIFORM_BORDER if !scope.has_picture => {
                Some((PICTURE_APPLIES, "The timeline has no picture."))
            }
            _ => None,
        };
        if let Some((applies_when, reason)) = not_applicable {
            *check = not_applicable_check(&check.name, applies_when, reason);
        } else if check.name == cn::BLACK_OR_FROZEN_FRAMES && scope.all_picture_still {
            drop_frozen_half(check);
        }
    }
}

/// On an all-still timeline a frozen picture is the design: only black
/// stretches decide the check; the frozen spans stay in evidence.
fn drop_frozen_half(check: &mut CheckResult) {
    let Some(black) = check
        .details
        .get("black_span_count")
        .and_then(|v| v.as_u64())
    else {
        return; // not measured: leave the row as it is
    };
    if let Some(details) = check.details.as_object_mut() {
        details.insert(
            "frozen_not_applicable".into(),
            json!(
                "every picture on the timeline is a still image, so frozen frames are the design"
            ),
        );
    }
    check.pass = black == 0;
}

/// What the person or the caller asked the export to be checked against.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Requested {
    /// A footage profile was chosen explicitly (editorial battery).
    pub editorial: bool,
    /// A loudness target (LUFS) was set for this export.
    pub loudness_target: Option<f64>,
}

const LOUDNESS_APPLIES: &str =
    "a loudness target is set for this export (normalize_loudness) or a footage profile is chosen";
const EDGES_APPLY: &str =
    "a footage profile is chosen for this export (profile), e.g. by autopilot.run";

/// Scope the delivery standards to what was asked (see the module note).
/// A check asked for by name (a loudness target, an explicit profile) is
/// never silently waived: even on a timeline without sound it runs on the
/// rendered output (or reads "not measured" when it could not be). Checks a
/// profile waived stay waived; that waiver is recorded in the row.
pub fn apply_request(
    checks: &mut [CheckResult],
    facts: &crate::checks::RenderFacts,
    req: Requested,
) {
    for check in checks.iter_mut() {
        if check.details.get("waived_by_profile").is_some() {
            continue;
        }
        if cut_core::is_not_applicable(check) {
            match check.name.as_str() {
                cn::LUFS if req.loudness_target.is_some() || req.editorial => {
                    *check = crate::checks::lufs(facts, req.loudness_target.unwrap_or(-16.0), 2.0)
                }
                cn::SILENCE_AT_EDGES if req.editorial => {
                    *check = crate::checks::silence_at_edges(facts, 500)
                }
                _ => {}
            }
            continue;
        }
        match check.name.as_str() {
            cn::LUFS => match req.loudness_target {
                Some(target) => *check = crate::checks::lufs(facts, target, 2.0),
                None if !req.editorial => {
                    *check = not_applicable_check(
                        cn::LUFS,
                        LOUDNESS_APPLIES,
                        "No loudness target was set for this export.",
                    )
                }
                None => {}
            },
            cn::SILENCE_AT_EDGES if !req.editorial => {
                *check = not_applicable_check(
                    cn::SILENCE_AT_EDGES,
                    EDGES_APPLY,
                    "No footage profile was chosen, so silence at the start or end is not judged.",
                )
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_core::{edl_from_project, CaptionClip, Clip, MediaClip, ProjectSettings, Track};

    fn project(probe: serde_json::Value, captions: Option<bool>) -> Project {
        let mut p = Project::new("t", ProjectSettings::default());
        p.assets.insert(
            "a1".into(),
            serde_json::from_value(json!({"path": "/x.mp4", "hash": "h", "probe": probe})).unwrap(),
        );
        let clip: MediaClip = serde_json::from_value(
            json!({"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 2_000}),
        )
        .unwrap();
        p.track_mut("v1").unwrap().clips.push(Clip::Media(clip));
        if let Some(visible) = captions {
            p.tracks.push(Track {
                id: "cap".into(),
                kind: TrackKind::Caption,
                clips: vec![Clip::Caption(CaptionClip {
                    id: "k1".into(),
                    text: "hello".into(),
                    style_ref: None,
                    range_ms: [0, 1_000],
                })],
                gain_db: 0.0,
                gain_windows: vec![],
                blend_mode: None,
                visible,
                locked: false,
                muted: false,
                solo: false,
                pan: 0.0,
            });
        }
        p
    }

    /// The engine's measurement of the fixture's one source `a1`.
    fn measured(has_audio: bool, still: bool) -> MeasuredSources {
        MeasuredSources::from([("a1".to_string(), SourceFacts { has_audio, still })])
    }

    fn rows(p: &Project, checks: &[&str]) -> Vec<CheckResult> {
        rows_with(p, checks, &measured(true, false))
    }

    fn rows_with(p: &Project, checks: &[&str], m: &MeasuredSources) -> Vec<CheckResult> {
        let edl = edl_from_project(p);
        let mut rows: Vec<CheckResult> = checks
            .iter()
            .map(|n| CheckResult {
                name: (*n).into(),
                pass: false,
                details: json!({"black_span_count": 0, "frozen_span_count": 1}),
                evidence: json!({}),
            })
            .collect();
        apply_scope(&mut rows, &timeline_scope(p, &edl, &[], m));
        rows
    }

    #[test]
    fn delivery_standards_apply_only_when_asked() {
        let facts = crate::checks::RenderFacts {
            duration_ms: 2_000,
            loudness: Some(crate::types::Loudness {
                integrated_lufs: -23.4,
                true_peak_dbtp: -9.5,
                windows: vec![],
            }),
            output_report: None,
            streams: vec![],
        };
        let rows = || {
            vec![
                crate::checks::lufs(&facts, -16.0, 2.0),
                CheckResult {
                    name: cn::SILENCE_AT_EDGES.into(),
                    pass: false,
                    details: json!({}),
                    evidence: json!({}),
                },
            ]
        };
        // A plain export: neither standard was asked for.
        let mut plain = rows();
        apply_request(&mut plain, &facts, Requested::default());
        assert_eq!(
            plain[0].details["reason"],
            "No loudness target was set for this export."
        );
        assert_eq!(plain[1].details["status"], "not_applicable");
        // A target of -23 LUFS: measured against THAT target, and it passes.
        let mut target = rows();
        apply_request(
            &mut target,
            &facts,
            Requested {
                editorial: false,
                loudness_target: Some(-23.0),
            },
        );
        assert!(target[0].pass, "{:?}", target[0]);
        assert_eq!(target[0].details["target_lufs"], -23.0);
        // An explicit profile: both apply at the profile's -16 target.
        let mut asked = rows();
        apply_request(
            &mut asked,
            &facts,
            Requested {
                editorial: true,
                loudness_target: None,
            },
        );
        assert!(!asked[0].pass && !asked[1].pass);
    }

    #[test]
    fn an_ordinary_clip_without_captions_or_transcript_is_not_asked_for_them() {
        let p = project(json!({"kind": "video", "has_audio": true}), None);
        let r = rows(&p, &[cn::CAPTION_PRESENCE, cn::CUT_ON_WORD, cn::LUFS]);
        assert_eq!(r[0].details["status"], "not_applicable");
        assert_eq!(r[0].details["reason"], "This export has no captions.");
        assert!(r[0].pass);
        assert_eq!(r[1].details["status"], "not_applicable");
        assert!(!r[2].pass, "sound is there, so loudness still applies");
    }

    #[test]
    fn a_hidden_caption_track_burns_nothing_and_a_visible_one_is_checked() {
        let hidden = rows(
            &project(json!({"has_audio": true}), Some(false)),
            &[cn::CAPTION_PRESENCE],
        );
        assert_eq!(
            hidden[0].details["reason"],
            "The caption track is hidden, so this export carries no captions."
        );
        let shown = rows(
            &project(json!({"has_audio": true}), Some(true)),
            &[cn::CAPTION_PRESENCE],
        );
        assert!(
            shown[0].details.get("status").is_none(),
            "visible captions are checked"
        );
    }

    #[test]
    fn no_sound_and_still_pictures_scope_the_output_checks() {
        let p = project(json!({"kind": "image", "has_audio": false}), None);
        let r = rows_with(
            &p,
            &[
                cn::LUFS,
                cn::SILENCE_AT_EDGES,
                cn::BLACK_OR_FROZEN_FRAMES,
                cn::UNIFORM_BORDER,
            ],
            &measured(false, true),
        );
        assert_eq!(r[0].details["reason"], "The timeline has no sound.");
        assert_eq!(r[1].details["status"], "not_applicable");
        assert!(r[2].pass, "a still is frozen by design; no black stretch");
        assert!(r[2].details["frozen_not_applicable"].is_string());
        assert!(!r[3].pass, "a border still applies to a picture");
    }

    /// A received project may state its cached probe falsely: "silent" for an
    /// audible clip, "image" for moving video. Nothing is waived on that: only
    /// the engine's own measurement of the placed source waives a check, and an
    /// unmeasured source waives nothing.
    #[test]
    fn cached_probe_facts_never_waive_a_check() {
        let forged = project(json!({"kind": "image", "has_audio": false}), None);
        let checks = [cn::LUFS, cn::SILENCE_AT_EDGES, cn::BLACK_OR_FROZEN_FRAMES];
        // Measured by this engine: audible, moving. The forged cache is ignored.
        let r = rows_with(&forged, &checks, &measured(true, false));
        assert!(r.iter().all(|c| c.details.get("status").is_none()), "{r:?}");
        assert!(!r[2].pass, "frozen frames still count on moving video");
        // Not measured at all: unknown, so nothing is waived either.
        let r = rows_with(&forged, &checks, &MeasuredSources::new());
        assert!(r.iter().all(|c| c.details.get("status").is_none()), "{r:?}");
        assert!(!r[2].pass);
    }

    /// A check asked for by name is never silently waived: a loudness target on
    /// a timeline without sound is measured on the output all the same.
    #[test]
    fn an_asked_loudness_target_is_measured_even_without_sound() {
        let facts = crate::checks::RenderFacts {
            duration_ms: 2_000,
            loudness: None,
            output_report: None,
            streams: vec![],
        };
        let mut rows = vec![not_applicable_check(
            cn::LUFS,
            SOUND_APPLIES,
            "The timeline has no sound.",
        )];
        apply_request(
            &mut rows,
            &facts,
            Requested {
                editorial: false,
                loudness_target: Some(-23.0),
            },
        );
        assert!(rows[0].details.get("status").is_none(), "{:?}", rows[0]);
        assert!(!rows[0].pass, "no loudness measured is never a pass");
    }
}
