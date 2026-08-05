//! multicam.rs — active-speaker multicam SWITCHING (`edit.multicam_switch`), PURE core.
//!
//! Role: the dependency-free, deterministic heart of `edit.multicam_switch` — the
//! interview / podcast "cut to whoever is talking" multicam edit. Given a per-window
//! audio-ENERGY timeline for ≥2 synced camera angles (each window carries one energy
//! sample per camera — the louder camera is the active speaker), it decides which
//! camera is ON SCREEN over time and emits the ordered SHOT list:
//!   1. per window, the loudest camera wins, with a small ENERGY HYSTERESIS so two
//!      near-equal levels don't flicker ([`switch_shots`] step 1);
//!   2. adjacent same-camera windows merge into runs, and any run shorter than
//!      `min_shot_ms` is DISSOLVED into its neighbour ([`dissolve_short_runs`]) so a
//!      brief loud blip never produces a sub-shot — the result has no cut faster than
//!      `min_shot_ms`.
//!
//! The dispatch layer ([`server::dispatch::edit_multicam_switch`]) feeds these
//! functions live perception facts (each camera clip's `Loudness.windows` envelope,
//! the SAME audio facts `edit.duck` / `edit.auto_zoom` consume, mapped to timeline
//! coordinates) and then LOWERS each shot to a plain, replay-safe `edit.insert` of
//! the active angle's segment onto a single PROGRAM video track — so exactly one
//! angle shows at a time and replay needs no perception. Nothing here touches media
//! or I/O, so every branch is unit-tested.
//!
//! Dependencies: none (std only). Primary caller: `server::dispatch::edit_multicam_switch`.

/// One contiguous SHOT of the switched program: the camera `camera` is on screen
/// over the timeline range `[start_ms, end_ms)`. `energy` is the active camera's
/// MEAN energy over the shot's windows (the metric that won it the shot — momentary
/// LUFS in practice; larger / less-negative = louder). `camera` is an index into the
/// caller's camera-track list.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    pub start_ms: u64,
    pub end_ms: u64,
    pub camera: usize,
    pub energy: f64,
}

/// A window-index run of one camera: `[start, end)` windows, camera `cam`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    start: usize,
    end: usize,
    cam: usize,
}

/// Group a per-window camera assignment into contiguous [`Run`]s.
fn runs_of(assign: &[usize]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut i = 0usize;
    while i < assign.len() {
        let cam = assign[i];
        let mut j = i + 1;
        while j < assign.len() && assign[j] == cam {
            j += 1;
        }
        runs.push(Run {
            start: i,
            end: j,
            cam,
        });
        i = j;
    }
    runs
}

/// Coalesce adjacent runs that share a camera (e.g. after a dissolution rewrites a
/// run's camera so it now matches a neighbour). In-place; preserves order.
fn coalesce(runs: &mut Vec<Run>) {
    let mut out: Vec<Run> = Vec::new();
    for r in runs.drain(..) {
        match out.last_mut() {
            Some(last) if last.cam == r.cam => last.end = r.end,
            _ => out.push(r),
        }
    }
    *runs = out;
}

