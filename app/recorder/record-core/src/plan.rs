//! plan.rs — `EditPlan`: the auto-generated, non-destructive polish description.
//!
//! The engine reads an `EventTrack` and writes an `EditPlan`; the renderer reads
//! the `EditPlan` (+ source video) and bakes the polished output. Nothing here is
//! pixels-baked — the plan is config, re-rendered deterministically (mirrors how
//! ShellX Cut renders from project state). Coordinates that describe focus/position
//! are FRACTIONS of the source frame (0..1) so the plan is resolution-independent.

use serde::{Deserialize, Serialize};

use crate::color::Rgba;
use crate::ease::Ease;
use crate::event::CursorSample;

/// One auto-zoom keyframe: focus center (fraction of source) + zoom scale.
/// `ease` describes interpolation FROM this key to the next.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ZoomKey {
    pub t_ms: u64,
    /// Zoom factor, >= 1.0 (1.0 = no zoom).
    pub scale: f64,
    /// Focus center X as a fraction of source width [0,1].
    pub cx: f64,
    /// Focus center Y as a fraction of source height [0,1].
    pub cy: f64,
    #[serde(default)]
    pub ease: Ease,
}

/// The auto-zoom timeline: time-sorted keys. `eval` returns (scale, cx, cy).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ZoomTrack {
    pub keys: Vec<ZoomKey>,
}

impl ZoomTrack {
    /// Evaluate (scale, cx, cy) at `t_ms`. Empty track = no zoom, centered.
    /// Clamps before first / after last key.
    pub fn eval(&self, t_ms: u64) -> (f64, f64, f64) {
        let n = self.keys.len();
        if n == 0 {
            return (1.0, 0.5, 0.5);
        }
        let first = self.keys[0];
        if t_ms <= first.t_ms {
            return (first.scale, first.cx, first.cy);
        }
        let last = self.keys[n - 1];
        if t_ms >= last.t_ms {
            return (last.scale, last.cx, last.cy);
        }
        let mut i = 0;
        while i + 1 < n && self.keys[i + 1].t_ms <= t_ms {
            i += 1;
        }
        let a = self.keys[i];
        let b = self.keys[i + 1];
        let span = (b.t_ms - a.t_ms).max(1) as f64;
        let raw = (t_ms - a.t_ms) as f64 / span;
        let f = a.ease.apply(raw);
        (
            a.scale + (b.scale - a.scale) * f,
            a.cx + (b.cx - a.cx) * f,
            a.cy + (b.cy - a.cy) * f,
        )
    }
}

/// Synthetic-cursor render config + the smoothed path the engine produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CursorStyle {
    /// Whether the OS cursor was hidden at capture (so we render our own).
    pub hide_real: bool,
    /// Cursor render scale multiplier (1.0 = native).
    pub scale: f64,
    /// Draw expanding ripples on click.
    pub click_ripple: bool,
    /// Spline-smoothed cursor path (screen pixels). Filled by the engine.
    #[serde(default)]
    pub smoothed: Vec<CursorSample>,
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self {
            hide_real: true,
            scale: 1.6,
            click_ripple: true,
            smoothed: Vec::new(),
        }
    }
}

/// A click highlight effect (ripple) at a point in time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClickFx {
    pub t_ms: u64,
    /// Fraction of source frame.
    pub x: f64,
    pub y: f64,
}

/// A key-cast overlay event (rendered chip of recently-typed keys).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyCastEvent {
    pub t_ms: u64,
    pub text: String,
    /// How long the chip stays on screen (ms).
    pub hold_ms: u64,
}

/// Drop shadow under the styled frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    pub blur: f64,
    pub offset_y: f64,
    /// 0..1.
    pub opacity: f64,
    pub color: Rgba,
}

impl Default for Shadow {
    fn default() -> Self {
        Self {
            blur: 48.0,
            offset_y: 24.0,
            opacity: 0.35,
            color: Rgba::BLACK,
        }
    }
}

/// The styled frame around the captured surface (padding + rounded + shadow).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameStyle {
    pub enabled: bool,
    /// Padding as a fraction of the OUTPUT min dimension.
    pub padding: f64,
    /// Corner radius in output pixels.
    pub corner_radius: f64,
    #[serde(default)]
    pub shadow: Option<Shadow>,
}

impl Default for FrameStyle {
    fn default() -> Self {
        Self {
            enabled: true,
            padding: 0.06,
            corner_radius: 18.0,
            shadow: Some(Shadow::default()),
        }
    }
}

/// The background behind the framed capture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Background {
    Solid {
        color: Rgba,
    },
    LinearGradient {
        from: Rgba,
        to: Rgba,
        angle_deg: f64,
    },
    Image {
        path: String,
    },
    /// A blurred, scaled copy of the source frame.
    BlurScreen {
        sigma: f64,
    },
}

