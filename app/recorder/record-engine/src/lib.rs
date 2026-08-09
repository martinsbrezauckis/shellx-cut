//! record-engine — the auto-edit heuristics ("polished without editing").
//!
//! Role: read a captured `EventTrack` and synthesize an `EditPlan` — the same
//! input-event stream used to infer edit suggestions.
//! Pure + deterministic (no I/O, no platform deps) so it runs and unit-tests
//! anywhere. The renderer (record-render) consumes the plan this produces.
//!
//! Heuristics:
//! - `build_zoom` — cluster click anchors in time, emit eased zoom in→hold→out
//!   segments centered on each cluster (clamped to stay in-frame).
//! - `smooth_cursor` — moving-average de-jitter of the cursor path (synthetic cursor
//!   follows this, not the raw OS path).
//! - `build_clicks` — click-downs → ripple highlight effects (fraction coords).
//! - `build_keycast` — coalesce typed keys into on-screen key-cast chips.
//! - `find_idle` — long no-input spans (for optional auto-cut; opt-in).
//!
//! Public entry: `autoedit(&EventTrack, &EngineConfig) -> EditPlan`.
//! Primary callers: record-cli `autoedit`, record-render (full pipeline), cutd later.

use serde::{Deserialize, Serialize};

use record_core::{
    ClickFx, CursorSample, CursorStyle, Ease, EditPlan, EventTrack, KeyCastEvent, KeySample,
    ZoomKey, ZoomTrack,
};

/// Tunables for the auto-edit heuristics. Defaults are a sensible demo feel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineConfig {
    /// Peak zoom factor on a focus cluster (>= 1.0).
    pub max_zoom: f64,
    /// Zoom-in ramp duration (ms) before the focus moment.
    pub zoom_in_ms: u64,
    /// Minimum hold at peak zoom (ms).
    pub zoom_hold_min_ms: u64,
    /// Zoom-out ramp duration (ms).
    pub zoom_out_ms: u64,
    /// Click anchors closer than this in time merge into one zoom cluster (ms).
    pub dwell_merge_ms: u64,
    /// If the next cluster starts within this gap after the current hold ends,
    /// stay zoomed and pan to it instead of zooming out
    /// and back in. Larger = more continuous-zoom tours.
    pub stay_zoomed_gap_ms: u64,
    /// Cursor moving-average window (samples; odd numbers center cleanly).
    pub cursor_window: usize,
    /// Typed keys with gaps under this coalesce into one key-cast chip (ms).
    pub keycast_gap_ms: u64,
    /// How long a key-cast chip stays on screen (ms).
    pub keycast_hold_ms: u64,
    /// Output frame rate written into the plan.
    pub out_fps: f32,
    /// No-input span longer than this is reported as idle (ms).
    pub idle_threshold_ms: u64,
    /// Whether to populate idle spans (auto-cut is applied downstream).
    pub enable_idle: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_zoom: 2.0,
            zoom_in_ms: 550,
            zoom_hold_min_ms: 500,
            zoom_out_ms: 500,
            dwell_merge_ms: 700,
            stay_zoomed_gap_ms: 2500,
            cursor_window: 5,
            keycast_gap_ms: 700,
            keycast_hold_ms: 1200,
            out_fps: 30.0,
            idle_threshold_ms: 4000,
            enable_idle: false,
        }
    }
}

/// A detected idle span (no cursor/click/scroll/key activity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleSpan {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Run the full auto-edit pass: EventTrack → EditPlan.
pub fn autoedit(events: &EventTrack, cfg: &EngineConfig) -> EditPlan {
    let mut plan = EditPlan::empty(
        events.screen_w,
        events.screen_h,
        events.duration_ms,
        cfg.out_fps,
    );
    plan.zoom = build_zoom(events, cfg);
    plan.cursor = CursorStyle {
        smoothed: smooth_cursor(&events.cursor, cfg.cursor_window),
        ..CursorStyle::default()
    };
    plan.clicks = build_clicks(events);
    plan.keycast = build_keycast(&events.keys, cfg.keycast_gap_ms, cfg.keycast_hold_ms);
    plan
}

