//! Rebase clip-local keys without restarting a clipped easing curve.
use crate::{Keyframe, KfPoint};

/// An eased segment's original preceding control point, before local zero.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LeadIn {
    pub t_ms: i64,
    pub value: f64,
}

/// Effective control points. The visible zero key is the evaluated cut value;
/// its lead-in replaces it for interpolation, retaining the original phase.
pub fn points(key: &Keyframe) -> Vec<(i64, f64)> {
    let mut points: Vec<_> = key
        .points
        .iter()
        .map(|p| (p.t_ms as i64, p.value))
        .collect();
    if let (Some(first), Some(lead)) = (points.first_mut(), &key.lead_in) {
        *first = (lead.t_ms, lead.value);
    }
    points
}

fn evaluate(points: &[(i64, f64)], interp: crate::KfInterp, at: i64) -> f64 {
    if at < points[0].0 {
        return points[0].1;
    }
    for pair in points.windows(2) {
        if at < pair[1].0 {
            return pair[0].1
                + (pair[1].1 - pair[0].1)
                    * interp.sample((at - pair[0].0) as f64 / (pair[1].0 - pair[0].0) as f64);
        }
    }
    points.last().unwrap().1
}

/// Positive offsets remove a beginning; negative offsets extend it. A new zero
/// key replaces at least one preceding point, so even a 64-point track fits.
pub fn shifted(keys: &[Keyframe], offset: i64) -> Vec<Keyframe> {
    if offset == 0 {
        return keys.to_vec();
    }
    keys.iter()
        .map(|key| {
            if key.points.is_empty() {
                return key.clone();
            }
            let mut pts = points(key);
            for (time, _) in &mut pts {
                *time -= offset;
            }
            let mut out = key.clone();
            out.lead_in = None;
            let before = pts.iter().rposition(|(t, _)| *t <= 0);
            out.points = pts
                .iter()
                .filter(|(t, _)| *t > 0)
                .map(|(t, v)| KfPoint {
                    t_ms: *t as u64,
                    value: *v,
                })
                .collect();
            if let Some(i) = before {
                let value = evaluate(&pts, key.interp, 0);
                out.points.insert(0, KfPoint { t_ms: 0, value });
                if key.interp.is_eased() && pts[i].0 < 0 && i + 1 < pts.len() {
                    out.lead_in = Some(LeadIn {
                        t_ms: pts[i].0,
                        value: pts[i].1,
                    });
                }
            }
            out
        })
        .collect()
}

pub(crate) fn retime_lead_in(key: &mut Keyframe, old: u64, new: u64) {
    if let Some(lead) = &mut key.lead_in {
        lead.t_ms = ((lead.t_ms as f64 * new as f64 / old as f64).round() as i64).min(-1);
    }
}

pub(crate) fn trim_in(clip: &mut crate::types::MediaClip, new_in: u64) {
    let delta = if new_in >= clip.src_in_ms {
        crate::types::src_off_to_tl(new_in - clip.src_in_ms, clip.speed) as i64
    } else {
        -(crate::types::src_off_to_tl(clip.src_in_ms - new_in, clip.speed) as i64)
    };
    clip.keyframes = shifted(&clip.keyframes, delta);
}
