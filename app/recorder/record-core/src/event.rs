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

/// A mouse button transition (press or release).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClickSample {
    pub t_ms: u64,
    pub x: f64,
    pub y: f64,
    pub button: MouseButton,
    /// true = press, false = release.
    pub down: bool,
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
        self.clicks.iter().filter(|c| c.down)
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
                },
                ClickSample {
                    t_ms: 40,
                    x: 5.0,
                    y: 5.0,
                    button: MouseButton::Left,
                    down: false,
                },
            ],
            scrolls: vec![],
            keys: vec![],
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
