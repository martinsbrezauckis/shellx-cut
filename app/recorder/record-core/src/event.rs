//! event.rs — `EventTrack`: the captured input stream that drives auto-edit.
//!
//! This is the data half of "record rich data, auto-infer the edit". The capture
//! layer (rdev/scap) fills these vectors; the engine reads them to synthesize the
//! zoom/cursor/keycast plan. All coordinates are SCREEN PIXELS (origin top-left of
//! the captured surface). Samples within each vector are sorted ascending by `t_ms`.

use serde::{Deserialize, Serialize};

/// Mouse button identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// A cursor position sample.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CursorSample {
    pub t_ms: u64,
    pub x: f64,
    pub y: f64,
}

/// How confidently a click's coordinates identify the cursor on the captured frame.
///
/// Older event tracks omit this field and deserialize as `Exact` to preserve their
/// established rendering behavior. New Wayland evdev clicks are deliberately marked
/// `Approximate` until a fresh PipeWire cursor-metadata sample is paired with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClickPositionQuality {
    #[default]
    Exact,
    Approximate,
    Unavailable,
}

/// A mouse button transition (press or release).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClickSample {
    pub t_ms: u64,
    pub x: f64,
    pub y: f64,
    pub button: MouseButton,
    /// true = press, false = release.
    pub down: bool,
    /// Whether `x`/`y` were measured exactly on the captured surface.
    #[serde(default)]
    pub position_quality: ClickPositionQuality,
}

/// The capture mechanism that supplied the cursor/click coordinate contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CursorCoordinateSource {
    /// Older captures have no preserved source identity.
    #[default]
    LegacyUnknown,
    /// rdevin supplied global desktop coordinates; an `Exact` state requires the
    /// active backend to validate and map them into the captured surface.
    RdevinAbsolute,
    /// SPA_META_Cursor was transformed from Wayland compositor space to frame pixels.
    WaylandPipewireMetadata,
    /// evdev supplied only accumulated relative deltas; never exact on its own.
    WaylandEvdevRelative,
}

/// Overall confidence in a recording's click coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CursorCoordinateState {
    Exact,
    Approximate,
    #[default]
    Unavailable,
}

/// Persisted cursor/click provenance for receipts, debug responses, and the Record UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CursorCorrelation {
    pub source: CursorCoordinateSource,
    pub state: CursorCoordinateState,
    /// Counts button transitions (press and release), matching `clicks`.
    #[serde(default)]
    pub exact_clicks: u32,
    /// Counts button transitions (press and release), matching `clicks`.
    #[serde(default)]
    pub approximate_clicks: u32,
    /// Counts button transitions (press and release), matching `clicks`.
    #[serde(default)]
    pub unavailable_clicks: u32,
    /// The maximum allowed click-to-metadata distance when PipeWire metadata is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_metadata_age_ms: Option<u64>,
    /// Human-readable degradation reason without exposing host-local paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl CursorCorrelation {
    pub fn exact(source: CursorCoordinateSource, click_count: usize) -> Self {
        Self {
            source,
            state: CursorCoordinateState::Exact,
            exact_clicks: u32::try_from(click_count).unwrap_or(u32::MAX),
            ..Self::default()
        }
    }
}

/// A scroll-wheel sample (dx/dy in wheel notches; negative dy = scroll down).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScrollSample {
    pub t_ms: u64,
    pub x: f64,
    pub y: f64,
    pub dx: f64,
    pub dy: f64,
}

/// A keyboard transition (for the key-cast overlay).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeySample {
    pub t_ms: u64,
    /// Human-readable key label (e.g. "a", "Enter", "Ctrl").
    pub key: String,
    pub down: bool,
}

/// A captured monitor's geometry in the virtual desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Monitor {
    pub id: u32,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub primary: bool,
}

/// The full captured input track for one recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTrack {
    /// Total recording length (ms).
    pub duration_ms: u64,
    /// Captured surface size (pixels) — the coordinate space for all samples.
    pub screen_w: u32,
    pub screen_h: u32,
    #[serde(default)]
    pub monitors: Vec<Monitor>,
    #[serde(default)]
    pub cursor: Vec<CursorSample>,
    #[serde(default)]
    pub clicks: Vec<ClickSample>,
    #[serde(default)]
    pub scrolls: Vec<ScrollSample>,
    #[serde(default)]
    pub keys: Vec<KeySample>,
    /// Provenance for cursor/click coordinates. New captures must not represent
    /// relative Wayland evdev positions as exact screen pixels.
    #[serde(default)]
    pub cursor_correlation: CursorCorrelation,
}

impl EventTrack {
    /// Linearly-interpolated cursor position (pixels) at `t_ms`.
    /// Returns `None` only when there are no cursor samples at all.
    /// Before the first / after the last sample, clamps to that endpoint.
    pub fn cursor_at(&self, t_ms: u64) -> Option<(f64, f64)> {
        let n = self.cursor.len();
        if n == 0 {
            return None;
        }
        let first = self.cursor[0];
        if t_ms <= first.t_ms {
            return Some((first.x, first.y));
        }
        let last = self.cursor[n - 1];
        if t_ms >= last.t_ms {
            return Some((last.x, last.y));
        }
        // Find bracket: cursor[i].t_ms <= t_ms < cursor[i+1].t_ms.
        let mut i = 0;
        while i + 1 < n && self.cursor[i + 1].t_ms <= t_ms {
            i += 1;
        }
        let a = self.cursor[i];
        let b = self.cursor[i + 1];
        let span = (b.t_ms - a.t_ms).max(1) as f64;
        let f = (t_ms - a.t_ms) as f64 / span;
        Some((a.x + (b.x - a.x) * f, a.y + (b.y - a.y) * f))
    }

