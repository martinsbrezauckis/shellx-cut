//! An edit point never leaves a piece shorter than one frame.
//!
//! Large editors keep their edit points on the frame grid, so a cut, an insert
//! or a move can never leave a clip or gap shorter than one frame. ShellX Cut
//! takes points in milliseconds: an insert 1 ms before a clip's end split it
//! and left a 1 ms clip (seen after `edit.speed 2` on a 1513 ms clip, whose
//! end is at 757 ms, when the next clip went in at 756). Here, at the verb
//! layer, a point that falls less than one frame from an edge on its track (a
//! clip's or gap's start or end, or the track's end) moves to that edge. The
//! op records the moved point, so replay stays verbatim (an older op log
//! replays its own recorded points, a sliver it made stays), and the verb
//! result says it moved: `snapped {from, to, why}`.

use crate::state::AppState;
use cut_core::{error_codes, Actor, Clip, CutError, Project, VerbResult};
use serde_json::{json, Value};

/// edit.insert: the point on `track`.
pub(in crate::dispatch) async fn edit_insert(
    state: &AppState,
    mut args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let snap = snap_open_project(state, &mut args, &["track"]).await;
    super::timeline::edit_insert_at(state, args, actor)
        .await
        .map(|r| with_snap(r, snap))
}

/// edit.insert_linked: the point on its video track (the base video track when
/// none is named), else its audio track.
pub(in crate::dispatch) async fn edit_insert_linked(
    state: &AppState,
    mut args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let snap = snap_open_project(
        state,
        &mut args,
        &["video_track", BASE_VIDEO, "audio_track"],
    )
    .await;
    super::linked_insert::edit_insert_linked_at(state, args, actor)
        .await
        .map(|r| with_snap(r, snap))
}

/// edit.move: the point on `to_track` (the moved clip leaves a gap of its own
/// length, so the track's edges stay where they are).
pub(in crate::dispatch) async fn edit_move(
    state: &AppState,
    mut args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let snap = snap_open_project(state, &mut args, &["to_track"]).await;
    super::timeline::edit_move_at(state, args, actor)
        .await
        .map(|r| with_snap(r, snap))
}

/// edit.paste: the point on `to_track`.
pub(in crate::dispatch) async fn edit_paste(
    state: &AppState,
    mut args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let snap = snap_open_project(state, &mut args, &["to_track"]).await;
    super::timeline::edit_paste_at(state, args, actor)
        .await
        .map(|r| with_snap(r, snap))
}

/// edit.split: a point less than one frame from an edge is that edge, where
/// the track is already cut: nothing is split, and the refusal says why.
pub(in crate::dispatch) async fn edit_split(
    state: &AppState,
    mut args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    if let Some(s) = snap_open_project(state, &mut args, &["track"]).await {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "nothing to split at {} ms: the track is already cut at {} ms",
                s.from, s.to
            ),
            s.why,
        )
        .with_at_ms(s.from)
        .with_suggested_action(format!(
            "split at least one frame from a clip edge, or use the cut at {} ms",
            s.to
        )));
    }
    super::timeline::edit_split_at(state, args, actor).await
}

/// Stands for the base video track in a list of track keys.
const BASE_VIDEO: &str = "\0base-video";

/// A point moved to an edge.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::dispatch) struct Snap {
    pub(in crate::dispatch) from: u64,
    pub(in crate::dispatch) to: u64,
    pub(in crate::dispatch) why: String,
}

/// One frame of the project, in milliseconds.
fn frame_ms(project: &Project) -> f64 {
    let fps = project.settings.fps;
    if fps.is_finite() && fps > 0.0 {
        1000.0 / fps
    } else {
        1000.0 / 30.0
    }
}

/// The edges on `track`: 0, every clip's and gap's end (= the next start), the
/// track's end. `(position, what ends or starts there)`.
fn edges(project: &Project, track: &str) -> Vec<(u64, String)> {
    let Some(t) = project.track(track) else {
        return Vec::new();
    };
    let mut out = vec![(0, "the start of the track".to_string())];
    let mut cursor = 0u64;
    for clip in &t.clips {
        cursor += clip.timeline_duration_ms();
        let name = match clip {
            Clip::Media(c) => format!("the end of clip {}", c.id),
            Clip::Gap(_) => "the end of a gap".to_string(),
            Clip::Caption(c) => format!("the end of caption {}", c.id),
        };
        out.push((cursor, name));
    }
    out
}

