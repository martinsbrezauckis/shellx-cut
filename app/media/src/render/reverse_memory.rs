//! Reversed clips and render memory (item 170).
//!
//! ffmpeg's `reverse` filter writes nothing until it has read its whole input,
//! so it holds every decoded frame of the reversed part in memory, at the
//! SOURCE size (it runs before the conform). Measured on Linux, ffmpeg 8.0.1
//! (peak RSS of decode + trim + reverse, two lengths, difference per frame):
//! 1920x1080 8-bit 4:2:0 2.98 MiB a frame, 3840x2160 8-bit 4:2:0 11.97 MiB,
//! 3840x2160 10-bit 4:2:0 23.9 MiB; i.e. width x height (both rounded up to 16)
//! x the bytes of one pixel. A 30 s reversed 4K clip at 30 fps holds 10.5 GiB.
//!
//! In 0.6.115 a render window's cgroup limit (MemoryHigh) had no term for
//! these frames: a reversed part over the limit, on a machine with no swap,
//! was held there by the kernel and the render never finished. Since item 95 a
//! serial window gets the whole render budget, but any reversed part larger
//! than that budget still stalls the same way. So the render plan bounds it:
//! - a window whose reversed frames exceed the window budget
//!   (`SHELLX_CUT_RENDER_WINDOW_BUDGET_MB`, default 1500) is split into
//!   parts of whole seconds that each fit (never under 1 s, never inside a
//!   dissolve), so a reversed clip is reversed part by part;
//! - a timeline whose reversed clips need more than [`SINGLE_PASS_LIMIT_MB`]
//!   is rendered in windows even without an overlay.
//!
//! The frames are correct across these seams because each window trims its
//! reversed part from the right end of the source range (`Edl::window`, item
//! 46). Windows that hold no reversed frames are unchanged, so renders without
//! a long reversed clip keep their bytes.

use cut_core::{Asset, Edl, EdlSegment, Project, TrackKind};

/// A single-pass render holds at most this much reversed video (MiB). Chosen
/// so that it fits next to a 4K decode and encode (about 2 GB) in the
/// single-render budget (75 % of RAM) of an 8 GB computer.
pub(super) const SINGLE_PASS_LIMIT_MB: u64 = 4096;
/// Shortest window part a reversed clip is split into, and the step of the
/// split points (whole seconds from the window start).
const MIN_PART_MS: u64 = 1000;
const MIB: u64 = 1024 * 1024;

/// The per-window memory budget (MiB): the same setting the window length
/// model uses (`render_window_ms`).
pub(super) fn window_budget_bytes() -> u64 {
    let mb = std::env::var("SHELLX_CUT_RENDER_WINDOW_BUDGET_MB")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|n| (256.0..=8192.0).contains(n))
        .unwrap_or(1500.0);
    (mb * MIB as f64) as u64
}

/// Bytes of one decoded frame of `asset`, as `reverse` holds it.
pub(super) fn frame_bytes(asset: &Asset) -> u64 {
    let (w, h) = super::source_dims(asset).unwrap_or((1920, 1080));
    let round16 = |n: u32| u64::from(n).div_ceil(16) * 16;
    let pix = asset
        .probe
        .as_ref()
        .and_then(|p| p["raw"]["streams"].as_array())
        .and_then(|streams| {
            streams
                .iter()
                .find(|s| s["codec_type"].as_str() == Some("video"))
        })
        .and_then(|s| s["pix_fmt"].as_str())
        .unwrap_or("yuv420p");
    // Bytes per pixel in eighths (4:2:0 8-bit = 12/8). Checked: the sides
    // come from a probe, and an impossible size must read as too big, never
    // overflow to a small number.
    let eighths = pixel_eighths(pix);
    round16(w)
        .checked_mul(round16(h))
        .and_then(|px| px.checked_mul(eighths))
        .map_or(u64::MAX, |b| b / 8)
}

