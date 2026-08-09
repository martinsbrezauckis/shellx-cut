//! title.rs — native animated motion-graphics TITLE rendering (no browser).
//!
//! Role: turn a declarative [`TitleSpec`] (canvas geometry + fps + duration +
//! a stack of animated [`TitleLayer`]s) into per-frame UNPREMULTIPLIED RGBA8
//! bitmaps. We OWN the animation: for each frame we interpolate every layer's
//! keyframes (opacity / translate / scale) under the layer's easing curve and
//! EMIT ONE STATIC SVG string for that instant. resvg/usvg only rasterize a
//! single static SVG — they have NO animation engine, which is exactly why we
//! drive the timeline ourselves and hand them a fresh snapshot each frame.
//!
//! Pipeline per frame:  TitleSpec + frame_idx
//!   → interpolate layers at this instant
//!   → build SVG string (`<g transform opacity>` per layer; `<rect>`/`<text>`)
//!   → usvg::Tree::from_str(svg, &Options{ fontdb with system fonts })
//!   → resvg::render(tree, identity, &mut pixmap)
//!   → un-premultiply tiny_skia's premultiplied BGRA-internal pixels into a
//!     plain RGBA8 Vec<u8> of length width*height*4.
//!
//! Dependencies: resvg 0.47 (re-exports usvg + tiny_skia), usvg 0.47,
//! tiny-skia 0.12, fontdb 0.23. NO ffmpeg and NO Node runtime.
//!
//! Primary callers: the renderer (render.rs / the title-clip path) calls
//! [`render_frame`] once per output frame and feeds the bytes into its frame
//! pipeline (overlay / encode). [`render_all_frames`] is a batch convenience.
//!
//! Determinism: output depends only on the spec + frame index + the resolved
//! system font for the requested family, so the same inputs on the same host
//! produce the same bytes. (Font availability is host-dependent — a missing
//! custom family falls back to the configured default so text still renders.)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// Public contract
// ---------------------------------------------------------------------------

/// A complete animated title: canvas geometry, timing, and a back-to-front
/// stack of layers. Layer order in the `Vec` is paint order (index 0 painted
/// first / underneath). All spatial fields on layers/keyframes are normalized
/// so a spec is resolution-independent; [`render_frame`] resolves them against
/// `width`/`height` at raster time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleSpec {
    /// Canvas width in pixels (the project geometry). Output rows are this wide.
    pub width: u32,
    /// Canvas height in pixels (the project geometry).
    pub height: u32,
    /// Frames per second of the rendered title (e.g. 30).
    pub fps: u32,
    /// Title length in milliseconds; drives [`frame_count`] and the time map.
    pub duration_ms: u64,
    /// Back-to-front layer stack. Must be non-empty (validated).
    pub layers: Vec<TitleLayer>,
}

/// One animated layer: a piece of content placed in a normalized box, animated
/// over time by its keyframes under a single easing curve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleLayer {
    /// What this layer draws (text or a filled rounded rectangle).
    pub content: LayerContent,
    /// Layer box, NORMALIZED canvas coords in `[0,1]`: left edge.
    pub x: f64,
    /// Layer box top edge, normalized `[0,1]`.
    pub y: f64,
    /// Layer box width, normalized fraction of canvas width.
    pub w: f64,
    /// Layer box height, normalized fraction of canvas height.
    pub h: f64,
    /// Animation keyframes; must contain ≥1 entry sorted by `t` ascending
    /// (validated). With exactly one keyframe the layer is static at it.
    pub keyframes: Vec<Keyframe>,
    /// Easing applied to the fraction BETWEEN each surrounding keyframe pair.
    pub easing: Easing,
}

/// The drawable payload of a [`TitleLayer`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LayerContent {
    /// A single run of text rendered with a system font.
    Text {
        /// The string to render (raw text; XML-escaped before emission).
        text: String,
        /// Preferred font family (e.g. "DejaVu Sans"); falls back to the
        /// configured default family if unavailable so text still renders.
        font_family: String,
        /// Font size in pixels (canvas px, not normalized).
        font_px: f64,
        /// Fill color as `#RRGGBB` (validated; bad hex → error, no panic).
        color: String,
        /// Horizontal alignment of the text within the layer box.
        align: TextAlign,
        /// CSS font weight (e.g. 400 normal, 700 bold).
        weight: u32,
    },
    /// A filled rounded rectangle covering the layer box.
    Rect {
        /// Fill color as `#RRGGBB` (validated).
        color: String,
        /// Constant fill opacity `0..=1` (multiplied with keyframe opacity).
        opacity: f64,
        /// Corner radius in pixels (canvas px); 0 = sharp corners.
        radius_px: f64,
    },
    /// A rounded rectangle with an OPTIONAL fill AND an optional stroke (border)
    /// covering the layer box — the styled-callout / outlined-box primitive
    ///. Distinct from [`LayerContent::Rect`] (which the title presets use
    /// unchanged) so existing titles render byte-identically.
    StrokeBox {
        /// Fill color `#RRGGBB`, or None for no fill (outline only).
        fill: Option<String>,
        /// Fill opacity `0..=1` (ignored when `fill` is None).
        opacity: f64,
        /// Stroke color `#RRGGBB`, or None for no border.
        stroke: Option<String>,
        /// Stroke width in canvas px (0 / None stroke = no border).
        stroke_px: f64,
        /// Corner radius in canvas px; 0 = sharp corners.
        radius_px: f64,
    },
    /// An ellipse inscribed in the layer box — optional fill + optional
    /// stroke, same conventions as [`LayerContent::StrokeBox`].
    Ellipse {
        /// Fill color `#RRGGBB`, or None for outline only.
        fill: Option<String>,
        /// Fill opacity `0..=1` (ignored when `fill` is None).
        opacity: f64,
        /// Stroke color `#RRGGBB`, or None for no border.
        stroke: Option<String>,
        /// Stroke width in canvas px.
        stroke_px: f64,
    },
    /// A straight line between two NORMALIZED canvas points. The layer box
    /// should be the endpoints' bounding box (so translate/scale animate sanely).
    Line {
        /// Start point, normalized canvas coords `[0,1]`.
        x1: f64,
        y1: f64,
        /// End point, normalized canvas coords `[0,1]`.
        x2: f64,
        y2: f64,
        /// Stroke color `#RRGGBB`.
        color: String,
        /// Stroke width in canvas px.
        width_px: f64,
    },
    /// A straight line with a solid arrowhead at the end point — the
    /// annotation arrow. Same coord conventions as [`LayerContent::Line`].
    Arrow {
        /// Start (tail) point, normalized canvas coords `[0,1]`.
        x1: f64,
        y1: f64,
        /// End (head) point, normalized canvas coords `[0,1]`.
        x2: f64,
        y2: f64,
        /// Color `#RRGGBB` (line + head).
        color: String,
        /// Line width in canvas px.
        width_px: f64,
        /// Arrowhead length in canvas px (the head's reach back along the line).
        head_px: f64,
    },
}

/// Horizontal text alignment within the layer box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    /// Anchor text to the box's left edge.
    Left,
    /// Anchor text to the box's horizontal centre.
    Center,
    /// Anchor text to the box's right edge.
    Right,
}

/// Easing curve mapping a linear inter-keyframe fraction `[0,1]` to an eased
/// fraction `[0,1]`. Applied to opacity, translate, and scale alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Easing {
    /// Identity — constant rate.
    Linear,
    /// Slow start, accelerating (quadratic).
    EaseIn,
    /// Fast start, decelerating (quadratic).
    EaseOut,
    /// Slow start and end (smoothstep-like cubic).
    EaseInOut,
}

/// One animation keyframe at normalized time `t`. Between two surrounding
/// keyframes each field is interpolated; the easing curve reshapes the
/// fraction used for that interpolation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Keyframe {
    /// Normalized time `[0,1]` over `duration_ms` (0 = start, 1 = end).
    pub t: f64,
    /// Layer opacity `0..=1` at this instant.
    pub opacity: f64,
    /// Translate X as a FRACTION OF CANVAS WIDTH (0 = no shift).
    pub tx: f64,
    /// Translate Y as a FRACTION OF CANVAS HEIGHT (0 = no shift).
    pub ty: f64,
    /// Uniform scale about the layer-box centre (1 = no scaling).
    pub scale: f64,
}

/// Errors produced while validating a [`TitleSpec`] or rasterizing a frame.
/// Display strings are actionable so a caller can surface them verbatim.
#[derive(Debug, thiserror::Error)]
pub enum TitleError {
    /// `layers` was empty — nothing to draw.
    #[error("title spec has no layers")]
    NoLayers,
    /// `fps` was zero — cannot derive a frame timeline.
    #[error("title spec fps must be > 0")]
    ZeroFps,
    /// `duration_ms` was zero — cannot normalize time.
    #[error("title spec duration_ms must be > 0")]
    ZeroDuration,
    /// A layer had zero keyframes (carries the layer index).
    #[error("layer {0} has no keyframes (need ≥1)")]
    NoKeyframes(usize),
    /// A layer's keyframes were not sorted ascending by `t` (carries index).
    #[error("layer {0} keyframes are not sorted ascending by t")]
    UnsortedKeyframes(usize),
    /// A color string was not a valid `#RRGGBB` (carries the offending value).
    #[error("invalid color '{0}': expected #RRGGBB")]
    BadColor(String),
    /// A numeric layout/keyframe invariant was invalid.
    #[error("invalid title spec: {0}")]
    InvalidSpec(String),
    /// usvg failed to parse the SVG we emitted (should not happen; defensive).
    #[error("svg parse failed: {0}")]
    SvgParse(String),
    /// Writing a PNG frame failed.
    #[error("png write failed: {0}")]
    PngWrite(String),
    /// tiny-skia could not allocate the target pixmap (e.g. 0-sized canvas).
    #[error("failed to allocate {0}x{1} pixmap")]
    PixmapAlloc(u32, u32),
}

// ---------------------------------------------------------------------------
// Frame timing
// ---------------------------------------------------------------------------