/// Clamp a focus center (fraction) so the zoom window stays inside the frame.
/// At scale `s` the visible window is 1/s wide, so the center must sit in
/// [0.5/s, 1 - 0.5/s].
fn clamp_center(c: f64, scale: f64) -> f64 {
    let half = 0.5 / scale.max(1.0);
    c.clamp(half, 1.0 - half)
}

/// One temporal cluster of click anchors (centroid + span).
struct Cluster {
    first: u64,
    last: u64,
    sum_x: f64,
    sum_y: f64,
    n: f64,
}

/// Build the eased auto-zoom timeline from click anchors.
pub fn build_zoom(events: &EventTrack, cfg: &EngineConfig) -> ZoomTrack {
    let w = events.screen_w.max(1) as f64;
    let h = events.screen_h.max(1) as f64;
    let dur = events.duration_ms;

    // Temporal clustering of click-down anchors.
    let mut clusters: Vec<Cluster> = Vec::new();
    for c in events.click_downs() {
        match clusters.last_mut() {
            Some(cl) if c.t_ms.saturating_sub(cl.last) <= cfg.dwell_merge_ms => {
                cl.last = c.t_ms;
                cl.sum_x += c.x;
                cl.sum_y += c.y;
                cl.n += 1.0;
            }
            _ => clusters.push(Cluster {
                first: c.t_ms,
                last: c.t_ms,
                sum_x: c.x,
                sum_y: c.y,
                n: 1.0,
            }),
        }
    }

    let mut track = ZoomTrack::default();
    if clusters.is_empty() {
        return track; // no anchors → no zoom (eval returns neutral)
    }
    let z = cfg.max_zoom;
    let eps = 1e-9;

    // Start neutral.
    track.keys.push(ZoomKey {
        t_ms: 0,
        scale: 1.0,
        cx: 0.5,
        cy: 0.5,
        ease: Ease::EaseInOut,
    });

    for (i, cl) in clusters.iter().enumerate() {
        let cx = clamp_center((cl.sum_x / cl.n) / w, z);
        let cy = clamp_center((cl.sum_y / cl.n) / h, z);
        let prev = *track.keys.last().unwrap();
        let already_zoomed = prev.scale > 1.0 + eps;

        if already_zoomed {
            // STAY zoomed: pan the focus from the previous cluster to this one,
            // arriving (still at peak zoom) by this cluster's first click.
            let arrive = cl.first.max(prev.t_ms + 1);
            track.keys.push(ZoomKey {
                t_ms: arrive,
                scale: z,
                cx,
                cy,
                ease: Ease::EaseInOut,
            });
        } else {
            // Zoom in: pre-position the center, then reach peak by the focus moment.
            let in_start = cl.first.saturating_sub(cfg.zoom_in_ms).max(prev.t_ms + 1);
            track.keys.push(ZoomKey {
                t_ms: in_start,
                scale: 1.0,
                cx,
                cy,
                ease: Ease::EaseInOut,
            });
            let in_end = cl.first.max(in_start + 1);
            track.keys.push(ZoomKey {
                t_ms: in_end,
                scale: z,
                cx,
                cy,
                ease: Ease::EaseInOut,
            });
        }

        // Hold at peak across the cluster's span.
        let hold_end = (cl.last + cfg.zoom_hold_min_ms).max(track.keys.last().unwrap().t_ms + 1);
        track.keys.push(ZoomKey {
            t_ms: hold_end,
            scale: z,
            cx,
            cy,
            ease: Ease::EaseInOut,
        });

        // Stay zoomed and pan to the next cluster if it starts soon; else zoom out.
        let stay_to_next = clusters
            .get(i + 1)
            .map(|n| n.first.saturating_sub(hold_end) <= cfg.stay_zoomed_gap_ms)
            .unwrap_or(false);
        if !stay_to_next {
            let out_end = (hold_end + cfg.zoom_out_ms).max(hold_end + 1);
            track.keys.push(ZoomKey {
                t_ms: out_end,
                scale: 1.0,
                cx,
                cy,
                ease: Ease::EaseInOut,
            });
        }
    }

    // Ensure we finish neutral.
    let last = *track.keys.last().unwrap();
    if last.scale > 1.0 + eps {
        let out_end = (last.t_ms + cfg.zoom_out_ms).max(last.t_ms + 1);
        track.keys.push(ZoomKey {
            t_ms: out_end,
            scale: 1.0,
            cx: last.cx,
            cy: last.cy,
            ease: Ease::EaseInOut,
        });
    }
    let last_t = track.keys.last().unwrap().t_ms;
    if dur > last_t {
        track.keys.push(ZoomKey {
            t_ms: dur,
            scale: 1.0,
            cx: 0.5,
            cy: 0.5,
            ease: Ease::EaseInOut,
        });
    }

    track
}

