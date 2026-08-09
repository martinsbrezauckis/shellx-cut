//! split_edit.rs — the PURE decision core for `edit.split_edit` (J-cut / L-cut).
//!
//! Role: the dependency-free, deterministic planner for a SPLIT EDIT — the
//! standard smooth-dialogue NLE move that OFFSETS the audio transition relative
//! to the video cut at a clip boundary, so one clip's audio leads or lags its
//! video.
//!
//! MODEL (established by `edit.detach_audio`, see detach.rs): the renderer mixes
//! audio from `TrackKind::Audio` tracks ONLY — a video clip's muxed audio is
//! never in the render — and a video's audio normally lives as a SEPARATE
//! audio-track clip (the auto-place v1/a1t mirror: the same asset + same source
//! window + same timeline start as its video clip). Video and audio clips trim
//! and move INDEPENDENTLY by id (`edit.trim` / `edit.move` operate on one clip).
//! So a J/L cut here is NOT a new timeline primitive: it is a ROLL of the AUDIO
//! edit point relative to the (untouched) VIDEO cut, expressed as two ordinary
//! `edit.trim`s on the two linked AUDIO clips around the boundary.
//!
//! WHY TWO TRIMS SUFFICE (no split, no move, no id allocation): clip positions
//! are CUMULATIVE (a clip's timeline start is the sum of the preceding clips'
//! durations on its track — the edit.rs mutation invariant). So extending the outgoing
//! audio clip's out-edge pushes the incoming audio clip's start LATER
//! automatically, and trimming the incoming clip's in-edge by the same amount
//! pulls its end back to where it was — the net effect on everything downstream
//! is ZERO (a true roll), and only the A|B audio boundary moves. The video track
//! is never touched.
//!
//! L-CUT (video leads, audio lags) — roll the audio boundary RIGHT by `offset`:
//!   A's audio CONTINUES past the video cut into B's region.
//!   - extend A_audio out-edge by `offset` (needs `offset` ms of source AFTER
//!     A_audio's out — else `InsufficientHeadroom`).
//!   - trim B_audio in-edge later by `offset` (B_audio must be longer than
//!     `offset` — else `InsufficientHeadroom`). B_audio then plays from its
//!     natural sync point at the new boundary, staying in sync with B's video.
//!
//! J-CUT (audio leads, video lags) — roll the audio boundary LEFT by `offset`:
//!   B's audio STARTS before its video.
//!   - extend B_audio in-edge EARLIER by `offset` (needs `offset` ms of source
//!     BEFORE B_audio's in — i.e. `b_in >= offset` — else `InsufficientHeadroom`).
//!   - trim A_audio out-edge earlier by `offset` (A_audio must be longer than
//!     `offset` — else `InsufficientHeadroom`).
//!
//! REQUIRES PRE-SPLIT AUDIO (the clean v1 path): the audio at the boundary must
//! ALREADY be two clips butted at the cut — A_audio ending exactly at the video
//! cut, B_audio starting exactly there (the natural state when two distinct
//! sources are butted, each with its own a1t mirror). When the audio is ONE
//! continuous clip across the cut (a single source split only on the video
//! track), there is NO audio transition to offset — a J/L cut would be a no-op
//! made audible by inventing a seam. The planner then errors honestly
//! (`NoLinkedAudio`) and points the caller at `edit.split`/`edit.detach_audio`,
//! rather than silently splitting the audio (which would allocate ids and so
//! complicate replay for zero semantic gain). This keeps the verb a pure pair
//! of trims: single-responsibility, replay-trivial, id-allocation-free.
//!
//! This module answers ONLY the decision ([`plan_split_edit`]); the server
//! dispatch layer lowers the result onto exactly two replay-safe `edit.trim`
//! ops (one per audio clip), grouped under one undo tag.
//!
//! Dependencies: crate types + `edl_from_project` (std only otherwise). Primary
//! caller: `server::dispatch::edit_split_edit`.

use crate::{edl_from_project, EdlSegment, Project, TrackKind};

/// Which split edit to make — the audio either lags (L) or leads (J) the video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitEditKind {
    /// J-cut: audio leads, video lags — the audio boundary rolls LEFT of the cut.
    J,
    /// L-cut: video leads, audio lags — the audio boundary rolls RIGHT of the cut.
    L,
}