/// Dissolve every run shorter than `min_windows` into a neighbour so the final shot
/// list has NO run below the minimum-shot floor (the anti-flicker / no-sub-second-cut
/// guarantee). Greedy + deterministic: repeatedly take the SHORTEST sub-minimum run,
/// fold it into its LONGER adjacent neighbour (ties → the LEFT neighbour, the earlier
/// shot), re-coalesce, and repeat. A run with only one neighbour folds into it; the
/// loop stops once a single run remains (a program shorter than one shot is one shot
/// — clean, not an error) or all runs clear the floor. `min_windows >= 1`.
fn dissolve_short_runs(mut runs: Vec<Run>, min_windows: usize) -> Vec<Run> {
    loop {
        if runs.len() <= 1 {
            break;
        }
        // The shortest run still below the floor (earliest on a length tie).
        let mut idx: Option<usize> = None;
        let mut best_len = usize::MAX;
        for (k, r) in runs.iter().enumerate() {
            let len = r.end - r.start;
            if len < min_windows && len < best_len {
                best_len = len;
                idx = Some(k);
            }
        }
        let Some(k) = idx else { break };

        let left_len = (k > 0).then(|| runs[k - 1].end - runs[k - 1].start);
        let right_len = (k + 1 < runs.len()).then(|| runs[k + 1].end - runs[k + 1].start);
        // Fold into the LONGER neighbour (the more-established shot); ties / a single
        // neighbour resolve to that side. The folded windows TAKE the neighbour's
        // camera (the blip is absorbed into the surrounding angle).
        let merge_left = match (left_len, right_len) {
            (Some(l), Some(r)) => l >= r,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break, // unreachable while runs.len() > 1
        };
        if merge_left {
            runs[k - 1].end = runs[k].end;
            runs.remove(k);
        } else {
            runs[k + 1].start = runs[k].start;
            runs.remove(k);
        }
        coalesce(&mut runs);
    }
    runs
}

/// The active-speaker SWITCH PROGRAM: from a per-window energy timeline, decide which
/// camera is on screen over time and return the ordered [`Shot`] list.
///
/// `energies` is `[window][camera]` — `energies[w][c]` is camera `c`'s audio energy in
/// window `w` (rectangular: every window carries one sample per camera; a camera with
/// no coverage in a window should pass `f64::NEG_INFINITY` so it never wins). Window
/// `w` spans `[start_ms + w*window_ms, start_ms + (w+1)*window_ms)`.
///
/// Decision (pure, deterministic):
/// 1. WINNER + HYSTERESIS — scanning windows, the running camera holds unless another
///    camera leads it by MORE than `hysteresis` (in energy units); a tie / a margin
///    within `hysteresis` keeps the current camera, so two near-equal levels don't
///    flicker. The scan opens on `default_cam` (the reference/anchor angle).
/// 2. MIN-SHOT MERGE — adjacent same-camera windows merge into shots, then any shot
///    shorter than `min_shot_ms` is dissolved into a neighbour (see
///    [`dissolve_short_runs`]) so a brief loud blip is absorbed rather than cut to.
///
/// Each shot's `energy` is the active camera's mean energy over its windows. A single
/// dominant speaker → ONE shot (no switches — clean, not an error). Empty input or a
/// zero window/camera count → `[]`.
pub fn switch_shots(
    energies: &[Vec<f64>],
    start_ms: u64,
    window_ms: u64,
    min_shot_ms: u64,
    hysteresis: f64,
    default_cam: usize,
) -> Vec<Shot> {
    let n = energies.len();
    if n == 0 || window_ms == 0 {
        return Vec::new();
    }
    let n_cams = energies[0].len();
    if n_cams == 0 {
        return Vec::new();
    }
    let default_cam = default_cam.min(n_cams - 1);

    // Step 1 — per-window winner with energy hysteresis vs the running camera.
    let mut assign: Vec<usize> = Vec::new();
    let mut cur = default_cam;
    for row in energies {
        let visible_cams = row.len().min(n_cams);
        if visible_cams == 0 {
            assign.push(cur);
            continue;
        }
        // Loudest camera this window (tie → lowest index).
        let mut leader = 0usize;
        for c in 1..visible_cams {
            if row[c] > row[leader] {
                leader = c;
            }
        }
        if leader != cur {
            let cur_e = row.get(cur).copied().unwrap_or(f64::NEG_INFINITY);
            if row[leader] > cur_e + hysteresis {
                cur = leader;
            }
        }
        assign.push(cur);
    }

    // Step 2 — merge into runs, dissolve sub-minimum-shot runs, emit shots in ms.
    shots_from_assignment(&assign, energies, start_ms, window_ms, min_shot_ms)
}