/// Number of frames the title produces: `ceil(duration_ms * fps / 1000)`,
/// clamped to a minimum of 1. Uses integer ceil to avoid float rounding drift.
///
/// Edge cases: `fps == 0` or `duration_ms == 0` would be invalid specs; this
/// function is total and returns 1 in those cases (validation in
/// [`render_frame`] rejects them before any rasterization).
pub fn frame_count(spec: &TitleSpec) -> u32 {
    if spec.fps == 0 || spec.duration_ms == 0 {
        return 1;
    }
    // ceil(duration_ms * fps / 1000) in u128 to avoid overflow on large specs.
    let num = (spec.duration_ms as u128) * (spec.fps as u128);
    let frames = num.div_ceil(1000);
    (frames.max(1)).min(u32::MAX as u128) as u32
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render a single frame to UNPREMULTIPLIED RGBA8 bytes of length
/// `width * height * 4`.
///
/// Steps (matching the module pipeline):
/// 1. Validate the spec (empty layers / zero fps / zero duration / bad
///    keyframes / bad color → [`TitleError`]).
/// 2. Clamp `frame_idx` to the last frame if it is `>= frame_count` (NOT an
///    error — callers may overshoot the final frame).
/// 3. Map the frame to normalized time `tnorm = (frame_idx*1000/fps /
///    duration_ms).clamp(0,1)` and interpolate every layer there.
/// 4. Emit one static SVG and rasterize it with resvg into a tiny-skia pixmap.
/// 5. Convert tiny-skia's premultiplied pixels to plain unpremultiplied RGBA8.
///
/// The returned bytes are row-major, top-left origin, R,G,B,A per pixel.
pub fn render_frame(spec: &TitleSpec, frame_idx: u32) -> Result<Vec<u8>, TitleError> {
    validate(spec)?;

    let total = frame_count(spec);
    // Clamp overshoot to the last frame instead of erroring (per contract).
    let idx = frame_idx.min(total.saturating_sub(1));

    // Frame → milliseconds → normalized time over the title duration.
    // t_ms uses u128 so frame_idx*1000 cannot overflow for any u32 frame.
    let t_ms = (idx as u128) * 1000 / (spec.fps as u128);
    let tnorm = ((t_ms as f64) / (spec.duration_ms as f64)).clamp(0.0, 1.0);

    let svg = build_svg(spec, tnorm)?;
    rasterize(spec.width, spec.height, &svg)
}

/// Render a single frame directly to a PNG file. This uses the same
/// spec-validation, timing, SVG emission, font database, and resvg rasterizer as
/// [`render_frame`]; only the final sink differs.
pub fn render_frame_png(spec: &TitleSpec, frame_idx: u32, out: &Path) -> Result<(), TitleError> {
    validate(spec)?;

    let total = frame_count(spec);
    let idx = frame_idx.min(total.saturating_sub(1));
    let t_ms = (idx as u128) * 1000 / (spec.fps as u128);
    let tnorm = ((t_ms as f64) / (spec.duration_ms as f64)).clamp(0.0, 1.0);

    let svg = build_svg(spec, tnorm)?;
    rasterize_png(spec.width, spec.height, &svg, out)
}

/// Convenience: render every frame of the title in order. Returns a `Vec` of
/// per-frame RGBA8 buffers (each `width*height*4` long). Fails fast on the
/// first error (validation errors surface on frame 0).
pub fn render_all_frames(spec: &TitleSpec) -> Result<Vec<Vec<u8>>, TitleError> {
    let total = frame_count(spec);
    let mut out = Vec::new();
    for i in 0..total {
        out.push(render_frame(spec, i)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Presets — friendly TitleSpec builders behind the `title.add` verb. Each takes
// the project geometry/fps + the placement duration and returns a ready spec.
// These encode the animation taste (entry/exit fades, slide/scale) so callers
// pass only text + a few knobs; the raw TitleSpec stays available for full
// control.
// ---------------------------------------------------------------------------

/// Common 4-keyframe entry/exit envelope: fade (+ optional slide `ty0`/scale
/// `s0` on entry) in over the first 15%, hold, fade out over the last 15%.
fn envelope(ty0: f64, s0: f64) -> Vec<Keyframe> {
    vec![
        Keyframe {
            t: 0.0,
            opacity: 0.0,
            tx: 0.0,
            ty: ty0,
            scale: s0,
        },
        Keyframe {
            t: 0.15,
            opacity: 1.0,
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
        },
        Keyframe {
            t: 0.85,
            opacity: 1.0,
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
        },
        Keyframe {
            t: 1.0,
            opacity: 0.0,
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
        },
    ]
}

/// Generalized entry/exit envelope: fade (+ optional translate `tx0`/`ty0` and
/// scale `s0` on entry) in over the first 15%, hold, fade out over the last 15%.
/// `envelope(ty0, s0)` is `envelope_full(0, ty0, s0)`.
fn envelope_full(tx0: f64, ty0: f64, s0: f64) -> Vec<Keyframe> {
    vec![
        Keyframe {
            t: 0.0,
            opacity: 0.0,
            tx: tx0,
            ty: ty0,
            scale: s0,
        },
        Keyframe {
            t: 0.15,
            opacity: 1.0,
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
        },
        Keyframe {
            t: 0.85,
            opacity: 1.0,
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
        },
        Keyframe {
            t: 1.0,
            opacity: 0.0,
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
        },
    ]
}

/// Entry/exit keyframes for a named ANIMATION (the `title.add {animation}` knob),
/// applied as an ORTHOGONAL override on top of any preset's layers (position is
/// preserved; only the motion changes). Returns None for an unknown name so the
/// verb can reject it. `none` = a single static, fully-opaque keyframe (no fade).
pub fn animation_keyframes(anim: &str) -> Option<Vec<Keyframe>> {
    let kf = match anim {
        "fade" => envelope_full(0.0, 0.0, 1.0),
        "slide_up" => envelope_full(0.0, 0.06, 1.0),
        "slide_down" => envelope_full(0.0, -0.06, 1.0),
        "slide_left" => envelope_full(0.06, 0.0, 1.0),
        "slide_right" => envelope_full(-0.06, 0.0, 1.0),
        "pop" => envelope_full(0.0, 0.0, 0.6), // scale up from 60%
        "none" => vec![Keyframe {
            t: 0.0,
            opacity: 1.0,
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
        }],
        _ => return None,
    };
    Some(kf)
}

/// The animation names `title.add {animation}` accepts (and `animation_keyframes`
/// resolves). Drives the verb's validation + the schema enum.
pub const TITLE_ANIMATIONS: &[&str] = &[
    "fade",
    "slide_up",
    "slide_down",
    "slide_left",
    "slide_right",
    "pop",
    "none",
];

/// "Top bar": text near the TOP-left (optionally on a translucent bar) — a
/// lower-third pinned to the top of frame (chyron / source label). Slides DOWN in.
pub fn top_bar(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
    bg: bool,
) -> TitleSpec {
    let font_px = fit_font_px_to_width(text, "DejaVu Sans", font_px, 700, width as f64 * 0.56);
    let mut layers = Vec::new();
    if bg {
        layers.push(TitleLayer {
            content: LayerContent::Rect {
                color: "#000000".into(),
                opacity: 0.5,
                radius_px: 8.0,
            },
            x: 0.06,
            y: 0.06,
            w: 0.62,
            h: 0.11,
            keyframes: envelope_full(0.0, -0.05, 1.0),
            easing: Easing::EaseOut,
        });
    }
    layers.push(TitleLayer {
        content: LayerContent::Text {
            text: text.into(),
            font_family: "DejaVu Sans".into(),
            font_px,
            color: color.into(),
            align: TextAlign::Left,
            weight: 700,
        },
        x: 0.09,
        y: 0.06,
        w: 0.56,
        h: 0.11,
        keyframes: envelope_full(0.0, -0.05, 1.0),
        easing: Easing::EaseOut,
    });
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    }
}

/// "Subtitle": small CENTERED text near the bottom (a subtitle/credit line). Pure
/// fade in/out, no bar by default — sits quietly under the action.
pub fn subtitle(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
) -> TitleSpec {
    let font_px = fit_font_px_to_width(text, "DejaVu Sans", font_px, 600, width as f64 * 0.80);
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers: vec![TitleLayer {
            content: LayerContent::Text {
                text: text.into(),
                font_family: "DejaVu Sans".into(),
                font_px,
                color: color.into(),
                align: TextAlign::Center,
                weight: 600,
            },
            x: 0.1,
            y: 0.86,
            w: 0.8,
            h: 0.09,
            keyframes: envelope_full(0.0, 0.0, 1.0),
            easing: Easing::EaseOut,
        }],
    }
}

/// "Headline": large BOLD centered text in the upper third — a news-style headline
/// / section banner. Fades + slides up on entry.
pub fn headline(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
) -> TitleSpec {
    let font_px = fit_font_px_to_width(text, "DejaVu Sans", font_px, 800, width as f64 * 0.84);
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers: vec![TitleLayer {
            content: LayerContent::Text {
                text: text.into(),
                font_family: "DejaVu Sans".into(),
                font_px,
                color: color.into(),
                align: TextAlign::Center,
                weight: 800,
            },
            x: 0.08,
            y: 0.16,
            w: 0.84,
            h: 0.15,
            keyframes: envelope_full(0.0, 0.05, 1.0),
            easing: Easing::EaseOut,
        }],
    }
}

/// "Lower third": text near the bottom-left (optionally on a translucent bar),
/// fading + sliding up on entry and fading out on exit. The default preset.
pub fn lower_third(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
    bg: bool,
) -> TitleSpec {
    let font_px = fit_font_px_to_width(text, "DejaVu Sans", font_px, 700, width as f64 * 0.56);
    let mut layers = Vec::new();
    if bg {
        layers.push(TitleLayer {
            content: LayerContent::Rect {
                color: "#000000".into(),
                opacity: 0.5,
                radius_px: 8.0,
            },
            x: 0.06,
            y: 0.785,
            w: 0.62,
            h: 0.11,
            keyframes: envelope(0.05, 1.0),
            easing: Easing::EaseOut,
        });
    }
    layers.push(TitleLayer {
        content: LayerContent::Text {
            text: text.into(),
            font_family: "DejaVu Sans".into(),
            font_px,
            color: color.into(),
            align: TextAlign::Left,
            weight: 700,
        },
        x: 0.09,
        y: 0.785,
        w: 0.56,
        h: 0.11,
        keyframes: envelope(0.05, 1.0),
        easing: Easing::EaseOut,
    });
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    }
}

/// "Title card": large centred text that fades + scales up on entry, holds,
/// then fades out — for intros / outros.
pub fn title_card(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
) -> TitleSpec {
    let font_px = fit_font_px_to_width(text, "DejaVu Sans", font_px, 800, width as f64 * 0.80);
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers: vec![TitleLayer {
            content: LayerContent::Text {
                text: text.into(),
                font_family: "DejaVu Sans".into(),
                font_px,
                color: color.into(),
                align: TextAlign::Center,
                weight: 800,
            },
            x: 0.1,
            y: 0.42,
            w: 0.8,
            h: 0.16,
            keyframes: envelope(0.0, 0.92),
            easing: Easing::EaseInOut,
        }],
    }
}

/// Free placement: text anchored at a normalized point `(cx, cy)` ∈ `[0,1]` —
/// the "drop the title anywhere on the frame" path behind `title.add {x, y}`.
/// `align` is the text's horizontal anchor at `cx` (Center is the natural choice
/// for a dropped point; Left/Right let it hug an edge). Fades in/out (no slide)
/// so a freely-placed label simply appears where the user put it. With `bg` a
/// translucent pill is drawn behind the text for legibility over busy footage.
///
/// Geometry: the text layer's anchor is `bx + bw/2` for Center (see `build_svg`),
/// and the baseline sits at the box vertical centre — so we position the box so
/// its centre lands on `(cx, cy)`. SVG text is NOT clipped to the box, so the
/// nominal width only places the anchor; long text still renders in full.
#[allow(clippy::too_many_arguments)]
pub fn free_title(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
    cx: f64,
    cy: f64,
    align: TextAlign,
    bg: bool,
) -> TitleSpec {
    let cx = cx.clamp(0.0, 1.0);
    let cy = cy.clamp(0.0, 1.0);
    let w = 0.9_f64;
    let h = 0.16_f64;
    let font_px = fit_font_px_to_width(text, "DejaVu Sans", font_px, 700, width as f64 * w);
    // Box left so the requested alignment anchors AT cx (Center → cx is the box
    // h-centre; Left → cx is the box left; Right → cx is the box right edge).
    let x = match align {
        TextAlign::Left => cx,
        TextAlign::Center => cx - w / 2.0,
        TextAlign::Right => cx - w,
    };
    let y = cy - h / 2.0;
    let mut layers = Vec::new();
    if bg {
        // A translucent pill sized to the actual fontdb text bounds, centred on
        // (cx, cy) and clamped on-canvas. Cosmetic — legibility, not layout.
        let measured = measure_text_width(text, "DejaVu Sans", font_px, 700);
        let bw = ((measured / width as f64) + 0.05).clamp(0.06, 0.96);
        let bh = ((font_px * 1.7) / height as f64).clamp(0.04, 0.5);
        layers.push(TitleLayer {
            content: LayerContent::Rect {
                color: "#000000".into(),
                opacity: 0.5,
                radius_px: 8.0,
            },
            x: (cx - bw / 2.0).clamp(0.0, 1.0 - bw),
            y: (cy - bh / 2.0).clamp(0.0, 1.0 - bh),
            w: bw,
            h: bh,
            keyframes: envelope(0.0, 1.0),
            easing: Easing::EaseOut,
        });
    }
    layers.push(TitleLayer {
        content: LayerContent::Text {
            text: text.into(),
            font_family: "DejaVu Sans".into(),
            font_px,
            color: color.into(),
            align,
            weight: 700,
        },
        x,
        y,
        w,
        h,
        keyframes: envelope(0.0, 1.0),
        easing: Easing::EaseOut,
    });
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    }
}

/// Build a KINETIC-CAPTION title from timed cues — the transcript-synced
/// animated subtitle. Each cue is `(start_norm, end_norm, text)` over the
/// title's `[0,1]` duration; a cue pops in (fade + scale-up) at its start,
/// holds, and fades out at its end, so the caption tracks the speech. `pos_y`
/// is the normalized vertical anchor (≈0.78 bottom, 0.45 centre). One Text layer
/// per cue (each invisible outside its window), so resvg only draws the active
/// cue(s). Keyframes are strictly increasing by construction (short cues
/// collapse their pop/hold cleanly).
pub fn kinetic(
    cues: &[(f64, f64, String)],
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
    pos_y: f64,
) -> TitleSpec {
    let pop = (300.0 / duration_ms.max(1) as f64).clamp(0.01, 0.2);
    let pos_y = if pos_y.is_finite() {
        pos_y.clamp(0.0, 1.0)
    } else {
        0.78
    };
    let layers = cues
        .iter()
        .map(|(a0, b0, text)| {
            let a = a0.clamp(0.0, 1.0);
            let b = b0.clamp(a, 1.0);
            let pin = (a + pop).min(b); // fully visible by here
            let fout = (b - pop).max(pin); // start fading here
                                           // Candidate keyframes; pushed only when strictly after the previous t
                                           // (collapses pop/hold for very short cues; validate() requires
                                           // ascending — and we keep it strict to avoid 0/0 interpolation).
            let cand: [(f64, f64, f64); 6] = [
                (0.0, 0.0, 0.85),
                (a, 0.0, 0.85),
                (pin, 1.0, 1.0),
                (fout, 1.0, 1.0),
                (b, 0.0, 1.0),
                (1.0, 0.0, 1.0),
            ];
            let mut kf: Vec<Keyframe> = Vec::new();
            for (t, op, sc) in cand {
                let t = t.clamp(0.0, 1.0);
                if kf.last().is_none_or(|k| t > k.t + 1e-9) {
                    kf.push(Keyframe {
                        t,
                        opacity: op,
                        tx: 0.0,
                        ty: 0.0,
                        scale: sc,
                    });
                }
            }
            if kf.is_empty() {
                kf.push(Keyframe {
                    t: 0.0,
                    opacity: 0.0,
                    tx: 0.0,
                    ty: 0.0,
                    scale: 1.0,
                });
            }
            TitleLayer {
                content: LayerContent::Text {
                    text: text.clone(),
                    font_family: "DejaVu Sans".into(),
                    font_px,
                    color: color.into(),
                    align: TextAlign::Center,
                    weight: 700,
                },
                x: 0.08,
                y: pos_y,
                w: 0.84,
                h: 0.12,
                keyframes: kf,
                easing: Easing::EaseOut,
            }
        })
        .collect();
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    }
}

// ---------------------------------------------------------------------------
// Animated-text TEMPLATES — keyframed, DATA-DRIVEN title looks built by
// decomposing the text into many staggered layers (per-character / per-word /
// per-row) on top of the SAME resvg per-frame engine. The invisible-layer cull
// in `build_svg` keeps the many-layer specs cheap: only the layer(s) visible at
// an instant are rasterized, so a 40-char typewriter (40 layers) draws ~1 layer
// per frame. Each builder returns a ready [`TitleSpec`]; the catalog
// [`TITLE_TEMPLATES`] drives the `title.templates` list verb + `title.add
// {template}` validation. Templates OWN their motion — the `animation` override
// is intentionally NOT applied on top of a template.
//
// Layout: where words/chars sit side by side, positions come from
// [`measure_text_width`] (the real laid-out width under the SAME fontdb the
// rasterizer uses), so spacing matches the final render rather than guessing.
// ---------------------------------------------------------------------------

/// One template's metadata for the `title.templates` catalog: a stable name, a
/// one-line human description, and the parameter keys the template honors.
#[derive(Debug, Clone, Serialize)]
pub struct TemplateInfo {
    /// Stable template id (the `title.add {template}` value).
    pub name: &'static str,
    /// One-line human description (shown in pickers / `title.templates`).
    pub description: &'static str,
    /// Parameter keys this template reads (besides the always-present text).
    pub params: &'static [&'static str],
}

