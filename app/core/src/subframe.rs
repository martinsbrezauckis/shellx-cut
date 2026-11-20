//! A media clip is at least one frame long.
//!
//! Large editors keep every edit on the frame grid, so no clip can be shorter
//! than a frame. ShellX Cut edits in milliseconds: a trim, an inserted source
//! range, an overwrite or range edge, a speed change or a copy of an existing
//! sliver could leave a clip of a few milliseconds that shows no picture and
//! cannot be seen in the timeline. A live edit (`ProjectStore::apply`,
//! `apply_lowered`) that would leave a NEW clip shorter than one frame (a clip
//! that did not exist before, or one that was at least a frame long) is
//! refused with the reason and the nearest valid value. Replay of an op log
//! does not pass here: an older log replays as it was recorded, and a clip
//! that was already shorter than a frame stays as it is (the timeline draws it
//! as a marker; `project.state` lists it under `subframe_clips`).

use crate::error::{codes, CutError};
use crate::types::{Clip, Project};
use serde_json::{json, Value};
use std::collections::HashMap;

/// One frame of the project, in milliseconds.
pub fn frame_ms(project: &Project) -> f64 {
    let fps = project.settings.fps;
    if fps.is_finite() && fps > 0.0 {
        1000.0 / fps
    } else {
        1000.0 / 30.0
    }
}

/// A media clip shorter than one frame: `(clip, track, at_ms, duration_ms)`.
pub type Short = (String, String, u64, u64);

/// Every media clip of the project shorter than one frame, in track order.
pub fn short_clips(project: &Project) -> Vec<Short> {
    let frame = frame_ms(project);
    let mut out = Vec::new();
    for t in &project.tracks {
        let mut at = 0u64;
        for c in &t.clips {
            let d = c.timeline_duration_ms();
            if let Clip::Media(m) = c {
                if (d as f64) < frame - 1e-9 {
                    out.push((m.id.clone(), t.id.clone(), at, d));
                }
            }
            at += d;
        }
    }
    out
}

/// `subframe_clips` for `project.state`: the clips shorter than one frame,
/// or `None` when there is none.
pub fn report(project: &Project) -> Option<Value> {
    let short = short_clips(project);
    (!short.is_empty()).then(|| {
        Value::Array(
            short
                .into_iter()
                .map(|(clip, track, at, d)| {
                    json!({"clip": clip, "track": track, "at_ms": at, "duration_ms": d})
                })
                .collect(),
        )
    })
}

/// Refuse a live `verb` whose result `after` holds a media clip shorter than
/// one frame that `before` did not (new, or shrunk below a frame).
pub fn guard(before: &Project, after: &Project, verb: &str, args: &Value) -> Result<(), CutError> {
    if !verb.starts_with("edit.") || verb == "edit.restore" {
        return Ok(());
    }
    let was: HashMap<String, u64> = short_clips(before)
        .into_iter()
        .map(|(c, _, _, d)| (c, d))
        .collect();
    let Some((clip, _track, at, d)) = short_clips(after)
        .into_iter()
        .find(|(c, ..)| !was.contains_key(c))
    else {
        return Ok(());
    };
    let frame = frame_ms(after);
    let fps = after.settings.fps;
    Err(CutError::new(
        codes::INVALID_ARGS,
        format!(
            "{verb} would leave clip {clip} {d} ms long at {at} ms; a clip must be at least one \
             frame: {frame:.1} ms at {fps} fps"
        ),
        "large editors keep every edit on the frame grid; a clip shorter than a frame shows no \
         picture and cannot be seen in the timeline",
    )
    .with_clip(&clip)
    .with_suggested_action(nearest(after, verb, args, &clip, frame)))
}