/// Lower a per-window camera assignment into the ordered [`Shot`] list: merge
/// adjacent same-camera windows into runs, dissolve every run shorter than the
/// `min_shot_ms` floor (the anti-flicker guarantee), and emit each surviving run as
/// a `Shot` carrying the active camera's MEAN energy over its windows. Shared by the
/// energy program ([`switch_shots`]) and the speaker program
/// ([`switch_shots_by_speaker`]) so BOTH paths inherit the identical, unit-tested
/// run/dissolve/min-shot machinery and the same `[start_ms,end_ms)` window math.
/// `assign[w]` is the winning camera index for window `w`; `energies[w][c]` supplies
/// the per-window energy for the `Shot.energy` summary only (NOT the winner choice).
pub fn shots_from_assignment(
    assign: &[usize],
    energies: &[Vec<f64>],
    start_ms: u64,
    window_ms: u64,
    min_shot_ms: u64,
) -> Vec<Shot> {
    if assign.is_empty() || window_ms == 0 {
        return Vec::new();
    }
    let min_windows = min_shot_ms.div_ceil(window_ms).max(1) as usize;
    let runs = dissolve_short_runs(runs_of(assign), min_windows);
    runs.into_iter()
        .map(|r| {
            let mut sum = 0.0;
            let mut cnt = 0u64;
            for row in energies.get(r.start..r.end).unwrap_or(&[]) {
                if let Some(&v) = row.get(r.cam) {
                    if v.is_finite() {
                        sum += v;
                        cnt += 1;
                    }
                }
            }
            Shot {
                start_ms: start_ms + r.start as u64 * window_ms,
                end_ms: start_ms + r.end as u64 * window_ms,
                camera: r.cam,
                energy: if cnt > 0 {
                    sum / cnt as f64
                } else {
                    f64::NEG_INFINITY
                },
            }
        })
        .collect()
}

/// Map each diarized SPEAKER to the camera whose mic is loudest while that speaker
/// talks — the bridge from "who is speaking" to "which angle to show" when each angle
/// has its own mic of the same room (the on-mic speaker is loudest on their OWN
/// camera's mic). `speaker_active[w]` is the active speaker index in window `w` (or
/// `None` for silence/no-speech); `energies[w][c]` is camera `c`'s energy in window
/// `w` (the SAME envelope the energy mode reads). For each speaker, take the mean
/// energy of every camera over that speaker's ACTIVE windows, then assign cameras
/// GREEDILY by descending mean (each camera used once) so two speakers never collapse
/// onto one angle. Returns `speaker_idx → camera_idx` (length `n_speakers`).
///
/// Degenerate cases: a speaker with no active windows, or more speakers than cameras,
/// falls back to its single best-mean camera (which may then collide — the honest
/// >cameras-than-angles limit). Pure + deterministic (ties break to the lower
/// speaker, then lower camera index).
pub fn map_speakers_to_cameras(
    speaker_active: &[Option<usize>],
    energies: &[Vec<f64>],
    n_speakers: usize,
) -> Vec<usize> {
    let n_cams = energies.first().map(|r| r.len()).unwrap_or(0);
    if n_speakers == 0 {
        return Vec::new();
    }
    if n_cams == 0 {
        return vec![0; n_speakers];
    }

    // Per-speaker camera-energy sums over that speaker's active windows.
    let mut sums = vec![vec![0.0f64; n_cams]; n_speakers];
    let mut cnts = vec![0u64; n_speakers];
    for (w, act) in speaker_active.iter().enumerate() {
        let (Some(spk), Some(row)) = (*act, energies.get(w)) else {
            continue;
        };
        if spk >= n_speakers {
            continue;
        }
        for (c, slot) in sums[spk].iter_mut().enumerate() {
            if let Some(&v) = row.get(c) {
                if v.is_finite() {
                    *slot += v;
                }
            }
        }
        cnts[spk] += 1;
    }
    let mean = |spk: usize, c: usize| -> f64 {
        if cnts[spk] > 0 {
            sums[spk][c] / cnts[spk] as f64
        } else {
            f64::NEG_INFINITY
        }
    };

    // Greedy unique assignment: highest (mean,spk,cam) first, each camera once.
    let mut triples: Vec<(f64, usize, usize)> = Vec::new();
    for spk in 0..n_speakers {
        for c in 0..n_cams {
            triples.push((mean(spk, c), spk, c));
        }
    }
    triples.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
    });
    let mut map = vec![usize::MAX; n_speakers];
    let mut cam_used = vec![false; n_cams];
    for (_, spk, c) in triples {
        if map[spk] == usize::MAX && !cam_used[c] {
            map[spk] = c;
            cam_used[c] = true;
        }
    }
    // Any speaker still unassigned (more speakers than cameras) → its best camera,
    // allowing reuse (the documented >cameras-than-angles fallback).
    for (spk, slot) in map.iter_mut().enumerate() {
        if *slot == usize::MAX {
            let mut best = 0usize;
            let mut best_e = f64::NEG_INFINITY;
            for c in 0..n_cams {
                let m = mean(spk, c);
                if m > best_e {
                    best_e = m;
                    best = c;
                }
            }
            *slot = best;
        }
    }
    map
}