/// The animated-text templates `title.add {template}` accepts. Slice order =
/// display order in `title.templates`. Adding one here + a `build_template` arm
/// is the entire surface (plus the schema enum, kept in lockstep).
pub const TITLE_TEMPLATES: &[TemplateInfo] = &[
    TemplateInfo {
        name: "typewriter",
        description: "Characters reveal left-to-right like typing, hold, then fade out.",
        params: &["color", "font_px"],
    },
    TemplateInfo {
        name: "word_pop",
        description: "Each word pops in (scale + fade) one at a time, centered — a punchy hook/lyric look.",
        params: &["color", "font_px"],
    },
    TemplateInfo {
        name: "slide_stack",
        description: "Words stack on rows, each sliding in from the left, staggered top-to-bottom.",
        params: &["color", "font_px"],
    },
    TemplateInfo {
        name: "kinetic_emphasis",
        description: "The phrase appears; one word scale-pops and recolors to `accent` for emphasis (pick it with `emphasis`, else the longest word).",
        params: &["color", "accent", "emphasis", "font_px"],
    },
    TemplateInfo {
        name: "lower_third_reveal",
        description: "A bar slides in from the left, then a primary (and optional secondary, split on '|') line reveals — a broadcast lower-third build.",
        params: &["color", "font_px"],
    },
    TemplateInfo {
        name: "caption_karaoke",
        description: "A dim full line with an `accent` highlight that fills word-by-word — a karaoke caption.",
        params: &["color", "accent", "font_px"],
    },
];

/// Just the template names — drives the schema enum + the verb's validation.
pub const TITLE_TEMPLATE_NAMES: &[&str] = &[
    "typewriter",
    "word_pop",
    "slide_stack",
    "kinetic_emphasis",
    "lower_third_reveal",
    "caption_karaoke",
];

/// True when `s` is a valid `#RRGGBB` color. Exposed so the verb can reject a
/// bad `color`/`accent` up front with a clean message instead of failing later
/// at encode time.
pub fn is_valid_color(s: &str) -> bool {
    parse_hex_color(s).is_ok()
}

/// Measure the laid-out WIDTH (canvas px) of `text` at the given font, using the
/// SAME fontdb the rasterizer uses — so the measured width matches what the
/// final frame actually draws. Parses a one-line SVG and reads the root group's
/// absolute bounding box. Empty / whitespace-only text → `0.0`. If usvg fails to
/// parse (should not happen for our generated SVG), falls back to a coarse
/// char-count estimate so layout still produces something sane.
pub fn measure_text_width(text: &str, font_family: &str, font_px: f64, weight: u32) -> f64 {
    if text.trim().is_empty() {
        return 0.0;
    }
    let fam = present_or_default(font_family, weight);
    // A very wide canvas so the text never clips the measurement viewport.
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200000\" height=\"{h}\">\
         <text x=\"0\" y=\"{y:.2}\" font-family=\"{fam}\" font-size=\"{fs:.4}\" \
         font-weight=\"{wt}\">{t}</text></svg>",
        h = (font_px * 2.0).ceil().max(2.0) as i64,
        y = font_px,
        fam = xml_escape(&fam),
        fs = font_px,
        wt = weight,
        t = xml_escape(text),
    );
    let f = fonts();
    let mut opt = usvg::Options {
        font_family: f.family.clone(),
        ..usvg::Options::default()
    };
    *opt.fontdb_mut() = f.db.clone();
    match usvg::Tree::from_str(&svg, &opt) {
        Ok(tree) => tree.root().abs_bounding_box().width() as f64,
        // Defensive fallback (never expected): ~0.6em/char average advance.
        Err(_) => text.chars().count() as f64 * font_px * 0.6,
    }
}

/// One space's advance (canvas px) at the given font, derived as
/// `width("A A") - width("AA")` — robust because ink bounds drop leading/trailing
/// whitespace, so a space can't be measured directly. Used to lay words out with
/// correct inter-word gaps.
fn space_width(font_family: &str, font_px: f64, weight: u32) -> f64 {
    let with = measure_text_width("A A", font_family, font_px, weight);
    let without = measure_text_width("AA", font_family, font_px, weight);
    (with - without).max(font_px * 0.22) // floor so a pathological font still spaces
}

fn fit_font_px_to_width(
    text: &str,
    font_family: &str,
    font_px: f64,
    weight: u32,
    max_width_px: f64,
) -> f64 {
    if !font_px.is_finite() || font_px <= 0.0 || !max_width_px.is_finite() || max_width_px <= 1.0 {
        return font_px;
    }
    let measured = measure_text_width(text, font_family, font_px, weight);
    if measured > max_width_px && measured > 0.0 {
        (font_px * (max_width_px / measured)).max(1.0)
    } else {
        font_px
    }
}

/// The normalized LEFT edge that horizontally centers a line of width
/// `line_w_px` on a `canvas_w`-wide canvas, clamped to a small left margin when
/// the line is wider than the canvas.
fn centered_left_norm(line_w_px: f64, canvas_w: u32) -> f64 {
    let cw = canvas_w as f64;
    (((cw - line_w_px) / 2.0) / cw).clamp(0.02, 0.98)
}

/// Push a keyframe only if its `t` is strictly after the last one (keeps the
/// vector strictly ascending, which `validate` requires and `interpolate` likes).
/// `t` is clamped to `[0,1]`. Coincident / non-increasing entries are dropped.
fn push_kf(kf: &mut Vec<Keyframe>, t: f64, opacity: f64, tx: f64, ty: f64, scale: f64) {
    let t = t.clamp(0.0, 1.0);
    if kf.last().is_none_or(|k| t > k.t + 1e-6) {
        kf.push(Keyframe {
            t,
            opacity,
            tx,
            ty,
            scale,
        });
    }
}

/// Hard-cut visibility keyframes for a prefix/segment layer: invisible, snap ON
/// at `on`, snap OFF at `off` — UNLESS `last`, in which case it holds visible
/// through the body and fades over the final 12%. `eps` is the snap ramp width
/// (a few ms) so the cut is crisp but never a zero-width span.
fn cut_keyframes(on: f64, off: f64, last: bool) -> Vec<Keyframe> {
    let eps = 0.004_f64;
    let on = on.clamp(0.0, 1.0);
    let mut kf: Vec<Keyframe> = Vec::new();
    push_kf(&mut kf, (on - eps).max(0.0), 0.0, 0.0, 0.0, 1.0);
    push_kf(&mut kf, on, 1.0, 0.0, 0.0, 1.0);
    if last {
        push_kf(&mut kf, 0.88, 1.0, 0.0, 0.0, 1.0);
        push_kf(&mut kf, 1.0, 0.0, 0.0, 0.0, 1.0);
    } else {
        push_kf(&mut kf, (off - eps).max(on + 1e-5), 1.0, 0.0, 0.0, 1.0);
        push_kf(&mut kf, off, 0.0, 0.0, 0.0, 1.0);
    }
    if kf.is_empty() {
        kf.push(Keyframe {
            t: 0.0,
            opacity: 0.0,
            tx: 0.0,
            ty: 0.0,
            scale: 1.0,
        });
    }
    kf
}

/// Build a named template into a ready [`TitleSpec`], or `None` for an unknown
/// name (so the verb can reject it). `accent` is used by `kinetic_emphasis` /
/// `caption_karaoke`; `emphasis` (a case-insensitive substring) selects the word
/// for `kinetic_emphasis` (else the longest word). Color validity is the
/// caller's pre-check (`is_valid_color`); `render_frame` validates again.
#[allow(clippy::too_many_arguments)]
pub fn build_template(
    name: &str,
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
    accent: &str,
    emphasis: Option<&str>,
) -> Option<TitleSpec> {
    let spec = match name {
        "typewriter" => tpl_typewriter(text, width, height, fps, duration_ms, color, font_px),
        "word_pop" => tpl_word_pop(text, width, height, fps, duration_ms, color, font_px),
        "slide_stack" => tpl_slide_stack(text, width, height, fps, duration_ms, color, font_px),
        "kinetic_emphasis" => tpl_kinetic_emphasis(
            text,
            width,
            height,
            fps,
            duration_ms,
            color,
            font_px,
            accent,
            emphasis,
        ),
        "lower_third_reveal" => {
            tpl_lower_third_reveal(text, width, height, fps, duration_ms, color, font_px)
        }
        "caption_karaoke" => tpl_caption_karaoke(
            text,
            width,
            height,
            fps,
            duration_ms,
            color,
            font_px,
            accent,
        ),
        _ => return None,
    };
    Some(spec)
}

const TPL_FONT: &str = "DejaVu Sans";

/// typewriter — reveal the text left-to-right one step at a time, hold, fade.
/// Each STEP is a left-aligned layer holding the FULL prefix so far (so kerning
/// and advances are exactly the engine's — no per-glyph layout math), shown for
/// its window with a hard cut to the next. Steps are characters when the title
/// is short, else words (keeps the layer count bounded for long titles).
fn tpl_typewriter(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
) -> TitleSpec {
    let chars: Vec<char> = text.chars().collect();
    // Prefix STEPS: per-character for short titles, per-word past the cap so the
    // layer count (and the few-ms typing cadence) stays sane.
    let steps: Vec<String> = if chars.len() <= 48 && !chars.is_empty() {
        (1..=chars.len())
            .map(|i| chars[..i].iter().collect())
            .collect()
    } else {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            vec![text.to_string()]
        } else {
            (1..=words.len()).map(|i| words[..i].join(" ")).collect()
        }
    };
    let n = steps.len().max(1);
    // The longest step (the full line) fixes the centered left edge; every step
    // shares it so the text grows in place rather than re-centering each frame.
    let full = steps.last().cloned().unwrap_or_default();
    let weight = 600;
    let font_px = fit_font_px_to_width(&full, TPL_FONT, font_px, weight, width as f64 * 0.92);
    let full_w = measure_text_width(&full, TPL_FONT, font_px, weight);
    let x = centered_left_norm(full_w, width);
    let w = ((full_w / width as f64) + 0.02).min(1.0 - x);
    let y = 0.6_f64;
    let h = (font_px * 1.6 / height as f64).clamp(0.06, 0.4);
    let typing = 0.72_f64; // typing occupies the first 72% of the duration

    let layers = steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let on = (i as f64 + 1.0) / n as f64 * typing;
            let off = (i as f64 + 2.0) / n as f64 * typing;
            let last = i == n - 1;
            TitleLayer {
                content: LayerContent::Text {
                    text: s.clone(),
                    font_family: TPL_FONT.into(),
                    font_px,
                    color: color.into(),
                    align: TextAlign::Left,
                    weight,
                },
                x,
                y,
                w,
                h,
                keyframes: cut_keyframes(on, off, last),
                easing: Easing::Linear,
            }
        })
        .collect();
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    }
}

/// word_pop — words pop in (scale-up + fade) one at a time and ACCUMULATE side by
/// side, the line building left-to-right, then the whole line holds and fades.
/// Each word sits at its measured position (so spacing matches the final render)
/// and pops about its own center. Accumulating (not replacing) means there is no
/// blank moment between words in a word-by-word karaoke build. If the
/// line would overflow the frame the font is scaled down to fit.
fn tpl_word_pop(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
) -> TitleSpec {
    let words: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<&str> = if words.is_empty() { vec![text] } else { words };
    let k = words.len().max(1);
    let weight = 800;

    // Measure each word + a space, then shrink the font to fit ~92% of the frame.
    let mut widths: Vec<f64> = words
        .iter()
        .map(|w| measure_text_width(w, TPL_FONT, font_px, weight))
        .collect();
    let mut sp = space_width(TPL_FONT, font_px, weight);
    let mut font_px = font_px;
    let total: f64 = widths.iter().sum::<f64>() + sp * (k as f64 - 1.0);
    let limit = width as f64 * 0.92;
    if total > limit && total > 0.0 {
        // Width scales ~linearly with font size for fixed text, so scale both the
        // font and the measured widths by the same fit factor (no re-measure).
        let fit = limit / total;
        font_px *= fit;
        for wv in &mut widths {
            *wv *= fit;
        }
        sp *= fit;
    }
    let total_fit: f64 = widths.iter().sum::<f64>() + sp * (k as f64 - 1.0);
    let start_px = ((width as f64 - total_fit) / 2.0).max(width as f64 * 0.02);

    let y = 0.40_f64;
    let h = (font_px * 1.7 / height as f64).clamp(0.08, 0.5);
    let mut acc = start_px;
    let layers = words
        .iter()
        .enumerate()
        .map(|(i, word)| {
            let left = acc;
            acc += widths[i] + sp;
            let s = i as f64 / k as f64 * 0.5; // last word starts popping by ~50%
            let mut kf = Vec::new();
            push_kf(&mut kf, 0.0, 0.0, 0.0, 0.0, 0.55);
            push_kf(&mut kf, s, 0.0, 0.0, 0.0, 0.55);
            push_kf(&mut kf, s + 0.06, 1.0, 0.0, 0.0, 1.12); // overshoot
            push_kf(&mut kf, s + 0.12, 1.0, 0.0, 0.0, 1.0); // settle
            push_kf(&mut kf, 0.88, 1.0, 0.0, 0.0, 1.0); // hold (accumulated)
            push_kf(&mut kf, 1.0, 0.0, 0.0, 0.0, 1.0); // fade out together
            TitleLayer {
                content: LayerContent::Text {
                    text: (*word).to_string(),
                    font_family: TPL_FONT.into(),
                    font_px,
                    color: color.into(),
                    align: TextAlign::Left,
                    weight,
                },
                x: (left / width as f64).clamp(0.0, 1.0),
                y,
                w: (widths[i] / width as f64 + 0.01).min(1.0),
                h,
                keyframes: kf,
                easing: Easing::EaseOut,
            }
        })
        .collect();
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    }
}