    /// Press events only (button-down), in time order — the auto-zoom anchors.
    pub fn click_downs(&self) -> impl Iterator<Item = &ClickSample> {
        self.clicks
            .iter()
            .filter(|c| c.down && c.position_quality == ClickPositionQuality::Exact)
    }

    /// A copy with all coordinates and screen dimensions scaled by `factor`.
    /// Screen dims are rounded to even numbers (libx264 yuv420p needs even WxH).
    /// Used to render lighter/faster demos (e.g. a 1080p fixture down to 720p).
    pub fn scaled(&self, factor: f64) -> EventTrack {
        let sc = |v: f64| v * factor;
        let even = |v: f64| ((v.round() as u32) & !1).max(2);
        EventTrack {
            duration_ms: self.duration_ms,
            screen_w: even(self.screen_w as f64 * factor),
            screen_h: even(self.screen_h as f64 * factor),
            monitors: self
                .monitors
                .iter()
                .map(|m| Monitor {
                    id: m.id,
                    x: (m.x as f64 * factor) as i32,
                    y: (m.y as f64 * factor) as i32,
                    w: (m.w as f64 * factor) as u32,
                    h: (m.h as f64 * factor) as u32,
                    primary: m.primary,
                })
                .collect(),
            cursor: self
                .cursor
                .iter()
                .map(|c| CursorSample {
                    t_ms: c.t_ms,
                    x: sc(c.x),
                    y: sc(c.y),
                })
                .collect(),
            clicks: self
                .clicks
                .iter()
                .map(|c| ClickSample {
                    t_ms: c.t_ms,
                    x: sc(c.x),
                    y: sc(c.y),
                    button: c.button,
                    down: c.down,
                    position_quality: c.position_quality,
                })
                .collect(),
            scrolls: self
                .scrolls
                .iter()
                .map(|s| ScrollSample {
                    t_ms: s.t_ms,
                    x: sc(s.x),
                    y: sc(s.y),
                    dx: s.dx,
                    dy: s.dy,
                })
                .collect(),
            keys: self.keys.clone(),
            cursor_correlation: self.cursor_correlation.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> EventTrack {
        EventTrack {
            duration_ms: 1000,
            screen_w: 1000,
            screen_h: 1000,
            monitors: vec![],
            cursor: vec![
                CursorSample {
                    t_ms: 0,
                    x: 0.0,
                    y: 0.0,
                },
                CursorSample {
                    t_ms: 1000,
                    x: 100.0,
                    y: 200.0,
                },
            ],
            clicks: vec![
                ClickSample {
                    t_ms: 10,
                    x: 5.0,
                    y: 5.0,
                    button: MouseButton::Left,
                    down: true,
                    position_quality: ClickPositionQuality::Exact,
                },
                ClickSample {
                    t_ms: 40,
                    x: 5.0,
                    y: 5.0,
                    button: MouseButton::Left,
                    down: false,
                    position_quality: ClickPositionQuality::Exact,
                },
            ],
            scrolls: vec![],
            keys: vec![],
            cursor_correlation: CursorCorrelation::default(),
        }
    }

    #[test]
    fn cursor_interpolates_midpoint() {
        let t = track();
        let (x, y) = t.cursor_at(500).unwrap();
        assert!((x - 50.0).abs() < 1e-6, "x={x}");
        assert!((y - 100.0).abs() < 1e-6, "y={y}");
    }

    #[test]
    fn cursor_clamps_ends() {
        let t = track();
        assert_eq!(t.cursor_at(0), Some((0.0, 0.0)));
        assert_eq!(t.cursor_at(5000), Some((100.0, 200.0)));
    }

    #[test]
    fn click_downs_filters_releases() {
        let t = track();
        assert_eq!(t.click_downs().count(), 1);
    }

    #[test]
    fn empty_cursor_is_none() {
        let mut t = track();
        t.cursor.clear();
        assert_eq!(t.cursor_at(100), None);
    }

    #[test]
    fn approximate_clicks_do_not_become_auto_zoom_anchors() {
        let mut t = track();
        t.clicks[0].position_quality = ClickPositionQuality::Approximate;
        assert_eq!(t.click_downs().count(), 0);
    }

    #[test]
    fn legacy_tracks_preserve_clicks_but_admit_unknown_provenance() {
        let legacy: EventTrack = serde_json::from_str(
            r#"{"duration_ms":1,"screen_w":1,"screen_h":1,"clicks":[{"t_ms":0,"x":0.0,"y":0.0,"button":"left","down":true}]}"#,
        )
        .unwrap();
        assert_eq!(
            legacy.clicks[0].position_quality,
            ClickPositionQuality::Exact
        );
        assert_eq!(
            legacy.cursor_correlation.source,
            CursorCoordinateSource::LegacyUnknown
        );
        assert_eq!(
            legacy.cursor_correlation.state,
            CursorCoordinateState::Unavailable
        );
    }

    #[test]
    fn scaled_halves_dims_and_coords() {
        let mut t = track();
        t.screen_w = 1920;
        t.screen_h = 1080;
        let s = t.scaled(0.5);
        assert_eq!((s.screen_w, s.screen_h), (960, 540));
        assert_eq!(s.cursor[1].x, 50.0); // 100 * 0.5
        assert_eq!(s.cursor[1].y, 100.0); // 200 * 0.5
        assert_eq!(s.duration_ms, t.duration_ms);
    }
}
