//! detach.rs — the PURE decision core for `edit.detach_audio`.
//!
//! Role: the dependency-free, deterministic heart of the "detach / extract
//! audio" verb. In a classic NLE "Detach Audio" UN-links a muxed A/V clip so the
//! two halves move independently. In THIS engine that framing does not apply: the
//! renderer mixes audio from `TrackKind::Audio` tracks ONLY (cut_media
//! `build_graph`), a video clip's muxed audio is never in the render, and a
//! video's audio normally lives as a SEPARATE audio-track clip (the auto-place
//! v1/a1t mirror). There is no clip-level link to clear — a video clip and its
//! sibling audio clip are already two independent, independently-movable clips.
//!
//! So `edit.detach_audio` here is EXTRACT / PROMOTE, not unlink: for a video-track
//! clip whose asset carries audio but which has NO sibling audio clip yet (a
//! plain `edit.insert` of a video clip — its audio is silently dropped from the
//! render, the `audio_not_mixed` guardrail), it promotes that audio onto its own
//! audio track so it becomes an editable, independently-movable, J/L-splittable
//! timeline element. This RECOVERS audio that was absent from the output (the
//! correct fix, deliberately NOT audio-neutral). When a sibling audio clip
//! already exists (the auto-placed case), the audio is already detached → a clean
//! informational no-op.
//!
//! This module answers ONLY the decision ([`plan_detach_audio`]); the server
//! dispatch layer lowers the result onto ONE ordinary, replay-safe `edit.insert`
//! op (optionally preceded by an `edit.add_track` when no audio track exists) —
//! it invents NO new timeline primitive and no new audio path.
//!
//! Dependencies: crate types + `edl_from_project` (std only otherwise). Primary
//! caller: `server::dispatch::edit_detach_audio`.

use crate::{edl_from_project, Clip, Project, TrackKind};

/// Pick an audio track on which ordinary `edit.insert` cannot move any
/// pre-existing clip. The cumulative track model always shifts target-track
/// content after an insertion point even with `ripple:false`, so a track is
/// safe only when it is empty or ends at/before the detached clip's start.
/// Returning `None` tells dispatch to create a fresh audio track.
pub fn find_safe_detach_audio_track(project: &Project, at_ms: u64) -> Option<String> {
    project
        .tracks
        .iter()
        .find(|track| {
            track.kind == TrackKind::Audio
                && track.duration_ms() <= at_ms
                && track
                    .gain_windows
                    .iter()
                    .all(|window| window.range_ms[1] <= at_ms)
        })
        .map(|track| track.id.clone())
}

/// What detaching a clip's audio should do (the planner's success outcome).
#[derive(Debug, Clone, PartialEq)]
pub enum DetachPlan {
    /// The clip's audio is NOT yet a timeline element — promote it: create an
    /// audio clip from `asset`'s source window `src_range_ms`, at timeline
    /// `at_ms` (the video clip's absolute start). This adds the previously-
    /// dropped audio to the render.
    Extract {
        asset: String,
        at_ms: u64,
        src_range_ms: [u64; 2],
    },
    /// A sibling audio clip already carries this clip's audio (same asset + same
    /// timeline start on an audio track — the auto-place / linked signature) →
    /// nothing to do; the audio is already on its own, independently-movable
    /// track. Reports the existing audio clip id.
    AlreadyDetached { audio_clip_id: String },
}

