//! Emphasis-driven auto-zoom (`edit.auto_zoom`) — the PURE core.
//!
//! Role: the dependency-free, deterministic heart of the `edit.auto_zoom` verb —
//! the dynamic short-form / talking-head "punch-in on the important beats" look.
//! Given an audio ENERGY envelope (per-window momentary loudness, mapped to
//! clip-local time) or a list of transcript utterance-START times, it
//!   1. picks the emphasis MOMENTS ([`pick_energy_peaks`] / [`space_times`]), and
//!   2. lowers each one to a `scale` keyframe RAMP ([`build_zoom_points`]) —
//!      `1.0 → 1.0+intensity → 1.0` punches.
//!
//! The ramp lowers onto the EXISTING time-varying-scale path the renderer already
//! honors: `edit.keyframe {param:"scale"}` → a centred `zoompan` (the multi-point,
//! eased generalization of `edit.animate`'s 2-state Ken Burns; see
//! `cut_media::render::scale_kf_zoompan` and the `KfParam::Scale` doc — "the native
//! target the integrated recorder's eased auto-zoom lowers onto"). So `edit.auto_zoom` invents
//! NO new render path: the dispatch layer feeds these functions live perception
//! facts and commits the resulting points as a normal, replay-safe `edit.keyframe`
//! op. Nothing here touches media or I/O, so every branch is unit-tested.
//!
//! Dependencies: only [`crate::types::KfPoint`]. Primary caller:
//! `server::dispatch::edit_auto_zoom`.

use crate::types::KfPoint;
use std::cmp::Ordering;

/// Pick the emphasis PEAKS of an energy envelope (the `trigger:"energy"` path).
///
/// `env` = `(t_ms, energy)` samples ascending by `t_ms` — e.g. per-second momentary
/// LUFS from perception, mapped to clip-local time (louder = larger / less
/// negative). A peak is a STRICT interior local maximum whose energy clears the
/// relative threshold `min + threshold_frac*(max - min)`, so the silence FLOORS
/// (local minima) and a dead-flat envelope never qualify. Candidates are then
/// thinned by NON-MAXIMUM SUPPRESSION: the loudest is taken first, and any later
/// candidate within `min_spacing_ms` of one already taken is dropped (so the
/// punches don't bunch). Returns at most `max_peaks`, ascending by `t_ms`.
///
/// Pure + deterministic (strength ties broken by earlier `t_ms`). A flat, empty,
/// or `< 3`-sample envelope yields `[]` (a clean upstream no-op).
pub fn pick_energy_peaks(
    env: &[(u64, f64)],
    min_spacing_ms: u64,
    max_peaks: usize,
    threshold_frac: f64,
) -> Vec<(u64, f64)> {
    if env.len() < 3 || max_peaks == 0 {
        return Vec::new();
    }
    let (mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(_, v) in env {
        if v < vmin {
            vmin = v;
        }
        if v > vmax {
            vmax = v;
        }
    }
    // Dead-flat (or non-finite) envelope → no emphasis to find.
    if matches!(
        vmax.partial_cmp(&vmin),
        None | Some(Ordering::Less | Ordering::Equal)
    ) {
        return Vec::new();
    }
    let threshold = vmin + threshold_frac.clamp(0.0, 1.0) * (vmax - vmin);
    // Strict interior local maxima above the threshold (endpoints excluded — a
    // punch right at the clip edge reads worse than a clean interior beat).
    let mut cands: Vec<(u64, f64)> = Vec::new();
    for i in 1..env.len() - 1 {
        let (t, v) = env[i];
        if v > threshold && v > env[i - 1].1 && v > env[i + 1].1 {
            cands.push((t, v));
        }
    }
    // NMS: greedy by strength (desc), tie-break earlier time; reject any candidate
    // within min_spacing_ms of one already chosen; stop at max_peaks.
    cands.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    let mut chosen: Vec<(u64, f64)> = Vec::new();
    for c in cands {
        if chosen.len() >= max_peaks {
            break;
        }
        if chosen
            .iter()
            .all(|&(t, _)| t.abs_diff(c.0) >= min_spacing_ms)
        {
            chosen.push(c);
        }
    }
    chosen.sort_by_key(|&(t, _)| t);
    chosen
}

/// Forward-greedy min-spacing of emphasis TIMES (the `trigger:"transcript"` path:
/// utterance-start times in clip-local ms). Keeps the first, then each next time at
/// least `min_spacing_ms` after the last KEPT one, capped at `max_peaks`.
/// Pure; sorts defensively so direct helper use cannot bunch unsorted punches.
pub fn space_times(times: &[u64], min_spacing_ms: u64, max_peaks: usize) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    for t in sorted {
        if out.len() >= max_peaks {
            break;
        }
        match out.last() {
            Some(&last) if t < last.saturating_add(min_spacing_ms) => {}
            _ => out.push(t),
        }
    }
    out
}