impl SplitEditKind {
    /// Parse the verb's `kind` arg ("j" / "l", case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "j" => Some(SplitEditKind::J),
            "l" => Some(SplitEditKind::L),
            _ => None,
        }
    }
    /// The lowercase token used in receipts/errors.
    pub fn as_str(self) -> &'static str {
        match self {
            SplitEditKind::J => "j",
            SplitEditKind::L => "l",
        }
    }
}

/// The planner's success outcome: roll the audio boundary by trimming the two
/// linked audio clips. The caller lowers this onto two `edit.trim` ops — no id
/// allocation, so the log replays byte-identically.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitEditPlan {
    /// Outgoing clip's linked audio id (the clip ending at the video cut).
    pub audio_a: String,
    /// Incoming clip's linked audio id (the clip starting at the video cut).
    pub audio_b: String,
    /// The audio track both clips live on.
    pub audio_track: String,
    /// A_audio source window before / after the trim.
    pub a_old_src: [u64; 2],
    pub a_new_src: [u64; 2],
    /// B_audio source window before / after the trim.
    pub b_old_src: [u64; 2],
    pub b_new_src: [u64; 2],
    /// The video cut position (unchanged — the video edit is not touched).
    pub video_cut_ms: u64,
    /// Where the audio transition lands after the roll: `at_ms + offset` for an
    /// L-cut, `at_ms - offset` for a J-cut.
    pub audio_boundary_ms: u64,
}

/// Why a split edit was rejected — each maps to an actionable verb error.
#[derive(Debug, Clone, PartialEq)]
pub enum SplitEditReject {
    /// `offset_ms` was 0 — a split edit with no offset is a no-op.
    ZeroOffset,
    /// No video cut at `at_ms`: it is not the exact boundary between two
    /// adjacent VIDEO media clips on the chosen video track.
    NoVideoCut { at_ms: u64, video_track: String },
    /// The audio at the boundary is not two clips butted at the cut (a single
    /// continuous source, or no a1t mirror). `side` says which half is missing.
    NoLinkedAudio { side: &'static str },
    /// Both audio sides exist, but not on the same track, so there is no linked
    /// butted audio pair to roll.
    LinkedAudioDifferentTracks {
        outgoing_track: String,
        incoming_track: String,
    },
    /// One of the audio clips is retimed (speed ≠ 1×). v1 refuses: the
    /// timeline→source offset mapping is non-linear there, so a 1:1 ms roll
    /// would desync. `clip` + `speed` name the offender.
    Retimed { clip: String, speed: f64 },
    /// Not enough source / clip length to roll by `offset`. `what` names the
    /// exhausted budget; `available` < `needed` (= offset).
    InsufficientHeadroom {
        what: &'static str,
        available: u64,
        needed: u64,
    },
}

/// Find the audio media segment ENDING exactly at `at_ms` on `track` with the
/// given `asset` (the outgoing clip's linked audio, butted at the cut).
fn audio_seg_ending_at<'a>(
    edl: &'a crate::Edl,
    track: &str,
    at_ms: u64,
    asset: &str,
) -> Option<&'a EdlSegment> {
    edl.segments.iter().find(|s| {
        s.track == track
            && s.track_kind == TrackKind::Audio
            && s.clip_id.is_some()
            && s.asset.as_deref() == Some(asset)
            && s.timeline_out_ms == at_ms
    })
}

/// Find the audio media segment STARTING exactly at `at_ms` on `track` with the
/// given `asset` (the incoming clip's linked audio, butted at the cut).
fn audio_seg_starting_at<'a>(
    edl: &'a crate::Edl,
    track: &str,
    at_ms: u64,
    asset: &str,
) -> Option<&'a EdlSegment> {
    edl.segments.iter().find(|s| {
        s.track == track
            && s.track_kind == TrackKind::Audio
            && s.clip_id.is_some()
            && s.asset.as_deref() == Some(asset)
            && s.timeline_in_ms == at_ms
    })
}