/// slide_stack — words (grouped into rows past a cap) stack vertically, each row
/// sliding in from the left with a staggered start; the block holds then fades.
/// Each row is centered full-width, so no horizontal layout math is needed.
fn tpl_slide_stack(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
) -> TitleSpec {
    const MAX_ROWS: usize = 7;
    let words: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<&str> = if words.is_empty() { vec![text] } else { words };
    let lines: Vec<String> = if words.len() <= MAX_ROWS {
        words.iter().map(|w| (*w).to_string()).collect()
    } else {
        let per = words.len().div_ceil(MAX_ROWS);
        words.chunks(per).map(|c| c.join(" ")).collect()
    };
    let r = lines.len().max(1);
    let weight = 800;
    let mut font_px = font_px;
    let max_line = lines
        .iter()
        .map(|line| measure_text_width(line, TPL_FONT, font_px, weight))
        .fold(0.0_f64, f64::max);
    let limit = width as f64 * 0.90;
    if max_line > limit && max_line > 0.0 {
        font_px = (font_px * (limit / max_line)).max(1.0);
    }
    let row_h = (font_px * 1.35 / height as f64).clamp(0.07, 0.9 / r as f64);
    let block_h = row_h * r as f64;
    let top = ((1.0 - block_h) / 2.0).clamp(0.04, 0.9);
    let stagger = (0.45 / r as f64).min(0.12);
    let layers = lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let s = i as f64 * stagger;
            let mut kf = Vec::new();
            push_kf(&mut kf, 0.0, 0.0, -0.30, 0.0, 1.0);
            push_kf(&mut kf, s, 0.0, -0.30, 0.0, 1.0);
            push_kf(&mut kf, s + 0.14, 1.0, 0.0, 0.0, 1.0);
            push_kf(&mut kf, 0.86, 1.0, 0.0, 0.0, 1.0);
            push_kf(&mut kf, 1.0, 0.0, 0.0, 0.0, 1.0);
            TitleLayer {
                content: LayerContent::Text {
                    text: line.clone(),
                    font_family: TPL_FONT.into(),
                    font_px,
                    color: color.into(),
                    align: TextAlign::Center,
                    weight,
                },
                x: 0.05,
                y: top + i as f64 * row_h,
                w: 0.90,
                h: row_h,
                keyframes: kf,
                easing: Easing::EaseOut,
            }
        })
        .collect();
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    }
}

/// kinetic_emphasis — the whole phrase slides up in `color`; ONE word
/// (the `emphasis` substring, else the longest word) is overlaid in `accent` and
/// scale-pops in place. The overlay shares the base line's baseline and the word
/// position is measured against the centered line so it lands exactly over the
/// base word.
#[allow(clippy::too_many_arguments)]
fn tpl_kinetic_emphasis(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
    accent: &str,
    emphasis: Option<&str>,
) -> TitleSpec {
    let weight = 800;
    let font_px = fit_font_px_to_width(text, TPL_FONT, font_px, weight, width as f64 * 0.90);
    let y = 0.40_f64;
    let h = (font_px * 1.7 / height as f64).clamp(0.08, 0.5);

    // Base line: full phrase, centered, slide-up + fade in/out.
    let mut base_kf = Vec::new();
    push_kf(&mut base_kf, 0.0, 0.0, 0.0, 0.06, 1.0);
    push_kf(&mut base_kf, 0.16, 1.0, 0.0, 0.0, 1.0);
    push_kf(&mut base_kf, 0.88, 1.0, 0.0, 0.0, 1.0);
    push_kf(&mut base_kf, 1.0, 0.0, 0.0, 0.0, 1.0);
    let mut layers = vec![TitleLayer {
        content: LayerContent::Text {
            text: text.to_string(),
            font_family: TPL_FONT.into(),
            font_px,
            color: color.into(),
            align: TextAlign::Center,
            weight,
        },
        x: 0.05,
        y,
        w: 0.90,
        h,
        keyframes: base_kf,
        easing: Easing::EaseOut,
    }];

    // Choose the emphasized word: explicit substring match, else the longest.
    let words: Vec<&str> = text.split_whitespace().collect();
    let target_idx = emphasis
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .and_then(|e| {
            let el = e.to_lowercase();
            words.iter().position(|w| w.to_lowercase().contains(&el))
        })
        .or_else(|| {
            words
                .iter()
                .enumerate()
                .max_by_key(|(_, w)| w.chars().count())
                .map(|(i, _)| i)
        });

    if let Some(ti) = target_idx {
        if !words.is_empty() {
            // Position the accent overlay over the base word using measured widths.
            let full_w = measure_text_width(text, TPL_FONT, font_px, weight);
            let line_left = centered_left_norm(full_w, width) * width as f64;
            let sp = space_width(TPL_FONT, font_px, weight);
            let before: String = words[..ti].join(" ");
            let before_w = measure_text_width(&before, TPL_FONT, font_px, weight);
            let word_left = line_left + before_w + if ti > 0 { sp } else { 0.0 };
            let word_w = measure_text_width(words[ti], TPL_FONT, font_px, weight);

            let mut em_kf = Vec::new();
            push_kf(&mut em_kf, 0.0, 0.0, 0.0, 0.0, 0.7);
            push_kf(&mut em_kf, 0.24, 0.0, 0.0, 0.0, 0.7);
            push_kf(&mut em_kf, 0.36, 1.0, 0.0, 0.0, 1.18);
            push_kf(&mut em_kf, 0.46, 1.0, 0.0, 0.0, 1.0);
            push_kf(&mut em_kf, 0.88, 1.0, 0.0, 0.0, 1.0);
            push_kf(&mut em_kf, 1.0, 0.0, 0.0, 0.0, 1.0);
            layers.push(TitleLayer {
                content: LayerContent::Text {
                    text: words[ti].to_string(),
                    font_family: TPL_FONT.into(),
                    font_px,
                    color: accent.into(),
                    align: TextAlign::Left,
                    weight,
                },
                x: (word_left / width as f64).clamp(0.0, 1.0),
                y,
                w: (word_w / width as f64 + 0.02).min(1.0),
                h,
                keyframes: em_kf,
                easing: Easing::EaseOut,
            });
        }
    }
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    }
}

/// lower_third_reveal — a translucent bar slides in from the left, then a primary
/// line reveals (slide-up + fade), then an optional secondary line (split the
/// text on '|' or newline) reveals smaller beneath it; everything holds, then
/// fades. A staged broadcast lower-third.
fn tpl_lower_third_reveal(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
) -> TitleSpec {
    let mut parts = text.splitn(2, ['|', '\n']);
    let primary = parts.next().unwrap_or(text).trim().to_string();
    let secondary = parts
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let has_sec = secondary.is_some();
    let primary_font_px =
        fit_font_px_to_width(&primary, TPL_FONT, font_px, 700, width as f64 * 0.56);

    let bar_y = 0.78_f64;
    let bar_h = if has_sec { 0.16 } else { 0.11 };

    // Bar: slide in from the left + fade, settle by ~22%.
    let mut bar_kf = Vec::new();
    push_kf(&mut bar_kf, 0.0, 0.0, -0.45, 0.0, 1.0);
    push_kf(&mut bar_kf, 0.22, 1.0, 0.0, 0.0, 1.0);
    push_kf(&mut bar_kf, 0.88, 1.0, 0.0, 0.0, 1.0);
    push_kf(&mut bar_kf, 1.0, 0.0, 0.0, 0.0, 1.0);
    let mut layers = vec![TitleLayer {
        content: LayerContent::Rect {
            color: "#000000".into(),
            opacity: 0.55,
            radius_px: 8.0,
        },
        x: 0.06,
        y: bar_y,
        w: 0.62,
        h: bar_h,
        keyframes: bar_kf,
        easing: Easing::EaseOut,
    }];

    // Primary line: reveal AFTER the bar lands (slide-up + fade).
    let mut p_kf = Vec::new();
    push_kf(&mut p_kf, 0.0, 0.0, 0.0, 0.05, 1.0);
    push_kf(&mut p_kf, 0.30, 0.0, 0.0, 0.05, 1.0);
    push_kf(&mut p_kf, 0.42, 1.0, 0.0, 0.0, 1.0);
    push_kf(&mut p_kf, 0.88, 1.0, 0.0, 0.0, 1.0);
    push_kf(&mut p_kf, 1.0, 0.0, 0.0, 0.0, 1.0);
    layers.push(TitleLayer {
        content: LayerContent::Text {
            text: primary,
            font_family: TPL_FONT.into(),
            font_px: primary_font_px,
            color: color.into(),
            align: TextAlign::Left,
            weight: 700,
        },
        x: 0.09,
        y: if has_sec { bar_y + 0.005 } else { bar_y },
        w: 0.56,
        h: if has_sec { 0.085 } else { bar_h },
        keyframes: p_kf,
        easing: Easing::EaseOut,
    });

    // Secondary line: reveal after the primary, smaller, beneath it.
    if let Some(sec) = secondary {
        let secondary_font_px =
            fit_font_px_to_width(&sec, TPL_FONT, font_px * 0.62, 500, width as f64 * 0.56);
        let mut s_kf = Vec::new();
        push_kf(&mut s_kf, 0.0, 0.0, 0.0, 0.05, 1.0);
        push_kf(&mut s_kf, 0.46, 0.0, 0.0, 0.05, 1.0);
        push_kf(&mut s_kf, 0.58, 1.0, 0.0, 0.0, 1.0);
        push_kf(&mut s_kf, 0.88, 1.0, 0.0, 0.0, 1.0);
        push_kf(&mut s_kf, 1.0, 0.0, 0.0, 0.0, 1.0);
        layers.push(TitleLayer {
            content: LayerContent::Text {
                text: sec,
                font_family: TPL_FONT.into(),
                font_px: secondary_font_px,
                color: color.into(),
                align: TextAlign::Left,
                weight: 500,
            },
            x: 0.09,
            y: bar_y + 0.09,
            w: 0.56,
            h: 0.06,
            keyframes: s_kf,
            easing: Easing::EaseOut,
        });
    }
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    }
}

/// caption_karaoke — a dim full line is held under an `accent` highlight that
/// fills word-by-word (the prefix of highlighted words grows left-to-right in
/// hard cuts, exactly like a karaoke bouncing fill). The base line and the
/// accent prefixes share a left edge (the centered line's left), measured once.
fn tpl_caption_karaoke(
    text: &str,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
    color: &str,
    font_px: f64,
    accent: &str,
) -> TitleSpec {
    let words: Vec<&str> = text.split_whitespace().collect();
    let words: Vec<&str> = if words.is_empty() { vec![text] } else { words };
    let k = words.len().max(1);
    let weight = 700;
    let font_px = fit_font_px_to_width(text, TPL_FONT, font_px, weight, width as f64 * 0.92);
    let full_w = measure_text_width(text, TPL_FONT, font_px, weight);
    let x = centered_left_norm(full_w, width);
    let w = ((full_w / width as f64) + 0.02).min(1.0 - x);
    let y = 0.6_f64;
    let h = (font_px * 1.6 / height as f64).clamp(0.06, 0.4);

    // Base dim line: fade in, hold at 50% opacity, fade out.
    let mut base_kf = Vec::new();
    push_kf(&mut base_kf, 0.0, 0.0, 0.0, 0.0, 1.0);
    push_kf(&mut base_kf, 0.08, 0.5, 0.0, 0.0, 1.0);
    push_kf(&mut base_kf, 0.92, 0.5, 0.0, 0.0, 1.0);
    push_kf(&mut base_kf, 1.0, 0.0, 0.0, 0.0, 1.0);
    let mut layers = vec![TitleLayer {
        content: LayerContent::Text {
            text: text.to_string(),
            font_family: TPL_FONT.into(),
            font_px,
            color: color.into(),
            align: TextAlign::Left,
            weight,
        },
        x,
        y,
        w,
        h,
        keyframes: base_kf,
        easing: Easing::EaseOut,
    }];

    // Accent prefixes: highlighted first-(i+1) words, hard cut at each word time.
    let fill = 0.84_f64; // the highlight finishes filling by 84% of the duration
    for i in 0..k {
        let prefix: String = words[..=i].join(" ");
        let on = (i as f64) / k as f64 * fill;
        let off = (i as f64 + 1.0) / k as f64 * fill;
        let last = i == k - 1;
        layers.push(TitleLayer {
            content: LayerContent::Text {
                text: prefix,
                font_family: TPL_FONT.into(),
                font_px,
                color: accent.into(),
                align: TextAlign::Left,
                weight,
            },
            x,
            y,
            w,
            h,
            keyframes: cut_keyframes(on, off, last),
            easing: Easing::Linear,
        });
    }
    TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    }
}

// ---------------------------------------------------------------------------
// Vector SHAPES — rect / ellipse / line / arrow + styled text boxes, on
// the same resvg keyframe engine (so a shape gets overlay placement, keyframe
// animation, and content-addressing for free). `build_shape` returns a
// [`TitleSpec`] the `edit.add_shape` verb places like a title; the default
// motion is a tasteful fade (the verb's `animation` override can restyle it).
// ---------------------------------------------------------------------------

/// The shape kinds `edit.add_shape` accepts.
pub const SHAPE_KINDS: &[&str] = &["rect", "ellipse", "line", "arrow"];

/// Paint + geometry knobs shared by the shape builders. The verb fills these
/// from its args (with sensible defaults).
#[derive(Debug, Clone)]
pub struct ShapeParams {
    /// Fill color `#RRGGBB`, or None for outline-only (rect/ellipse).
    pub fill: Option<String>,
    /// Fill opacity `0..=1`.
    pub opacity: f64,
    /// Stroke/border color `#RRGGBB`, or None for no border. For line/arrow this
    /// is the line color (falls back to `fill`, then white).
    pub stroke: Option<String>,
    /// Stroke width / line width in canvas px.
    pub stroke_px: f64,
    /// Corner radius in canvas px (rect).
    pub radius_px: f64,
    /// Arrowhead length in canvas px (arrow).
    pub head_px: f64,
    /// Optional label drawn centered in the box (rect/ellipse → a styled box).
    pub text: Option<String>,
    /// Label color `#RRGGBB`.
    pub text_color: String,
    /// Label font size in canvas px.
    pub font_px: f64,
}

/// Resolve the line/arrow color: stroke, else fill, else white.
fn line_color(p: &ShapeParams) -> String {
    p.stroke
        .clone()
        .or_else(|| p.fill.clone())
        .unwrap_or_else(|| "#FFFFFF".to_string())
}