/// Moving-average de-jitter of the cursor path. Preserves sample count + times.
pub fn smooth_cursor(cursor: &[CursorSample], window: usize) -> Vec<CursorSample> {
    let n = cursor.len();
    if n <= 2 || window <= 1 {
        return cursor.to_vec();
    }
    let half = window / 2;
    (0..n)
        .map(|i| {
            let start = i.saturating_sub(half);
            let end = i.saturating_add(half).min(n - 1);
            let samples = &cursor[start..=end];
            let (mut sx, mut sy) = (0.0f64, 0.0f64);
            for sample in samples {
                sx += sample.x;
                sy += sample.y;
            }
            let count = samples.len() as f64;
            CursorSample {
                t_ms: cursor[i].t_ms,
                x: sx / count,
                y: sy / count,
            }
        })
        .collect()
}

/// Click-downs → ripple highlight effects, in fraction-of-frame coordinates.
pub fn build_clicks(events: &EventTrack) -> Vec<ClickFx> {
    let w = events.screen_w.max(1) as f64;
    let h = events.screen_h.max(1) as f64;
    events
        .click_downs()
        .map(|c| ClickFx {
            t_ms: c.t_ms,
            x: c.x / w,
            y: c.y / h,
        })
        .collect()
}

/// Render a key label for the key-cast chip (special keys → glyphs).
fn render_key(k: &str) -> String {
    match k {
        " " | "Space" | "space" => "␣".to_string(),
        "Enter" | "Return" => "⏎".to_string(),
        "Backspace" => "⌫".to_string(),
        "Tab" => "⇥".to_string(),
        "Escape" | "Esc" => "⎋".to_string(),
        other => other.to_string(),
    }
}

/// Coalesce key-down events into on-screen key-cast chips.
pub fn build_keycast(keys: &[KeySample], gap_ms: u64, hold_ms: u64) -> Vec<KeyCastEvent> {
    let mut out: Vec<KeyCastEvent> = Vec::new();
    let mut text = String::new();
    let mut start = 0u64;
    let mut last = 0u64;

    for k in keys.iter().filter(|k| k.down) {
        if text.is_empty() {
            start = k.t_ms;
            text.push_str(&render_key(&k.key));
        } else if k.t_ms.saturating_sub(last) <= gap_ms {
            text.push_str(&render_key(&k.key));
        } else {
            out.push(KeyCastEvent {
                t_ms: start,
                text: std::mem::take(&mut text),
                hold_ms,
            });
            start = k.t_ms;
            text.push_str(&render_key(&k.key));
        }
        last = k.t_ms;
    }
    if !text.is_empty() {
        out.push(KeyCastEvent {
            t_ms: start,
            text,
            hold_ms,
        });
    }
    out
}

