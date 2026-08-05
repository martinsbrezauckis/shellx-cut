//! trim_edit.rs — the pro trim trio: SLIP / ROLL / SLIDE.
//!
//! Verbs `edit.slip`, `edit.roll`, `edit.slide_edit` (named beside
//! `edit.split_edit`; the bare `edit.slide` name is taken by the slide-in/out
//! ANIMATION presets). Each is ONE op mutating one/two/three clips atomically —
//! never a chain of trims that would transiently ripple downstream:
//!
//!   slip        change WHICH source content a clip shows — shift its source
//!               window; timeline position + duration untouched.
//!   roll        move the shared CUT between two adjacent clips — the left
//!               clip's out-point and the right clip's in-point move together;
//!               total timeline length unchanged.
//!   slide_edit  move a clip left/right between its two media neighbors — the
//!               neighbors absorb the move (left out-point + right in-point);
//!               the clip's own content and duration untouched.
//!
//! Discipline shared with edit.trim / edit.ripple_delete:
//!   • media clips only; speed RAMPS refused (ramp points are offsets into the
//!     source window — clear, edit, re-ramp);
//!   • constant speed is honored (timeline Δ × speed = source Δ), exact at
//!     speed 1.0, ±1 ms rounding documented for retimed clips;
//!   • crossfades on a moving boundary are refused (clear the transition
//!     first — silently re-basing an overlap would corrupt the seam);
//!   • bounds validated against the asset probe when present; empty spans
//!     refused; errors are actionable.
//!
//! Callers: store.rs replay table ("edit.slip"/"edit.roll"/"edit.slide_edit"),
//! dispatched via server commit_core. Tests at the bottom mirror edit.rs style.

use crate::edit::{clip_not_found, fx, ramp_conflict, track_not_found};
use crate::error::{codes, CutError};
use crate::ops::OpEffect;
use crate::types::{Clip, MediaClip, Project};
use serde_json::json;

/// Signed source-delta for a timeline delta at a clip's constant speed
/// (timeline→source: 1 timeline ms consumes `speed` source ms).
fn tl_delta_to_src(by_ms: i64, speed: f64) -> i64 {
    let s = if speed.is_finite() && speed > 0.0 {
        speed
    } else {
        1.0
    };
    (by_ms as f64 * s).round() as i64
}

/// Checked signed shift of a u64 source time; refuses to go below zero.
fn shift(v: u64, d: i64, what: &str, clip: &str) -> Result<u64, CutError> {
    let n = v as i64 + d;
    if n < 0 {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!(
                "{what} of clip '{clip}' would go {}ms before the source start",
                -n
            ),
            "reduce the amount — the source has no content before 0",
        )
        .with_clip(clip));
    }
    Ok(n as u64)
}

/// Asset duration from the probe (None when unprobed — bounds then unenforced,
/// matching edit.trim's behavior).
fn asset_duration(project: &Project, asset: &str) -> Option<u64> {
    project
        .assets
        .get(asset)
        .and_then(|a| a.probe.as_ref())
        .and_then(|p| p.get("duration_ms"))
        .and_then(|v| v.as_u64())
}

fn check_tail(project: &Project, m: &MediaClip, new_out: u64) -> Result<(), CutError> {
    if let Some(d) = asset_duration(project, &m.asset) {
        if new_out > d {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!(
                    "clip '{}' would need source up to {new_out}ms but asset '{}' is only {d}ms long",
                    m.id, m.asset
                ),
                "reduce the amount — there is no more source material past the asset end",
            )
            .with_clip(&m.id));
        }
    }
    Ok(())
}

fn require_media<'a>(clip: &'a mut Clip, verb: &str) -> Result<&'a mut MediaClip, CutError> {
    match clip {
        Clip::Media(m) => Ok(m),
        other => {
            let id = other.id().unwrap_or("<gap>").to_string();
            Err(CutError::new(
                codes::INVALID_ARGS,
                format!("'{id}' is not a media clip"),
                format!(
                    "{verb} works on video/audio media clips (captions are edited via captions.*)"
                ),
            )
            .with_clip(&id))
        }
    }
}

fn require_nonzero(by_ms: i64) -> Result<(), CutError> {
    if by_ms == 0 {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            "by_ms is 0 — nothing to do".to_string(),
            "pass a positive (later/right) or negative (earlier/left) millisecond amount",
        ));
    }
    Ok(())
}