/// Decide what `edit.split_edit {at_ms, kind, offset_ms}` should do, as a pure
/// function of the project. No ids are allocated — the caller lowers the plan to
/// two `edit.trim`s on the two linked audio clips. Deterministic.
///
/// Pipeline:
///   1. `offset_ms` must be > 0 (`ZeroOffset`);
///   2. resolve the video track (explicit `video_track`, else the base video
///      track) and find the cut: adjacent VIDEO media segments A (out == at_ms)
///      and B (in == at_ms). Missing either → `NoVideoCut`;
///   3. find the two linked AUDIO clips butted at the cut — A_audio (ends at
///      at_ms, same asset as A) and B_audio (starts at at_ms, same asset as B),
///      on the same audio track (explicit `audio_track`, else the first audio
///      track that holds BOTH). Missing either → `NoLinkedAudio`;
///   4. both audio clips must be normal speed (`Retimed` otherwise);
///   5. compute the rolled source edges per `kind` and check headroom
///      (`InsufficientHeadroom`); return the `SplitEditPlan`.
pub fn plan_split_edit(
    project: &Project,
    video_track: Option<&str>,
    audio_track: Option<&str>,
    at_ms: u64,
    kind: SplitEditKind,
    offset_ms: u64,
) -> Result<SplitEditPlan, SplitEditReject> {
    // 1. A zero offset is a no-op, not a split edit.
    if offset_ms == 0 {
        return Err(SplitEditReject::ZeroOffset);
    }
    let edl = edl_from_project(project);

    // 2. Resolve the video track + locate the cut between two adjacent media clips.
    let vtrack = match video_track {
        Some(t) => t.to_string(),
        None => edl
            .base_video_track()
            .ok_or_else(|| SplitEditReject::NoVideoCut {
                at_ms,
                video_track: "<none>".into(),
            })?
            .to_string(),
    };
    let a_vid = edl.segments.iter().find(|s| {
        s.track == vtrack
            && s.track_kind == TrackKind::Video
            && s.asset.is_some()
            && s.timeline_out_ms == at_ms
    });
    let b_vid = edl.segments.iter().find(|s| {
        s.track == vtrack
            && s.track_kind == TrackKind::Video
            && s.asset.is_some()
            && s.timeline_in_ms == at_ms
    });
    let (a_vid, b_vid) = match (a_vid, b_vid) {
        (Some(a), Some(b)) => (a, b),
        _ => {
            return Err(SplitEditReject::NoVideoCut {
                at_ms,
                video_track: vtrack,
            })
        }
    };
    let a_asset = a_vid.asset.as_deref().expect("media seg has asset");
    let b_asset = b_vid.asset.as_deref().expect("media seg has asset");

    // 3. Find the two linked audio clips butted at the cut. The audio clip from
    //    A must END at at_ms and the one from B must START at at_ms — the mirror
    //    invariant. Restrict to `audio_track` when given, else scan audio tracks
    //    (in project order) for the first track that holds BOTH.
    let candidate_tracks: Vec<String> = match audio_track {
        Some(t) => vec![t.to_string()],
        None => project
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Audio)
            .map(|t| t.id.clone())
            .collect(),
    };
    let mut a_aud: Option<&EdlSegment> = None;
    let mut b_aud: Option<&EdlSegment> = None;
    for tid in &candidate_tracks {
        let a = audio_seg_ending_at(&edl, tid, at_ms, a_asset);
        let b = audio_seg_starting_at(&edl, tid, at_ms, b_asset);
        if a.is_some() && b.is_some() {
            a_aud = a;
            b_aud = b;
            break;
        }
        // Remember a one-sided match for a precise error if no track has both.
        if a_aud.is_none() {
            a_aud = a;
        }
        if b_aud.is_none() {
            b_aud = b;
        }
    }
    // Only a track holding BOTH is usable; if the matches we kept are on
    // different tracks (or one is missing), report which side is missing.
    let (a_aud, b_aud) = match (a_aud, b_aud) {
        (Some(a), Some(b)) if a.track == b.track => (a, b),
        (None, _) => return Err(SplitEditReject::NoLinkedAudio { side: "outgoing" }),
        (_, None) => return Err(SplitEditReject::NoLinkedAudio { side: "incoming" }),
        // Both matched but on different tracks → not a butted linked pair.
        (Some(a), Some(b)) => {
            return Err(SplitEditReject::LinkedAudioDifferentTracks {
                outgoing_track: a.track.clone(),
                incoming_track: b.track.clone(),
            })
        }
    };
    let audio_track_id = a_aud.track.clone();

    // 4. Refuse retimed audio clips (the 1:1 ms roll assumes timeline == source).
    if (a_aud.speed - 1.0).abs() > f64::EPSILON {
        return Err(SplitEditReject::Retimed {
            clip: a_aud.clip_id.clone().expect("audio seg has id"),
            speed: a_aud.speed,
        });
    }
    if (b_aud.speed - 1.0).abs() > f64::EPSILON {
        return Err(SplitEditReject::Retimed {
            clip: b_aud.clip_id.clone().expect("audio seg has id"),
            speed: b_aud.speed,
        });
    }

    // 5. Compute the rolled source edges + headroom.
    let a_in = a_aud.src_in_ms.expect("audio media seg has src_in");
    let a_out = a_aud.src_out_ms.expect("audio media seg has src_out");
    let b_in = b_aud.src_in_ms.expect("audio media seg has src_in");
    let b_out = b_aud.src_out_ms.expect("audio media seg has src_out");
    let a_dur = a_out - a_in;
    let b_dur = b_out - b_in;
    // A_audio's asset full duration (for the L-cut tail check); None when unprobed.
    let a_asset_dur = project
        .assets
        .get(a_asset)
        .and_then(|a| a.probe.as_ref())
        .and_then(|p| p.get("duration_ms"))
        .and_then(|v| v.as_u64());

    let (a_new_src, b_new_src, audio_boundary_ms) = match kind {
        SplitEditKind::L => {
            // Roll RIGHT: extend A_audio out, trim B_audio in.
            // A needs `offset` ms of source AFTER its out point.
            let Some(dur) = a_asset_dur else {
                return Err(SplitEditReject::InsufficientHeadroom {
                    what: "outgoing audio source duration (run media.probe first)",
                    available: 0,
                    needed: offset_ms,
                });
            };
            let tail = dur.saturating_sub(a_out);
            if tail < offset_ms {
                return Err(SplitEditReject::InsufficientHeadroom {
                    what: "outgoing audio source tail (no audio past A's out point)",
                    available: tail,
                    needed: offset_ms,
                });
            }
            // B must keep at least 1ms after giving up `offset` from its front.
            if b_dur <= offset_ms {
                return Err(SplitEditReject::InsufficientHeadroom {
                    what: "incoming audio clip length (B is too short to start later)",
                    available: b_dur,
                    needed: offset_ms,
                });
            }
            (
                [a_in, a_out + offset_ms],
                [b_in + offset_ms, b_out],
                at_ms + offset_ms,
            )
        }
        SplitEditKind::J => {
            // Roll LEFT: extend B_audio in earlier, trim A_audio out.
            // The rolled audio boundary must stay on the timeline; otherwise
            // `at_ms - offset_ms` would underflow and produce a bogus boundary.
            if offset_ms > at_ms {
                return Err(SplitEditReject::InsufficientHeadroom {
                    what: "video cut position (offset would roll the boundary before the timeline start)",
                    available: at_ms,
                    needed: offset_ms,
                });
            }
            // B needs `offset` ms of source BEFORE its in point.
            if b_in < offset_ms {
                return Err(SplitEditReject::InsufficientHeadroom {
                    what: "incoming audio source head (no audio before B's in point)",
                    available: b_in,
                    needed: offset_ms,
                });
            }
            // A must keep at least 1ms after giving up `offset` from its end.
            if a_dur <= offset_ms {
                return Err(SplitEditReject::InsufficientHeadroom {
                    what: "outgoing audio clip length (A is too short to end earlier)",
                    available: a_dur,
                    needed: offset_ms,
                });
            }
            (
                [a_in, a_out - offset_ms],
                [b_in - offset_ms, b_out],
                at_ms - offset_ms,
            )
        }
    };

    Ok(SplitEditPlan {
        audio_a: a_aud.clip_id.clone().expect("audio seg has id"),
        audio_b: b_aud.clip_id.clone().expect("audio seg has id"),
        audio_track: audio_track_id,
        a_old_src: [a_in, a_out],
        a_new_src,
        b_old_src: [b_in, b_out],
        b_new_src,
        video_cut_ms: at_ms,
        audio_boundary_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Asset, GapClip, MediaClip, Track, TrackKind};
    use serde_json::json;
    use std::collections::BTreeMap;

    /// A probed asset of `dur_ms` total length.
    fn asset(dur_ms: u64) -> Asset {
        Asset {
            path: "/testdata/clip.mp4".into(),
            hash: "sha256:deadbeef".into(),
            probe: Some(json!({ "duration_ms": dur_ms, "has_audio": true })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        }
    }

    /// A default-speed media clip [src_in, src_out) of asset `a` with id `id`.
    fn media(id: &str, a: &str, src_in: u64, src_out: u64) -> crate::Clip {
        crate::Clip::Media(MediaClip {
            id: id.into(),
            asset: a.into(),
            src_in_ms: src_in,
            src_out_ms: src_out,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed: 1.0,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        })
    }

    /// Two DISTINCT sources butted on v1, each mirrored on a1t:
    ///   v1:  A(aX src[2000,5000)) [tl 0..3000) | B(aY src[1000,5000)) [tl 3000..7000)
    ///   a1t: A(aX src[2000,5000)) [tl 0..3000) | B(aY src[1000,5000)) [tl 3000..7000)
    /// Cut at at_ms = 3000. aX/aY durations are caller-set for headroom tests.
    fn butted_pair(ax_dur: u64, ay_dur: u64) -> Project {
        let mut p = Project::new("t", Default::default());
        let mut assets = BTreeMap::new();
        assets.insert("aX".to_string(), asset(ax_dur));
        assets.insert("aY".to_string(), asset(ay_dur));
        p.assets = assets;
        p.track_mut("v1").unwrap().clips = vec![
            media("c1", "aX", 2000, 5000), // A video
            media("c2", "aY", 1000, 5000), // B video
        ];
        p.track_mut("a1t").unwrap().clips = vec![
            media("c3", "aX", 2000, 5000), // A audio (ends at tl 3000)
            media("c4", "aY", 1000, 5000), // B audio (starts at tl 3000)
        ];
        p
    }

    #[test]
    fn l_cut_extends_a_out_and_trims_b_in() {
        // A's source tail: aX dur 10000, a_out 5000 → 5000ms of tail (≥1000 ok).
        let p = butted_pair(10_000, 10_000);
        let plan = plan_split_edit(&p, None, None, 3000, SplitEditKind::L, 1000).unwrap();
        assert_eq!(plan.audio_a, "c3");
        assert_eq!(plan.audio_b, "c4");
        assert_eq!(plan.audio_track, "a1t");
        // A_audio out-edge extended by 1000; in unchanged.
        assert_eq!(plan.a_old_src, [2000, 5000]);
        assert_eq!(plan.a_new_src, [2000, 6000]);
        // B_audio in-edge pushed 1000 later; out unchanged.
        assert_eq!(plan.b_old_src, [1000, 5000]);
        assert_eq!(plan.b_new_src, [2000, 5000]);
        // Audio transition lands 1000ms AFTER the video cut.
        assert_eq!(plan.video_cut_ms, 3000);
        assert_eq!(plan.audio_boundary_ms, 4000);
    }

    #[test]
    fn j_cut_extends_b_in_earlier_and_trims_a_out() {
        let p = butted_pair(10_000, 10_000);
        let plan = plan_split_edit(&p, None, None, 3000, SplitEditKind::J, 1000).unwrap();
        // A_audio out-edge pulled 1000 earlier; in unchanged.
        assert_eq!(plan.a_new_src, [2000, 4000]);
        // B_audio in-edge pulled 1000 earlier (b_in 1000 → 0); out unchanged.
        assert_eq!(plan.b_new_src, [0, 5000]);
        // Audio transition lands 1000ms BEFORE the video cut.
        assert_eq!(plan.audio_boundary_ms, 2000);
    }

    #[test]
    fn l_cut_rejects_when_no_source_tail() {
        // aX dur 5200, a_out 5000 → only 200ms of tail, need 1000.
        let p = butted_pair(5_200, 10_000);
        let err = plan_split_edit(&p, None, None, 3000, SplitEditKind::L, 1000).unwrap_err();
        assert_eq!(
            err,
            SplitEditReject::InsufficientHeadroom {
                what: "outgoing audio source tail (no audio past A's out point)",
                available: 200,
                needed: 1000,
            }
        );
    }

    #[test]
    fn l_cut_rejects_when_outgoing_source_duration_is_unknown() {
        let mut p = butted_pair(10_000, 10_000);
        p.assets.get_mut("aX").unwrap().probe = None;
        let err = plan_split_edit(&p, None, None, 3000, SplitEditKind::L, 1000).unwrap_err();
        assert_eq!(
            err,
            SplitEditReject::InsufficientHeadroom {
                what: "outgoing audio source duration (run media.probe first)",
                available: 0,
                needed: 1000,
            }
        );
    }

    #[test]
    fn j_cut_rejects_when_no_source_head() {
        // B's in is 1000; a 2000ms J-cut needs 2000ms of head before it.
        let p = butted_pair(10_000, 10_000);
        let err = plan_split_edit(&p, None, None, 3000, SplitEditKind::J, 2000).unwrap_err();
        assert_eq!(
            err,
            SplitEditReject::InsufficientHeadroom {
                what: "incoming audio source head (no audio before B's in point)",
                available: 1000,
                needed: 2000,
            }
        );
    }

    #[test]
    fn j_cut_rejects_when_offset_rolls_before_timeline_start() {
        // B has enough source head and A is long enough, but the timeline cut is
        // too close to zero for a 1000ms J-cut boundary.
        let mut p = butted_pair(10_000, 10_000);
        p.track_mut("v1").unwrap().clips =
            vec![media("c1", "aX", 4800, 5000), media("c2", "aY", 2000, 6000)];
        p.track_mut("a1t").unwrap().clips =
            vec![media("c3", "aX", 4800, 5000), media("c4", "aY", 2000, 6000)];
        let err = plan_split_edit(&p, None, None, 200, SplitEditKind::J, 1000).unwrap_err();
        assert_eq!(
            err,
            SplitEditReject::InsufficientHeadroom {
                what:
                    "video cut position (offset would roll the boundary before the timeline start)",
                available: 200,
                needed: 1000,
            }
        );
    }

    #[test]
    fn l_cut_rejects_when_b_too_short() {
        // B_audio duration is 4000; an offset ≥ that can't shorten B to ≥1ms.
        let p = butted_pair(10_000, 10_000);
        let err = plan_split_edit(&p, None, None, 3000, SplitEditKind::L, 4000).unwrap_err();
        assert!(matches!(
            err,
            SplitEditReject::InsufficientHeadroom {
                what: "incoming audio clip length (B is too short to start later)",
                available: 4000,
                needed: 4000,
            }
        ));
    }

    #[test]
    fn rejects_zero_offset() {
        let p = butted_pair(10_000, 10_000);
        assert_eq!(
            plan_split_edit(&p, None, None, 3000, SplitEditKind::L, 0),
            Err(SplitEditReject::ZeroOffset)
        );
    }

    #[test]
    fn rejects_when_no_video_cut_at_position() {
        let p = butted_pair(10_000, 10_000);
        // 3500ms is mid-B, not a boundary.
        let err = plan_split_edit(&p, None, None, 3500, SplitEditKind::L, 1000).unwrap_err();
        assert!(matches!(
            err,
            SplitEditReject::NoVideoCut { at_ms: 3500, .. }
        ));
    }

    #[test]
    fn rejects_when_audio_is_one_continuous_clip() {
        // Single source split only on VIDEO: v1 has two clips, a1t has ONE
        // continuous clip across the cut → no butted audio boundary.
        let mut p = Project::new("t", Default::default());
        let mut assets = BTreeMap::new();
        assets.insert("aX".to_string(), asset(10_000));
        p.assets = assets;
        p.track_mut("v1").unwrap().clips = vec![
            media("c1", "aX", 0, 3000),    // A video [tl 0..3000)
            media("c2", "aX", 3000, 7000), // B video [tl 3000..7000)
        ];
        p.track_mut("a1t").unwrap().clips = vec![
            media("c3", "aX", 0, 7000), // ONE continuous audio [tl 0..7000)
        ];
        let err = plan_split_edit(&p, None, None, 3000, SplitEditKind::L, 1000).unwrap_err();
        assert!(matches!(err, SplitEditReject::NoLinkedAudio { .. }));
    }

    #[test]
    fn rejects_when_audio_sides_are_on_different_tracks() {
        let mut p = butted_pair(10_000, 10_000);
        let incoming = p.track_mut("a1t").unwrap().clips.pop().unwrap();
        p.tracks.push(Track {
            id: "a2t".into(),
            kind: TrackKind::Audio,
            clips: vec![crate::Clip::Gap(GapClip::new(3000)), incoming],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        });
        let err = plan_split_edit(&p, None, None, 3000, SplitEditKind::L, 1000).unwrap_err();
        assert_eq!(
            err,
            SplitEditReject::LinkedAudioDifferentTracks {
                outgoing_track: "a1t".into(),
                incoming_track: "a2t".into(),
            }
        );
    }

    #[test]
    fn rejects_retimed_audio_clip() {
        // A_audio retimed 2× with src[2000,8000) (6000ms source) lays onto a
        // 3000ms timeline span → it STILL ends at the cut (tl 3000), so the
        // linked-audio search finds it and the retime guard fires.
        let mut p = butted_pair(10_000, 10_000);
        if let crate::Clip::Media(mc) = &mut p.track_mut("a1t").unwrap().clips[0] {
            mc.src_out_ms = 8000; // 6000ms of source
            mc.speed = 2.0; // /2 → 3000ms timeline, ends at the cut
        }
        let err = plan_split_edit(&p, None, None, 3000, SplitEditKind::L, 1000).unwrap_err();
        assert!(matches!(
            err,
            SplitEditReject::Retimed { speed, .. } if (speed - 2.0).abs() < 1e-9
        ));
    }

    /// REPLAY-SAFETY: the plan lowers to two `edit.trim`s (no id allocation), so
    /// a log rebuild reproduces the post-split-edit timeline byte-for-byte. This
    /// stages the butted pair through a real ProjectStore, applies the two
    /// lowered trims, and asserts `rebuild_from_log == live`.
    #[test]
    fn lowering_to_two_trims_replays_byte_identical() {
        use crate::{rebuild_from_log, ProjectStore};

        fn actor() -> crate::Actor {
            crate::Actor {
                kind: crate::ActorKind::Agent,
                name: "claude".into(),
                via: "test".into(),
                request: None,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let mut s = ProjectStore::create(dir.path(), "demo", None).unwrap();
        let (ax, _) = s.record_import(None, asset(10_000), actor(), None).unwrap();
        let (ay, _) = s.record_import(None, asset(10_000), actor(), None).unwrap();
        assert_eq!((ax.as_str(), ay.as_str()), ("a1", "a2"));

        // Butt A then B on v1 + mirror both on a1t (four explicit inserts —
        // core has no auto-place; ripple:false to keep the placement aligned).
        for (asset, track, at, range) in [
            ("a1", "v1", 0u64, [2000u64, 5000u64]),
            ("a2", "v1", 3000, [1000, 5000]),
            ("a1", "a1t", 0, [2000, 5000]),
            ("a2", "a1t", 3000, [1000, 5000]),
        ] {
            s.apply(
                "edit.insert",
                json!({"asset": asset, "track": track, "at_ms": at, "src_range_ms": range, "ripple": false}),
                actor(),
                None,
            )
            .unwrap();
        }

        let plan = plan_split_edit(&s.project, None, None, 3000, SplitEditKind::L, 1000).unwrap();
        // Lower to the two trims dispatch will emit.
        s.apply(
            "edit.trim",
            json!({"clip": plan.audio_a, "src_out_ms": plan.a_new_src[1]}),
            actor(),
            None,
        )
        .unwrap();
        s.apply(
            "edit.trim",
            json!({"clip": plan.audio_b, "src_in_ms": plan.b_new_src[0]}),
            actor(),
            None,
        )
        .unwrap();

        // The audio boundary rolled to 4000; verify the a1t clip ranges + that
        // the track length is unchanged (a true roll touches nothing downstream).
        let a1t = s.project.track("a1t").unwrap();
        let (c3, c4) = match (&a1t.clips[0], &a1t.clips[1]) {
            (crate::Clip::Media(a), crate::Clip::Media(b)) => (a, b),
            _ => unreachable!("expected two media clips on a1t"),
        };
        assert_eq!(
            [c3.src_in_ms, c3.src_out_ms],
            [2000, 6000],
            "A_audio extended"
        );
        assert_eq!(
            [c4.src_in_ms, c4.src_out_ms],
            [2000, 5000],
            "B_audio trimmed in"
        );
        // A_audio now 4000ms, B_audio now 3000ms → boundary at 4000, end at 7000.
        assert_eq!(
            a1t.duration_ms(),
            7000,
            "track length unchanged (roll is local)"
        );

        // Replay: rebuild from the op log and compare byte-identically.
        let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
        assert_eq!(
            serde_json::to_string(&rebuilt.tracks).unwrap(),
            serde_json::to_string(&s.project.tracks).unwrap(),
            "rebuild_from_log == live timeline (two trims, no id allocation)"
        );
    }
}