/// Find idle spans (no activity of any kind) longer than `threshold_ms`.
pub fn find_idle(events: &EventTrack, threshold_ms: u64) -> Vec<IdleSpan> {
    // Merge every event timestamp into one sorted activity timeline.
    let mut ts: Vec<u64> = Vec::new();
    ts.extend(events.cursor.iter().map(|s| s.t_ms));
    ts.extend(events.clicks.iter().map(|s| s.t_ms));
    ts.extend(events.scrolls.iter().map(|s| s.t_ms));
    ts.extend(events.keys.iter().map(|s| s.t_ms));
    ts.sort_unstable();
    ts.dedup();

    let mut spans = Vec::new();
    let mut prev = 0u64;
    for &t in &ts {
        if t.saturating_sub(prev) >= threshold_ms {
            spans.push(IdleSpan {
                start_ms: prev,
                end_ms: t,
            });
        }
        prev = t;
    }
    if events.duration_ms.saturating_sub(prev) >= threshold_ms {
        spans.push(IdleSpan {
            start_ms: prev,
            end_ms: events.duration_ms,
        });
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use record_core::fixtures;

    fn cfg() -> EngineConfig {
        EngineConfig::default()
    }

    #[test]
    fn zoom_peaks_at_each_click() {
        let ev = fixtures::generate("click-walkthrough").unwrap();
        let z = build_zoom(&ev, &cfg());
        // 6 click anchors, all > dwell_merge apart → 6 clusters.
        assert!(!z.keys.is_empty());
        // strictly increasing key times
        for w in z.keys.windows(2) {
            assert!(
                w[1].t_ms > w[0].t_ms,
                "non-monotonic zoom keys: {:?} {:?}",
                w[0],
                w[1]
            );
        }
        // at each click-down time the zoom is at (or essentially at) peak
        for c in ev.click_downs() {
            let (s, _, _) = z.eval(c.t_ms);
            assert!(
                s >= cfg().max_zoom - 1e-6,
                "click @{}ms zoom={s} (< max)",
                c.t_ms
            );
        }
    }

    #[test]
    fn zoom_stays_zoomed_and_pans_between_near_clusters() {
        // click-walkthrough clicks are ~2.7s apart → within stay_zoomed_gap, so the
        // zoom STAYS engaged and pans rather than zooming out between every click
        // (continuous-focus behavior).
        let ev = fixtures::generate("click-walkthrough").unwrap();
        let z = build_zoom(&ev, &cfg());
        let (s, _, _) = z.eval(1500);
        assert!(
            s > 1.5,
            "expected to stay zoomed while panning, got scale={s}"
        );
    }

    #[test]
    fn zoom_outs_on_long_gap() {
        // Only a very-early and a very-late click → gap > stay_zoomed_gap → the zoom
        // returns to neutral in the long middle.
        let mut ev = fixtures::generate("click-walkthrough").unwrap();
        ev.clicks.retain(|c| c.t_ms < 200 || c.t_ms > 13000);
        let z = build_zoom(&ev, &cfg());
        let (s, _, _) = z.eval(6000);
        assert!(
            (s - 1.0).abs() < 1e-6,
            "expected neutral in the long gap, got scale={s}"
        );
    }

    #[test]
    fn zoom_center_stays_in_frame() {
        let ev = fixtures::generate("click-walkthrough").unwrap();
        let z = build_zoom(&ev, &cfg());
        let half = 0.5 / cfg().max_zoom;
        for k in &z.keys {
            if k.scale > 1.0 + 1e-9 {
                assert!(
                    k.cx >= half - 1e-9 && k.cx <= 1.0 - half + 1e-9,
                    "cx {} out of frame",
                    k.cx
                );
                assert!(
                    k.cy >= half - 1e-9 && k.cy <= 1.0 - half + 1e-9,
                    "cy {} out of frame",
                    k.cy
                );
            }
        }
    }

    #[test]
    fn no_clicks_means_no_zoom() {
        let ev = fixtures::generate("scroll-read").unwrap();
        let z = build_zoom(&ev, &cfg());
        assert!(z.keys.is_empty(), "scroll-read has no clicks → no zoom");
        assert_eq!(z.eval(1234), (1.0, 0.5, 0.5));
    }

    #[test]
    fn cursor_smoothing_preserves_shape() {
        let ev = fixtures::generate("click-walkthrough").unwrap();
        let sm = smooth_cursor(&ev.cursor, 5);
        assert_eq!(sm.len(), ev.cursor.len());
        for (a, b) in ev.cursor.iter().zip(sm.iter()) {
            assert_eq!(a.t_ms, b.t_ms); // timestamps preserved
            assert!(b.x >= 0.0 && b.x <= ev.screen_w as f64);
            assert!(b.y >= 0.0 && b.y <= ev.screen_h as f64);
        }
    }

    #[test]
    fn cursor_smoothing_handles_an_extreme_window_without_index_wrap() {
        let cursor = vec![
            CursorSample {
                t_ms: 0,
                x: 0.0,
                y: 10.0,
            },
            CursorSample {
                t_ms: 1,
                x: 10.0,
                y: 20.0,
            },
            CursorSample {
                t_ms: 2,
                x: 20.0,
                y: 30.0,
            },
        ];
        let smoothed = smooth_cursor(&cursor, usize::MAX);
        assert_eq!(smoothed.len(), cursor.len());
        assert!(smoothed.iter().all(|sample| sample.x == 10.0));
        assert!(smoothed.iter().all(|sample| sample.y == 20.0));
    }

    #[test]
    fn clicks_are_fractional_and_counted() {
        let ev = fixtures::generate("click-walkthrough").unwrap();
        let fx = build_clicks(&ev);
        assert_eq!(fx.len(), 6);
        for f in &fx {
            assert!((0.0..=1.0).contains(&f.x) && (0.0..=1.0).contains(&f.y));
        }
    }

    #[test]
    fn keycast_coalesces_into_one_chip() {
        let ev = fixtures::generate("scroll-read").unwrap();
        let kc = build_keycast(&ev.keys, 700, 1200);
        assert_eq!(kc.len(), 1, "typed 'shellx' should be one chip");
        assert_eq!(kc[0].text, "shellx");
    }

    #[test]
    fn keycast_splits_on_large_gap() {
        let keys = vec![
            KeySample {
                t_ms: 0,
                key: "a".into(),
                down: true,
            },
            KeySample {
                t_ms: 100,
                key: "b".into(),
                down: true,
            },
            KeySample {
                t_ms: 5000,
                key: "c".into(),
                down: true,
            },
        ];
        let kc = build_keycast(&keys, 700, 1000);
        assert_eq!(kc.len(), 2);
        assert_eq!(kc[0].text, "ab");
        assert_eq!(kc[1].text, "c");
    }

    #[test]
    fn autoedit_assembles_full_plan() {
        let ev = fixtures::generate("click-walkthrough").unwrap();
        let plan = autoedit(&ev, &cfg());
        assert_eq!(plan.source_w, ev.screen_w);
        assert_eq!(plan.source_h, ev.screen_h);
        assert_eq!(plan.fps, cfg().out_fps);
        assert!(!plan.zoom.keys.is_empty());
        assert_eq!(plan.cursor.smoothed.len(), ev.cursor.len());
        assert_eq!(plan.clicks.len(), 6);
        assert!(plan.frame.enabled);
    }

    #[test]
    fn idle_detection_finds_gaps() {
        // build-zoom fixture is dense; craft a sparse one
        let mut ev = fixtures::generate("scroll-read").unwrap();
        ev.cursor.push(CursorSample {
            t_ms: ev.duration_ms + 10_000,
            x: 0.0,
            y: 0.0,
        });
        ev.duration_ms += 10_000;
        let spans = find_idle(&ev, 4000);
        assert!(
            !spans.is_empty(),
            "expected an idle span before the late sample"
        );
    }
}
