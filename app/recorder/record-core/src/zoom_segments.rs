//! Editable source-clock zooms. A zero ramp-out joins the next segment with a
//! pan: its ramp-in starts at the preceding hold's end, not at neutral scale.
use crate::{error_codes, Ease, RecordError, Result, ZoomKey, ZoomTrack};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoomOrigin {
    Auto,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoomSegment {
    pub id: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub scale: f64,
    pub cx: f64,
    pub cy: f64,
    pub ramp_in_ms: u64,
    pub ramp_out_ms: u64,
    pub origin: ZoomOrigin,
}

fn bad(message: &str) -> RecordError {
    RecordError::new(error_codes::INVALID_ARGS, "invalid zoom", message)
}

/// Validate and clamp targets, returning a concrete replay-safe list.
pub fn normalize_segments(segments: &[ZoomSegment]) -> Result<Vec<ZoomSegment>> {
    let mut result = segments.to_vec();
    let mut previous_end = 0;
    let mut ids = std::collections::BTreeSet::new();
    for s in &mut result {
        if s.id.is_empty() || !ids.insert(s.id.clone()) {
            return Err(bad("segment ids must be nonempty and unique"));
        }
        if s.start_ms < previous_end || s.end_ms <= s.start_ms {
            return Err(bad(
                "segments must be ordered, positive length and not overlap",
            ));
        }
        if !s.scale.is_finite()
            || !(1.0..=8.0).contains(&s.scale)
            || !s.cx.is_finite()
            || !s.cy.is_finite()
        {
            return Err(bad("scale must be in [1,8] and targets finite"));
        }
        let length = s.end_ms - s.start_ms;
        if s.ramp_in_ms > length || s.ramp_out_ms > length - s.ramp_in_ms {
            return Err(bad("ramps must fit inside the segment"));
        }
        let half = 0.5 / s.scale;
        s.cx = s.cx.clamp(half, 1.0 - half);
        s.cy = s.cy.clamp(half, 1.0 - half);
        previous_end = s.end_ms;
    }
    Ok(result)
}

/// Convert the engine's canonical in/hold/out key layout without quantizing
/// times or coordinates. Pan segments share a boundary and have zero ramp-out.
/// Arbitrary hand-authored key tracks are rejected rather than silently lost.
pub fn keys_to_segments(track: &ZoomTrack) -> Result<Vec<ZoomSegment>> {
    if track.keys.is_empty() {
        return Ok(Vec::new());
    }
    let keys = &track.keys;
    if keys[0].t_ms != 0 || keys[0].scale != 1.0 {
        return Err(bad("expected an initial neutral key"));
    }
    // max_zoom=1 is legal: the engine can omit out keys at stay-zoomed
    // boundaries even though every key is neutral. Preserve those timestamps
    // in neutral segments rather than guessing nonexistent peak boundaries.
    if keys
        .iter()
        .all(|k| k.scale == 1.0 && k.cx == 0.5 && k.cy == 0.5)
    {
        let mut segments = Vec::new();
        let mut offset = 1;
        while offset + 1 < keys.len() {
            let chunk = &keys[offset..(offset + 4).min(keys.len())];
            let start = chunk[0].t_ms;
            let peak = chunk[1].t_ms;
            let hold = chunk.get(2).unwrap_or(&chunk[1]).t_ms;
            let end = chunk.last().unwrap().t_ms;
            segments.push(ZoomSegment {
                id: format!("z{}", segments.len() + 1),
                start_ms: start,
                end_ms: end,
                scale: 1.0,
                cx: 0.5,
                cy: 0.5,
                ramp_in_ms: peak.saturating_sub(start),
                ramp_out_ms: end.saturating_sub(hold),
                origin: ZoomOrigin::Auto,
            });
            offset += chunk.len();
        }
        if segments_to_keys(&segments, keys.last().unwrap().t_ms)? != *track {
            return Err(bad("neutral zoom keys are not in canonical engine layout"));
        }
        return Ok(segments);
    }
    let mut result = Vec::new();
    let mut i = 1;
    let mut pan = false;
    while i + 1 < keys.len() {
        let start = if pan {
            keys[i - 1].t_ms
        } else {
            let t = keys[i].t_ms;
            i += 1;
            t
        };
        if i + 1 >= keys.len() {
            return Err(bad("missing zoom hold"));
        }
        let peak = keys[i];
        let hold = keys[i + 1];
        i += 2;
        let has_out = keys.get(i).is_some_and(|k| k.scale == 1.0);
        let end = if has_out {
            let t = keys[i].t_ms;
            i += 1;
            t
        } else {
            hold.t_ms
        };
        result.push(ZoomSegment {
            id: format!("z{}", result.len() + 1),
            start_ms: start,
            end_ms: end,
            scale: peak.scale,
            cx: peak.cx,
            cy: peak.cy,
            ramp_in_ms: peak.t_ms.saturating_sub(start),
            ramp_out_ms: end.saturating_sub(hold.t_ms),
            origin: ZoomOrigin::Auto,
        });
        pan = !has_out;
    }
    // Round-trip checking also rejects layouts not representable by segments.
    let duration = keys.last().map_or(0, |k| k.t_ms);
    if segments_to_keys(&result, duration)? != *track {
        return Err(bad("zoom keys are not in canonical engine layout"));
    }
    Ok(result)
}

/// Rebuild canonical keys. The duration adds the engine's final neutral key;
/// it does not truncate ramps extending beyond the final source frame.
pub fn segments_to_keys(segments: &[ZoomSegment], duration_ms: u64) -> Result<ZoomTrack> {
    let segments = normalize_segments(segments)?;
    if segments.is_empty() {
        return Ok(ZoomTrack::default());
    }
    let mut keys = vec![key(0, 1.0, 0.5, 0.5)];
    let mut pan = false;
    for (i, s) in segments.iter().enumerate() {
        if !pan {
            keys.push(key(s.start_ms, 1.0, s.cx, s.cy));
        }
        keys.push(key(s.start_ms + s.ramp_in_ms, s.scale, s.cx, s.cy));
        keys.push(key(s.end_ms - s.ramp_out_ms, s.scale, s.cx, s.cy));
        pan = segments.get(i + 1).is_some_and(|next| {
            (s.ramp_out_ms == 0 && next.start_ms == s.end_ms)
                || ((s.origin == ZoomOrigin::Manual || next.origin == ZoomOrigin::Manual)
                    && next.start_ms.saturating_sub(s.end_ms) <= 2500)
        });
        if !pan {
            keys.push(key(s.end_ms, 1.0, s.cx, s.cy));
        }
    }
    if duration_ms > keys.last().unwrap().t_ms {
        keys.push(key(duration_ms, 1.0, 0.5, 0.5));
    }
    // Zero-length ramps are instantaneous edits. Keep a single value at the
    // boundary, rather than publishing duplicate times rejected by the plan.
    let mut ordered: Vec<ZoomKey> = Vec::with_capacity(keys.len());
    for key in keys {
        if ordered.last().is_some_and(|last| last.t_ms == key.t_ms) {
            *ordered.last_mut().unwrap() = key;
        } else {
            ordered.push(key);
        }
    }
    Ok(ZoomTrack { keys: ordered })
}

fn key(t_ms: u64, scale: f64, cx: f64, cy: f64) -> ZoomKey {
    ZoomKey {
        t_ms,
        scale,
        cx,
        cy,
        ease: Ease::EaseInOut,
    }
}