/// Why a detach was rejected — each maps to an actionable verb error.
#[derive(Debug, Clone, PartialEq)]
pub enum DetachReject {
    /// No clip with that id is on the timeline.
    ClipNotFound,
    /// The clip is itself on an AUDIO track — it is already separate audio.
    AlreadyAudio,
    /// The clip is not a video media clip (a caption, a gap, or otherwise not
    /// extractable). `kind` is a short human label for the message.
    NotVideoClip { kind: &'static str },
    /// The clip's asset carries no audio stream (probe.has_audio false/unknown).
    NoAudio,
    /// The clip is retimed (speed ≠ 1×). v1 refuses: a normal-speed extracted
    /// audio clip would not match the video clip's stretched timeline span, so
    /// the two would desync. `speed` is the offending factor.
    Retimed { speed: f64 },
    /// The clip carries a VARIABLE-speed ramp (edit.speed_ramp). Same desync
    /// reason as `Retimed`, but the warp is non-linear — clearing the ramp first
    /// is the fix, not edit.speed.
    Ramped,
}

/// Decide what `edit.detach_audio {clip}` should do, as a pure function of the
/// project state. No ids are allocated here — the caller lowers an `Extract` to a
/// single replay-safe `edit.insert`. Deterministic.
///
/// Pipeline:
///   1. locate the clip + its track (else `ClipNotFound`);
///   2. it must be a VIDEO-track media clip (audio-track clip → `AlreadyAudio`;
///      caption/gap/non-media → `NotVideoClip`);
///   3. reject a retimed clip (speed ≠ 1×) — v1 keeps A/V length-locked
///      (`Retimed`);
///   4. the asset must carry audio (`probe.has_audio`; else `NoAudio`);
///   5. find the clip's ABSOLUTE timeline start via the EDL (positions are
///      cumulative); if a sibling audio clip already sits at that start with the
///      same asset → `AlreadyDetached`; otherwise → `Extract` of the clip's
///      source window at that start.
pub fn plan_detach_audio(project: &Project, clip_id: &str) -> Result<DetachPlan, DetachReject> {
    // 1. Locate the clip + its track.
    let (track_id, idx) = project
        .find_clip(clip_id)
        .ok_or(DetachReject::ClipNotFound)?;
    let track = project
        .track(track_id)
        .expect("find_clip returned a real track id");

    // 2. Must be a VIDEO-track media clip.
    match track.kind {
        TrackKind::Video => {}
        TrackKind::Audio => return Err(DetachReject::AlreadyAudio),
        TrackKind::Caption => return Err(DetachReject::NotVideoClip { kind: "caption" }),
    }
    let Clip::Media(mc) = &track.clips[idx] else {
        // A video track only holds Media/Gap; a gap has no id so find_clip never
        // returns it — this arm is defensive (e.g. a future non-media clip kind).
        return Err(DetachReject::NotVideoClip { kind: "non-media" });
    };

    // 3. Refuse a retimed clip (v1 keeps the extracted audio length-locked). A
    //    variable-speed ramp warps the timeline non-linearly, so a normal-speed
    //    extracted audio clip would desync just as badly — refuse it too.
    if mc.has_speed_ramp() {
        return Err(DetachReject::Ramped);
    }
    if (mc.speed - 1.0).abs() > f64::EPSILON {
        return Err(DetachReject::Retimed { speed: mc.speed });
    }

    // 4. Asset must carry an audio stream (probe.has_audio — the same fact the
    //    audio_drop_warning guardrail reads).
    let has_audio = project
        .assets
        .get(&mc.asset)
        .and_then(|a| a.probe.as_ref())
        .and_then(|p| p.get("has_audio"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !has_audio {
        return Err(DetachReject::NoAudio);
    }

    // 5. Absolute timeline start of this clip (clips are positional → use the EDL).
    let edl = edl_from_project(project);
    let at_ms = edl
        .segments
        .iter()
        .find(|s| s.clip_id.as_deref() == Some(clip_id))
        .map(|s| s.timeline_in_ms)
        // Defensive: in find_clip but absent from the EDL should be impossible.
        .ok_or(DetachReject::ClipNotFound)?;

    // Already-detached? A sibling audio clip with the SAME asset at the SAME
    // timeline start on an audio track (the auto-place / linked-insert signature
    // edit.paste also keys off). The audio is then already independent.
    if let Some(sib) = edl.segments.iter().find(|s| {
        s.track_kind == TrackKind::Audio
            && s.asset.as_deref() == Some(mc.asset.as_str())
            && s.timeline_in_ms == at_ms
            && s.src_in_ms == Some(mc.src_in_ms)
            && s.src_out_ms == Some(mc.src_out_ms)
            && s.clip_id.is_some()
    }) {
        return Ok(DetachPlan::AlreadyDetached {
            audio_clip_id: sib.clip_id.clone().expect("filtered clip_id.is_some()"),
        });
    }

    // Extract: the audio range is the clip's own source window, placed at its
    // timeline start so it stays frame-locked until the user moves it.
    Ok(DetachPlan::Extract {
        asset: mc.asset.clone(),
        at_ms,
        src_range_ms: [mc.src_in_ms, mc.src_out_ms],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Asset, MediaClip};
    use serde_json::json;
    use std::collections::BTreeMap;

    /// A probed asset; `has_audio` toggles the audio-stream fact the planner reads.
    fn asset(has_audio: bool) -> Asset {
        Asset {
            path: "/testdata/talking_head.mp4".into(),
            hash: "sha256:deadbeef".into(),
            probe: Some(json!({ "duration_ms": 10_000, "has_audio": has_audio })),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        }
    }

    /// A default-speed media clip [src_in, src_out) of asset `a` with id `id`.
    fn media(id: &str, a: &str, src_in: u64, src_out: u64) -> Clip {
        Clip::Media(MediaClip {
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

    /// Project with one video clip `c1` on v1 (asset a1) and an empty a1t.
    fn proj_video_only(has_audio: bool) -> Project {
        let mut p = Project::new("t", Default::default());
        let mut assets = BTreeMap::new();
        assets.insert("a1".to_string(), asset(has_audio));
        p.assets = assets;
        p.track_mut("v1")
            .unwrap()
            .clips
            .push(media("c1", "a1", 0, 8000));
        p
    }

    #[test]
    fn safe_target_selection_never_chooses_content_that_insert_would_shift() {
        let mut p = proj_video_only(true);
        assert_eq!(
            find_safe_detach_audio_track(&p, 0).as_deref(),
            Some("a1t"),
            "the default empty audio track is safe"
        );

        p.track_mut("a1t")
            .unwrap()
            .clips
            .push(media("existing", "a1", 0, 2000));
        assert_eq!(
            find_safe_detach_audio_track(&p, 0),
            None,
            "inserting at zero would move existing audio"
        );
        assert_eq!(
            find_safe_detach_audio_track(&p, 2000).as_deref(),
            Some("a1t"),
            "appending at the exact track end is safe"
        );

        p.track_mut("a1t")
            .unwrap()
            .gain_windows
            .push(crate::GainWindow {
                range_ms: [2000, 3000],
                db: -6.0,
                attack_ms: 0,
            });
        assert_eq!(
            find_safe_detach_audio_track(&p, 2000),
            None,
            "target-track gain automation must not be shifted either"
        );
    }

    #[test]
    fn extract_when_video_has_audio_and_no_sibling() {
        let p = proj_video_only(true);
        let plan = plan_detach_audio(&p, "c1").expect("a video clip with audio extracts");
        assert_eq!(
            plan,
            DetachPlan::Extract {
                asset: "a1".into(),
                at_ms: 0,
                src_range_ms: [0, 8000],
            }
        );
    }

    #[test]
    fn extract_uses_clips_timeline_start_and_source_window() {
        // c1 [0,3000) then c2 [2000,9000) (a 7000ms span) on v1 — c2 starts at 3000.
        let mut p = proj_video_only(true);
        p.track_mut("v1").unwrap().clips =
            vec![media("c1", "a1", 0, 3000), media("c2", "a1", 2000, 9000)];
        let plan = plan_detach_audio(&p, "c2").unwrap();
        assert_eq!(
            plan,
            DetachPlan::Extract {
                asset: "a1".into(),
                at_ms: 3000,                // cumulative: after c1's 3000ms
                src_range_ms: [2000, 9000], // c2's own trimmed source window
            }
        );
    }

    #[test]
    fn already_detached_when_sibling_audio_at_same_start() {
        let mut p = proj_video_only(true);
        // Mirror clip on a1t at the SAME start (the auto-place signature).
        p.track_mut("a1t")
            .unwrap()
            .clips
            .push(media("c1a", "a1", 0, 8000));
        let plan = plan_detach_audio(&p, "c1").unwrap();
        assert_eq!(
            plan,
            DetachPlan::AlreadyDetached {
                audio_clip_id: "c1a".into()
            }
        );
    }

    #[test]
    fn sibling_at_a_different_start_does_not_count_as_detached() {
        let mut p = proj_video_only(true);
        // An audio clip of the SAME asset but at a DIFFERENT start (1000ms) is not
        // this clip's audio — extraction must still proceed.
        p.track_mut("a1t").unwrap().clips = vec![
            Clip::Gap(crate::types::GapClip::new(1000)),
            media("other", "a1", 0, 8000),
        ];
        let plan = plan_detach_audio(&p, "c1").unwrap();
        assert!(matches!(plan, DetachPlan::Extract { at_ms: 0, .. }));
    }

    #[test]
    fn sibling_with_different_source_window_does_not_count_as_detached() {
        let mut p = proj_video_only(true);
        // Same asset and same timeline start, but a different source window. This
        // is a different take/range, not c1's linked audio.
        p.track_mut("a1t")
            .unwrap()
            .clips
            .push(media("other", "a1", 1000, 9000));
        let plan = plan_detach_audio(&p, "c1").unwrap();
        assert_eq!(
            plan,
            DetachPlan::Extract {
                asset: "a1".into(),
                at_ms: 0,
                src_range_ms: [0, 8000],
            }
        );
    }

    #[test]
    fn rejects_audio_track_clip_as_already_audio() {
        let mut p = proj_video_only(true);
        p.track_mut("a1t")
            .unwrap()
            .clips
            .push(media("ca", "a1", 0, 8000));
        assert_eq!(plan_detach_audio(&p, "ca"), Err(DetachReject::AlreadyAudio));
    }

    #[test]
    fn rejects_asset_without_audio() {
        let p = proj_video_only(false);
        assert_eq!(plan_detach_audio(&p, "c1"), Err(DetachReject::NoAudio));
    }

    #[test]
    fn rejects_unknown_clip() {
        let p = proj_video_only(true);
        assert_eq!(
            plan_detach_audio(&p, "nope"),
            Err(DetachReject::ClipNotFound)
        );
    }

    #[test]
    fn rejects_retimed_clip() {
        let mut p = proj_video_only(true);
        if let Clip::Media(mc) = &mut p.track_mut("v1").unwrap().clips[0] {
            mc.speed = 2.0;
        }
        assert_eq!(
            plan_detach_audio(&p, "c1"),
            Err(DetachReject::Retimed { speed: 2.0 })
        );
    }

    /// REPLAY-SAFETY: detach lowers to a single ordinary `edit.insert` (the audio
    /// clip), so a log rebuild must reproduce the post-detach timeline byte-for-
    /// byte (the extracted clip's id is pinned by the insert op). After the
    /// extract, the planner sees the new sibling → AlreadyDetached.
    #[test]
    fn extract_lowering_replays_byte_identical() {
        use crate::{rebuild_from_log, ProjectStore};

        fn audio_asset() -> Asset {
            asset(true)
        }
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
        let (aid, _) = s.record_import(None, audio_asset(), actor(), None).unwrap();
        assert_eq!(aid, "a1");

        // Place the video clip on v1 (no auto-place in core) — a1t stays empty,
        // so the asset's audio is NOT yet a timeline element.
        s.apply(
            "edit.insert",
            json!({ "asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 8000], "ripple": false }),
            actor(),
            None,
        )
        .unwrap();
        let vid_id = s.project.track("v1").unwrap().clips[0]
            .id()
            .unwrap()
            .to_string();

        // Plan → Extract, then lower it to the audio insert (what dispatch does).
        let plan = plan_detach_audio(&s.project, &vid_id).unwrap();
        let DetachPlan::Extract {
            asset,
            at_ms,
            src_range_ms,
        } = plan
        else {
            unreachable!("expected Extract, got {plan:?}");
        };
        s.apply(
            "edit.insert",
            json!({ "asset": asset, "track": "a1t", "at_ms": at_ms, "src_range_ms": src_range_ms, "ripple": false }),
            actor(),
            None,
        )
        .unwrap();

        // The audio is now an independent a1t clip.
        let a1t = s.project.track("a1t").unwrap();
        assert_eq!(a1t.clips.len(), 1, "extracted audio landed on a1t");
        let audio_id = a1t.clips[0].id().unwrap().to_string();
        assert_ne!(
            audio_id, vid_id,
            "the audio clip is a distinct, movable element"
        );

        // Replay: rebuild the timeline from the op log and compare byte-identically.
        let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
        assert_eq!(
            serde_json::to_string(&rebuilt.tracks).unwrap(),
            serde_json::to_string(&s.project.tracks).unwrap(),
            "rebuild_from_log == live timeline (the extracted audio clip id is pinned)"
        );

        // The planner now sees the sibling → no double-extract.
        assert_eq!(
            plan_detach_audio(&s.project, &vid_id),
            Ok(DetachPlan::AlreadyDetached {
                audio_clip_id: audio_id
            }),
        );
    }
}