impl Default for Background {
    fn default() -> Self {
        // A soft neutral gradient — pleasant default backdrop.
        Background::LinearGradient {
            from: Rgba::rgb(36, 40, 54),
            to: Rgba::rgb(18, 20, 28),
            angle_deg: 135.0,
        }
    }
}

/// Webcam bubble shape.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum WebcamShape {
    Circle,
    RoundedRect { radius: f64 },
}

/// Corner anchor for overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    TopLeft,
    TopRight,
    #[default]
    BottomRight,
    BottomLeft,
}

/// A timed update captured from Recording Studio. Coordinates are normalized
/// fractions of the output frame. `x`/`y` represent the overlay's top-left
/// corner and `size` is the diameter/height fraction of output height.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WebcamKeyframe {
    pub t_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<WebcamShape>,
}

/// Resolved webcam state for one output frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WebcamPlacement {
    pub visible: bool,
    /// Normalized output-frame X, top-left corner.
    pub x: f64,
    /// Normalized output-frame Y, top-left corner.
    pub y: f64,
    /// Diameter/height fraction of output height.
    pub size: f64,
    pub shape: WebcamShape,
}

/// Circular/rounded webcam overlay composited over the output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebcamOverlay {
    /// Path to the webcam video stream.
    pub source: String,
    pub shape: WebcamShape,
    pub anchor: Anchor,
    /// Margin from the anchored corner, fraction of output height.
    pub margin: f64,
    /// Bubble diameter, fraction of output height.
    pub size: f64,
    /// Timed Studio keyframes captured while recording. Empty keeps the static
    /// anchor/margin/size behavior used by older plans.
    #[serde(default)]
    pub timeline: Vec<WebcamKeyframe>,
}

impl WebcamOverlay {
    /// Resolve the webcam placement at a render timestamp. Older plans with an
    /// empty timeline use the static anchor/margin/size fields.
    pub fn placement_at(&self, t_ms: u64, out_w: u32, out_h: u32) -> WebcamPlacement {
        let out_w = out_w.max(1) as f64;
        let out_h = out_h.max(1) as f64;
        let d = (self.size * out_h).max(2.0);
        let margin = self.margin.max(0.0) * out_h;
        let (x_px, y_px) = match self.anchor {
            Anchor::TopLeft => (margin, margin),
            Anchor::TopRight => (out_w - d - margin, margin),
            Anchor::BottomLeft => (margin, out_h - d - margin),
            Anchor::BottomRight => (out_w - d - margin, out_h - d - margin),
        };

        let mut placement = WebcamPlacement {
            visible: true,
            x: (x_px / out_w).clamp(0.0, 1.0),
            y: (y_px / out_h).clamp(0.0, 1.0),
            size: self.size,
            shape: self.shape,
        };

        for key in self.timeline.iter().take_while(|key| key.t_ms <= t_ms) {
            if let Some(visible) = key.visible {
                placement.visible = visible;
            }
            if let Some(x) = key.x {
                placement.x = x;
            }
            if let Some(y) = key.y {
                placement.y = y;
            }
            if let Some(size) = key.size {
                placement.size = size;
            }
            if let Some(shape) = key.shape {
                placement.shape = shape;
            }
        }

        placement
    }
}

/// Output aspect reframing (e.g. vertical 9:16, square 1:1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Reframe {
    /// Keep the source aspect.
    #[default]
    None,
    /// Reframe to w:h, smart-filling around the cursor focus.
    Aspect { w: u32, h: u32 },
}

/// Caption burn-in config. Consumes Cut's word-span transcript
/// (`receipts/<asset>.words.json`, Parakeet-TDT/onnx-asr — schema-compatible).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionStyle {
    /// Path to the word-span JSON (`{words:[{idx,word,start_ms,end_ms,...}]}`).
    pub words_path: String,
    pub font_px: f64,
    pub color: Rgba,
    pub box_color: Rgba,
}

/// The complete polish description for one recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditPlan {
    /// Source capture dimensions (pixels).
    pub source_w: u32,
    pub source_h: u32,
    pub duration_ms: u64,
    pub fps: f32,
    #[serde(default)]
    pub zoom: ZoomTrack,
    pub cursor: CursorStyle,
    #[serde(default)]
    pub clicks: Vec<ClickFx>,
    #[serde(default)]
    pub keycast: Vec<KeyCastEvent>,
    pub frame: FrameStyle,
    pub background: Background,
    #[serde(default)]
    pub webcam: Option<WebcamOverlay>,
    #[serde(default)]
    pub reframe: Reframe,
    #[serde(default)]
    pub captions: Option<CaptionStyle>,
}