/// SLIP (verb `edit.slip {clip, by_ms}`): shift the clip's source window by
/// `by_ms` SOURCE milliseconds (positive = show later content). Timeline
/// position, duration, fades, and keyframes are untouched — only WHAT plays.
pub fn slip(project: &mut Project, clip_id: &str, by_ms: i64) -> Result<Vec<OpEffect>, CutError> {
    require_nonzero(by_ms)?;
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    // Validate against the probe BEFORE mutating (single-clip op — keep it pure).
    let (new_in, new_out) = {
        let t = project
            .tracks
            .iter_mut()
            .find(|t| t.id == track_id)
            .unwrap();
        let m = require_media(&mut t.clips[idx], "edit.slip")?;
        if m.speed_ramp.is_some() {
            return Err(ramp_conflict(clip_id, "edit.slip"));
        }
        (
            shift(m.src_in_ms, by_ms, "slip in-point", clip_id)?,
            shift(m.src_out_ms, by_ms, "slip out-point", clip_id)?,
        )
    };
    {
        // immutable probe check between the two mutable scopes
        let t = project.tracks.iter().find(|t| t.id == track_id).unwrap();
        if let Clip::Media(m) = &t.clips[idx] {
            check_tail(project, m, new_out)?;
        }
    }
    let t = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    if let Clip::Media(m) = &mut t.clips[idx] {
        let old = [m.src_in_ms, m.src_out_ms];
        m.src_in_ms = new_in;
        m.src_out_ms = new_out;
        return Ok(vec![fx(
            Some(&track_id),
            json!({"clip_id": clip_id, "slipped_ms": by_ms, "old_src": old, "src": [new_in, new_out]}),
        )]);
    }
    unreachable!("clip kind checked above")
}

/// ROLL (verb `edit.roll {track, at_ms, by_ms}`): move the shared cut between
/// the two adjacent media clips whose boundary sits exactly at `at_ms`
/// (timeline). Positive `by_ms` moves the cut later (left clip grows, right
/// clip shrinks); total timeline length is unchanged, downstream never moves.
pub fn roll(
    project: &mut Project,
    track_id: &str,
    at_ms: u64,
    by_ms: i64,
) -> Result<Vec<OpEffect>, CutError> {
    require_nonzero(by_ms)?;
    let track_exists = project.tracks.iter().any(|t| t.id == track_id);
    if !track_exists {
        return Err(track_not_found(track_id));
    }
    // Locate the seam: cumulative end of clip i == at_ms, clip i+1 exists.
    let (li, ri) = {
        let t = project.tracks.iter().find(|t| t.id == track_id).unwrap();
        let mut cum = 0u64;
        let mut seam: Option<(usize, usize)> = None;
        for (i, c) in t.clips.iter().enumerate() {
            cum += c.timeline_duration_ms();
            if cum == at_ms && i + 1 < t.clips.len() {
                seam = Some((i, i + 1));
                break;
            }
            if cum > at_ms {
                break;
            }
        }
        seam.ok_or_else(|| {
            CutError::new(
                codes::INVALID_ARGS,
                format!("no clip-to-clip cut at {at_ms}ms on track '{track_id}'"),
                "at_ms must be the exact boundary between two adjacent clips (project.state shows durations)",
            )
        })?
    };
    // Read + validate both sides, then mutate. Clone-then-swap in store.rs makes
    // a mid-way error safe, but we still validate everything up front.
    let (l_new_out, r_new_in, l_old, r_old, l_id, r_id) = {
        let t = project.tracks.iter().find(|t| t.id == track_id).unwrap();
        let (l, r) = match (&t.clips[li], &t.clips[ri]) {
            (Clip::Media(l), Clip::Media(r)) => (l, r),
            _ => {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("the cut at {at_ms}ms is not between two media clips"),
                    "roll works on video/audio media clips on both sides of the cut",
                ))
            }
        };
        if l.speed_ramp.is_some() {
            return Err(ramp_conflict(&l.id, "edit.roll"));
        }
        if r.speed_ramp.is_some() {
            return Err(ramp_conflict(&r.id, "edit.roll"));
        }
        if r.xfade_in_ms > 0 {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("the cut at {at_ms}ms carries a crossfade"),
                "clear the transition first (edit.crossfade {duration_ms: 0}), then roll",
            )
            .with_clip(&r.id));
        }
        let l_new_out = shift(
            l.src_out_ms,
            tl_delta_to_src(by_ms, l.speed),
            "roll left out-point",
            &l.id,
        )?;
        let r_new_in = shift(
            r.src_in_ms,
            tl_delta_to_src(by_ms, r.speed),
            "roll right in-point",
            &r.id,
        )?;
        if l_new_out <= l.src_in_ms {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("roll would make clip '{}' empty", l.id),
                "reduce the amount — the left clip must keep some content",
            )
            .with_clip(&l.id));
        }
        if r_new_in >= r.src_out_ms {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("roll would make clip '{}' empty", r.id),
                "reduce the amount — the right clip must keep some content",
            )
            .with_clip(&r.id));
        }
        check_tail(project, l, l_new_out)?;
        (
            l_new_out,
            r_new_in,
            [l.src_in_ms, l.src_out_ms],
            [r.src_in_ms, r.src_out_ms],
            l.id.clone(),
            r.id.clone(),
        )
    };
    let t = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    if let Clip::Media(l) = &mut t.clips[li] {
        l.src_out_ms = l_new_out;
    }
    if let Clip::Media(r) = &mut t.clips[ri] {
        r.src_in_ms = r_new_in;
    }
    Ok(vec![fx(
        Some(track_id),
        json!({
            "at_ms": at_ms, "rolled_ms": by_ms,
            "left": {"clip_id": l_id, "old_src": l_old, "src_out_ms": l_new_out},
            "right": {"clip_id": r_id, "old_src": r_old, "src_in_ms": r_new_in},
        }),
    )])
}

