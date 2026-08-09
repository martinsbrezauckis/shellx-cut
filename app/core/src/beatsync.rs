//! beatsync.rs — the PURE beat-selection core for `edit.cut_to_beat`.
//!
//! Role: the dependency-free, deterministic heart of the music-beat-sync verb
//! (a beat-synced montage where each clip change lands on
//! a beat). Given a list of BEAT TIMES (timeline ms, surfaced by
//! `audio.add_music` as `beat:N` markers off the bed's perception `BeatGrid`)
//! and the track's current cut geometry, it answers two questions:
//!   1. `mode:"split"` — WHICH beat times to cut the base track at
//!      ([`pick_split_cuts`]) — the beat-synced montage skeleton.
//!   2. `mode:"snap"` — where each EXISTING cut boundary should MOVE to lock an
//!      already-assembled sequence to the beat ([`snap_boundaries`]).
//!
//! Everything here is a pure function of integers, so every branch is unit-
//! tested below. The dispatch layer ([`server::dispatch::edit_cut_to_beat`])
//! feeds these the live project facts and lowers the result onto ordinary,
//! replay-safe `edit.split` / `edit.trim` ops — this module invents NO new
//! timeline primitive.
//!
//! Dependencies: none (std only). Primary caller: `server::dispatch`.

/// Sort + dedup `beats`, then keep every `every_n`-th in time order (positions
/// 0, n, 2n, …). `every_n` selects the cutting density: 1 = every beat, 2 =
/// every 2nd beat, 4 = every bar in 4/4 (the slow-montage feel). `every_n` is
/// clamped to ≥ 1, so a `0` from the caller degrades to "every beat" rather than
/// dividing by zero. Pure + deterministic.
fn select_every_nth(beats: &[u64], every_n: usize) -> Vec<u64> {
    let n = every_n.max(1);
    let mut b = beats.to_vec();
    b.sort_unstable();
    b.dedup();
    b.into_iter()
        .enumerate()
        .filter(|(i, _)| i % n == 0)
        .map(|(_, v)| v)
        .collect()
}

/// SPLIT picker — the timeline positions to cut the track at so each cut lands
/// on a beat. Selection pipeline (all pure):
///   1. thin the beats by `every_n` ([`select_every_nth`]);
///   2. keep beats STRICTLY inside the track's covered `extent` `(lo, hi)` — a
///      beat exactly at the program start/end is a program EDGE, not an internal
///      cut (and `edit.split` would reject it anyway);
///   3. keep beats inside `range_ms` `[r0, r1)` when the caller limited the span;
///   4. drop a beat within `epsilon_ms` of an EXISTING cut, or of a beat already
///      chosen — so no zero-length / sub-frame sliver is created.
/// Returns ascending, deduped cut positions. The handler still splits each one
/// defensively (a beat landing in a GAP is skipped there), so this list is the
/// CANDIDATE set; `extent` + `existing_cuts` just prune the obvious non-cuts.
pub fn pick_split_cuts(
    beats: &[u64],
    existing_cuts: &[u64],
    extent: (u64, u64),
    range_ms: Option<[u64; 2]>,
    every_n: usize,
    epsilon_ms: u64,
) -> Vec<u64> {
    let (lo, hi) = extent;
    let mut existing = existing_cuts.to_vec();
    existing.sort_unstable();
    existing.dedup();
    let mut out: Vec<u64> = Vec::new();
    for b in select_every_nth(beats, every_n) {
        if b <= lo || b >= hi {
            continue; // program edge / outside the track's content
        }
        if let Some([r0, r1]) = range_ms {
            if b < r0 || b >= r1 {
                continue; // outside the requested span
            }
        }
        if existing.iter().any(|&c| c.abs_diff(b) <= epsilon_ms) {
            continue; // already a cut here (or within a sliver of one)
        }
        if out.last().is_some_and(|&p| b.abs_diff(p) <= epsilon_ms) {
            continue; // too close to a beat we just took — would be a sliver
        }
        out.push(b);
    }
    out
}

/// One snapped boundary move: the existing cut at `from` (timeline ms) should
/// relocate to the beat at `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeatSnap {
    pub from: u64,
    pub to: u64,
}