impl EditPlan {
    /// A no-effects plan sized to a source — the engine starts here and fills it.
    pub fn empty(source_w: u32, source_h: u32, duration_ms: u64, fps: f32) -> Self {
        Self {
            source_w,
            source_h,
            duration_ms,
            fps,
            zoom: ZoomTrack::default(),
            cursor: CursorStyle::default(),
            clicks: Vec::new(),
            keycast: Vec::new(),
            frame: FrameStyle::default(),
            background: Background::default(),
            webcam: None,
            reframe: Reframe::None,
            captions: None,
        }
    }

    /// Validate dimensions / fps / effect counts before rendering, so malformed
    /// input (untrusted plans, huge media) returns a clear error instead of
    /// panicking on a divide-by-zero / `.expect()` or over-allocating a pixmap.
    /// Bounds are generous for a standalone tool; a daemon should tighten them.
    pub fn validate(&self) -> crate::error::Result<()> {
        use crate::error::{error_codes, RecordError};
        const MAX_DIM: u32 = 8192;
        const MAX_FPS: f32 = 240.0;
        const MAX_EFFECTS: usize = 200_000;
        let bad = |m: &str| {
            RecordError::new(
                error_codes::INVALID_ARGS,
                "invalid edit plan",
                m.to_string(),
            )
        };

        if self.source_w == 0 || self.source_h == 0 {
            return Err(bad("source dimensions must be non-zero"));
        }
        if self.source_w > MAX_DIM || self.source_h > MAX_DIM {
            return Err(bad("source dimensions exceed the 8192px limit"));
        }
        if !(self.fps.is_finite() && self.fps > 0.0 && self.fps <= MAX_FPS) {
            return Err(bad("fps must be finite and in (0, 240]"));
        }
        if let Reframe::Aspect { w, h } = self.reframe {
            if w == 0 || h == 0 {
                return Err(bad("reframe aspect width/height must be non-zero"));
            }
        }
        if self.zoom.keys.len() > MAX_EFFECTS
            || self.clicks.len() > MAX_EFFECTS
            || self.keycast.len() > MAX_EFFECTS
        {
            return Err(bad("too many zoom keys / click fx / key-cast events"));
        }
        if let Some(wc) = &self.webcam {
            if wc.source.trim().is_empty() {
                return Err(bad("webcam source must not be empty"));
            }
            if !(wc.margin.is_finite() && wc.margin >= 0.0 && wc.margin <= 1.0) {
                return Err(bad("webcam margin must be in [0, 1]"));
            }
            if !(wc.size.is_finite() && wc.size > 0.0 && wc.size <= 1.0) {
                return Err(bad("webcam bubble size must be in (0, 1]"));
            }
            let mut last_t = None;
            for key in &wc.timeline {
                if let Some(prev) = last_t {
                    if key.t_ms < prev {
                        return Err(bad("webcam timeline must be sorted by t_ms"));
                    }
                }
                last_t = Some(key.t_ms);

                if let Some(x) = key.x {
                    if !(x.is_finite() && (0.0..=1.0).contains(&x)) {
                        return Err(bad("webcam timeline x must be in [0, 1]"));
                    }
                }
                if let Some(y) = key.y {
                    if !(y.is_finite() && (0.0..=1.0).contains(&y)) {
                        return Err(bad("webcam timeline y must be in [0, 1]"));
                    }
                }
                if let Some(size) = key.size {
                    if !(size.is_finite() && size > 0.0 && size <= 1.0) {
                        return Err(bad("webcam timeline size must be in (0, 1]"));
                    }
                }
                if let Some(WebcamShape::RoundedRect { radius }) = key.shape {
                    if !(radius.is_finite() && radius >= 0.0) {
                        return Err(bad(
                            "webcam timeline rounded radius must be finite and >= 0",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod validate_tests {
    use super::*;

    #[test]
    fn rejects_zero_and_huge_dims() {
        assert!(EditPlan::empty(0, 1080, 1000, 30.0).validate().is_err());
        assert!(EditPlan::empty(1920, 0, 1000, 30.0).validate().is_err());
        assert!(EditPlan::empty(99999, 1080, 1000, 30.0).validate().is_err());
        assert!(EditPlan::empty(1920, 1080, 1000, 30.0).validate().is_ok());
    }

    #[test]
    fn rejects_bad_fps_and_reframe() {
        assert!(EditPlan::empty(1920, 1080, 1000, 0.0).validate().is_err());
        let mut p = EditPlan::empty(1920, 1080, 1000, 30.0);
        p.reframe = Reframe::Aspect { w: 9, h: 0 };
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_invalid_webcam_timeline() {
        let mut p = EditPlan::empty(1920, 1080, 1000, 30.0);
        p.webcam = Some(WebcamOverlay {
            source: "cam.mp4".into(),
            shape: WebcamShape::Circle,
            anchor: Anchor::BottomRight,
            margin: 0.04,
            size: 0.22,
            timeline: vec![WebcamKeyframe {
                t_ms: 0,
                visible: None,
                x: Some(1.2),
                y: Some(0.5),
                size: Some(0.22),
                shape: None,
            }],
        });
        assert!(p.validate().is_err());
    }

    #[test]
    fn rejects_unsorted_webcam_timeline() {
        let mut p = EditPlan::empty(1920, 1080, 1000, 30.0);
        p.webcam = Some(WebcamOverlay {
            source: "cam.mp4".into(),
            shape: WebcamShape::Circle,
            anchor: Anchor::BottomRight,
            margin: 0.04,
            size: 0.22,
            timeline: vec![
                WebcamKeyframe {
                    t_ms: 800,
                    visible: Some(true),
                    x: Some(0.70),
                    y: Some(0.68),
                    size: Some(0.22),
                    shape: None,
                },
                WebcamKeyframe {
                    t_ms: 400,
                    visible: Some(false),
                    x: None,
                    y: None,
                    size: None,
                    shape: None,
                },
            ],
        });
        assert!(p.validate().is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_eval_eases_between_keys() {
        let z = ZoomTrack {
            keys: vec![
                ZoomKey {
                    t_ms: 0,
                    scale: 1.0,
                    cx: 0.5,
                    cy: 0.5,
                    ease: Ease::Linear,
                },
                ZoomKey {
                    t_ms: 1000,
                    scale: 2.0,
                    cx: 0.2,
                    cy: 0.8,
                    ease: Ease::Linear,
                },
            ],
        };
        let (s, cx, cy) = z.eval(500);
        assert!((s - 1.5).abs() < 1e-6, "scale={s}");
        assert!((cx - 0.35).abs() < 1e-6, "cx={cx}");
        assert!((cy - 0.65).abs() < 1e-6, "cy={cy}");
    }

    #[test]
    fn zoom_eval_clamps_and_handles_empty() {
        let empty = ZoomTrack::default();
        assert_eq!(empty.eval(123), (1.0, 0.5, 0.5));
        let z = ZoomTrack {
            keys: vec![ZoomKey {
                t_ms: 100,
                scale: 3.0,
                cx: 0.1,
                cy: 0.1,
                ease: Ease::EaseInOut,
            }],
        };
        assert_eq!(z.eval(0), (3.0, 0.1, 0.1));
        assert_eq!(z.eval(99999), (3.0, 0.1, 0.1));
    }

    #[test]
    fn editplan_roundtrips_json() {
        let mut p = EditPlan::empty(1920, 1080, 20000, 30.0);
        p.zoom.keys.push(ZoomKey {
            t_ms: 0,
            scale: 1.0,
            cx: 0.5,
            cy: 0.5,
            ease: Ease::EaseInOut,
        });
        p.webcam = Some(WebcamOverlay {
            source: "cam.mp4".into(),
            shape: WebcamShape::Circle,
            anchor: Anchor::BottomRight,
            margin: 0.04,
            size: 0.22,
            timeline: vec![WebcamKeyframe {
                t_ms: 1200,
                visible: Some(false),
                x: None,
                y: None,
                size: None,
                shape: None,
            }],
        });
        p.reframe = Reframe::Aspect { w: 9, h: 16 };
        let json = serde_json::to_string(&p).unwrap();
        let back: EditPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn webcam_timeline_resolves_static_anchor_and_live_events() {
        let wc = WebcamOverlay {
            source: "cam.mp4".into(),
            shape: WebcamShape::Circle,
            anchor: Anchor::BottomRight,
            margin: 0.05,
            size: 0.20,
            timeline: vec![
                WebcamKeyframe {
                    t_ms: 1000,
                    visible: Some(true),
                    x: Some(0.10),
                    y: Some(0.20),
                    size: Some(0.30),
                    shape: Some(WebcamShape::RoundedRect { radius: 18.0 }),
                },
                WebcamKeyframe {
                    t_ms: 2000,
                    visible: Some(false),
                    x: None,
                    y: None,
                    size: None,
                    shape: None,
                },
            ],
        };

        let before = wc.placement_at(0, 1920, 1080);
        assert!(before.visible);
        assert!((before.size - 0.20).abs() < 1e-6);
        assert!(
            before.x > 0.80,
            "bottom-right anchor should start near right edge"
        );

        let after_move = wc.placement_at(1500, 1920, 1080);
        assert!(after_move.visible);
        assert!((after_move.x - 0.10).abs() < 1e-6);
        assert!((after_move.y - 0.20).abs() < 1e-6);
        assert!((after_move.size - 0.30).abs() < 1e-6);
        assert_eq!(after_move.shape, WebcamShape::RoundedRect { radius: 18.0 });

        let hidden = wc.placement_at(2500, 1920, 1080);
        assert!(!hidden.visible);
    }
}