/// Build a shape spec. `(x,y,w,h)` is the box for rect/ellipse; for line/arrow
/// `(x,y)` is the start and `(x2,y2)` the end (normalized canvas coords). Returns
/// None for an unknown kind. Layers carry a default fade envelope so the shape
/// animates in/out unless the verb overrides the motion.
#[allow(clippy::too_many_arguments)]
pub fn build_shape(
    kind: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    x2: f64,
    y2: f64,
    p: &ShapeParams,
    width: u32,
    height: u32,
    fps: u32,
    duration_ms: u64,
) -> Option<TitleSpec> {
    let env = || envelope_full(0.0, 0.0, 1.0); // pure fade in/out
    let mut layers: Vec<TitleLayer> = Vec::new();
    match kind {
        "rect" => {
            layers.push(TitleLayer {
                content: LayerContent::StrokeBox {
                    fill: p.fill.clone(),
                    opacity: p.opacity,
                    stroke: p.stroke.clone(),
                    stroke_px: p.stroke_px,
                    radius_px: p.radius_px,
                },
                x,
                y,
                w,
                h,
                keyframes: env(),
                easing: Easing::EaseOut,
            });
        }
        "ellipse" => {
            layers.push(TitleLayer {
                content: LayerContent::Ellipse {
                    fill: p.fill.clone(),
                    opacity: p.opacity,
                    stroke: p.stroke.clone(),
                    stroke_px: p.stroke_px,
                },
                x,
                y,
                w,
                h,
                keyframes: env(),
                easing: Easing::EaseOut,
            });
        }
        "line" | "arrow" => {
            // Box = endpoints' bounding box, so translate/scale animate sanely.
            let (bx, by) = (x.min(x2), y.min(y2));
            let (bw, bh) = ((x - x2).abs().max(0.02), (y - y2).abs().max(0.02));
            let content = if kind == "arrow" {
                LayerContent::Arrow {
                    x1: x,
                    y1: y,
                    x2,
                    y2,
                    color: line_color(p),
                    width_px: p.stroke_px.max(1.0),
                    head_px: if p.head_px > 0.0 {
                        p.head_px
                    } else {
                        p.stroke_px.max(1.0) * 4.0
                    },
                }
            } else {
                LayerContent::Line {
                    x1: x,
                    y1: y,
                    x2,
                    y2,
                    color: line_color(p),
                    width_px: p.stroke_px.max(1.0),
                }
            };
            layers.push(TitleLayer {
                content,
                x: bx,
                y: by,
                w: bw,
                h: bh,
                keyframes: env(),
                easing: Easing::EaseOut,
            });
        }
        _ => return None,
    }
    // Optional centered label for box shapes (the styled text-box use case).
    if matches!(kind, "rect" | "ellipse") {
        if let Some(t) = &p.text {
            if !t.trim().is_empty() {
                layers.push(TitleLayer {
                    content: LayerContent::Text {
                        text: t.clone(),
                        font_family: TPL_FONT.into(),
                        font_px: p.font_px,
                        color: p.text_color.clone(),
                        align: TextAlign::Center,
                        weight: 700,
                    },
                    x,
                    y,
                    w,
                    h,
                    keyframes: env(),
                    easing: Easing::EaseOut,
                });
            }
        }
    }
    Some(TitleSpec {
        width,
        height,
        fps,
        duration_ms,
        layers,
    })
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate structural invariants once, up front, so rasterization never has to
/// handle a malformed spec. Color validity is checked here too (every color in
/// every layer) so a bad hex never reaches the SVG string.
fn validate(spec: &TitleSpec) -> Result<(), TitleError> {
    if spec.width == 0 || spec.height == 0 {
        return Err(TitleError::InvalidSpec(
            "width and height must be greater than zero".into(),
        ));
    }
    if spec.layers.is_empty() {
        return Err(TitleError::NoLayers);
    }
    if spec.fps == 0 {
        return Err(TitleError::ZeroFps);
    }
    if spec.duration_ms == 0 {
        return Err(TitleError::ZeroDuration);
    }
    for (i, layer) in spec.layers.iter().enumerate() {
        for (name, value) in [
            ("x", layer.x),
            ("y", layer.y),
            ("w", layer.w),
            ("h", layer.h),
        ] {
            if !value.is_finite() {
                return Err(TitleError::InvalidSpec(format!(
                    "layer {i} {name} must be finite"
                )));
            }
        }
        if layer.w <= 0.0 || layer.h <= 0.0 {
            return Err(TitleError::InvalidSpec(format!(
                "layer {i} width/height must be greater than zero"
            )));
        }
        if layer.keyframes.is_empty() {
            return Err(TitleError::NoKeyframes(i));
        }
        // Keyframes must be ascending by t (windows() compares adjacent pairs).
        if layer.keyframes.windows(2).any(|w| w[1].t < w[0].t) {
            return Err(TitleError::UnsortedKeyframes(i));
        }
        for (ki, kf) in layer.keyframes.iter().enumerate() {
            for (name, value) in [
                ("t", kf.t),
                ("opacity", kf.opacity),
                ("tx", kf.tx),
                ("ty", kf.ty),
                ("scale", kf.scale),
            ] {
                if !value.is_finite() {
                    return Err(TitleError::InvalidSpec(format!(
                        "layer {i} keyframe {ki} {name} must be finite"
                    )));
                }
            }
            if kf.scale <= 0.0 {
                return Err(TitleError::InvalidSpec(format!(
                    "layer {i} keyframe {ki} scale must be greater than zero"
                )));
            }
            if !(0.0..=1.0).contains(&kf.t) {
                return Err(TitleError::InvalidSpec(format!(
                    "layer {i} keyframe {ki} t must be in [0,1]"
                )));
            }
            if !(0.0..=1.0).contains(&kf.opacity) {
                return Err(TitleError::InvalidSpec(format!(
                    "layer {i} keyframe {ki} opacity must be in [0,1]"
                )));
            }
            if kf.scale > 10.0 {
                return Err(TitleError::InvalidSpec(format!(
                    "layer {i} keyframe {ki} scale must be <= 10"
                )));
            }
        }
        match &layer.content {
            LayerContent::Text { color, font_px, .. } => {
                if !font_px.is_finite() || *font_px <= 0.0 {
                    return Err(TitleError::InvalidSpec(format!(
                        "layer {i} font_px must be greater than zero"
                    )));
                }
                parse_hex_color(color)?;
            }
            LayerContent::Rect { color, .. } => {
                parse_hex_color(color)?;
            }
            LayerContent::StrokeBox { fill, stroke, .. }
            | LayerContent::Ellipse { fill, stroke, .. } => {
                if let Some(c) = fill {
                    parse_hex_color(c)?;
                }
                if let Some(c) = stroke {
                    parse_hex_color(c)?;
                }
            }
            LayerContent::Line { color, .. } | LayerContent::Arrow { color, .. } => {
                parse_hex_color(color)?;
            }
        }
    }
    Ok(())
}

/// Parse a strict `#RRGGBB` color into (r,g,b). Anything else → [`TitleError::BadColor`].
/// Returned for both validation and (defensively) at emit time.
fn parse_hex_color(s: &str) -> Result<(u8, u8, u8), TitleError> {
    let bytes = s.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' {
        return Err(TitleError::BadColor(s.to_string()));
    }
    let hex = |a: u8, b: u8| -> Option<u8> {
        let h = |c: u8| -> Option<u8> {
            match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                b'A'..=b'F' => Some(c - b'A' + 10),
                _ => None,
            }
        };
        Some(h(a)? * 16 + h(b)?)
    };
    let r = hex(bytes[1], bytes[2]);
    let g = hex(bytes[3], bytes[4]);
    let b = hex(bytes[5], bytes[6]);
    match (r, g, b) {
        (Some(r), Some(g), Some(b)) => Ok((r, g, b)),
        _ => Err(TitleError::BadColor(s.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Keyframe interpolation
// ---------------------------------------------------------------------------

/// The interpolated animatable state of a layer at one instant.
#[derive(Debug, Clone, Copy)]
struct LayerState {
    opacity: f64,
    /// Translate in CANVAS PIXELS (already resolved from fractions).
    tx_px: f64,
    ty_px: f64,
    scale: f64,
}

/// Apply an easing curve to a linear fraction `f` in `[0,1]`.
fn ease(easing: Easing, f: f64) -> f64 {
    let f = f.clamp(0.0, 1.0);
    match easing {
        Easing::Linear => f,
        Easing::EaseIn => f * f,
        Easing::EaseOut => 1.0 - (1.0 - f) * (1.0 - f),
        // Smoothstep: 3f² − 2f³ — symmetric ease-in-out.
        Easing::EaseInOut => f * f * (3.0 - 2.0 * f),
    }
}

/// Interpolate a layer's animatable fields at normalized time `tnorm`.
///
/// Finds the surrounding keyframe pair (clamping at the ends), computes the
/// linear fraction between them, reshapes it with the layer's easing, and lerps
/// each field. Translate fractions are resolved to pixels here against the
/// canvas geometry so SVG emission can use raw pixel transforms.
fn interpolate(layer: &TitleLayer, spec: &TitleSpec, tnorm: f64) -> LayerState {
    let kfs = &layer.keyframes; // non-empty (validated)
    let to_px = |kf: &Keyframe| (kf.tx * spec.width as f64, kf.ty * spec.height as f64);

    // Before the first / after the last keyframe → clamp to the endpoint.
    let first = &kfs[0];
    if tnorm <= first.t || kfs.len() == 1 {
        let (tx_px, ty_px) = to_px(first);
        return LayerState {
            opacity: first.opacity,
            tx_px,
            ty_px,
            scale: first.scale,
        };
    }
    let last = &kfs[kfs.len() - 1];
    if tnorm >= last.t {
        let (tx_px, ty_px) = to_px(last);
        return LayerState {
            opacity: last.opacity,
            tx_px,
            ty_px,
            scale: last.scale,
        };
    }

    // Find the pair [a,b] with a.t <= tnorm < b.t.
    let mut a = first;
    let mut b = last;
    for w in kfs.windows(2) {
        if tnorm >= w[0].t && tnorm < w[1].t {
            a = &w[0];
            b = &w[1];
            break;
        }
    }

    // Linear fraction within the pair, guarded against a zero-width span.
    let span = b.t - a.t;
    let lin = if span > f64::EPSILON {
        (tnorm - a.t) / span
    } else {
        0.0
    };
    let f = ease(layer.easing, lin);

    let lerp = |x: f64, y: f64| x + (y - x) * f;
    let (ax, ay) = to_px(a);
    let (bx, by) = to_px(b);
    LayerState {
        opacity: lerp(a.opacity, b.opacity),
        tx_px: lerp(ax, bx),
        ty_px: lerp(ay, by),
        scale: lerp(a.scale, b.scale),
    }
}

// ---------------------------------------------------------------------------
// SVG emission
// ---------------------------------------------------------------------------

/// Escape the five XML special chars so user text/colors can't break the SVG.
fn xml_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\u{9}' | '\u{a}' | '\u{d}' => out.push(c),
            c if c.is_control() => {}
            _ => out.push(c),
        }
    }
    out
}

/// Build the `fill`/`fill-opacity`/`stroke`/`stroke-width` SVG attributes for a
/// shape. `fill: None` → `fill="none"` (outline only); a `stroke` with
/// `stroke_px <= 0` is omitted. Colors are re-parsed defensively (validate()
/// already checked them) so a bad hex errors instead of emitting broken SVG.
fn paint_attrs(
    fill: &Option<String>,
    fill_op: f64,
    stroke: &Option<String>,
    stroke_px: f64,
) -> Result<String, TitleError> {
    let mut out = String::new();
    match fill {
        Some(c) => {
            let (r, g, b) = parse_hex_color(c)?;
            out.push_str(&format!(
                "fill=\"#{r:02x}{g:02x}{b:02x}\" fill-opacity=\"{:.4}\"",
                fill_op.clamp(0.0, 1.0)
            ));
        }
        None => out.push_str("fill=\"none\""),
    }
    if let Some(c) = stroke {
        if stroke_px > 0.0 {
            let (r, g, b) = parse_hex_color(c)?;
            out.push_str(&format!(
                " stroke=\"#{r:02x}{g:02x}{b:02x}\" stroke-width=\"{stroke_px:.4}\""
            ));
        }
    }
    Ok(out)
}