/// The edge `at` moves to on `track`, when it falls less than one frame from
/// one (and not on it). The nearer edge wins; past the track's end, its end.
pub(in crate::dispatch) fn snap_point(project: &Project, track: &str, at: u64) -> Option<Snap> {
    let frame = frame_ms(project);
    let edges = edges(project, track);
    let (end, _) = edges.last()?.clone();
    let (to, what) = if at > end {
        (end, "the end of the track".to_string())
    } else {
        let (pos, what) = edges
            .iter()
            .filter(|(p, _)| *p != at)
            .min_by_key(|(p, _)| p.abs_diff(at))?
            .clone();
        if edges.iter().any(|(p, _)| *p == at) {
            return None; // already on an edge
        }
        (pos, what)
    };
    let gap = to.abs_diff(at);
    if gap == 0 || gap as f64 >= frame {
        return None;
    }
    Some(Snap {
        from: at,
        to,
        why: format!(
            "{at} ms is {gap} ms from {what} on track '{track}', less than one frame \
             ({frame:.1} ms at {fps} fps): an edit there would leave a piece shorter than a \
             frame, so the point moved to {to} ms",
            fps = project.settings.fps
        ),
    })
}

/// Snap `args.at_ms` against the first of `track_keys` that names a track of
/// the project (the verb's target track) and write the moved point into the
/// args the op will record.
pub(in crate::dispatch) fn snap_args(
    project: &Project,
    args: &mut Value,
    track_keys: &[&str],
) -> Option<Snap> {
    let at = args.get("at_ms").and_then(Value::as_u64)?;
    let base = cut_core::edl_from_project(project)
        .base_video_track()
        .map(str::to_string);
    let track = track_keys
        .iter()
        .filter_map(|k| match *k {
            BASE_VIDEO => base.as_deref(),
            k => args.get(k).and_then(Value::as_str),
        })
        .find(|t| project.track(t).is_some())?
        .to_string();
    let snap = snap_point(project, &track, at)?;
    args["at_ms"] = json!(snap.to);
    Some(snap)
}

/// [`snap_args`] against the open project (no project: nothing moves; the
/// verb reports that itself).
pub(in crate::dispatch) async fn snap_open_project(
    state: &crate::state::AppState,
    args: &mut Value,
    track_keys: &[&str],
) -> Option<Snap> {
    let guard = state.project.read().await;
    let store = guard.as_ref()?;
    snap_args(&store.project, args, track_keys)
}

/// The verb result with `snapped {from, to, why}` when the point moved.
pub(in crate::dispatch) fn with_snap(mut res: VerbResult, snap: Option<Snap>) -> VerbResult {
    if let Some(s) = snap {
        let note = json!({"from": s.from, "to": s.to, "why": s.why});
        match res.result.as_mut() {
            Some(Value::Object(m)) => {
                m.insert("snapped".into(), note);
            }
            _ => res.result = Some(json!({ "snapped": note })),
        }
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        // v1: clip c1 of 757 ms (1513 ms at 2x), then clip c2 of 1000 ms; 60 fps.
        serde_json::from_value(json!({
            "schema": "shellx-cut/1", "name": "edit-point",
            "settings": {"width": 320, "height": 180, "fps": 60.0, "audio_rate": 48000},
            "assets": {"a1": {"path": "/m/a.mp4", "hash": "sha256:a"}},
            "tracks": [{"id": "v1", "kind": "video", "clips": [
                {"id": "c1", "asset": "a1", "src_in_ms": 8000, "src_out_ms": 9513, "speed": 2.0},
                {"id": "c2", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000}]}],
        }))
        .expect("test project")
    }

    #[test]
    fn a_point_less_than_one_frame_from_an_edge_moves_to_it() {
        let p = project();
        // 1 ms before the sped clip's end (757): the sliver case.
        let s = snap_point(&p, "v1", 756).expect("snaps");
        assert_eq!((s.from, s.to), (756, 757));
        assert!(s.why.contains("clip c1"), "{}", s.why);
        // 16 ms after the start (one frame at 60 fps is 16.7 ms): snaps back to 0.
        assert_eq!(snap_point(&p, "v1", 16).map(|s| s.to), Some(0));
        // 17 ms is a frame away: kept.
        assert_eq!(snap_point(&p, "v1", 17), None);
        // On an edge, in the middle: kept.
        assert_eq!(snap_point(&p, "v1", 757), None);
        assert_eq!(snap_point(&p, "v1", 1200), None);
        // Just past the track's end (1757): to the end; well past: kept (a gap of frames).
        assert_eq!(snap_point(&p, "v1", 1760).map(|s| s.to), Some(1757));
        assert_eq!(snap_point(&p, "v1", 1800), None);
    }

    #[test]
    fn the_recorded_args_carry_the_moved_point_and_the_result_says_so() {
        let p = project();
        let mut args = json!({"track": "v1", "at_ms": 756, "asset": "a1"});
        let snap = snap_args(&p, &mut args, &["track"]);
        assert_eq!(args["at_ms"], json!(757));
        let res = with_snap(VerbResult::ok(json!({"clip_id": "c3"})), snap);
        let r = res.result.unwrap();
        assert_eq!(r["snapped"]["from"], json!(756));
        assert_eq!(r["snapped"]["to"], json!(757));
        assert_eq!(r["clip_id"], json!("c3"));
        // An unknown track: nothing moves (core reports the track).
        let mut other = json!({"track": "v9", "at_ms": 756});
        assert_eq!(snap_args(&p, &mut other, &["track"]), None);
        assert_eq!(other["at_ms"], json!(756));
    }
}