/// SNAP picker — map each existing INTERNAL cut `boundary` to the nearest beat
/// within `max_snap_ms`, so an already-assembled sequence locks to the beat.
/// NON-CROSSING by construction: boundaries are processed left→right and a
/// chosen target must sit STRICTLY between the previous boundary's new position
/// (the moving lower fence) and the next boundary's ORIGINAL position (upper
/// fence) — a boundary can never be pushed past a neighbour. A boundary with no
/// beat in range (or whose only in-range beat would cross a neighbour) is left
/// untouched. Beats are thinned by `every_n` first, and only boundaries inside
/// `range_ms` are considered. Returns the moves (target ≠ origin) in order.
pub fn snap_boundaries(
    boundaries: &[u64],
    beats: &[u64],
    extent: (u64, u64),
    range_ms: Option<[u64; 2]>,
    every_n: usize,
    max_snap_ms: u64,
) -> Vec<BeatSnap> {
    let (lo, hi) = extent;
    let beats = select_every_nth(beats, every_n);
    let mut bounds: Vec<u64> = boundaries.to_vec();
    bounds.sort_unstable();
    bounds.dedup();
    bounds.retain(|&p| p > lo && p < hi); // internal boundaries only

    let mut out: Vec<BeatSnap> = Vec::new();
    let mut lower = lo; // moving lower fence: last chosen position (or extent start)
    for (i, &p) in bounds.iter().enumerate() {
        let upper = bounds.get(i + 1).copied().unwrap_or(hi); // next ORIGINAL boundary
        let in_range = range_ms.is_none_or(|[r0, r1]| p >= r0 && p < r1);
        let best = if in_range {
            beats
                .iter()
                .copied()
                .filter(|&b| b > lower && b < upper && b.abs_diff(p) <= max_snap_ms)
                .min_by_key(|&b| b.abs_diff(p))
        } else {
            None
        };
        match best {
            Some(b) if b != p => {
                out.push(BeatSnap { from: p, to: b });
                lower = b; // fence advances to the new position
            }
            _ => lower = p, // unmoved — fence stays at the original boundary
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_nth_thins_and_dedups() {
        // Unsorted + a duplicate; n=1 returns the sorted unique set.
        assert_eq!(
            select_every_nth(&[2000, 0, 1000, 1000, 3000], 1),
            vec![0, 1000, 2000, 3000]
        );
        // n=2 halves them (positions 0, 2): 0 and 2000.
        assert_eq!(select_every_nth(&[0, 1000, 2000, 3000], 2), vec![0, 2000]);
        // n=4 keeps only the first (a "every bar" downbeat).
        assert_eq!(select_every_nth(&[0, 500, 1000, 1500], 4), vec![0]);
        // n=0 is clamped to "every beat" (no panic).
        assert_eq!(select_every_nth(&[10, 20], 0), vec![10, 20]);
    }

    /// SPLIT: with a clip extent and a beat grid, the picker keeps exactly the
    /// IN-EXTENT beats, skips the one sitting on an existing cut, and `every_n=2`
    /// halves the result.
    #[test]
    fn split_selects_in_extent_skips_existing_cut() {
        // Beats every 500ms 0..=3000; the clip covers (0, 3000) exclusive.
        let beats = [0, 500, 1000, 1500, 2000, 2500, 3000];
        let extent = (0, 3000);
        // No existing internal cuts → every interior beat is a cut.
        let all = pick_split_cuts(&beats, &[0, 3000], extent, None, 1, 20);
        assert_eq!(
            all,
            vec![500, 1000, 1500, 2000, 2500],
            "interior beats only"
        );

        // An existing cut at 1000 (±15ms here at 1008) removes that beat.
        let with_cut = pick_split_cuts(&beats, &[0, 1008, 3000], extent, None, 1, 20);
        assert_eq!(
            with_cut,
            vec![500, 1500, 2000, 2500],
            "beat within epsilon of the 1008 cut is skipped"
        );

        // every_n=2 → thin to beats 0,1000,2000,3000 then keep interior 1000,2000.
        let halved = pick_split_cuts(&beats, &[0, 3000], extent, None, 2, 20);
        assert_eq!(halved, vec![1000, 2000], "every 2nd beat halves the cuts");
    }

    /// SPLIT: `range_ms` limits the cut span to a window.
    #[test]
    fn split_range_limits_span() {
        let beats = [500, 1000, 1500, 2000, 2500];
        let cuts = pick_split_cuts(&beats, &[0, 3000], (0, 3000), Some([1000, 2100]), 1, 20);
        assert_eq!(cuts, vec![1000, 1500, 2000], "only beats in [1000,2100)");
    }

    /// SNAP: each existing boundary maps to the nearest beat inside the window;
    /// a boundary with no beat close enough is left exactly where it is.
    #[test]
    fn snap_maps_to_nearest_beat_or_leaves_alone() {
        // Beats on a clean 1000ms grid.
        let beats = [0, 1000, 2000, 3000, 4000];
        // Boundary 980 → 1000 (20ms away, in window). Boundary 2500 → no beat
        // within 120ms → unmoved. Boundary 3040 → 3000 (40ms away).
        let snaps = snap_boundaries(&[980, 2500, 3040], &beats, (0, 5000), None, 1, 120);
        assert_eq!(
            snaps,
            vec![
                BeatSnap {
                    from: 980,
                    to: 1000
                },
                BeatSnap {
                    from: 3040,
                    to: 3000
                },
            ],
            "980→1000 and 3040→3000; 2500 left alone (no beat within 120ms)"
        );
    }

    /// SNAP: a boundary already exactly on a beat is NOT reported as a move.
    #[test]
    fn snap_skips_already_on_beat() {
        let beats = [0, 1000, 2000];
        let snaps = snap_boundaries(&[1000], &beats, (0, 3000), None, 1, 120);
        assert!(snaps.is_empty(), "boundary already on the beat → no move");
    }

    /// SNAP: never cross a neighbour — two boundaries competing for the same beat
    /// each stay on their own side. 900 and 1100 both want 1000; 900 takes it
    /// (nearer? equal — 900 is processed first and claims it), 1100 must then find
    /// a beat strictly > 1000 within the window, else stay put.
    #[test]
    fn snap_does_not_cross_neighbour() {
        let beats = [0, 1000, 2000];
        // 900→1000 (100ms). 1100 then needs a beat in (1000, next) within 120ms:
        // 2000 is 900ms away → too far → 1100 stays.
        let snaps = snap_boundaries(&[900, 1100], &beats, (0, 3000), None, 1, 120);
        assert_eq!(
            snaps,
            vec![BeatSnap {
                from: 900,
                to: 1000
            }],
            "900 claims 1000; 1100 cannot cross back onto it and stays"
        );
    }
}
