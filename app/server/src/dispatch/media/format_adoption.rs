//! "New timeline from file": the first placed video sets the project format.
//!
//! When the first import becomes the timeline and the project still has the
//! built-in default format (1920x1080 at 30 fps, never chosen by anyone), the
//! project takes the clip's size and frame rate, recorded as a `project.format`
//! op by the system actor. Every outcome is reported in the import job result
//! as `format`, so nothing about it is silent:
//! - adopted: the size and rate, rounded to even sides when the clip has odd
//!   ones, and a note when the clip is variable frame rate;
//! - kept: why (the format was chosen before, or the clip is larger than the
//!   largest frame Cut makes; then also a job warning).

use super::*;

/// Decide and apply the format adoption for the first placed video. Returns
/// the `format` record for the import job result and an optional warning.
pub(super) async fn adopt_first_clip_format(
    state: &AppState,
    probed: &cut_media::MediaProbe,
) -> (Value, Option<String>) {
    let (Some(w), Some(h)) = (probed.width, probed.height) else {
        return (Value::Null, None);
    };
    let rate = cut_media::probe::rate::timeline_rate(probed)
        .filter(|(fps, _)| *fps > 0.0 && *fps <= 240.0);
    let mut guard = state.project.write().await;
    let Some(store) = guard.as_mut() else {
        return (Value::Null, None);
    };
    let st = &store.project.settings;
    let is_default = st.width == 1920
        && st.height == 1080
        && (st.fps - 30.0).abs() < 0.01
        && st.audio_rate == 48_000;
    // Even sides: every export is 4:2:0 video.
    let (ew, eh) = (w - w % 2, h - h % 2);
    let target_fps = rate.as_ref().map(|(f, _)| *f).unwrap_or(st.fps);
    let differs = ew != st.width || eh != st.height || (target_fps - st.fps).abs() >= 0.01;
    let rate_note = rate.as_ref().and_then(|(_, note)| note.clone());
    if !is_default {
        return (
            json!({
                "adopted": false,
                "reason": "The project format was chosen before this import, so it was kept.",
            }),
            None,
        );
    }
    if !differs {
        return (
            json!({"adopted": true, "unchanged": true, "width": ew, "height": eh, "fps": target_fps}),
            None,
        );
    }
    if let Err(e) = cut_core::frame_size::check_project_frame_size(ew, eh) {
        let reason = format!(
            "The project stays at {}x{} because the clip ({w}x{h}) is larger than the largest frame Cut makes ({}). Exports use the project size unless you choose another.",
            st.width,
            st.height,
            cut_core::frame_size::FRAME_LIMIT_TEXT
        );
        return (
            json!({"adopted": false, "reason": reason, "limit": e.message}),
            Some(reason),
        );
    }
    let mut notes: Vec<String> = Vec::new();
    if (ew, eh) != (w, h) {
        notes.push(format!(
            "The clip is {w}x{h}; the project uses {ew}x{eh} because video frames need even sides."
        ));
    }
    if let Some(note) = rate_note {
        notes.push(note);
    }
    match store.set_format(
        Some(ew),
        Some(eh),
        rate.as_ref().map(|(f, _)| *f),
        Actor::system(),
        Some("new timeline from file: adopt the first clip's resolution + frame rate".into()),
    ) {
        Ok(op) => {
            state.events.publish(Event::OpApplied { op });
            let mut record = json!({
                "adopted": true,
                "width": ew,
                "height": eh,
                "fps": target_fps,
                "variable_frame_rate": probed.variable_frame_rate,
            });
            if !notes.is_empty() {
                record["note"] = json!(notes.join(" "));
            }
            (record, None)
        }
        Err(e) => {
            let reason = format!(
                "The project format could not be set from the clip: {} ({}).",
                e.message, e.cause
            );
            (json!({"adopted": false, "reason": reason}), Some(reason))
        }
    }
}