/// Build the static SVG document for the title at normalized time `tnorm`.
///
/// Each layer becomes a `<g transform="translate(tx,ty) scale(s about centre)"
/// opacity="o">` wrapping either a `<rect>` (with `rx` for rounded corners) or
/// a `<text>`. The scale is applied about the layer-box centre by translating
/// to the centre, scaling, then translating back — so scaling grows/shrinks in
/// place rather than from the origin.
fn build_svg(spec: &TitleSpec, tnorm: f64) -> Result<String, TitleError> {
    let (w, h) = (spec.width as f64, spec.height as f64);
    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        spec.width, spec.height, spec.width, spec.height
    ));

    for layer in &spec.layers {
        let st = interpolate(layer, spec, tnorm);
        // Layer box in canvas pixels.
        let bx = layer.x * w;
        let by = layer.y * h;
        let bw = layer.w * w;
        let bh = layer.h * h;
        // Scale about the box centre, then the keyframe translate, composed
        // outer→inner: translate(tx,ty) then scale-about-centre.
        let cx = bx + bw / 2.0;
        let cy = by + bh / 2.0;
        let transform = format!(
            "translate({tx:.4},{ty:.4}) translate({cx:.4},{cy:.4}) scale({s:.6}) translate({ncx:.4},{ncy:.4})",
            tx = st.tx_px,
            ty = st.ty_px,
            cx = cx,
            cy = cy,
            s = st.scale,
            ncx = -cx,
            ncy = -cy,
        );
        let opacity = st.opacity.clamp(0.0, 1.0);
        // Cull fully-transparent layers. An opacity≈0 layer draws nothing, but
        // resvg still parses + lays out its text/rect every frame. A per-word
        // kinetic title has ONE layer per spoken word (e.g. 115) yet only ~1-2
        // are visible at any instant — culling the rest turns a ~115-element SVG
        // per frame into a ~2-element one (the per-word render was minutes →
        // seconds; caught. Output is byte-identical: the skipped
        // layer is invisible. Threshold well below 8-bit quantization (1/255).
        if opacity < 1e-3 {
            continue;
        }
        svg.push_str(&format!(
            "<g transform=\"{transform}\" opacity=\"{opacity:.4}\">"
        ));

        match &layer.content {
            LayerContent::Rect {
                color,
                opacity: fill_op,
                radius_px,
            } => {
                // color validated in validate(); re-parse defensively.
                let (r, g, b) = parse_hex_color(color)?;
                let fill = format!("#{r:02x}{g:02x}{b:02x}");
                let rx = radius_px.max(0.0);
                svg.push_str(&format!(
                    "<rect x=\"{bx:.4}\" y=\"{by:.4}\" width=\"{bw:.4}\" height=\"{bh:.4}\" \
                     rx=\"{rx:.4}\" fill=\"{fill}\" fill-opacity=\"{fo:.4}\"/>",
                    bx = bx,
                    by = by,
                    bw = bw.max(0.0),
                    bh = bh.max(0.0),
                    rx = rx,
                    fill = fill,
                    fo = fill_op.clamp(0.0, 1.0),
                ));
            }
            LayerContent::Text {
                text,
                font_family,
                font_px,
                color,
                align,
                weight,
            } => {
                let (r, g, b) = parse_hex_color(color)?;
                let fill = format!("#{r:02x}{g:02x}{b:02x}");
                // Anchor + x within the box per alignment. usvg 0.47 supports
                // dominant-baseline=central, so y can be the true box center
                // instead of a fixed font-size approximation.
                let (anchor, tx) = match align {
                    TextAlign::Left => ("start", bx),
                    TextAlign::Center => ("middle", bx + bw / 2.0),
                    TextAlign::Right => ("end", bx + bw),
                };
                let ty = by + bh / 2.0;
                svg.push_str(&format!(
                    "<text x=\"{tx:.4}\" y=\"{ty:.4}\" font-family=\"{fam}\" \
                     font-size=\"{fs:.4}\" font-weight=\"{wt}\" text-anchor=\"{anchor}\" \
                     dominant-baseline=\"central\" fill=\"{fill}\">{text}</text>",
                    tx = tx,
                    ty = ty,
                    fam = xml_escape(&present_or_default(font_family, *weight)),
                    fs = font_px,
                    wt = weight,
                    anchor = anchor,
                    fill = fill,
                    text = xml_escape(text),
                ));
            }
            LayerContent::StrokeBox {
                fill,
                opacity: fill_op,
                stroke,
                stroke_px,
                radius_px,
            } => {
                let attrs = paint_attrs(fill, *fill_op, stroke, *stroke_px)?;
                svg.push_str(&format!(
                    "<rect x=\"{bx:.4}\" y=\"{by:.4}\" width=\"{bw:.4}\" height=\"{bh:.4}\" \
                     rx=\"{rx:.4}\" {attrs}/>",
                    bw = bw.max(0.0),
                    bh = bh.max(0.0),
                    rx = radius_px.max(0.0),
                ));
            }
            LayerContent::Ellipse {
                fill,
                opacity: fill_op,
                stroke,
                stroke_px,
            } => {
                let attrs = paint_attrs(fill, *fill_op, stroke, *stroke_px)?;
                svg.push_str(&format!(
                    "<ellipse cx=\"{cx:.4}\" cy=\"{cy:.4}\" rx=\"{rx:.4}\" ry=\"{ry:.4}\" {attrs}/>",
                    cx = bx + bw / 2.0,
                    cy = by + bh / 2.0,
                    rx = (bw / 2.0).max(0.0),
                    ry = (bh / 2.0).max(0.0),
                ));
            }
            LayerContent::Line {
                x1,
                y1,
                x2,
                y2,
                color,
                width_px,
            } => {
                let (r, g, b) = parse_hex_color(color)?;
                svg.push_str(&format!(
                    "<line x1=\"{:.4}\" y1=\"{:.4}\" x2=\"{:.4}\" y2=\"{:.4}\" \
                     stroke=\"#{r:02x}{g:02x}{b:02x}\" stroke-width=\"{width_px:.4}\" \
                     stroke-linecap=\"round\"/>",
                    x1 * w,
                    y1 * h,
                    x2 * w,
                    y2 * h,
                ));
            }
            LayerContent::Arrow {
                x1,
                y1,
                x2,
                y2,
                color,
                width_px,
                head_px,
            } => {
                let (r, g, b) = parse_hex_color(color)?;
                let col = format!("#{r:02x}{g:02x}{b:02x}");
                let (sx, sy, ex, ey) = (x1 * w, y1 * h, x2 * w, y2 * h);
                let (dx, dy) = (ex - sx, ey - sy);
                let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                let (ux, uy) = (dx / len, dy / len); // unit direction
                let head = head_px.max(1.0);
                let (basex, basey) = (ex - ux * head, ey - uy * head); // head base
                let (perpx, perpy) = (-uy, ux); // perpendicular
                let hw = head * 0.55; // half base width
                                      // Shaft stops at the head base so it never pokes through the tip.
                svg.push_str(&format!(
                    "<line x1=\"{sx:.4}\" y1=\"{sy:.4}\" x2=\"{basex:.4}\" y2=\"{basey:.4}\" \
                     stroke=\"{col}\" stroke-width=\"{width_px:.4}\" stroke-linecap=\"round\"/>",
                ));
                svg.push_str(&format!(
                    "<polygon points=\"{ex:.4},{ey:.4} {:.4},{:.4} {:.4},{:.4}\" fill=\"{col}\"/>",
                    basex + perpx * hw,
                    basey + perpy * hw,
                    basex - perpx * hw,
                    basey - perpy * hw,
                ));
            }
        }
        svg.push_str("</g>");
    }

    svg.push_str("</svg>");
    Ok(svg)
}

// ---------------------------------------------------------------------------
// Rasterization (resvg + tiny-skia)
// ---------------------------------------------------------------------------

/// Preferred default font family. Listed FIRST in every per-OS candidate set so
/// a Linux host (where DejaVu Sans ships) keeps byte-identical title rendering;
/// on macOS/Windows it is ABSENT and `resolved_default_family` falls through to
/// a face that actually exists there. Defaulting to a missing family rendered
/// titles BLANK on macOS (no glyphs) — caught on the macOS QA host (arm64).
const PREFERRED_FONT_FAMILY: &str = "DejaVu Sans";

fn font_weight_class(weight: u32) -> u16 {
    if weight >= 600 {
        700
    } else {
        400
    }
}

fn face_matches_requested_weight(face_weight: fontdb::Weight, requested: u32) -> bool {
    match font_weight_class(requested) {
        700 => face_weight.0 >= 600,
        _ => face_weight.0 < 600,
    }
}

/// True when the system fontdb contains a normal-stretch face for `name` in the
/// requested normal/bold class. This avoids reporting a family as usable for
/// bold text when the host only has a regular face.
fn family_present(db: &fontdb::Database, name: &str, weight: u32) -> bool {
    db.faces().any(|face| {
        face.style == fontdb::Style::Normal
            && face.stretch == fontdb::Stretch::Normal
            && face_matches_requested_weight(face.weight, weight)
            && face
                .families
                .iter()
                .any(|(family, _)| family.eq_ignore_ascii_case(name))
    })
}

/// Resolve a default family GUARANTEED present in `db`. Probes a per-OS
/// candidate list (preferred family first) and returns the first face the host
/// actually has, falling back to the first family the DB knows about as a last
/// resort — so title text is never invisible on any platform.
fn resolved_default_family(db: &fontdb::Database) -> String {
    let candidates: &[&str] = if cfg!(target_os = "macos") {
        &[
            PREFERRED_FONT_FAMILY,
            "Helvetica Neue",
            "Helvetica",
            "Arial",
            "Geneva",
        ]
    } else if cfg!(windows) {
        &[
            PREFERRED_FONT_FAMILY,
            "Segoe UI",
            "Arial",
            "Tahoma",
            "Verdana",
        ]
    } else {
        &[
            PREFERRED_FONT_FAMILY,
            "Liberation Sans",
            "Noto Sans",
            "FreeSans",
            "Arial",
        ]
    };
    for fam in candidates {
        if family_present(db, fam, 400) {
            return (*fam).to_string();
        }
    }
    db.faces()
        .next()
        .and_then(|f| f.families.first().map(|(n, _)| n.clone()))
        .unwrap_or_else(|| "sans-serif".to_string())
}

/// Shared, lazily-initialized font state: the system font database (loaded once
/// — the disk scan is the expensive part) plus the resolved default family that
/// is GUARANTEED present on this host. fontdb is `Arc`-backed so cloning the db
/// into each per-frame `usvg::Options` is cheap.
struct Fonts {
    db: fontdb::Database,
    family: String,
    presence_cache: Mutex<HashMap<(String, u16), String>>,
}

fn fonts() -> &'static Fonts {
    static F: OnceLock<Fonts> = OnceLock::new();
    F.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let family = resolved_default_family(&db);
        // Point the generic families at a REAL present face so a missing custom
        // family resolves to something visible on every OS.
        db.set_serif_family(&family);
        db.set_sans_serif_family(&family);
        db.set_monospace_family(&family);
        Fonts {
            db,
            family,
            presence_cache: Mutex::new(HashMap::new()),
        }
    })
}

/// The requested family if the system has it, else the resolved present
/// default. Guarantees the emitted SVG names a face that exists on THIS host,
/// so an uninstalled custom family (or "DejaVu Sans" on macOS/Windows) still
/// renders real glyphs instead of blank.
fn present_or_default(requested: &str, weight: u32) -> String {
    let f = fonts();
    let key = (requested.to_string(), font_weight_class(weight));
    if let Some(cached) = f
        .presence_cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(&key)
        .cloned()
    {
        return cached;
    }
    let resolved = if family_present(&f.db, requested, weight) {
        requested.to_string()
    } else {
        f.family.clone()
    };
    f.presence_cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key, resolved.clone());
    resolved
}

/// Parse `svg` and rasterize it into a `width × height` pixmap, returning plain
/// unpremultiplied RGBA8 bytes (`width*height*4`).
///
/// tiny-skia stores pixels PREMULTIPLIED internally; we convert each pixel back
/// to unpremultiplied straight-alpha RGBA via `tiny_skia::Color`/`PremultipliedColorU8`
/// (`.demultiply()`), which the renderer's overlay/encode stages expect.
fn rasterize(width: u32, height: u32, svg: &str) -> Result<Vec<u8>, TitleError> {
    // Per-frame Options. Set the default family to the host-resolved present
    // face and inject the shared fontdb (cloned — Arc-backed, cheap) so a
    // missing custom family still falls back to a visible glyph on every OS.
    let f = fonts();
    let mut opt = usvg::Options {
        font_family: f.family.clone(),
        ..usvg::Options::default()
    };
    *opt.fontdb_mut() = f.db.clone();

    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| TitleError::SvgParse(e.to_string()))?;

    let mut pixmap =
        tiny_skia::Pixmap::new(width, height).ok_or(TitleError::PixmapAlloc(width, height))?;

    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );

    // Convert premultiplied → unpremultiplied (straight-alpha) RGBA8.
    // pixmap.pixels() yields PremultipliedColorU8; demultiply() gives ColorU8
    // with straight-alpha components in R,G,B,A order.
    let mut out = Vec::new();
    for px in pixmap.pixels() {
        let c = px.demultiply();
        out.push(c.red());
        out.push(c.green());
        out.push(c.blue());
        out.push(c.alpha());
    }
    Ok(out)
}