/// Build the `scale` keyframe control points for a set of emphasis times.
///
/// Each emphasis time `t` (clip-local ms) becomes a PUNCH-IN: rest `1.0` rising to
/// `1.0+intensity` reached AT `t`, held for `hold_ms`, then eased back to `1.0` —
/// emitted as four points `(t-rise, 1.0) (t, peak) (t+hold, peak) (t+hold+fall, 1.0)`,
/// all clamped to `[0, clip_dur_ms]`. The track RESTS at `1.0` everywhere else: the
/// renderer clamps to the first / last value outside the point range, and both ends
/// of every punch are `1.0`, so a single shared keyframe track of these punches
/// reads as flat-then-punch-then-flat. (The actual ease curve is carried by the
/// keyframe track's `interp`, applied by the dispatch layer.)
///
/// `intensity <= 0` or no peaks → `[]` (a clean flat no-op). Overlapping punch
/// windows merge into one continuous excursion so an earlier fall never drops the
/// scale to rest inside the next punch's rise. Points are returned ascending by
/// `t_ms`; exact-time collisions prefer a chronological fall-end REST over a
/// same-time peak, otherwise the peak wins over a rise-start rest. Pure +
/// deterministic.
pub fn build_zoom_points(
    peaks: &[u64],
    clip_dur_ms: u64,
    intensity: f64,
    rise_ms: u64,
    hold_ms: u64,
    fall_ms: u64,
) -> Vec<KfPoint> {
    if intensity <= 0.0 || peaks.is_empty() || clip_dur_ms == 0 {
        return Vec::new();
    }
    let peak = 1.0 + intensity;
    let clamp_t = |t: i64| -> u64 { t.clamp(0, clip_dur_ms as i64) as u64 };
    let mut sorted: Vec<u64> = peaks.to_vec();
    sorted.sort_unstable();

    #[derive(Clone, Copy)]
    struct Punch {
        start: u64,
        peak_at: u64,
        hold_end: u64,
        end: u64,
    }

    let mut punches: Vec<Punch> = Vec::new();
    for &t in &sorted {
        let ti = t as i64;
        let s = clamp_t(ti - rise_ms as i64); // rise start (rest)
        let a = clamp_t(ti); // full zoom reached AT the beat
        let b = clamp_t(ti + hold_ms as i64); // hold end
        let e = clamp_t(ti + hold_ms as i64 + fall_ms as i64); // back to rest
        punches.push(Punch {
            start: s,
            peak_at: a,
            hold_end: b,
            end: e,
        });
    }

    let mut raw: Vec<(u64, f64, u8)> = Vec::new();
    let mut i = 0usize;
    while i < punches.len() {
        let mut cluster_start = punches[i].start;
        let mut cluster_end = punches[i].end;
        let mut peak_times: Vec<u64> = Vec::new();
        while i < punches.len() {
            let p = punches[i];
            if !peak_times.is_empty() && p.start >= cluster_end {
                break;
            }
            cluster_start = cluster_start.min(p.start);
            cluster_end = cluster_end.max(p.end);
            peak_times.push(p.peak_at);
            peak_times.push(p.hold_end);
            i += 1;
        }
        raw.push((cluster_start, 1.0, 0)); // rise-start rest, lowest priority
        raw.extend(peak_times.into_iter().map(|t| (t, peak, 1)));
        raw.push((cluster_end, 1.0, 2)); // fall-end rest, highest priority
    }

    // Sort by time, then same-time priority: fall-end rest > peak > rise-start rest.
    raw.sort_by(|x, y| {
        x.0.cmp(&y.0)
            .then(y.2.cmp(&x.2))
            .then(y.1.partial_cmp(&x.1).unwrap_or(Ordering::Equal))
    });
    let mut out: Vec<KfPoint> = Vec::new();
    for (t, v, _) in raw {
        if let Some(last) = out.last_mut() {
            if last.t_ms == t {
                continue;
            }
        }
        out.push(KfPoint { t_ms: t, value: v });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a fine (every 200ms) synthetic envelope at a flat baseline, then poke
    /// in single-sample spikes — each spike, surrounded by the baseline, is a strict
    /// local maximum by construction.
    fn envelope(baseline: f64, spikes: &[(u64, f64)], span_ms: u64) -> Vec<(u64, f64)> {
        let mut env: Vec<(u64, f64)> = (0..=span_ms / 200).map(|i| (i * 200, baseline)).collect();
        for &(t, v) in spikes {
            let idx = (t / 200) as usize;
            env[idx].1 = v;
        }
        env
    }

    #[test]
    fn peaks_two_clear_one_rejected_by_spacing() {
        // Spikes at 2000 (1.0), 2600 (0.8 — 600ms from 2000), 8000 (0.9). With a
        // 1000ms min-spacing the 2600 spike is suppressed by the stronger 2000 one;
        // 8000 is far enough to survive. So exactly the 2000 + 8000 punches.
        let env = envelope(0.2, &[(2000, 1.0), (2600, 0.8), (8000, 0.9)], 10000);
        let peaks = pick_energy_peaks(&env, 1000, 5, 0.5);
        assert_eq!(peaks.len(), 2, "expected two peaks, got {peaks:?}");
        assert_eq!(peaks[0].0, 2000);
        assert_eq!(peaks[1].0, 8000);
        // Returned strongest-set, ascending by time, with the right energies.
        assert!((peaks[0].1 - 1.0).abs() < 1e-9);
        assert!((peaks[1].1 - 0.9).abs() < 1e-9);
    }

    #[test]
    fn peaks_flat_envelope_none() {
        let env = envelope(0.5, &[], 10000); // no spikes → dead flat
        assert!(pick_energy_peaks(&env, 1000, 5, 0.5).is_empty());
    }

    #[test]
    fn peaks_respect_max_cap_taking_strongest() {
        // Three well-spaced spikes; max_peaks=2 keeps the two STRONGEST (1.0, 0.9),
        // dropping the weakest (0.7), and returns them ascending by time.
        let env = envelope(0.2, &[(2000, 0.7), (6000, 1.0), (10000, 0.9)], 12000);
        let peaks = pick_energy_peaks(&env, 1000, 2, 0.5);
        assert_eq!(
            peaks.iter().map(|p| p.0).collect::<Vec<_>>(),
            vec![6000, 10000]
        );
    }

    #[test]
    fn ramp_builder_shape() {
        // One peak at 5000 in a 10s clip → a 1.0 → 1.12 → 1.12 → 1.0 trapezoid with
        // the full zoom reached AT the beat and the hold/fall after it.
        let pts = build_zoom_points(&[5000], 10000, 0.12, 280, 500, 420);
        let got: Vec<(u64, f64)> = pts.iter().map(|p| (p.t_ms, p.value)).collect();
        assert_eq!(
            got,
            vec![(4720, 1.0), (5000, 1.12), (5500, 1.12), (5920, 1.0)]
        );
    }

    #[test]
    fn ramp_builder_intensity_zero_is_flat_noop() {
        assert!(build_zoom_points(&[5000], 10000, 0.0, 280, 500, 420).is_empty());
    }

    #[test]
    fn ramp_builder_no_peaks_is_empty() {
        assert!(build_zoom_points(&[], 10000, 0.12, 280, 500, 420).is_empty());
    }

    #[test]
    fn ramp_builder_clamps_to_clip_bounds() {
        // A peak near the clip end: the fall would run past clip_dur and clamps to it,
        // and the rise/peak stay inside. Times never exceed clip_dur, values stay sane.
        let pts = build_zoom_points(&[9800], 10000, 0.12, 280, 500, 420);
        assert!(pts.iter().all(|p| p.t_ms <= 10000));
        assert!(pts.iter().all(|p| p.value >= 1.0 && p.value <= 1.12 + 1e-9));
        // The full-zoom point sits at the beat, the trailing rest clamps to the end.
        assert!(pts
            .iter()
            .any(|p| p.t_ms == 9800 && (p.value - 1.12).abs() < 1e-9));
        assert_eq!(pts.last().unwrap().t_ms, 10000);
        assert_eq!(
            pts.last().unwrap().value,
            1.0,
            "a clipped fall must end at rest, not freeze on the peak"
        );
    }

    #[test]
    fn ramp_builder_merges_overlapping_punch_windows() {
        let pts = build_zoom_points(&[5000, 5600], 10000, 0.12, 280, 500, 420);
        let got: Vec<(u64, f64)> = pts.iter().map(|p| (p.t_ms, p.value)).collect();

        assert!(
            got.iter()
                .filter(|(t, _)| *t > 4720 && *t < 6520)
                .all(|(_, v)| (*v - 1.12).abs() < 1e-9),
            "overlapping punches must not inject an interior rest point: {got:?}"
        );
        assert_eq!(got.first(), Some(&(4720, 1.0)));
        assert_eq!(got.last(), Some(&(6520, 1.0)));
    }

    #[test]
    fn space_times_forward_greedy_with_cap() {
        let times = [0, 1000, 5000, 5500, 9000];
        // min-spacing 4000: keep 0, drop 1000, keep 5000, drop 5500, keep 9000;
        // cap 3 keeps the first three kept.
        assert_eq!(space_times(&times, 4000, 3), vec![0, 5000, 9000]);
        // Tighter cap.
        assert_eq!(space_times(&times, 4000, 2), vec![0, 5000]);
    }

    #[test]
    fn space_times_sorts_defensively_before_spacing() {
        let times = [5000, 0, 5500, 9000, 1000];
        assert_eq!(space_times(&times, 4000, 3), vec![0, 5000, 9000]);
    }
}