/// Size of one pixel of `pix_fmt` in eighths of a byte. Deeper than 8 bits is
/// stored in 16-bit samples; alpha is one more full plane.
fn pixel_eighths(pix: &str) -> u64 {
    let deep = ["9", "10", "12", "14", "16", "p010", "p016", "48", "64"]
        .iter()
        .any(|d| pix.contains(d));
    let sample = if deep { 2 } else { 1 };
    let alpha = pix.starts_with("yuva") || pix.starts_with("gbrap");
    let planes_eighths = if pix.starts_with("gray") || pix.starts_with("ya") {
        8
    } else if pix.contains("444") || pix.starts_with("gbr") {
        24
    } else if pix.contains("422") || pix.contains("yuyv") || pix.contains("uyvy") {
        16
    } else if pix.contains("rgba")
        || pix.contains("bgra")
        || pix.contains("argb")
        || pix.contains("abgr")
        || pix.contains("rgb0")
        || pix.contains("bgr0")
        || pix.contains("0rgb")
        || pix.contains("0bgr")
        || pix.contains("rgba64")
    {
        32
    } else if pix.starts_with("rgb") || pix.starts_with("bgr") {
        24
    } else {
        12 // 4:2:0 and anything unknown
    };
    let rgb_packed = pix.starts_with("rgb") || pix.starts_with("bgr") || pix.contains("rgb0");
    let alpha_eighths = if alpha && !rgb_packed { 8 } else { 0 };
    (planes_eighths + alpha_eighths) * sample
}

/// What reversing a clip needs at the least (row 197a): the frames of its
/// smallest render part, one timeline second (or the whole clip when shorter),
/// at the source size, pixel format and frame rate, sped up by `speed`.
/// Longer clips are reversed part by part, so their length does not matter.
pub fn reverse_part_bytes(asset: &Asset, project_fps: f64, speed: f64, clip_ms: u64) -> u64 {
    let fps = super::source_fps(asset).unwrap_or(project_fps.max(1.0));
    let speed = if speed.is_finite() && speed > 0.0 {
        speed
    } else {
        1.0
    };
    let part_ms = clip_ms.clamp(1, MIN_PART_MS) as f64;
    let frames = (part_ms * speed * fps / 1000.0).ceil() as u64;
    frames.saturating_mul(frame_bytes(asset))
}

/// The `edit.reverse` refusal: `Some(part bytes)` when even the smallest part
/// needs more than the memory a render may use on this computer (75 % of its
/// RAM, the render budget), so the clip can never render here. `None` admits
/// it: every longer reversed clip renders in parts of at most a second.
pub fn reverse_refusal(part_bytes: u64, total_ram_bytes: Option<u64>) -> Option<u64> {
    let budget = total_ram_bytes? / 100 * 75;
    (budget > 0 && part_bytes > budget).then_some(part_bytes)
}

/// One reversed video part of the timeline: where it is and what it costs.
struct Reversed<'a> {
    seg: &'a EdlSegment,
    /// Bytes `reverse` holds per timeline millisecond of this part.
    bytes_per_ms: f64,
}

fn reversed_parts<'a>(project: &Project, edl: &'a Edl) -> Vec<Reversed<'a>> {
    let visible: Vec<String> = super::planned_video_tracks(project)
        .into_iter()
        .filter(|t| t.visible)
        .map(|t| t.id.to_string())
        .collect();
    edl.segments
        .iter()
        .filter(|s| s.track_kind == TrackKind::Video && s.reverse && s.freeze.is_none())
        .filter(|s| visible.contains(&s.track))
        .filter_map(|seg| {
            let asset = project.assets.get(seg.asset.as_deref()?)?;
            let fps = super::source_fps(asset).unwrap_or(project.settings.fps.max(1.0));
            let speed = if seg.speed.is_finite() && seg.speed > 0.0 {
                seg.speed
            } else {
                1.0
            };
            Some(Reversed {
                seg,
                bytes_per_ms: frame_bytes(asset) as f64 * fps * speed / 1000.0,
            })
        })
        .collect()
}