fn rasterize_png(width: u32, height: u32, svg: &str, out: &Path) -> Result<(), TitleError> {
    let f = fonts();
    let mut opt = usvg::Options {
        font_family: f.family.clone(),
        ..usvg::Options::default()
    };
    *opt.fontdb_mut() = f.db.clone();

    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| TitleError::SvgParse(e.to_string()))?;
    let mut pixmap =
        tiny_skia::Pixmap::new(width, height).ok_or(TitleError::PixmapAlloc(width, height))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap
        .save_png(out)
        .map_err(|e| TitleError::PngWrite(e.to_string()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal one-Text-layer spec ramping opacity 0→1 over the title.
    fn text_fade_spec() -> TitleSpec {
        TitleSpec {
            width: 320,
            height: 180,
            fps: 30,
            duration_ms: 1000,
            layers: vec![TitleLayer {
                content: LayerContent::Text {
                    text: "Hello".to_string(),
                    font_family: "DejaVu Sans".to_string(),
                    font_px: 64.0,
                    color: "#FFFFFF".to_string(),
                    align: TextAlign::Center,
                    weight: 700,
                },
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
                keyframes: vec![
                    Keyframe {
                        t: 0.0,
                        opacity: 0.0,
                        tx: 0.0,
                        ty: 0.0,
                        scale: 1.0,
                    },
                    Keyframe {
                        t: 1.0,
                        opacity: 1.0,
                        tx: 0.0,
                        ty: 0.0,
                        scale: 1.0,
                    },
                ],
                easing: Easing::Linear,
            }],
        }
    }

    /// Max alpha byte across an RGBA8 buffer (every 4th byte is alpha).
    fn max_alpha(buf: &[u8]) -> u8 {
        buf.iter().skip(3).step_by(4).copied().max().unwrap_or(0)
    }

    #[test]
    fn presets_build_renderable_specs() {
        // lower_third with a bar = 2 layers (rect + text); renders a frame.
        let lt = lower_third("Hello", 1920, 1080, 30, 2000, "#FFFFFF", 64.0, true);
        assert_eq!(lt.layers.len(), 2);
        assert!(matches!(lt.layers[0].content, LayerContent::Rect { .. }));
        assert!(matches!(lt.layers[1].content, LayerContent::Text { .. }));
        assert_eq!(frame_count(&lt), 60);
        let mid = render_frame(&lt, 30).expect("lower_third mid-frame renders");
        assert_eq!(mid.len() as u32, lt.width * lt.height * 4);
        // Some pixel is opaque at the hold (the bar/text are visible).
        assert!(
            mid.chunks_exact(4).any(|p| p[3] > 0),
            "lower_third has visible pixels mid-hold"
        );

        // title_card = one centred text layer; renders.
        let tc = title_card("Intro", 1280, 720, 30, 1500, "#FFCC00", 120.0);
        assert_eq!(tc.layers.len(), 1);
        assert!(render_frame(&tc, 22).is_ok());
        // bg:false lower third = single text layer.
        let nobar = lower_third("X", 1280, 720, 30, 1000, "#FFFFFF", 48.0, false);
        assert_eq!(nobar.layers.len(), 1);
    }

    #[test]
    fn render_frame_png_writes_real_png() {
        let spec = lower_third("Preview", 640, 360, 30, 2000, "#FFFFFF", 36.0, true);
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("preview.png");
        render_frame_png(&spec, frame_count(&spec) / 2, &out).expect("png frame renders");
        let bytes = std::fs::read(&out).expect("png written");
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(bytes.len() > 1024, "real rendered PNG, not a stub");
    }

    /// The NEW title presets (top_bar / subtitle / headline) build renderable
    /// specs with visible pixels, and the animation override resolves + restyles.
    #[test]
    fn new_presets_and_animation_override() {
        // Each new preset renders a visible mid-frame.
        for spec in [
            top_bar("Source: LIVE", 1280, 720, 30, 1500, "#FFFFFF", 44.0, true),
            subtitle("a quiet credit", 1280, 720, 30, 1500, "#FFFFFF", 40.0),
            headline("BREAKING", 1280, 720, 30, 1500, "#FFFFFF", 110.0),
        ] {
            let mid = render_frame(&spec, frame_count(&spec) / 2).expect("preset renders");
            assert_eq!(mid.len() as u32, spec.width * spec.height * 4);
            assert!(
                mid.chunks_exact(4).any(|p| p[3] > 0),
                "preset has visible pixels"
            );
        }
        // Every exposed animation resolves to ascending keyframes (validate-able)
        // when applied to a preset; an unknown one is None.
        let mut lt = lower_third("X", 640, 360, 30, 1000, "#FFFFFF", 40.0, false);
        for anim in TITLE_ANIMATIONS {
            let kf = animation_keyframes(anim).expect("known animation resolves");
            lt.layers[0].keyframes = kf;
            validate(&lt).unwrap_or_else(|e| unreachable!("animation {anim} validates: {e}"));
            render_frame(&lt, 5).unwrap_or_else(|_| unreachable!("animation {anim} renders"));
        }
        assert!(animation_keyframes("bogus").is_none());
        // "none" = a single static, fully-opaque keyframe (visible the whole range).
        let none = animation_keyframes("none").unwrap();
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].opacity, 1.0);
    }

    #[test]
    fn kinetic_cues_render_and_keyframes_ascend() {
        // Three cues over a 3s title: each occupies a third.
        let cues = vec![
            (0.0, 0.33, "one".to_string()),
            (0.33, 0.66, "two".to_string()),
            (0.66, 1.0, "three".to_string()),
        ];
        let spec = kinetic(&cues, 640, 360, 30, 3000, "#FFFFFF", 40.0, 0.78);
        assert_eq!(spec.layers.len(), 3);
        // Validate() (run by render) requires strictly-ascending keyframes — a
        // frame render proves the whole spec is well-formed.
        validate(&spec).expect("kinetic spec validates (keyframes ascend)");
        // First cue visible early (frame 5 ≈ 0.16), third cue NOT yet.
        let early = render_frame(&spec, 5).expect("early frame");
        assert!(
            early.chunks_exact(4).any(|p| p[3] > 0),
            "cue 'one' visible early"
        );
        // Late in the title (frame 85 ≈ 0.94) something is visible (cue 'three').
        let late = render_frame(&spec, 85).expect("late frame");
        assert!(late.chunks_exact(4).any(|p| p[3] > 0), "a cue visible late");
        // A very short cue must not produce a degenerate (equal-t) keyframe pair.
        let short = kinetic(
            &[(0.5, 0.51, "x".into())],
            320,
            180,
            30,
            1000,
            "#FFF000",
            32.0,
            0.5,
        );
        validate(&short).expect("short cue still validates");
    }

    /// Invisible-layer culling (per-word kinetic perf fix is
    /// BYTE-IDENTICAL: a frame where extra layers are fully transparent renders
    /// exactly the same RGBA as the frame without those layers. (The cull only
    /// skips opacity≈0 groups, which draw nothing — so output cannot change; the
    /// win is resvg not parsing them. This pins that invariant.)
    #[test]
    fn culling_invisible_layers_is_byte_identical() {
        // Two cues: at the very start only cue 'one' is visible; 'two' (window
        // 0.66–1.0) is fully transparent at frame 0. The whole-spec frame 0 must
        // equal a spec that contains ONLY cue 'one'.
        let both = kinetic(
            &[(0.0, 0.4, "one".into()), (0.66, 1.0, "two".into())],
            480,
            270,
            30,
            2000,
            "#FFFFFF",
            36.0,
            0.78,
        );
        let only_one = kinetic(
            &[(0.0, 0.4, "one".into())],
            480,
            270,
            30,
            2000,
            "#FFFFFF",
            36.0,
            0.78,
        );
        let f_both = render_frame(&both, 0).expect("both frame 0");
        let f_one = render_frame(&only_one, 0).expect("one frame 0");
        assert_eq!(
            f_both, f_one,
            "culling a fully-transparent layer must not change the rasterized frame"
        );
        // Sanity: a per-word-scale spec (many cues) still renders every frame.
        let many: Vec<(f64, f64, String)> = (0..60)
            .map(|i| (i as f64 / 60.0, (i as f64 + 0.8) / 60.0, format!("w{i}")))
            .collect();
        let big = kinetic(&many, 480, 270, 30, 6000, "#FFFF00", 40.0, 0.78);
        assert_eq!(big.layers.len(), 60);
        assert!(
            render_all_frames(&big).is_ok(),
            "60-word kinetic renders all frames"
        );
    }

    #[test]
    fn free_title_anchors_and_renders() {
        // Centre placement: with Center align the text layer box centre must land
        // on (cx, cy) — x = cx - w/2, y = cy - h/2 (w=0.9, h=0.16).
        let spec = free_title(
            "Hello",
            640,
            360,
            30,
            1000,
            "#FFFFFF",
            48.0,
            0.5,
            0.5,
            TextAlign::Center,
            false,
        );
        let text_layer = spec.layers.last().expect("a text layer");
        assert!(
            (text_layer.x - (0.5 - 0.45)).abs() < 1e-9,
            "centre x = cx - w/2"
        );
        assert!(
            (text_layer.y - (0.5 - 0.08)).abs() < 1e-9,
            "centre y = cy - h/2"
        );
        validate(&spec).expect("free spec validates");
        // A mid frame (peak opacity) renders SOME opaque text pixels.
        let mid = render_frame(&spec, 15).expect("mid frame renders");
        assert!(
            mid.chunks_exact(4).any(|p| p[3] > 0),
            "free title text is visible"
        );

        // Left align anchors the box AT cx (x == cx); Right anchors box right (x == cx - w).
        let left = free_title(
            "L",
            640,
            360,
            30,
            1000,
            "#FFFFFF",
            40.0,
            0.2,
            0.8,
            TextAlign::Left,
            false,
        );
        assert!(
            (left.layers.last().unwrap().x - 0.2).abs() < 1e-9,
            "left: x == cx"
        );
        let right = free_title(
            "R",
            640,
            360,
            30,
            1000,
            "#FFFFFF",
            40.0,
            0.8,
            0.2,
            TextAlign::Right,
            false,
        );
        assert!(
            (right.layers.last().unwrap().x - (0.8 - 0.9)).abs() < 1e-9,
            "right: x == cx - w"
        );

        // With bg, a Rect pill is added BEHIND the text (2 layers, rect first).
        let with_bg = free_title(
            "Bg",
            640,
            360,
            30,
            1000,
            "#FFFFFF",
            40.0,
            0.5,
            0.5,
            TextAlign::Center,
            true,
        );
        assert_eq!(with_bg.layers.len(), 2, "bg adds a backing rect");
        assert!(
            matches!(with_bg.layers[0].content, LayerContent::Rect { .. }),
            "rect drawn behind text"
        );
    }

    /// Every catalog template builds a renderable spec with visible pixels at
    /// the hold, and the catalog/name lists agree with `build_template`'s arms.
    #[test]
    fn templates_build_and_render() {
        let names: Vec<&str> = TITLE_TEMPLATES.iter().map(|t| t.name).collect();
        assert_eq!(names, TITLE_TEMPLATE_NAMES, "catalog order == name list");
        for info in TITLE_TEMPLATES {
            let spec = build_template(
                info.name,
                "Make it pop now",
                1280,
                720,
                30,
                2500,
                "#FFFFFF",
                72.0,
                "#FFCC00",
                Some("pop"),
            )
            .unwrap_or_else(|| unreachable!("template {} builds", info.name));
            // Spec is structurally valid (validate runs inside render_frame).
            validate(&spec)
                .unwrap_or_else(|e| unreachable!("template {} validates: {e}", info.name));
            // A frame in the hold band has visible pixels.
            let mid = render_frame(&spec, frame_count(&spec) * 6 / 10)
                .unwrap_or_else(|_| unreachable!("template {} mid frame renders", info.name));
            assert_eq!(mid.len() as u32, spec.width * spec.height * 4);
            assert!(
                mid.chunks_exact(4).any(|p| p[3] > 0),
                "template {} has visible pixels mid-hold",
                info.name
            );
        }
        // Unknown template name → None (the verb rejects it).
        assert!(
            build_template("bogus", "x", 320, 180, 30, 1000, "#FFFFFF", 40.0, "#FF0000", None)
                .is_none()
        );
    }

    /// kinetic_emphasis paints the chosen word in the ACCENT color (a distinct
    /// red here, absent from a white-on-transparent base), proving the overlay
    /// recolor landed.
    #[test]
    fn kinetic_emphasis_paints_accent() {
        let spec = tpl_kinetic_emphasis(
            "buy this house",
            1280,
            720,
            30,
            2500,
            "#FFFFFF",
            96.0,
            "#FF0000",
            Some("house"),
        );
        // base line + accent overlay = 2 layers.
        assert_eq!(spec.layers.len(), 2, "base + emphasis overlay");
        // At the emphasis hold (~50%), some pixel is strongly red (the accent),
        // which a pure white/grey base could never produce.
        let f = render_frame(&spec, frame_count(&spec) / 2).expect("emphasis frame");
        let red = f
            .chunks_exact(4)
            .any(|p| p[3] > 120 && p[0] > 170 && p[1] < 90 && p[2] < 90);
        assert!(red, "the emphasized word renders in the accent color");
    }

    /// caption_karaoke holds the dim base + grows an accent prefix: the base line
    /// (1 layer) plus one accent prefix per word.
    #[test]
    fn karaoke_layers_and_fill() {
        let spec = tpl_caption_karaoke(
            "one two three four",
            1280,
            720,
            30,
            4000,
            "#FFFFFF",
            56.0,
            "#22DD88",
        );
        assert_eq!(spec.layers.len(), 1 + 4, "dim base + 4 accent prefixes");
        // Early (~10%) only the first word(s) are highlighted; late (~80%) more
        // accent is visible — both frames render without error.
        assert!(render_frame(&spec, 4).is_ok());
        assert!(render_frame(&spec, frame_count(&spec) * 8 / 10).is_ok());
    }

    /// word_pop ACCUMULATES (no blank between words): by the hold band every word
    /// is on screen, so the late frame has clearly MORE ink than mid-build, and
    /// no interior frame in the hold is empty. Guards the sequential-gap bug that
    /// blanked the frame exactly at a word handoff.
    #[test]
    fn word_pop_accumulates_no_blank() {
        let spec = tpl_word_pop(
            "BUY NOW SAVE BIG TODAY",
            1280,
            720,
            30,
            2500,
            "#FFFFFF",
            88.0,
        );
        assert_eq!(spec.layers.len(), 5, "one layer per word");
        let opaque = |f: u32| {
            render_frame(&spec, f)
                .unwrap()
                .chunks_exact(4)
                .filter(|p| p[3] > 40)
                .count()
        };
        let n = frame_count(&spec);
        // The full line is up by the hold (~80%); more ink than early build (~20%).
        assert!(opaque(n * 8 / 10) > opaque(n * 2 / 10), "line accumulates");
        // No blank frame across the build+hold band (every 10% step is non-empty
        // once the first word is in) — the bug produced a zero-ink frame at a
        // word boundary.
        for f in (n * 2 / 10)..=(n * 85 / 100) {
            if f % 3 == 0 {
                assert!(opaque(f) > 0, "frame {f} is non-blank during build/hold");
            }
        }
    }

    /// typewriter reveals progressively: an early frame has strictly fewer
    /// opaque pixels than a near-end frame (more characters typed).
    #[test]
    fn typewriter_reveals_progressively() {
        let spec = tpl_typewriter("HELLO WORLD", 1280, 720, 30, 3000, "#FFFFFF", 80.0);
        let opaque = |buf: &[u8]| buf.chunks_exact(4).filter(|p| p[3] > 40).count();
        let early = render_frame(&spec, frame_count(&spec) / 10).expect("early frame");
        let late = render_frame(&spec, frame_count(&spec) * 65 / 100).expect("late frame");
        assert!(
            opaque(&late) > opaque(&early),
            "more text is visible later ({} > {})",
            opaque(&late),
            opaque(&early)
        );
    }

    /// measure_text_width is monotonic in content and zero for empty/whitespace.
    #[test]
    fn measure_text_width_is_sane() {
        assert_eq!(measure_text_width("", "DejaVu Sans", 64.0, 400), 0.0);
        assert_eq!(measure_text_width("   ", "DejaVu Sans", 64.0, 400), 0.0);
        let short = measure_text_width("Hi", "DejaVu Sans", 64.0, 400);
        let long = measure_text_width("Hi there friend", "DejaVu Sans", 64.0, 400);
        assert!(short > 0.0, "non-empty text has width");
        assert!(long > short, "longer text is wider ({long} > {short})");
        // A space has a positive advance.
        assert!(space_width("DejaVu Sans", 64.0, 400) > 0.0);
    }

    fn assert_text_layers_fit(spec: &TitleSpec) {
        for (idx, layer) in spec.layers.iter().enumerate() {
            let LayerContent::Text {
                text,
                font_family,
                font_px,
                weight,
                ..
            } = &layer.content
            else {
                continue;
            };
            let measured = measure_text_width(text, font_family, *font_px, *weight);
            let limit = layer.w * spec.width as f64;
            assert!(
                measured <= limit + 1.0,
                "text layer {idx} must fit its declared box: measured={measured:.2}, limit={limit:.2}, text={text:?}"
            );
        }
    }

    #[test]
    fn plain_presets_shrink_long_text_to_layer_width() {
        let long = "A very long title that must still fit inside the rendered frame without disappearing past the edge";
        for spec in [
            top_bar(long, 1280, 720, 30, 2000, "#FFFFFF", 96.0, true),
            subtitle(long, 1280, 720, 30, 2000, "#FFFFFF", 96.0),
            headline(long, 1280, 720, 30, 2000, "#FFFFFF", 96.0),
            lower_third(long, 1280, 720, 30, 2000, "#FFFFFF", 96.0, true),
            title_card(long, 1280, 720, 30, 2000, "#FFFFFF", 96.0),
            free_title(
                long,
                1280,
                720,
                30,
                2000,
                "#FFFFFF",
                96.0,
                0.5,
                0.5,
                TextAlign::Center,
                true,
            ),
        ] {
            assert_text_layers_fit(&spec);
        }
    }

    #[test]
    fn animated_templates_shrink_long_text_to_layer_width() {
        let long = "A very long animated title should fit every generated text layer instead of overflowing the frame";
        for template in [
            "typewriter",
            "slide_stack",
            "kinetic_emphasis",
            "lower_third_reveal",
            "caption_karaoke",
        ] {
            let spec = build_template(
                template, long, 1280, 720, 30, 2000, "#FFFFFF", 96.0, "#FFD24A", None,
            )
            .unwrap_or_else(|| panic!("template {template} builds"));
            assert_text_layers_fit(&spec);
        }
    }

    #[test]
    fn present_or_default_is_weight_aware_and_keeps_visible_fallback() {
        let fallback = fonts().family.clone();
        assert_eq!(
            present_or_default("__definitely_missing_shellx_font__", 700),
            fallback
        );
        assert_eq!(present_or_default(&fallback, 700), fallback);
    }

    #[test]
    fn text_svg_uses_real_central_baseline() {
        let spec = text_fade_spec();
        let svg = build_svg(&spec, 1.0).expect("svg builds");
        assert!(
            svg.contains("dominant-baseline=\"central\""),
            "text should use usvg's central baseline support: {svg}"
        );
        assert!(
            !svg.contains("y=\"112.4000\""),
            "svg output must not use the old 0.35em baseline approximation: {svg}"
        );
    }

    /// Every shape kind builds a renderable spec with visible pixels at the
    /// hold; unknown kind → None; a styled box adds a text layer.
    #[test]
    fn shapes_build_and_render() {
        let base = ShapeParams {
            fill: Some("#3366FF".into()),
            opacity: 0.9,
            stroke: Some("#FFFFFF".into()),
            stroke_px: 6.0,
            radius_px: 16.0,
            head_px: 40.0,
            text: None,
            text_color: "#FFFFFF".into(),
            font_px: 48.0,
        };
        for kind in SHAPE_KINDS {
            let spec = build_shape(
                kind, 0.2, 0.3, 0.4, 0.25, 0.7, 0.6, &base, 1280, 720, 30, 2000,
            )
            .unwrap_or_else(|| unreachable!("shape {kind} builds"));
            validate(&spec).unwrap_or_else(|e| unreachable!("shape {kind} validates: {e}"));
            let mid = render_frame(&spec, frame_count(&spec) / 2)
                .unwrap_or_else(|_| unreachable!("shape {kind} renders"));
            assert!(
                mid.chunks_exact(4).any(|p| p[3] > 0),
                "shape {kind} has visible pixels"
            );
        }
        assert!(
            build_shape("bogus", 0.0, 0.0, 0.1, 0.1, 0.2, 0.2, &base, 320, 180, 30, 1000).is_none()
        );

        // A styled text box = StrokeBox + a Text layer.
        let mut p = base.clone();
        p.text = Some("CALLOUT".into());
        let boxed = build_shape(
            "rect", 0.1, 0.1, 0.5, 0.2, 0.0, 0.0, &p, 1280, 720, 30, 2000,
        )
        .unwrap();
        assert_eq!(boxed.layers.len(), 2, "box + label");
        assert!(matches!(
            boxed.layers[0].content,
            LayerContent::StrokeBox { .. }
        ));
        assert!(matches!(boxed.layers[1].content, LayerContent::Text { .. }));

        // An outline-only rect (no fill) still renders its border.
        let mut outline = base.clone();
        outline.fill = None;
        let o = build_shape(
            "rect", 0.2, 0.2, 0.4, 0.3, 0.0, 0.0, &outline, 640, 360, 30, 1000,
        )
        .unwrap();
        let f = render_frame(&o, frame_count(&o) / 2).expect("outline renders");
        assert!(
            f.chunks_exact(4).any(|p| p[3] > 0),
            "outline border visible"
        );
    }

    /// An arrow renders BOTH a shaft and a filled head (more ink than a bare line
    /// of the same geometry) — proves the polygon head emits.
    #[test]
    fn arrow_has_head() {
        let p = ShapeParams {
            fill: None,
            opacity: 1.0,
            stroke: Some("#FF0000".into()),
            stroke_px: 8.0,
            radius_px: 0.0,
            head_px: 60.0,
            text: None,
            text_color: "#FFFFFF".into(),
            font_px: 40.0,
        };
        let arrow = build_shape(
            "arrow", 0.2, 0.5, 0.0, 0.0, 0.8, 0.5, &p, 1280, 720, 30, 1500,
        )
        .unwrap();
        let line = build_shape(
            "line", 0.2, 0.5, 0.0, 0.0, 0.8, 0.5, &p, 1280, 720, 30, 1500,
        )
        .unwrap();
        let ink = |s: &TitleSpec| {
            render_frame(s, frame_count(s) / 2)
                .unwrap()
                .chunks_exact(4)
                .filter(|px| px[3] > 40)
                .count()
        };
        assert!(
            ink(&arrow) > ink(&line),
            "arrow (shaft+head) has more ink than a line"
        );
        // The head is red (#FF0000) — present in the arrow frame.
        let f = render_frame(&arrow, frame_count(&arrow) / 2).unwrap();
        assert!(
            f.chunks_exact(4)
                .any(|px| px[3] > 120 && px[0] > 170 && px[1] < 80 && px[2] < 80),
            "arrow renders in the stroke color"
        );
    }

    #[test]
    fn frame_count_math() {
        // 1000ms @ 30fps == 30 frames.
        let spec = text_fade_spec();
        assert_eq!(frame_count(&spec), 30);

        // ceil behaviour: 1001ms @ 30fps → 31 (30.03 → ceil 31).
        let mut s = text_fade_spec();
        s.duration_ms = 1001;
        assert_eq!(frame_count(&s), 31);

        // Sub-frame duration still yields at least 1 frame.
        s.duration_ms = 1;
        s.fps = 30;
        assert_eq!(frame_count(&s), 1);
    }

    #[test]
    fn buffer_length_matches_geometry() {
        let spec = text_fade_spec();
        let buf = render_frame(&spec, 0).expect("frame 0 renders");
        assert_eq!(buf.len(), 320 * 180 * 4);
    }

    #[test]
    fn text_opacity_ramp_animates() {
        let spec = text_fade_spec();
        let n = frame_count(&spec);

        // Frame 0: opacity ~0 → essentially transparent (max alpha ≈ 0).
        let f0 = render_frame(&spec, 0).expect("frame 0 renders");
        assert!(
            max_alpha(&f0) <= 2,
            "frame 0 should be ~transparent, max alpha was {}",
            max_alpha(&f0)
        );

        // Last frame: opacity 1 → text visible (some pixel alpha > 0).
        let f_last = render_frame(&spec, n - 1).expect("last frame renders");
        assert!(
            max_alpha(&f_last) > 0,
            "last frame should have visible text (alpha > 0)"
        );
    }

    #[test]
    fn frame_idx_overshoot_clamps_to_last() {
        let spec = text_fade_spec();
        let n = frame_count(&spec);
        let last = render_frame(&spec, n - 1).expect("last frame renders");
        // Overshooting the frame count must NOT error — it clamps to last.
        let over = render_frame(&spec, n + 500).expect("overshoot clamps, no error");
        assert_eq!(last, over, "overshoot frame should equal the last frame");
    }

    #[test]
    fn rect_renders_opaque_pixels() {
        let spec = TitleSpec {
            width: 64,
            height: 64,
            fps: 30,
            duration_ms: 1000,
            layers: vec![TitleLayer {
                content: LayerContent::Rect {
                    color: "#FF0000".to_string(),
                    opacity: 1.0,
                    radius_px: 0.0,
                },
                x: 0.1,
                y: 0.1,
                w: 0.8,
                h: 0.8,
                keyframes: vec![Keyframe {
                    t: 0.0,
                    opacity: 1.0,
                    tx: 0.0,
                    ty: 0.0,
                    scale: 1.0,
                }],
                easing: Easing::Linear,
            }],
        };
        let buf = render_frame(&spec, 0).expect("rect renders");
        // Full-opacity rect → at least one fully-opaque pixel.
        assert_eq!(max_alpha(&buf), 255, "opaque rect should produce alpha 255");
        // And it should be red where painted: find a pixel with high R, low G/B.
        let red_pixel = buf
            .chunks_exact(4)
            .any(|p| p[3] == 255 && p[0] > 200 && p[1] < 40 && p[2] < 40);
        assert!(red_pixel, "expected a solid red opaque pixel from the rect");
    }

    #[test]
    fn render_all_frames_count() {
        let spec = text_fade_spec();
        let frames = render_all_frames(&spec).expect("all frames render");
        assert_eq!(frames.len(), frame_count(&spec) as usize);
        assert!(frames.iter().all(|f| f.len() == 320 * 180 * 4));
    }

    #[test]
    fn invalid_specs_error_not_panic() {
        // Empty layers → NoLayers.
        let mut s = text_fade_spec();
        s.layers.clear();
        assert!(matches!(render_frame(&s, 0), Err(TitleError::NoLayers)));

        // Zero fps → ZeroFps.
        let mut s = text_fade_spec();
        s.fps = 0;
        assert!(matches!(render_frame(&s, 0), Err(TitleError::ZeroFps)));

        // Zero duration → ZeroDuration.
        let mut s = text_fade_spec();
        s.duration_ms = 0;
        assert!(matches!(render_frame(&s, 0), Err(TitleError::ZeroDuration)));

        // Zero keyframes → NoKeyframes.
        let mut s = text_fade_spec();
        s.layers[0].keyframes.clear();
        assert!(matches!(
            render_frame(&s, 0),
            Err(TitleError::NoKeyframes(0))
        ));

        // Bad color → BadColor (not a panic).
        let mut s = text_fade_spec();
        if let LayerContent::Text { color, .. } = &mut s.layers[0].content {
            *color = "not-a-color".to_string();
        }
        assert!(matches!(render_frame(&s, 0), Err(TitleError::BadColor(_))));
    }

    #[test]
    fn invalid_numeric_specs_error_before_svg_or_pixmap() {
        let mut s = text_fade_spec();
        s.width = 0;
        assert!(matches!(
            render_frame(&s, 0),
            Err(TitleError::InvalidSpec(_))
        ));

        let mut s = text_fade_spec();
        if let LayerContent::Text { font_px, .. } = &mut s.layers[0].content {
            *font_px = 0.0;
        }
        assert!(matches!(
            render_frame(&s, 0),
            Err(TitleError::InvalidSpec(_))
        ));

        let mut s = text_fade_spec();
        s.layers[0].keyframes[0].scale = f64::NAN;
        assert!(matches!(
            render_frame(&s, 0),
            Err(TitleError::InvalidSpec(_))
        ));
    }

    #[test]
    fn xml_escape_removes_xml_illegal_controls() {
        assert_eq!(xml_escape("A\u{0001}<B&\u{000b}C"), "A&lt;B&amp;C");
        assert_eq!(xml_escape("line\nbreak\tok"), "line\nbreak\tok");
    }

    #[test]
    fn kinetic_clamps_vertical_position_to_frame() {
        let low = kinetic(
            &[(0.0, 1.0, "hello".into())],
            1920,
            1080,
            30,
            1000,
            "#fff",
            64.0,
            -2.0,
        );
        let high = kinetic(
            &[(0.0, 1.0, "hello".into())],
            1920,
            1080,
            30,
            1000,
            "#fff",
            64.0,
            2.0,
        );
        assert_eq!(low.layers[0].y, 0.0);
        assert_eq!(high.layers[0].y, 1.0);
    }

    #[test]
    fn hex_color_parsing() {
        assert_eq!(parse_hex_color("#FFFFFF").unwrap(), (255, 255, 255));
        assert_eq!(parse_hex_color("#000000").unwrap(), (0, 0, 0));
        assert_eq!(parse_hex_color("#1a2B3c").unwrap(), (0x1a, 0x2b, 0x3c));
        assert!(parse_hex_color("FFFFFF").is_err()); // no #
        assert!(parse_hex_color("#FFF").is_err()); // too short
        assert!(parse_hex_color("#GGGGGG").is_err()); // bad hex digits
        assert!(parse_hex_color("").is_err());
    }

    #[test]
    fn unsorted_keyframes_error() {
        let mut s = text_fade_spec();
        s.layers[0].keyframes = vec![
            Keyframe {
                t: 1.0,
                opacity: 1.0,
                tx: 0.0,
                ty: 0.0,
                scale: 1.0,
            },
            Keyframe {
                t: 0.0,
                opacity: 0.0,
                tx: 0.0,
                ty: 0.0,
                scale: 1.0,
            },
        ];
        assert!(matches!(
            render_frame(&s, 0),
            Err(TitleError::UnsortedKeyframes(0))
        ));
    }

    #[test]
    fn invalid_keyframe_values_fail_validation() {
        for (field, bad) in [
            (
                "t",
                Keyframe {
                    t: 1.5,
                    opacity: 1.0,
                    tx: 0.0,
                    ty: 0.0,
                    scale: 1.0,
                },
            ),
            (
                "opacity",
                Keyframe {
                    t: 0.0,
                    opacity: 2.0,
                    tx: 0.0,
                    ty: 0.0,
                    scale: 1.0,
                },
            ),
            (
                "scale",
                Keyframe {
                    t: 0.0,
                    opacity: 1.0,
                    tx: 0.0,
                    ty: 0.0,
                    scale: 20.0,
                },
            ),
        ] {
            let mut s = text_fade_spec();
            s.layers[0].keyframes = vec![bad];
            let err = render_frame(&s, 0).expect_err("invalid keyframe should reject");
            assert!(
                matches!(err, TitleError::InvalidSpec(ref msg) if msg.contains(field)),
                "expected {field} validation error, got {err:?}"
            );
        }
    }
}