/// SLIDE (verb `edit.slide_edit {clip, by_ms}`): move a clip later/earlier on
/// its track while its two MEDIA neighbors absorb the move — the left
/// neighbor's out-point and the right neighbor's in-point shift by the same
/// timeline amount. The slid clip's own source window and duration are
/// untouched; total timeline length is unchanged.
pub fn slide_edit(
    project: &mut Project,
    clip_id: &str,
    by_ms: i64,
) -> Result<Vec<OpEffect>, CutError> {
    require_nonzero(by_ms)?;
    let (track_id, idx) = project
        .find_clip(clip_id)
        .map(|(t, i)| (t.to_string(), i))
        .ok_or_else(|| clip_not_found(clip_id))?;
    let (l_new_out, r_new_in, l_old, r_old, l_id, r_id) = {
        let t = project.tracks.iter().find(|t| t.id == track_id).unwrap();
        if idx == 0 || idx + 1 >= t.clips.len() {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("clip '{clip_id}' has no neighbor on both sides"),
                "slide needs an adjacent media clip BEFORE and AFTER (use edit.move otherwise)",
            )
            .with_clip(clip_id));
        }
        let mid = match &t.clips[idx] {
            Clip::Media(m) => m,
            _ => {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("'{clip_id}' is not a media clip"),
                    "slide works on video/audio media clips",
                )
                .with_clip(clip_id))
            }
        };
        let (l, r) = match (&t.clips[idx - 1], &t.clips[idx + 1]) {
            (Clip::Media(l), Clip::Media(r)) => (l, r),
            _ => {
                return Err(CutError::new(
                    codes::INVALID_ARGS,
                    format!("clip '{clip_id}' is not between two media clips"),
                    "slide's neighbors must both be video/audio media clips",
                )
                .with_clip(clip_id))
            }
        };
        for c in [l, mid, r] {
            if c.speed_ramp.is_some() {
                return Err(ramp_conflict(&c.id, "edit.slide_edit"));
            }
        }
        if mid.xfade_in_ms > 0 || r.xfade_in_ms > 0 {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("a boundary of clip '{clip_id}' carries a crossfade"),
                "clear the transitions first (edit.crossfade {duration_ms: 0}), then slide",
            )
            .with_clip(clip_id));
        }
        let l_new_out = shift(
            l.src_out_ms,
            tl_delta_to_src(by_ms, l.speed),
            "slide left out-point",
            &l.id,
        )?;
        let r_new_in = shift(
            r.src_in_ms,
            tl_delta_to_src(by_ms, r.speed),
            "slide right in-point",
            &r.id,
        )?;
        if l_new_out <= l.src_in_ms {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("slide would make clip '{}' empty", l.id),
                "reduce the amount — the left neighbor must keep some content",
            )
            .with_clip(&l.id));
        }
        if r_new_in >= r.src_out_ms {
            return Err(CutError::new(
                codes::INVALID_ARGS,
                format!("slide would make clip '{}' empty", r.id),
                "reduce the amount — the right neighbor must keep some content",
            )
            .with_clip(&r.id));
        }
        check_tail(project, l, l_new_out)?;
        (
            l_new_out,
            r_new_in,
            [l.src_in_ms, l.src_out_ms],
            [r.src_in_ms, r.src_out_ms],
            l.id.clone(),
            r.id.clone(),
        )
    };
    let t = project
        .tracks
        .iter_mut()
        .find(|t| t.id == track_id)
        .unwrap();
    if let Clip::Media(l) = &mut t.clips[idx - 1] {
        l.src_out_ms = l_new_out;
    }
    if let Clip::Media(r) = &mut t.clips[idx + 1] {
        r.src_in_ms = r_new_in;
    }
    Ok(vec![fx(
        Some(&track_id),
        json!({
            "clip_id": clip_id, "slid_ms": by_ms,
            "left": {"clip_id": l_id, "old_src": l_old, "src_out_ms": l_new_out},
            "right": {"clip_id": r_id, "old_src": r_old, "src_in_ms": r_new_in},
        }),
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Asset, ProjectSettings};

    /// One video track with three adjacent 5s clips of a 60s probed asset,
    /// windows [10s,15s) [20s,25s) [30s,35s) — headroom on every side.
    fn fixture() -> Project {
        let mut p = Project::new("t", ProjectSettings::default());
        p.assets.insert(
            "a1".into(),
            Asset {
                path: "/x.mp4".into(),
                hash: "sha256:x".into(),
                probe: Some(json!({"kind":"video","duration_ms":60_000,"has_audio":true})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        let clip = |id: &str, s: u64, e: u64| {
            Clip::Media(MediaClip {
                id: id.into(),
                asset: "a1".into(),
                src_in_ms: s,
                src_out_ms: e,
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
        };
        // Project::new already creates the default v1/a1t tracks — fill v1
        // (pushing a second "v1" would shadow it and break find_clip's index).
        p.track_mut("v1").unwrap().clips = vec![
            clip("c1", 10_000, 15_000),
            clip("c2", 20_000, 25_000),
            clip("c3", 30_000, 35_000),
        ];
        p
    }
    fn media(p: &Project, id: &str) -> MediaClip {
        for t in &p.tracks {
            for c in &t.clips {
                if let Clip::Media(m) = c {
                    if m.id == id {
                        return m.clone();
                    }
                }
            }
        }
        panic!("no clip {id}")
    }
    fn track_dur(p: &Project) -> u64 {
        p.tracks[0]
            .clips
            .iter()
            .map(|c| c.timeline_duration_ms())
            .sum()
    }

    #[test]
    fn slip_shifts_source_window_only() {
        let mut p = fixture();
        let before = track_dur(&p);
        slip(&mut p, "c2", 2000).unwrap();
        let m = media(&p, "c2");
        assert_eq!([m.src_in_ms, m.src_out_ms], [22_000, 27_000]);
        assert_eq!(track_dur(&p), before, "slip must not change the timeline");
        slip(&mut p, "c2", -4000).unwrap();
        let m = media(&p, "c2");
        assert_eq!([m.src_in_ms, m.src_out_ms], [18_000, 23_000]);
        // bounds: before source start / past asset end / zero
        assert_eq!(
            slip(&mut p, "c2", -19_000).unwrap_err().code,
            codes::INVALID_ARGS
        );
        assert_eq!(
            slip(&mut p, "c2", 40_000).unwrap_err().code,
            codes::INVALID_ARGS
        );
        assert_eq!(slip(&mut p, "c2", 0).unwrap_err().code, codes::INVALID_ARGS);
        assert_eq!(slip(&mut p, "nope", 10).unwrap_err().code, codes::NOT_FOUND);
    }

    #[test]
    fn roll_moves_the_cut_and_keeps_total_length() {
        let mut p = fixture();
        let before = track_dur(&p);
        // seam c1→c2 sits at 5000 (both 5s clips)
        roll(&mut p, "v1", 5000, 1500).unwrap();
        assert_eq!(media(&p, "c1").src_out_ms, 16_500, "left grew");
        assert_eq!(media(&p, "c2").src_in_ms, 21_500, "right shrank");
        assert_eq!(track_dur(&p), before, "roll keeps total length");
        // the seam MOVED to 6500 — the old address is mid-clip now and must fail…
        assert_eq!(
            roll(&mut p, "v1", 5000, -1500).unwrap_err().code,
            codes::INVALID_ARGS
        );
        // …and rolling back at the NEW seam restores the original cut exactly.
        roll(&mut p, "v1", 6500, -1500).unwrap();
        assert_eq!(
            media(&p, "c1").src_out_ms,
            15_000,
            "roll back restored the left out-point"
        );
        assert_eq!(
            media(&p, "c2").src_in_ms,
            20_000,
            "roll back restored the right in-point"
        );
    }

    #[test]
    fn roll_validates_seam_emptiness_and_ramp() {
        let mut p = fixture();
        assert_eq!(
            roll(&mut p, "v1", 4000, 100).unwrap_err().code,
            codes::INVALID_ARGS,
            "mid-clip is not a seam"
        );
        assert_eq!(
            roll(&mut p, "v1", 5000, -5000).unwrap_err().code,
            codes::INVALID_ARGS,
            "would empty the left clip"
        );
        assert_eq!(
            roll(&mut p, "v1", 5000, 5000).unwrap_err().code,
            codes::INVALID_ARGS,
            "would empty the right clip"
        );
        assert_eq!(
            roll(&mut p, "nope", 5000, 100).unwrap_err().code,
            codes::NOT_FOUND
        );
        // crossfade at the seam → refuse
        if let Clip::Media(m) = &mut p.tracks[0].clips[1] {
            m.xfade_in_ms = 300;
        }
        assert_eq!(
            roll(&mut p, "v1", 5000, 100).unwrap_err().code,
            codes::INVALID_ARGS
        );
    }

    #[test]
    fn slide_moves_clip_between_neighbors() {
        let mut p = fixture();
        let before = track_dur(&p);
        let mid_before = media(&p, "c2");
        slide_edit(&mut p, "c2", 2000).unwrap();
        assert_eq!(media(&p, "c1").src_out_ms, 17_000, "left neighbor grew");
        assert_eq!(media(&p, "c3").src_in_ms, 32_000, "right neighbor shrank");
        let mid = media(&p, "c2");
        assert_eq!(
            [mid.src_in_ms, mid.src_out_ms],
            [mid_before.src_in_ms, mid_before.src_out_ms],
            "slid clip content untouched"
        );
        assert_eq!(track_dur(&p), before, "slide keeps total length");
        // edges + validation
        assert_eq!(
            slide_edit(&mut p, "c1", 100).unwrap_err().code,
            codes::INVALID_ARGS,
            "no left neighbor"
        );
        assert_eq!(
            slide_edit(&mut p, "c3", 100).unwrap_err().code,
            codes::INVALID_ARGS,
            "no right neighbor"
        );
        assert_eq!(
            slide_edit(&mut p, "c2", 0).unwrap_err().code,
            codes::INVALID_ARGS
        );
        assert_eq!(
            slide_edit(&mut p, "c2", 30_000).unwrap_err().code,
            codes::INVALID_ARGS,
            "would empty the right neighbor"
        );
    }

    /// Constant speed: timeline Δ × speed = source Δ (left at 2×, right at 1×).
    #[test]
    fn roll_honors_constant_speed() {
        let mut p = fixture();
        if let Clip::Media(m) = &mut p.tracks[0].clips[0] {
            m.speed = 2.0; // c1 now 2.5s on the timeline ([10s,15s) at 2×)
        }
        // seam c1→c2 sits at 2500 now
        roll(&mut p, "v1", 2500, 1000).unwrap();
        assert_eq!(
            media(&p, "c1").src_out_ms,
            17_000,
            "left source moved 2× the timeline delta"
        );
        assert_eq!(
            media(&p, "c2").src_in_ms,
            21_000,
            "right source moved 1× the timeline delta"
        );
    }
}