fn held_in(parts: &[Reversed<'_>], w0: u64, w1: u64) -> u64 {
    parts
        .iter()
        .map(|p| {
            let a = p.seg.timeline_in_ms.max(w0);
            let b = p.seg.timeline_out_ms.min(w1);
            if b <= a {
                0
            } else {
                ((b - a) as f64 * p.bytes_per_ms).ceil() as u64
            }
        })
        .fold(0u64, u64::saturating_add)
}

/// Bytes of reversed video a render of `[w0, w1)` holds at once (every
/// reversed part in it: one decoder feeds them all, so they can overlap).
pub(super) fn held_bytes(project: &Project, edl: &Edl, w0: u64, w1: u64) -> u64 {
    held_in(&reversed_parts(project, edl), w0, w1)
}

/// True when one pass over the whole timeline would hold more reversed video
/// than [`SINGLE_PASS_LIMIT_MB`]: such a timeline renders in windows.
pub(super) fn single_pass_too_big(project: &Project, edl: &Edl) -> bool {
    held_bytes(project, edl, 0, edl.duration_ms) > SINGLE_PASS_LIMIT_MB * MIB
}

/// Split every window whose reversed frames exceed the window budget into
/// frame-aligned parts that fit. Windows without reversed video are returned
/// as they are.
pub(super) fn split_windows(
    project: &Project,
    edl: &Edl,
    windows: Vec<(u64, u64)>,
    fps: f64,
) -> Vec<(u64, u64)> {
    split_windows_within(project, edl, windows, fps, window_budget_bytes())
}

pub(super) fn split_windows_within(
    project: &Project,
    edl: &Edl,
    windows: Vec<(u64, u64)>,
    fps: f64,
    budget: u64,
) -> Vec<(u64, u64)> {
    let parts = reversed_parts(project, edl);
    if parts.is_empty() {
        return windows;
    }
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        30.0
    };
    let frame_at = |f: u64| (f as f64 * 1000.0 / fps).round() as u64;
    let frame_of = |t: u64| (t as f64 * fps / 1000.0).round() as u64;
    let dissolves: Vec<(u64, u64)> = edl
        .segments
        .iter()
        .filter(|s| s.xfade_in_ms > 0)
        .map(|s| (s.timeline_in_ms, s.timeline_in_ms + s.xfade_in_ms))
        .collect();
    let out_of_dissolve = |mut b: u64, limit: u64| loop {
        match dissolves.iter().find(|(x0, x1)| b > *x0 && b < *x1) {
            Some((_, x1)) => b = (*x1).min(limit),
            None => return b,
        }
    };
    let mut out = Vec::with_capacity(windows.len());
    for (w0, w1) in windows {
        if held_in(&parts, w0, w1) <= budget {
            out.push((w0, w1));
            continue;
        }
        let mut start = w0;
        while start < w1 {
            if held_in(&parts, start, w1) <= budget {
                out.push((start, w1));
                break;
            }
            // Longest part of whole seconds from `start` that fits (binary
            // search; held bytes only grow with the end). Whole seconds, like
            // the window planner's own seams: a seam between two frames of a
            // fractional millisecond position gains or loses a frame (seen at
            // 36 733 ms, item 170 run on the box).
            let seconds = (w1 - start).div_ceil(MIN_PART_MS);
            let end_at = |k: u64| frame_at(frame_of(start + k * MIN_PART_MS)).min(w1);
            let (mut lo, mut hi) = (1u64, seconds);
            while lo < hi {
                let mid = lo + (hi - lo).div_ceil(2);
                if held_in(&parts, start, end_at(mid)) <= budget {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            let mut end = out_of_dissolve(end_at(lo), w1);
            if end <= start {
                end = w1;
            }
            out.push((start, end));
            start = end;
        }
    }
    out
}

/// Name the part of the timeline and the clip in a stalled render's error
/// (the process layer knows only ffmpeg). `w0` is where `edl` starts on the
/// full timeline (0 for a single pass).
pub(super) fn stall_context(
    project: &Project,
    edl: &Edl,
    w0: u64,
    mut error: cut_core::CutError,
) -> cut_core::CutError {
    if error.code != cut_core::error_codes::RENDER_STALLED {
        return error;
    }
    let clock = |ms: u64| format!("{}:{:02}", ms / 60_000, (ms / 1000) % 60);
    let (a, b) = (w0, w0 + edl.duration_ms);
    // The clip to name: a reversed clip of the part (the usual cause), else
    // the first video clip in it.
    let parts = reversed_parts(project, edl);
    let named = parts.first().map(|p| (p.seg, true)).or_else(|| {
        edl.segments
            .iter()
            .find(|s| s.track_kind == TrackKind::Video && s.asset.is_some())
            .map(|s| (s, false))
    });
    let mut what = format!("while making {} to {} of the timeline", clock(a), clock(b));
    if let Some((seg, reversed)) = named {
        let file = seg
            .asset
            .as_deref()
            .and_then(|id| project.assets.get(id))
            .and_then(|asset| {
                std::path::Path::new(&asset.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        let kind = if reversed {
            "a reversed clip"
        } else {
            "a clip"
        };
        what.push_str(&match file.is_empty() {
            true => format!(", which holds {kind}"),
            false => format!(", which holds {kind} of {file}"),
        });
        if let Some(id) = &seg.clip_id {
            error = error.with_clip(id.clone());
        }
        error = error.with_at_ms(w0 + seg.timeline_in_ms);
    }
    error.message = format!("{} {what}", error.message);
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn asset(w: u32, h: u32, pix: &str, fps: f64) -> Value {
        json!({"path": format!("/m/src_{w}x{h}_{pix}.mp4"), "hash": format!("sha256:{w}{h}{pix}"),
               "probe": {"kind": "video", "width": w, "height": h, "fps": fps,
                         "raw": {"streams": [{"codec_type": "video", "pix_fmt": pix}]}}})
    }

    /// `(src_in, src_out, reverse, speed)` clips on V1 of asset a1; `overlay`
    /// adds a 2 s V2 clip of a2.
    fn project(
        w: u32,
        h: u32,
        src: Value,
        clips: &[(u64, u64, bool, f64)],
        overlay: bool,
    ) -> Project {
        let v1: Vec<Value> = clips
            .iter()
            .enumerate()
            .map(|(i, (a, b, rev, speed))| {
                json!({"id": format!("c{}", i + 1), "asset": "a1", "src_in_ms": a, "src_out_ms": b,
                       "reverse": rev, "speed": speed})
            })
            .collect();
        let mut tracks = vec![json!({"id": "v1", "kind": "video", "clips": v1})];
        if overlay {
            tracks.push(json!({"id": "v2", "kind": "video", "clips": [
                {"id": "o1", "asset": "a2", "src_in_ms": 0, "src_out_ms": 2000}]}));
        }
        serde_json::from_value(json!({
            "schema": "shellx-cut/1", "name": "t",
            "settings": {"width": w, "height": h, "fps": 30.0, "audio_rate": 48000},
            "assets": {"a1": src, "a2": asset(1920, 1080, "yuv420p", 30.0)},
            "tracks": tracks,
        }))
        .expect("test project")
    }

    #[test]
    fn an_impossible_probe_size_reads_as_too_big_not_as_a_small_number() {
        // A received project can state any size: u32::MAX sides overflowed the
        // multiplication (a panic in a debug build, a wrapped small number in a
        // release build that planned a window far above the bound).
        let forged: Asset =
            serde_json::from_value(asset(u32::MAX, u32::MAX, "yuva444p16le", 30.0)).unwrap();
        assert_eq!(frame_bytes(&forged), u64::MAX);
        assert_eq!(reverse_part_bytes(&forged, 30.0, 4.0, 60_000), u64::MAX);
        let p = project(
            1920,
            1080,
            asset(u32::MAX, u32::MAX, "yuv420p", 30.0),
            &[(0, 4000, true, 1.0)],
            false,
        );
        let edl = cut_core::edl_from_project(&p);
        assert_eq!(held_bytes(&p, &edl, 0, edl.duration_ms), u64::MAX);
        assert!(single_pass_too_big(&p, &edl));
    }

    #[test]
    fn reverse_is_refused_only_when_one_second_cannot_fit() {
        let gib = 1024 * MIB;
        let p4k: Asset = serde_json::from_value(asset(3840, 2160, "yuv420p", 30.0)).unwrap();
        // A 60 s reversed 4K clip needs one second of frames at the least: 356 MiB.
        let part = reverse_part_bytes(&p4k, 30.0, 1.0, 60_000);
        assert_eq!(part, 30 * 3840 * 2160 * 3 / 2);
        assert_eq!(reverse_refusal(part, Some(8 * gib)), None);
        // 8K 10-bit at 60 fps and 2x speed: 120 frames x 95 MiB = 11.1 GiB a second.
        let p8k: Asset = serde_json::from_value(asset(7680, 4320, "yuv420p10le", 60.0)).unwrap();
        let part = reverse_part_bytes(&p8k, 60.0, 2.0, 60_000);
        assert_eq!(reverse_refusal(part, Some(8 * gib)), Some(part));
        assert_eq!(reverse_refusal(part, Some(64 * gib)), None);
        // Unknown memory never refuses (the render preflight still decides).
        assert_eq!(reverse_refusal(part, None), None);
        // A clip shorter than a second needs only its own frames.
        assert_eq!(
            reverse_part_bytes(&p4k, 30.0, 1.0, 500),
            15 * 3840 * 2160 * 3 / 2
        );
    }

    #[test]
    fn frame_bytes_match_the_measured_frame_sizes() {
        // Measured (ffmpeg 8.0.1, peak RSS per held frame): 2.977 MiB, 11.97 MiB, 23.92 MiB.
        let mib = |x: f64| (x * MIB as f64) as u64;
        for (w, h, pix, measured) in [
            (1920, 1080, "yuv420p", mib(2.977)),
            (3840, 2160, "yuv420p", mib(11.97)),
            (3840, 2160, "yuv420p10le", mib(23.92)),
        ] {
            let a: Asset = serde_json::from_value(asset(w, h, pix, 30.0)).unwrap();
            let got = frame_bytes(&a);
            let off = (got as f64 - measured as f64).abs() / measured as f64;
            assert!(
                off < 0.02,
                "{w}x{h} {pix}: model {got} B, measured {measured} B"
            );
        }
        assert_eq!(pixel_eighths("yuv422p10le"), 32);
        assert_eq!(pixel_eighths("yuva444p12le"), 64);
        assert_eq!(pixel_eighths("rgba"), 32);
        assert_eq!(pixel_eighths("gray"), 8);
    }

    fn assert_tiles(windows: &[(u64, u64)], dur: u64) {
        assert_eq!(windows.first().map(|w| w.0), Some(0));
        assert_eq!(windows.last().map(|w| w.1), Some(dur));
        for pair in windows.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "windows must tile: {windows:?}");
        }
        for (a, b) in windows {
            assert!(b > a, "empty window in {windows:?}");
            // Every seam is on a whole second (a frame of every whole frame
            // rate; a fractional-millisecond seam gains or loses a frame).
            assert_eq!(a % 1000, 0, "{a} ms is not a whole second: {windows:?}");
        }
    }

    #[test]
    fn a_long_reversed_4k_clip_is_split_into_windows_that_fit() {
        let p = project(
            3840,
            2160,
            asset(3840, 2160, "yuv420p", 30.0),
            &[(0, 40_000, true, 1.0)],
            true,
        );
        let edl = cut_core::edl_from_project(&p);
        let budget = 1500 * MIB;
        let plan = super::super::plan_windows(&edl, 5000, 30.0);
        let split = split_windows_within(&p, &edl, plan.clone(), 30.0, budget);
        assert!(split.len() > plan.len(), "{plan:?} -> {split:?}");
        assert_tiles(&split, edl.duration_ms);
        for (a, b) in &split {
            assert!(
                held_bytes(&p, &edl, *a, *b) <= budget,
                "[{a},{b}) over budget"
            );
        }
        // Without reverse nothing changes (the bytes of such renders stay).
        let p = project(
            3840,
            2160,
            asset(3840, 2160, "yuv420p", 30.0),
            &[(0, 40_000, false, 1.0)],
            true,
        );
        let edl = cut_core::edl_from_project(&p);
        assert_eq!(
            split_windows_within(&p, &edl, plan.clone(), 30.0, budget),
            plan
        );
    }

    #[test]
    fn reversed_speed_counts_the_source_frames_it_consumes() {
        let src = asset(1920, 1080, "yuv420p", 30.0);
        let p1 = project(1920, 1080, src.clone(), &[(0, 30_000, true, 1.0)], false);
        let p2 = project(1920, 1080, src, &[(0, 30_000, true, 1.5)], false);
        let (e1, e2) = (
            cut_core::edl_from_project(&p1),
            cut_core::edl_from_project(&p2),
        );
        // Same source frames, 1.5x shorter on the timeline.
        let (h1, h2) = (
            held_bytes(&p1, &e1, 0, e1.duration_ms),
            held_bytes(&p2, &e2, 0, e2.duration_ms),
        );
        assert!(
            (h1 as f64 - h2 as f64).abs() / (h1 as f64) < 0.01,
            "{h1} vs {h2}"
        );
    }

    #[test]
    fn a_single_pass_holds_at_most_the_single_pass_limit() {
        let src = asset(1920, 1080, "yuv420p", 30.0);
        // 40 s of reversed 1080p = 3.5 GiB: stays a single pass (bytes unchanged).
        let p = project(1920, 1080, src.clone(), &[(0, 40_000, true, 1.0)], false);
        assert!(!single_pass_too_big(&p, &cut_core::edl_from_project(&p)));
        assert!(!super::super::should_segment(
            &p,
            &cut_core::edl_from_project(&p),
            1920,
            1080
        ));
        // 50 s = 4.4 GiB: rendered in windows.
        let p = project(1920, 1080, src, &[(0, 50_000, true, 1.0)], false);
        assert!(single_pass_too_big(&p, &cut_core::edl_from_project(&p)));
        // A reversed 4K clip of 15 s (5.3 GiB) too.
        let p = project(
            3840,
            2160,
            asset(3840, 2160, "yuv420p", 30.0),
            &[(0, 15_000, true, 1.0)],
            false,
        );
        assert!(single_pass_too_big(&p, &cut_core::edl_from_project(&p)));
    }

    #[test]
    fn a_split_never_lands_inside_a_dissolve() {
        // 10 s of reversed 4K (3.5 GiB) in one window with a 2 GiB budget: the
        // first split would fall near 5.7 s, inside a dissolve over [1, 9) s.
        let src = asset(3840, 2160, "yuv420p", 30.0);
        let p = project(
            3840,
            2160,
            src,
            &[(0, 10_000, true, 1.0), (10_000, 20_000, false, 1.0)],
            true,
        );
        let mut edl = cut_core::edl_from_project(&p);
        let second = edl
            .segments
            .iter()
            .position(|s| s.track == "v1" && s.timeline_in_ms == 10_000)
            .unwrap();
        edl.segments[second].timeline_in_ms = 1000; // dissolve region [1000, 9000)
        edl.segments[second].xfade_in_ms = 8000;
        let split =
            split_windows_within(&p, &edl, vec![(0, edl.duration_ms)], 30.0, 2 * 1024 * MIB);
        assert_eq!(split, vec![(0, 9000), (9000, 20_000)]);
    }

    #[test]
    fn a_stalled_window_names_its_part_and_reversed_clip() {
        let p = project(
            1920,
            1080,
            asset(1920, 1080, "yuv420p", 30.0),
            &[(0, 10_000, false, 1.0), (20_000, 55_000, true, 1.0)],
            true,
        );
        let edl = cut_core::edl_from_project(&p);
        let wedl = edl.window(20_000, 40_000);
        let stalled = cut_core::CutError::new(
            cut_core::error_codes::RENDER_STALLED,
            "The export stopped: ffmpeg made no progress for 120 s",
            "x",
        );
        let named = stall_context(&p, &wedl, 20_000, stalled);
        assert_eq!(named.clip_id.as_deref(), Some("c2"));
        assert_eq!(named.at_ms, Some(20_000));
        assert!(
            named.message.ends_with(
                "while making 0:20 to 0:40 of the timeline, which holds a reversed clip of \
                 src_1920x1080_yuv420p.mp4"
            ),
            "{}",
            named.message
        );
        let other = cut_core::CutError::new(cut_core::error_codes::FFMPEG, "boom", "x");
        assert_eq!(stall_context(&p, &wedl, 20_000, other).message, "boom");
    }
}