/// The SPEAKER-DRIVEN switch program: per window, show the camera of whoever is
/// SPEAKING (`speaker_to_cam[speaker_active[w]]`), holding the current angle through
/// silence (`speaker_active[w] == None`) so a pause doesn't cut away. The scan opens
/// on `default_cam`. Then the SAME run/dissolve/min-shot tail as the energy mode
/// ([`shots_from_assignment`]) → no cut faster than `min_shot_ms`. This fixes the
/// energy-only failure (an off-camera cough no longer steals the shot — only the
/// active speaker's camera does), while reusing every anti-flicker guarantee.
///
/// `speaker_to_cam` comes from [`map_speakers_to_cameras`]; `energies` is passed
/// through only for the `Shot.energy` summary. An empty `speaker_active` → `[]` (the
/// caller falls back to [`switch_shots`] when no diarization is available).
pub fn switch_shots_by_speaker(
    speaker_active: &[Option<usize>],
    speaker_to_cam: &[usize],
    energies: &[Vec<f64>],
    start_ms: u64,
    window_ms: u64,
    min_shot_ms: u64,
    default_cam: usize,
) -> Vec<Shot> {
    let n = speaker_active.len();
    if n == 0 || window_ms == 0 {
        return Vec::new();
    }
    let n_cams = energies.first().map(|r| r.len()).unwrap_or(0);
    if n_cams == 0 {
        return Vec::new();
    }
    let default_cam = default_cam.min(n_cams - 1);

    let mut assign: Vec<usize> = Vec::new();
    let mut cur = default_cam;
    for act in speaker_active {
        if let Some(spk) = *act {
            if let Some(&cam) = speaker_to_cam.get(spk) {
                cur = cam.min(n_cams - 1);
            }
        }
        // None (silence) → hold the current angle (no cut on a pause).
        assign.push(cur);
    }
    shots_from_assignment(&assign, energies, start_ms, window_ms, min_shot_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an `[window][camera]` energy grid where, over the given `[w0,w1)` window
    /// ranges, the named camera is LOUD (1.0) and the others are QUIET (0.0).
    fn grid(n_windows: usize, n_cams: usize, loud: &[(usize, usize, usize)]) -> Vec<Vec<f64>> {
        let mut g = vec![vec![0.0; n_cams]; n_windows];
        for &(w0, w1, cam) in loud {
            for row in g.iter_mut().take(w1).skip(w0) {
                row[cam] = 1.0;
            }
        }
        g
    }

    /// The headline case: cam A loud in [0,2 s], cam B in [2,4 s], A again in [4,6 s].
    /// At 250 ms windows (24 windows) with a 1500 ms min-shot, the program is exactly
    /// [A, B, A] with the cuts on the energy crossovers (2000 ms, 4000 ms).
    #[test]
    fn switches_at_energy_crossovers() {
        // A: windows 0..8 ; B: 8..16 ; A: 16..24.
        let g = grid(24, 2, &[(0, 8, 0), (8, 16, 1), (16, 24, 0)]);
        let shots = switch_shots(&g, 0, 250, 1500, 0.1, 0);
        assert_eq!(shots.len(), 3, "expected 3 shots, got {shots:?}");
        assert_eq!(
            (shots[0].start_ms, shots[0].end_ms, shots[0].camera),
            (0, 2000, 0)
        );
        assert_eq!(
            (shots[1].start_ms, shots[1].end_ms, shots[1].camera),
            (2000, 4000, 1)
        );
        assert_eq!(
            (shots[2].start_ms, shots[2].end_ms, shots[2].camera),
            (4000, 6000, 0)
        );
        // The active camera's energy over each shot is the LOUD level.
        assert!(shots.iter().all(|s| (s.energy - 1.0).abs() < 1e-9));
    }

    /// Two cameras within the hysteresis margin alternate the raw leader every window,
    /// but no margin clears `hysteresis` → no switch ever fires: one clean shot on the
    /// default camera (the anti-flicker guarantee for near-equal levels).
    #[test]
    fn near_equal_levels_no_spurious_switch() {
        let mut g = vec![vec![0.0; 2]; 24];
        for (w, row) in g.iter_mut().enumerate() {
            // Leader flips each window, but always by only 0.05 (< 0.1 hysteresis).
            if w % 2 == 0 {
                row[0] = 0.50;
                row[1] = 0.55;
            } else {
                row[0] = 0.55;
                row[1] = 0.50;
            }
        }
        let shots = switch_shots(&g, 0, 250, 1500, 0.1, 0);
        assert_eq!(
            shots.len(),
            1,
            "near-equal levels must not switch: {shots:?}"
        );
        assert_eq!(
            (shots[0].start_ms, shots[0].end_ms, shots[0].camera),
            (0, 6000, 0)
        );
    }

    /// A too-short loud blip on camera B (500 ms, below the 1500 ms min-shot) is
    /// absorbed into the surrounding camera A — no spurious sub-shot, no sub-second cut.
    #[test]
    fn short_blip_suppressed_by_min_shot() {
        // A: 0..8 (2 s) ; B blip: 8..10 (500 ms) ; A: 10..24.
        let g = grid(24, 2, &[(0, 8, 0), (8, 10, 1), (10, 24, 0)]);
        let shots = switch_shots(&g, 0, 250, 1500, 0.1, 0);
        assert_eq!(shots.len(), 1, "the 500 ms blip must dissolve: {shots:?}");
        assert_eq!(
            (shots[0].start_ms, shots[0].end_ms, shots[0].camera),
            (0, 6000, 0)
        );
    }

    /// The min-shot threshold is exact (ceil of min_shot_ms / window_ms = 6 windows at
    /// 1500/250): a B lead of EXACTLY 6 windows (1500 ms) survives as its own shot, but
    /// a 5-window (1250 ms) lead is dissolved.
    #[test]
    fn min_shot_threshold_is_exact() {
        // Exactly 1500 ms of B → survives → [A, B, A].
        let g6 = grid(24, 2, &[(0, 8, 0), (8, 14, 1), (14, 24, 0)]);
        let s6 = switch_shots(&g6, 0, 250, 1500, 0.1, 0);
        assert_eq!(s6.len(), 3, "a full-min-shot B must survive: {s6:?}");
        assert_eq!(
            (s6[1].start_ms, s6[1].end_ms, s6[1].camera),
            (2000, 3500, 1)
        );
        // Only 1250 ms of B → dissolved → one A shot.
        let g5 = grid(24, 2, &[(0, 8, 0), (8, 13, 1), (13, 24, 0)]);
        let s5 = switch_shots(&g5, 0, 250, 1500, 0.1, 0);
        assert_eq!(s5.len(), 1, "a sub-min-shot B must dissolve: {s5:?}");
        assert_eq!(s5[0].camera, 0);
    }

    /// A single dominant speaker (camera 0 loud throughout) → ONE shot, zero switches —
    /// the clean degenerate case, never an error.
    #[test]
    fn single_dominant_speaker_one_shot() {
        let g = grid(20, 3, &[(0, 20, 0)]);
        let shots = switch_shots(&g, 0, 250, 1500, 0.1, 0);
        assert_eq!(shots.len(), 1);
        assert_eq!(
            (shots[0].start_ms, shots[0].end_ms, shots[0].camera),
            (0, 5000, 0)
        );
    }

    /// `default_cam` (the reference/anchor angle) opens the program: with three silent
    /// cameras (all energy 0, no margin) the scan stays on the default the whole time.
    #[test]
    fn default_camera_opens_the_program() {
        let g = vec![vec![0.0; 3]; 12];
        let shots = switch_shots(&g, 0, 250, 1500, 0.1, 2);
        assert_eq!(shots.len(), 1);
        assert_eq!(
            shots[0].camera, 2,
            "should open on the default/reference camera"
        );
    }

    #[test]
    fn switch_shots_ignores_extra_ragged_camera_columns() {
        let mut g = vec![vec![0.0, 0.0]; 8];
        for row in g.iter_mut().skip(1) {
            row.push(10.0);
        }

        let shots = switch_shots(&g, 0, 250, 250, 0.1, 0);

        assert!(
            shots.iter().all(|s| s.camera < 2),
            "shots must never reference a camera outside the first row's camera count: {shots:?}"
        );
    }

    /// A blip at the very START (no left neighbour) folds into the next shot.
    #[test]
    fn leading_blip_folds_into_next_shot() {
        // B blip: 0..2 (500 ms) ; A: 2..24.
        let g = grid(24, 2, &[(0, 2, 1), (2, 24, 0)]);
        // Open on B (default 1) so window 0 actually starts as B; A then dominates.
        let shots = switch_shots(&g, 0, 250, 1500, 0.1, 1);
        assert_eq!(shots.len(), 1, "leading blip must fold forward: {shots:?}");
        assert_eq!(
            (shots[0].start_ms, shots[0].end_ms, shots[0].camera),
            (0, 6000, 0)
        );
    }

    /// `start_ms` offsets every shot boundary (the program may begin mid-timeline at
    /// the overlap start).
    #[test]
    fn start_offset_shifts_boundaries() {
        let g = grid(16, 2, &[(0, 8, 0), (8, 16, 1)]);
        let shots = switch_shots(&g, 1000, 250, 1500, 0.1, 0);
        assert_eq!(shots.len(), 2);
        assert_eq!((shots[0].start_ms, shots[0].end_ms), (1000, 3000));
        assert_eq!((shots[1].start_ms, shots[1].end_ms), (3000, 5000));
    }

    // ---- speaker mode (diarization-driven multicam) --------------------------

    /// Each speaker is loudest on their OWN camera's mic → the greedy unique map
    /// pairs them correctly (the core speaker→camera correlation).
    #[test]
    fn speakers_map_to_their_loudest_camera() {
        // S0 active w0..2 (loud on cam0); S1 active w2..4 (loud on cam1).
        let energies = vec![
            vec![1.0, 0.0],
            vec![1.0, 0.0],
            vec![0.0, 1.0],
            vec![0.0, 1.0],
        ];
        let active = vec![Some(0), Some(0), Some(1), Some(1)];
        assert_eq!(map_speakers_to_cameras(&active, &energies, 2), vec![0, 1]);
    }

    /// Two speakers loudest on the SAME raw camera still get UNIQUE cameras: the
    /// louder speaker claims it; the other takes its next-best (greedy, no collapse).
    #[test]
    fn map_is_unique_per_camera() {
        let energies = vec![
            vec![3.0, 1.0], // S0 window — loudest on cam0
            vec![2.0, 1.5], // S1 window — also leans cam0, but weaker there
        ];
        let active = vec![Some(0), Some(1)];
        assert_eq!(map_speakers_to_cameras(&active, &energies, 2), vec![0, 1]);
    }

    /// THE HEADLINE FIX: a sustained OFF-CAMERA loud noise does NOT steal the shot in
    /// speaker mode (the active speaker's camera holds), whereas ENERGY mode is fooled
    /// into cutting to the noisy mic — the exact regression that the diarization upgrade
    /// removes (the code's own receipt warned "wrong if an off-camera voice is loud").
    #[test]
    fn off_camera_noise_does_not_switch_in_speaker_mode() {
        // 24 windows @250ms. S0 speaks 0..12, S1 speaks 12..24.
        let mut energies = vec![vec![0.0, 0.0]; 24];
        for row in energies.iter_mut().take(12) {
            row[0] = 2.0; // cam0 = S0's mic, loud while S0 talks
        }
        for row in energies.iter_mut().skip(12) {
            row[1] = 2.0; // cam1 = S1's mic, loud while S1 talks
        }
        for row in energies.iter_mut().take(12).skip(6) {
            row[1] = 3.0; // OFF-CAMERA NOISE on cam1 over windows 6..12 (≥ min-shot)
        }
        let active: Vec<Option<usize>> = (0..24).map(|w| Some(usize::from(w >= 12))).collect();

        let map = map_speakers_to_cameras(&active, &energies, 2);
        assert_eq!(
            map,
            vec![0, 1],
            "each speaker maps to their own mic despite noise"
        );

        let spk = switch_shots_by_speaker(&active, &map, &energies, 0, 250, 1500, 0);
        assert_eq!(
            spk.len(),
            2,
            "speaker mode must NOT cut on off-cam noise: {spk:?}"
        );
        assert_eq!(
            (spk[0].camera, spk[0].start_ms, spk[0].end_ms),
            (0, 0, 3000)
        );
        assert_eq!(
            (spk[1].camera, spk[1].start_ms, spk[1].end_ms),
            (1, 3000, 6000)
        );

        // Contrast: ENERGY mode IS fooled — it cuts to cam1 during the noise (< 3 s).
        let eng = switch_shots(&energies, 0, 250, 1500, 0.1, 0);
        assert!(
            eng.iter().any(|s| s.camera == 1 && s.start_ms < 3000),
            "energy mode should be fooled into showing cam1 before 3 s: {eng:?}"
        );
    }

    /// Silence (`None`) HOLDS the current angle — a pause never cuts away.
    #[test]
    fn silence_holds_current_angle() {
        let energies = vec![vec![0.0, 0.0]; 12];
        let active = vec![
            Some(0),
            Some(0),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(1),
            Some(1),
            Some(1),
            Some(1),
        ];
        let map = vec![0usize, 1usize];
        let shots = switch_shots_by_speaker(&active, &map, &energies, 0, 250, 1000, 0);
        assert_eq!(shots.len(), 2);
        assert_eq!((shots[0].camera, shots[0].end_ms), (0, 2000));
        assert_eq!(shots[1].camera, 1);
    }

    /// Empty diarization → no shots (the dispatch falls back to switch_shots).
    #[test]
    fn empty_speaker_active_yields_no_shots() {
        let energies = vec![vec![0.0, 0.0]; 4];
        assert!(switch_shots_by_speaker(&[], &[0, 1], &energies, 0, 250, 1500, 0).is_empty());
    }
}