/// The nearest valid value for the edit that would leave `clip` too short.
fn nearest(after: &Project, verb: &str, args: &Value, clip: &str, frame: f64) -> String {
    let media = after
        .find_clip(clip)
        .and_then(|(t, i)| match &after.track(t)?.clips[i] {
            Clip::Media(m) => Some(m.clone()),
            _ => None,
        });
    let speed = media
        .as_ref()
        .map(|m| m.speed)
        .filter(|s| s.is_finite() && *s > 0.0);
    // The shortest source span that plays for one frame at the clip's speed.
    let span = (frame * speed.unwrap_or(1.0)).ceil() as u64;
    let n = |key: &str| args.get(key).and_then(Value::as_u64);
    match verb {
        "edit.trim" => match (media, n("src_in_ms"), n("src_out_ms")) {
            (Some(m), _, Some(_)) => format!(
                "use src_out_ms {} or more (the clip starts at {} ms of its source)",
                m.src_in_ms + span,
                m.src_in_ms
            ),
            (Some(m), Some(_), None) => format!(
                "use src_in_ms {} or less (the clip ends at {} ms of its source)",
                m.src_out_ms.saturating_sub(span),
                m.src_out_ms
            ),
            _ => format!("keep at least {span} ms of source in the clip"),
        },
        "edit.speed" => match media {
            Some(m) => format!(
                "use a factor of at most {:.3} for this clip ({} ms of source)",
                (m.src_out_ms - m.src_in_ms) as f64 / frame,
                m.src_out_ms - m.src_in_ms
            ),
            None => "use a lower speed factor".into(),
        },
        "edit.duplicate" | "edit.paste"
            if media
                .as_ref()
                .is_some_and(|m| m.src_out_ms - m.src_in_ms < span) =>
        {
            format!(
                "clip {clip} is itself shorter than one frame: lengthen it with edit.trim (at least \
                 {span} ms of source) or delete it"
            )
        }
        "edit.insert" | "edit.insert_linked" | "edit.paste" | "edit.overwrite" | "edit.replace"
        | "edit.fit_to_fill" => match n("duration_ms") {
            Some(_) => format!("use a duration of at least {} ms", frame.ceil() as u64),
            None => {
                format!("use a source range of at least {span} ms (src_range_ms [in, in + {span}])")
            }
        },
        _ => format!(
            "move the edit at least one frame ({:.1} ms) away from the clip's edge, or onto it",
            frame
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(clips: Value) -> Project {
        serde_json::from_value(json!({
            "schema": "shellx-cut/1", "name": "subframe",
            "settings": {"width": 320, "height": 180, "fps": 30.0, "audio_rate": 48000},
            "assets": {"a1": {"path": "/m/a.mp4", "hash": "sha256:a"}},
            "tracks": [{"id": "v1", "kind": "video", "clips": clips}],
        }))
        .expect("test project")
    }

    #[test]
    fn a_new_clip_shorter_than_a_frame_is_refused_with_the_nearest_value() {
        let before =
            project(json!([{"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 1000}]));
        let after = project(json!([{"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 20}]));
        let e = guard(
            &before,
            &after,
            "edit.trim",
            &json!({"clip": "c1", "src_out_ms": 20}),
        )
        .unwrap_err();
        assert!(
            e.message.contains("at least one frame: 33.3 ms at 30 fps"),
            "{e:?}"
        );
        assert!(
            e.suggested_action
                .as_deref()
                .unwrap()
                .contains("src_out_ms 34"),
            "{e:?}"
        );
        // 34 ms is a frame: fine.
        let ok = project(json!([{"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 34}]));
        assert!(guard(&before, &ok, "edit.trim", &json!({})).is_ok());
    }

    #[test]
    fn an_existing_sliver_is_listed_and_left_alone() {
        let p = project(json!([
            {"id": "c1", "asset": "a1", "src_in_ms": 0, "src_out_ms": 2000},
            {"id": "c2", "asset": "a1", "src_in_ms": 9000, "src_out_ms": 9001}]));
        assert_eq!(
            report(&p),
            Some(json!([{"clip": "c2", "track": "v1", "at_ms": 2000, "duration_ms": 1}]))
        );
        // An edit elsewhere keeps the old sliver: allowed.
        assert!(guard(&p, &p, "edit.gain", &json!({})).is_ok());
        // Not an edit verb (replay, restore): never checked.
        let empty = project(json!([]));
        assert!(guard(&empty, &p, "edit.restore", &json!({})).is_ok());
        // A copy of it is new: refused, and told to lengthen or delete it.
        let mut copy = p.clone();
        let c3 = copy.tracks[0].clips[1].clone();
        if let Clip::Media(mut m) = c3 {
            m.id = "c3".into();
            copy.tracks[0].clips.push(Clip::Media(m));
        }
        let e = guard(&p, &copy, "edit.duplicate", &json!({"clip": "c2"})).unwrap_err();
        assert!(e
            .suggested_action
            .unwrap()
            .contains("itself shorter than one frame"));
    }
}
