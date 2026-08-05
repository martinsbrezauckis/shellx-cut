//! render.rs — EDL rendering + frame extraction (public verb contract "render").
//!
//! Role: turn an Edl into ffmpeg filter_complex graphs:
//! - final render: trim/atrim + concat per track, caption burn-in via
//!   subtitles filter (generated SRT/ASS), gain via volume filter;
//!   DETERMINISTIC — fixed encoder params, ffmpeg::DETERMINISM_FLAGS;
//!   same input + EDL ⇒ same output hash.
//! - preview: fast low-res render of a window around at_ms.
//! - frame: render exactly ONE composed frame as JPEG (agent's eyes).
//! Dependencies: ffmpeg.rs, captions.rs, cut-core (Edl/Project).
//! Primary callers: server render.* verbs + render job; e2e.

use crate::ffmpeg::{
    concat_demuxer_file_line, escape_filter_path, run_ffmpeg, run_ffmpeg_with_progress,
    DETERMINISM_FLAGS,
};
use crate::paths::PathFence;
use cut_core::{error_codes, CutError, Edl, EdlSegment, Project, TrackKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Progress callback: fraction 0.0..=1.0, called from the render thread.
/// Boxed dyn so the server can forward into its WS event bus.
pub type ProgressFn = Box<dyn Fn(f32) + Send + Sync>;

mod input_paths;
mod options;
use input_paths::strip_verbatim_prefix;
pub use options::{
    apply_bitrate, format_codec_args, parse_bitrate_kbps, platform_spec, set_audio_bitrate, Fit,
    PlatformSpec, RenderOptions, RenderPreset, Resolution, FORMAT_NAMES, PLATFORM_NAMES,
    PRESET_NAMES,
};

/// Facts about a finished render — feeds the RenderReceipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderOutput {
    pub path: PathBuf,
    /// "sha256:<hex>" of the output file (determinism evidence).
    pub hash: String,
    /// Measured (ffprobe) output duration, ms.
    pub duration_ms: u64,
    pub preset: String,
    /// Resolved output geometry (px) — project settings, or the largest
    /// source under resolution=match_source. Default skips for byte-identical
    /// receipts of older renders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Fit mode used (contain|cover). None = the default contain, kept off the
    /// receipt so older receipts are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fit: Option<String>,
    /// Render pipeline: `Some("gpu")` for a GPU fast-track render (NVDEC +
    /// scale_cuda + nvenc — a NON-deterministic fast mode), `None` for the default
    /// software path. None is skipped so every software receipt stays byte-identical
    /// to a pre-GPU receipt; a GPU receipt records that byte-identical replay does
    /// NOT apply to it (GPU output varies by driver/hardware).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<String>,
}

/// One `-i` input of the graph. Stills (kind=image probes) are fed with
/// `-loop 1` so the single frame becomes an infinite stream the segment
/// chain can trim to the clip duration (the renderer "loops the still").
struct GraphInput {
    path: PathBuf,
    /// True when the asset probed as a still image (probe.kind == "image").
    image: bool,
    /// GPU fast-track: NVDEC-decode THIS input to CUDA frames (graph_args emits
    /// `-hwaccel cuda` for it). Per-INPUT (not whole-graph) because the GPU path
    /// keeps the expensive BASE track on the GPU (NVDEC→scale_cuda→nvenc) while
    /// OVERLAY inputs decode on the CPU — the small overlay needs CPU-only filters
    /// (pad/colorchannelmixer/transparent-filler) before a single hwupload into
    /// `overlay_cuda` (2b-ii). Software graph: always false (system-memory frames).
    gpu_decode: bool,
}

/// A built filter_complex program: the `-i` inputs plus the graph text and
/// the labels of its composed outputs. Internal shape shared by final /
/// preview / frame so all three render the SAME composition.
struct Graph {
    /// Resolved asset inputs, in `-i` order.
    inputs: Vec<GraphInput>,
    /// The filter_complex text.
    filter: String,
    /// Label of the composed video stream, e.g. "vout".
    video_out: String,
    /// Label of the mixed audio stream; None when no audio segments exist.
    audio_out: Option<String>,
    /// TempDir holding the generated ASS file — must stay alive until the
    /// ffmpeg run finishes (the graph references a path inside it).
    _ass_dir: Option<tempfile::TempDir>,
}

/// Format ms as fractional seconds for filter args ("1.500").
fn secs(ms: u64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// Format fps without a trailing ".000" for integer rates (filter syntax).
fn fps_str(fps: f64) -> String {
    if fps.fract() == 0.0 {
        format!("{}", fps as u64)
    } else {
        format!("{fps:.3}")
    }
}

/// Conform-to-frame filter for ONE video segment (the scale/pad/crop stage
/// that fits a source into the output WxH under a Fit mode.
///
/// - `Contain` (default): scale to fit INSIDE the frame
///   (force_original_aspect_ratio=decrease) then pad the remainder black —
///   letterbox/pillarbox, no pixels lost.
/// - `Cover`: scale to COVER the frame (force_original_aspect_ratio=increase)
///   then centre-crop the overflow to exactly WxH — fills the frame, overflow
///   lost.
/// Always ends `setsar=1` (square pixels before concat) — for Contain
/// the pad makes the stream exactly WxH, for Cover the crop does.
fn conform_filter(w: u32, h: u32, fit: Fit) -> String {
    match fit {
        Fit::Contain => format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1"
        ),
        Fit::Cover => format!(
            "scale={w}:{h}:force_original_aspect_ratio=increase,\
             crop={w}:{h},setsar=1"
        ),
    }
}

/// Per-segment conform filter. When a probed source (or source crop) already
/// matches the project frame exactly, avoid a redundant scale/fps-resample pass:
/// it softens stabilized footage and adds measurable inter-frame motion even
/// though the geometry is a no-op. Keep `setsar=1` so concat still sees square
/// pixels. Unknown/mismatched media takes the historical conform path.
fn conform_filter_for_asset(
    asset: &cut_core::Asset,
    crop: Option<&cut_core::ClipCrop>,
    w: u32,
    h: u32,
    fit: Fit,
) -> String {
    let dims = crop
        .filter(|c| c.w > 0 && c.h > 0)
        .map(|c| (c.w, c.h))
        .or_else(|| source_dims(asset));
    if dims == Some((w, h)) {
        "setsar=1".into()
    } else {
        conform_filter(w, h, fit)
    }
}

/// Per-segment frame-rate normalizer. `fps=N` is required when source fps differs
/// from the project, but on already-matching clips it can resample stabilized
/// frames enough to make a real stabilization look weak. For the no-op case,
/// normalize timestamps without dropping/duplicating frames.
fn fps_filter_for_asset(
    asset: &cut_core::Asset,
    project_fps: f64,
    speed: f64,
    frozen: bool,
) -> String {
    let fps = fps_str(project_fps);
    let same_fps = source_fps(asset)
        .map(|src| (src - project_fps).abs() < 0.01)
        .unwrap_or(false);
    if same_fps && !frozen && (speed - 1.0).abs() < 1e-6 {
        format!("settb=expr=1/{fps},setpts=N/({fps}*TB)")
    } else {
        format!("fps={fps}")
    }
}

/// Source-space crop filter suffix for a segment chain (`edit.crop`).
///
/// COMPOSE ORDER (the documented contract — see cut_core::ClipCrop): crop runs
/// in SOURCE space, BEFORE the conform scale/pad and any overlay transform. It
/// is emitted right after `setpts=PTS-STARTPTS` (still source geometry) so the
/// scale/pad that follows conforms the CROPPED picture to the project frame.
/// Returns `,crop=w:h:x:y` (ffmpeg's arg order is w:h:x:y) or "" when no crop.
/// Values are source px straight from ClipCrop — even-sized by construction in
/// the common case (content_bbox uses cropdetect round=2); ffmpeg tolerates
/// odd crop sizes here because the following scale re-rounds to the frame.
fn crop_filter(crop: Option<&cut_core::ClipCrop>) -> String {
    match crop {
        Some(c) if c.w > 0 && c.h > 0 => format!(",crop={}:{}:{}:{}", c.w, c.h, c.x, c.y),
        _ => String::new(),
    }
}

/// The color-grade filter chain for a video segment (edit.grade). Returns a
/// LEADING-comma filter string or "" when ungraded — applied AFTER conform
/// (grade is per-pixel; compose order crop → conform → GRADE → transform).
/// Parametric `eq` (contrast/brightness/saturation/gamma) is emitted whenever a
/// non-identity grade is present; `colortemperature` and the user `lut3d` are
/// appended only when set. An identity grade emits "" so a graded-then-reset
/// clip renders byte-identical to never-graded.
fn grade_filter(grade: Option<&cut_core::ClipGrade>) -> String {
    let Some(g) = grade else { return String::new() };
    if g.is_identity() {
        return String::new();
    }
    let mut s = format!(
        ",eq=contrast={}:brightness={}:saturation={}:gamma={}",
        fnum(g.contrast),
        fnum(g.brightness),
        fnum(g.saturation),
        fnum(g.gamma)
    );
    if let Some(k) = g.temperature_k {
        // ffmpeg `colortemperature` works in Kelvin; clamp to its supported band.
        let k = k.clamp(1000, 40000);
        s.push_str(&format!(",colortemperature=temperature={k}"));
    }
    if let Some(lut) = &g.lut {
        // The path is fenced at verb time (exists + ends .cube). lut3d reads it;
        // escape_filter_path handles filtergraph-special chars (same as the ass
        // subtitle path).
        s.push_str(&format!(
            ",lut3d=file={}",
            escape_filter_path(std::path::Path::new(lut))
        ));
    }
    s
}

/// The color-grade filter chain for a segment, honoring a LAYERED grade STACK
/// (edit.grade_stack) when present. When `stack` is EMPTY this is EXACTLY
/// [`grade_filter`] of the single `grade` — the legacy per-clip-grade path, so an
/// un-stacked clip renders BYTE-IDENTICAL. When `stack` is non-empty it is the
/// authority (the single `grade` is None on a stacked clip): each layer's
/// [`grade_filter`] is concatenated IN ORDER, so layer 2 grades layer 1's output, etc.
/// A SINGLE-element stack emits exactly `grade_filter(layer)`, so it is byte-identical
/// to the equivalent single `edit.grade`. Identity layers contribute "" (grade_filter
/// collapses them), though the verb already drops them at store time.
fn grade_stack_filter(
    grade: Option<&cut_core::ClipGrade>,
    stack: &[cut_core::ClipGrade],
) -> String {
    if stack.is_empty() {
        return grade_filter(grade);
    }
    stack.iter().map(|g| grade_filter(Some(g))).collect()
}

/// One `zscale` color-space conversion hop `from`→`to`. The INPUT space is stated
/// explicitly (`tin/pin/min`) rather than read from frame metadata, so the result is
/// deterministic regardless of how the upstream filters tagged the frame. The OUTPUT
/// tokens (`t/p/m`) also become the frame's new color metadata, which libx264/libx265
/// propagate to the encoded stream's VUI (so a working→output hop tags the file).
fn zscale_hop(from: cut_core::ColorSpace, to: cut_core::ColorSpace) -> String {
    format!(
        ",zscale=tin={ti}:pin={pi}:min={mi}:t={tt}:p={tp}:m={tm}",
        ti = from.zs_transfer(),
        pi = from.zs_primaries(),
        mi = from.zs_matrix(),
        tt = to.zs_transfer(),
        tp = to.zs_primaries(),
        tm = to.zs_matrix(),
    )
}

/// The COLOR-MANAGEMENT filter chain for a video segment (project.color +
/// edit.color_space). Converts the clip's pixels INPUT → WORKING → OUTPUT via up to
/// two `zscale` hops (each identity hop skipped). Returns a LEADING-comma filter
/// string, or "" when the whole chain is identity — i.e. the project is the default
/// rec709 working+output AND the clip carries no input tag — so a default-color
/// render is BYTE-IDENTICAL to a pre-color-management render.
///
/// `input` = the clip's tagged source space (edit.color_space); None ⇒ the source is
/// assumed already in the WORKING space, so the input→working hop is skipped. `color`
/// = the project working/output config (project.color). Compose order: this runs as
/// the FINAL per-clip pixel stage (after grade/effects), so grade/effects operate in
/// the source/working space and the result is delivered in the output space.
fn colorspace_filter(
    input: Option<&cut_core::ColorSpace>,
    color: &cut_core::ColorConfig,
) -> String {
    let working = color.working;
    let output = color.output;
    // An untagged clip is assumed already in the working space → no input hop.
    let eff_in = input.copied().unwrap_or(working);
    let mut s = String::new();
    // hop 1: input → working (only when the clip is tagged in a different space).
    if eff_in != working {
        s.push_str(&zscale_hop(eff_in, working));
    }
    // hop 2: working → output (only when they differ).
    if working != output {
        s.push_str(&zscale_hop(working, output));
    }
    s
}

/// Output-stream color TAGGING flags for the final encode (`-colorspace`/
/// `-color_primaries`/`-color_trc`) so ffprobe reports the delivered file's OUTPUT
/// color space. Emitted ONLY when output ≠ rec709 (the default): a rec709 output is
/// the universal assumed-default, so leaving it untagged keeps the default + near-
/// default renders byte-identical, while a rec2020/srgb/linear delivery is explicitly
/// tagged. The per-clip `zscale` already converts the PIXELS; these flags make the
/// container metadata explicit + robust across encoders. Empty Vec = no flags.
fn output_color_args(color: &cut_core::ColorConfig) -> Vec<String> {
    if color.output == cut_core::ColorSpace::Rec709 {
        return Vec::new();
    }
    vec![
        "-colorspace".into(),
        color.output.zs_matrix().to_string(),
        "-color_primaries".into(),
        color.output.zs_primaries().to_string(),
        "-color_trc".into(),
        color.output.zs_transfer().to_string(),
    ]
}

fn rounded_u32_clamped(v: f64, cap: u32) -> u32 {
    if cap == 0 || v.is_nan() {
        return 0;
    }
    if !v.is_finite() {
        return if v.is_sign_positive() { cap } else { 0 };
    }
    let target = v.round().clamp(0.0, f64::from(cap));
    let mut lo = 0u32;
    let mut hi = cap;
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if f64::from(mid) <= target {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

fn even_size_px(v: f64, cap: u32) -> u32 {
    let cap = cap & !1u32;
    if cap < 2 {
        return cap;
    }
    (rounded_u32_clamped(v, cap).max(2) & !1u32).min(cap)
}

fn even_pos_px(v: f64, max: u32) -> u32 {
    (rounded_u32_clamped(v, max) & !1u32).min(max & !1u32)
}

/// Scale/position a BASE-track clip per its `ClipTransform` (edit.transform).
///
/// Before this, `edit.transform` was a SILENT NO-OP on a base / single clip — it only
/// affected PiP OVERLAY tracks — so a user who selected their only clip and scaled or
/// moved it saw nothing change. This
/// returns a LEADING-comma `scale,setsar,pad` chain that shrinks the conformed WxH frame
/// to the transform's size and re-pads back to WxH with OPAQUE black around it (the base
/// is the canvas — nothing below to reveal, so the surround is black, not transparent).
/// `""` when the geometry is identity → the common case renders BYTE-IDENTICALLY. Geometry
/// mirrors the overlay PiP math (even-rounded for yuv420 chroma). Base opacity is handled by
/// [`base_opacity_filter`] so it can also follow opacity keyframes over the black canvas.
fn base_transform_filter(t: &cut_core::ClipTransform, w: u32, h: u32) -> String {
    if t.x == 0.0 && t.y == 0.0 && t.scale == 1.0 {
        return String::new();
    }
    let (ow, oh) = (
        even_size_px(f64::from(w) * t.scale, w),
        even_size_px(f64::from(h) * t.scale, h),
    );
    let (ox, oy) = (
        even_pos_px(f64::from(w) * t.x, w.saturating_sub(ow)),
        even_pos_px(f64::from(h) * t.y, h.saturating_sub(oh)),
    );
    format!(",scale={ow}:{oh},setsar=1,pad={w}:{h}:{ox}:{oy}:color=black")
}

/// Base clips have no lower layer, so opacity reveals the black canvas. Scale in
/// planar RGB: `geq`'s YUV source-plane lookups misaddress chroma on subsampled
/// yuv420p frames, which can preserve or tint the original instead of dimming it.
/// A keyframed opacity track overrides the static transform opacity, matching the
/// overlay path. Empty at a constant 1.0 keeps ordinary renders byte-identical.
fn base_opacity_filter(t: &cut_core::ClipTransform, keyframes: &[cut_core::Keyframe]) -> String {
    let opacity = match kf_points(keyframes, cut_core::KfParam::Opacity) {
        Some((pts, interp)) if !pts.is_empty() => {
            format!("clip(({}),0,1)", kf_expr(&pts, "T", interp))
        }
        _ if t.opacity != 1.0 => fnum(t.opacity.clamp(0.0, 1.0)),
        _ => return String::new(),
    };
    format!(
        ",format=gbrp,geq=r='r(X,Y)*{opacity}':g='g(X,Y)*{opacity}':b='b(X,Y)*{opacity}',format=yuv420p"
    )
}

/// Join a region-composited base clip back into its ordinary transform/opacity
/// chain. Both helpers intentionally return comma-PREFIXED snippets for the
/// common inline-chain call sites. A bare `[{in_label}]{vtransform}` therefore
/// becomes `[{in_label}],scale...`, which ffmpeg parses as an empty filter and
/// rejects (`No such filter: ''`). `null` is the explicit chain head, matching
/// the proven grade-window effect branch and preserving the helper contracts.
fn base_region_transform_block(
    in_label: &str,
    out_label: &str,
    vtransform: &str,
    vopacity: &str,
) -> String {
    format!("[{in_label}]null{vtransform}{vopacity}[{out_label}];")
}

/// ffmpeg filter snippet (comma-PREFIXED, appended to a clip's chain) for a
/// clip's visual EFFECTS (edit.effect), in order. `overlay` = is this an overlay
/// clip — chroma key is emitted ONLY there (it makes pixels TRANSPARENT to reveal
/// a lower track; on a base clip there is nothing below, and the verb refuses it,
/// so it is skipped). Every effect is a CPU-only ffmpeg filter, so a clip
/// carrying any effect keeps the timeline on the software renderer (the GPU gate
/// excludes `!effects.is_empty()`); these run on system-memory frames. Values are
/// clamped here as a belt-and-suspenders over the verb-side validation.
fn effect_filter(effects: &[cut_core::ClipEffect], overlay: bool) -> String {
    use cut_core::ClipEffect as E;
    let mut s = String::new();
    for e in effects {
        match e {
            E::Vignette { amount } => {
                // angle 0..PI/2 scaled by amount (PI/2 ≈ strong darkened corners).
                let a = amount.clamp(0.0, 1.0) * std::f64::consts::FRAC_PI_2;
                s.push_str(&format!(",vignette=angle={}", fnum(a)));
            }
            E::Sharpen { amount } => {
                // unsharp 5x5 luma mask; amount = luma_amount (0 = none).
                let a = amount.clamp(0.0, 5.0);
                s.push_str(&format!(",unsharp=5:5:{}:5:5:0.0", fnum(a)));
            }
            E::Blur { radius } => {
                let r = radius.clamp(0.1, 100.0);
                s.push_str(&format!(",gblur=sigma={}", fnum(r)));
            }
            E::Grain { amount } => {
                // additive temporal+uniform noise (film grain look).
                let a = amount.clamp(0.0, 100.0) as u32;
                s.push_str(&format!(",noise=alls={a}:allf=t+u"));
            }
            E::ChromaKey {
                color,
                similarity,
                blend,
            } => {
                // Emit only on overlay clips AND only for an allowlisted color
                // literal. set_effects rejects bad colors at the verb boundary,
                // but a project.json loaded from disk bypasses that path — so we
                // RE-CHECK here at point-of-use and SKIP (emit nothing) for an
                // invalid color rather than interpolate it. This closes
                // filtergraph injection (e.g. a ",movie=" payload) regardless of
                // how the effect reached the renderer. See is_valid_chroma_color.
                if overlay && cut_core::is_valid_chroma_color(color) {
                    let sim = similarity.clamp(0.0, 1.0);
                    let bl = blend.clamp(0.0, 1.0);
                    s.push_str(&format!(
                        ",chromakey=color={color}:similarity={}:blend={}",
                        fnum(sim),
                        fnum(bl)
                    ));
                }
            }
            E::Mirror => s.push_str(",hflip"),
            E::Flip => s.push_str(",vflip"),
            E::HueShift { degrees } => {
                if degrees.abs() > 1e-3 {
                    s.push_str(&format!(",hue=h={}", fnum(*degrees)));
                }
            }
            E::RgbSplit { amount } => {
                // Chromatic aberration: shift red right + blue left by `amount` px.
                let a = amount.clamp(0.0, 100.0).round() as i64;
                if a != 0 {
                    s.push_str(&format!(",rgbashift=rh={a}:bh=-{a}"));
                }
            }
            E::Pixelize { size } => {
                // Mosaic blocks; ffmpeg pixelize needs blocks ≥ 1px (even is safe).
                let n = (size.clamp(2.0, 256.0).round() as i64).max(2);
                s.push_str(&format!(",pixelize=width={n}:height={n}"));
            }
            E::Sepia => {
                // Fixed sepia colour matrix (classic old-photo warmth).
                s.push_str(",colorchannelmixer=.393:.769:.189:0:.349:.686:.168:0:.272:.534:.131");
            }
            E::AutoColor { amount } => {
                // Per-channel auto contrast + white balance; strength blends toward it.
                let a = amount.clamp(0.0, 1.0);
                s.push_str(&format!(",normalize=strength={}", fnum(a)));
            }
            E::Vhs { amount } => {
                // Retro-tape CHAIN: chroma shift + tape grain + soft blur, scaled by amount.
                let a = amount.clamp(0.0, 1.0);
                let rh = (1.0 + a * 6.0).round() as i64; // 1..7 px
                let nz = (a * 24.0).round() as u32;
                let bl = (0.2 + a * 0.8).max(0.1);
                s.push_str(&format!(
                    ",rgbashift=rh={rh}:bh=-{rh},noise=alls={nz}:allf=t,gblur=sigma={}",
                    fnum(bl)
                ));
            }
            E::Posterize { levels } => {
                // Quantize each channel to `levels` steps (lutrgb expr — no `:`/`,`/
                // spaces, so it survives the filtergraph unquoted; verified).
                let l = levels.clamp(2.0, 64.0).round();
                let step = (256.0 / l).round().max(1.0) as i64;
                let q = format!("floor(val/{step})*{step}");
                s.push_str(&format!(",lutrgb=r={q}:g={q}:b={q}"));
            }
            E::Invert => s.push_str(",negate"),
            E::Emboss => {
                // Relief look — a fixed emboss convolution kernel (alpha passthrough).
                s.push_str(
                    ",convolution=-2 -1 0 -1 1 1 0 1 2:-2 -1 0 -1 1 1 0 1 2:-2 -1 0 -1 1 1 0 1 2:0 0 0 0 1 0 0 0 0",
                );
            }
            // Audio effects are emitted in the audio chain (audio_effect_filter),
            // not here. Skip them in the video chain.
            E::Denoise { .. } | E::Compressor { .. } | E::Gate { .. } => {}
        }
    }
    s
}

/// The ffmpeg filter (NO leading comma) that produces the EFFECTED copy of a frame
/// for a mask region (edit.add_mask). Applied to a full-frame split copy; the
/// `alphamerge`+`overlay` then scopes it to the shape. `strength`: blur → gaussian
/// sigma px (default 15); pixelate → mosaic block px (default 16, even); black
/// ignores it (a solid black fill — a hard censor).
fn mask_effect_filter(mask: &cut_core::ClipMask) -> String {
    use cut_core::MaskEffect as ME;
    match mask.effect {
        ME::Blur => {
            let sigma = mask.strength.unwrap_or(15.0).clamp(0.5, 200.0);
            format!("gblur=sigma={}", fnum(sigma))
        }
        ME::Pixelate => {
            let b = ((mask.strength.unwrap_or(16.0).clamp(2.0, 256.0) as u32) & !1u32).max(2);
            format!("pixelize=width={b}:height={b}")
        }
        ME::Black => "drawbox=x=0:y=0:w=iw:h=ih:t=fill:color=black".to_string(),
    }
}

/// The masked-region COMPOSITE block (edit.add_mask) for a base/overlay segment.
/// Returns the multi-line filtergraph that splits `in_label` into base + effected
/// copies, scopes the effect to the baked shape alpha (white = inside; `,negate`
/// when `invert`), and `overlay`s it back — producing `out_label`. `mask_idx` is the
/// parallel mask-PNG input. Uses the proven matte `alphamerge`+`overlay` pattern
/// (NOT `maskedmerge`, which blends per-plane and would mishandle a gray mask's
/// chroma). `uniq` makes every intermediate label distinct. Empty when no mask.
#[allow(clippy::too_many_arguments)]
fn mask_block(
    mask: &cut_core::ClipMask,
    mask_idx: usize,
    in_label: &str,
    out_label: &str,
    uniq: &str,
    w: u32,
    h: u32,
    vfade: &str,
) -> String {
    let neg = if mask.invert { ",negate" } else { "" };
    let fx = mask_effect_filter(mask);
    // TIME-BOUNDING (edit.redact): gate the overlay to [start,end] (clip-local
    // seconds, the same time base the keyframe expressions use). Outside the window
    // the overlay is disabled, so the un-effected base copy `mb` shows through —
    // the effect is active ONLY in the range. None = whole clip (byte-identical to
    // the pre-range filtergraph: no `enable=` is emitted at all).
    let enable = match mask.range_ms {
        Some([a, b]) => format!(
            ":enable='between(t,{},{})'",
            fnum(a as f64 / 1000.0),
            fnum(b as f64 / 1000.0)
        ),
        None => String::new(),
    };
    format!(
        // overlay=shortest=1 is LOAD-BEARING: the mask PNG is a `-loop 1` still
        // (infinite), so alphamerge yields an infinite `ma`; without `shortest=1`
        // overlay never EOFs and the render HANGS. shortest=1 bounds the output to
        // the finite clip `mb`.
        "[{in_label}]split=2[mb{uniq}][mf{uniq}];\n\
         [mf{uniq}]{fx}[mx{uniq}];\n\
         [{mask_idx}:v]scale={w}:{h}:flags=bilinear,format=gray{neg}[mm{uniq}];\n\
         [mx{uniq}][mm{uniq}]alphamerge[ma{uniq}];\n\
         [mb{uniq}][ma{uniq}]overlay=shortest=1{enable},format=yuv420p{vfade}[{out_label}];"
    )
}

/// A mask is rendered by the PROCEDURAL geq path (vs the static baked-PNG path)
/// iff it carries a motion `track` AND is a rect/ellipse (the shapes geq paints
/// directly). polygon+track is rejected at edit time, so this never lies. The bake
/// loop skips these (no PNG); the render generates the moving alpha in-graph.
/// Use the PROCEDURAL geq path when ANY region (primary or an extra) is motion-
/// tracked — the geq paints all regions' moving alpha in one expression. (A fully-
/// static mask, even multi-region, takes the baked-PNG union path instead.) The
/// store-layer validation guarantees every region is rect/ellipse when any is tracked
/// (the geq can't paint a polygon), so we don't re-check shapes here.
fn mask_uses_geq(m: &cut_core::ClipMask) -> bool {
    m.track.is_some() || m.regions.iter().any(|r| r.track.is_some())
}

/// The 0/255 geq INDICATOR for ONE region: its rect/ellipse painted at a centre that
/// is either time-varying (`track` → piecewise-linear PIXEL expressions in geq's
/// clip-local seconds `T`, like keyframes) or constant (static centre from `points`).
/// The `points` give the SIZE (rect = half-span; ellipse = radii). Polygon → "0"
/// (rejected at edit time when tracked / multi-region-tracked).
fn region_indicator(
    shape: cut_core::MaskShape,
    points: &[[f64; 2]],
    track: Option<&[cut_core::MaskTrackPoint]>,
    wf: f64,
    hf: f64,
) -> String {
    use cut_core::MaskShape;
    if matches!(shape, MaskShape::Polygon) || points.len() < 2 {
        return "0".to_string();
    }
    // Size (half-width/height or radii) + the STATIC centre, in pixels.
    let (sa, sb, scx, scy) = match shape {
        MaskShape::Rect => (
            (points[1][0] - points[0][0]).abs() / 2.0 * wf,
            (points[1][1] - points[0][1]).abs() / 2.0 * hf,
            (points[0][0] + points[1][0]) / 2.0 * wf,
            (points[0][1] + points[1][1]) / 2.0 * hf,
        ),
        MaskShape::Ellipse => (
            (points[1][0] * wf).max(1.0),
            (points[1][1] * hf).max(1.0),
            points[0][0] * wf,
            points[0][1] * hf,
        ),
        MaskShape::Polygon => unreachable!(),
    };
    // Centre: time-varying (track) or constant (static).
    let (cx, cy) = match track {
        Some(tr) if tr.len() >= 2 => {
            let cxp: Vec<(f64, f64)> = tr
                .iter()
                .map(|p| (p.t_ms as f64 / 1000.0, p.cx * wf))
                .collect();
            let cyp: Vec<(f64, f64)> = tr
                .iter()
                .map(|p| (p.t_ms as f64 / 1000.0, p.cy * hf))
                .collect();
            (
                kf_expr(&cxp, "T", cut_core::KfInterp::Linear),
                kf_expr(&cyp, "T", cut_core::KfInterp::Linear),
            )
        }
        _ => (fnum(scx), fnum(scy)),
    };
    match shape {
        MaskShape::Rect => format!(
            "if(between(X,({cx})-{a},({cx})+{a})*between(Y,({cy})-{b},({cy})+{b}),255,0)",
            a = fnum(sa),
            b = fnum(sb)
        ),
        // ffmpeg's expression evaluator has NO `<=` operator — use `lte(a,b)`.
        MaskShape::Ellipse => format!(
            "if(lte(pow((X-({cx}))/{a},2)+pow((Y-({cy}))/{b},2),1),255,0)",
            a = fnum(sa),
            b = fnum(sb)
        ),
        MaskShape::Polygon => "0".to_string(),
    }
}

/// The PROCEDURAL moving-alpha filter for a TRACKED mask (`edit.redact{track}` /
/// `{faces}`): a `geq` whose luma is the UNION (`max`) of every region's indicator —
/// the primary region + each extra, each tracked or static. So N moving faces (or
/// a moving face + a static plate) paint one alpha plane in a single pass. Optional
/// feather = a trailing `gblur`; `invert` = a `negate`. Output is a gray alpha plane
/// the caller `alphamerge`s. A single tracked region reduces to the old expression
/// (byte-identical replay).
fn tracked_alpha_geq(m: &cut_core::ClipMask, w: u32, h: u32) -> String {
    let (wf, hf) = (w as f64, h as f64);
    let mut inds = vec![region_indicator(
        m.shape,
        &m.points,
        m.track.as_deref(),
        wf,
        hf,
    )];
    for r in &m.regions {
        inds.push(region_indicator(
            r.shape,
            &r.points,
            r.track.as_deref(),
            wf,
            hf,
        ));
    }
    // luma = 255 if in ANY region → nested binary max (single region = no wrapper).
    let lum = inds
        .into_iter()
        .reduce(|a, b| format!("max({a},{b})"))
        .unwrap_or_else(|| "0".to_string());
    let mut chain = format!("geq=lum='{lum}':cb=128:cr=128");
    let feather_px = m.feather * hf;
    if feather_px > 0.1 {
        chain.push_str(&format!(",gblur=sigma={}", fnum(feather_px)));
    }
    if m.invert {
        chain.push_str(",negate");
    }
    chain.push_str(",format=gray");
    chain
}

/// The masked-region COMPOSITE block for a TRACKED mask — the geq analog of
/// [`mask_block`]. Splits the conformed clip THREE ways (base / effected / the geq
/// alpha source), paints the moving alpha procedurally, `alphamerge`s the effect
/// into it, and `overlay`s it back (still `shortest=1`, still range-gated). No PNG
/// input is needed, and geq runs on a finite split → no `-loop 1` hang is possible.
fn mask_block_tracked(
    mask: &cut_core::ClipMask,
    in_label: &str,
    out_label: &str,
    uniq: &str,
    w: u32,
    h: u32,
    vfade: &str,
) -> String {
    let fx = mask_effect_filter(mask);
    let alpha = tracked_alpha_geq(mask, w, h);
    let enable = match mask.range_ms {
        Some([a, b]) => format!(
            ":enable='between(t,{},{})'",
            fnum(a as f64 / 1000.0),
            fnum(b as f64 / 1000.0)
        ),
        None => String::new(),
    };
    format!(
        "[{in_label}]split=3[mb{uniq}][mf{uniq}][mg{uniq}];\n\
         [mf{uniq}]{fx}[mx{uniq}];\n\
         [mg{uniq}]{alpha}[mm{uniq}];\n\
         [mx{uniq}][mm{uniq}]alphamerge[ma{uniq}];\n\
         [mb{uniq}][ma{uniq}]overlay=shortest=1{enable},format=yuv420p{vfade}[{out_label}];"
    )
}

/// The POWER-WINDOW composite block (edit.grade_window) — a REGION-scoped grade on the
/// running per-clip frame `in_label`, producing `out_label`. The geometric analog of
/// [`adjustment_block`]: it reuses the proven mask split→effect→overlay recipe but (1) the
/// "effect" is the window's [`grade_filter`] (run on a full-frame split copy) and (2) the
/// region is scoped by the baked shape ALPHA (`window_idx`, the parallel grade-window PNG
/// input) via `alphamerge`+`overlay` — NOT a time gate. Inside the shape the graded copy
/// shows; outside, the untouched base shows through. `,negate` on the alpha when the
/// window is inverted (grade the surround). `overlay=shortest=1` is load-bearing (the PNG
/// is a `-loop 1` infinite still — without it the graded copy never EOFs and the render
/// hangs). `vfade` is applied ONLY by the FINAL composite block of the clip. A grade that
/// collapses to "" (identity) is refused at verb time, so the block is never a no-op.
#[allow(clippy::too_many_arguments)]
fn grade_window_block(
    gw: &cut_core::GradeWindow,
    window_idx: usize,
    in_label: &str,
    out_label: &str,
    uniq: &str,
    w: u32,
    h: u32,
    vfade: &str,
) -> String {
    let neg = if gw.window.invert { ",negate" } else { "" };
    // The grade as the EXACT per-clip filter chain (leading comma). `null` heads the
    // branch so the leading-comma chain appends to a valid filter; `format=yuv420p` pins
    // the format so alphamerge + the overlay back onto the yuv420p base negotiate cleanly.
    let grade = grade_filter(Some(&gw.grade));
    format!(
        "[{in_label}]split=2[gb{uniq}][gf{uniq}];\n\
         [gf{uniq}]null{grade},format=yuv420p[gx{uniq}];\n\
         [{window_idx}:v]scale={w}:{h}:flags=bilinear,format=gray{neg}[gm{uniq}];\n\
         [gx{uniq}][gm{uniq}]alphamerge[ga{uniq}];\n\
         [gb{uniq}][ga{uniq}]overlay=shortest=1,format=yuv420p{vfade}[{out_label}];"
    )
}

/// The ADJUSTMENT-LAYER composite block (edit.adjustment) — a TIME-GATED non-
/// destructive grade/effect pass on the running composite `in_label`, producing
/// `out_label`. Mirrors the proven [`mask_block`] split→effect→gated-overlay recipe,
/// but gates the WHOLE grade+effect chain with a SINGLE `enable=` on the overlay
/// (not per filter): split the composite into a base copy + a graded copy, run the
/// SAME per-clip grade + effect filters on the graded copy, then `overlay` it back
/// gated to `between(t, start, end)`. Out of the span the overlay is disabled → the
/// untouched base shows through; inside, the full-frame opaque graded copy replaces
/// it → the layer affects everything beneath it ONLY within its span. Reusing
/// [`grade_filter`] + [`effect_filter`] keeps the look identical to a per-clip grade
/// and means any number of effect sub-filters are gated by the one overlay enable
/// (no per-filter timeline-support assumption, no comma-splitting of filter args).
/// `range_ms` is composition-local (window-rebased by `Edl::window`), matching the
/// composite's `t` timebase. `uniq` makes every intermediate label distinct.
fn adjustment_block(
    adj: &cut_core::EdlAdjustment,
    in_label: &str,
    out_label: &str,
    uniq: &str,
) -> String {
    // The grade + look effects, as the EXACT per-clip filter chains (leading comma).
    // overlay=false skips chroma-key (an adjustment has no single layer below to key;
    // the verb also refuses it). `null` heads the branch so the leading-comma chain
    // appends to a valid filter; `format=yuv420p` pins the format so the overlay back
    // onto the yuv420p base negotiates cleanly + deterministically.
    let grade = grade_filter(adj.grade.as_ref());
    let effects = effect_filter(&adj.effects, false);
    let [a, b] = adj.range_ms;
    let enable = format!(
        ":enable='between(t,{},{})'",
        fnum(a as f64 / 1000.0),
        fnum(b as f64 / 1000.0)
    );
    format!(
        "[{in_label}]split=2[jb{uniq}][jf{uniq}];\n\
         [jf{uniq}]null{grade}{effects},format=yuv420p[jg{uniq}];\n\
         [jb{uniq}][jg{uniq}]overlay=eof_action=pass{enable}[{out_label}];"
    )
}

/// ffmpeg AUDIO filter snippet (comma-PREFIXED) for a clip's AUDIO effects — the
/// audio-chain analog of [`effect_filter`]. Currently `denoise` → `afftdn`
/// (adaptive FFT denoiser): `amount` 0..1 maps to its noise-reduction in dB
/// (nr 0.01..30). Visual effects are skipped. Returns "" for none.
fn audio_effect_filter(effects: &[cut_core::ClipEffect]) -> String {
    use cut_core::ClipEffect as E;
    let mut s = String::new();
    for e in effects {
        match e {
            E::Denoise { amount } => {
                let nr = (amount.clamp(0.0, 1.0) * 30.0).clamp(0.01, 97.0);
                s.push_str(&format!(",afftdn=nr={}", fnum(nr)));
            }
            E::Compressor { amount } => {
                // amount 0..1 → ratio 1..8 (0 ≈ off) with modest auto makeup gain.
                let a = amount.clamp(0.0, 1.0);
                let ratio = 1.0 + a * 7.0;
                let makeup = 1.0 + a * 2.0;
                s.push_str(&format!(
                    ",acompressor=threshold=-20dB:ratio={}:attack=20:release=250:makeup={}",
                    fnum(ratio),
                    fnum(makeup)
                ));
            }
            E::Gate { amount } => {
                // amount 0..1 → threshold (0.002..0.06 linear ≈ -54..-24 dBFS) +
                // ratio (2..9). Higher amount = gates harder / at a higher floor, so
                // more between-phrase room tone drops out. Fixed fast attack / medium
                // release suits speech (doesn't clip word onsets, doesn't chatter).
                // Measured: threshold=0.05/ratio=9 passed a -21 dB tone, dropped a
                // -61 dB tail to -85 dB in the agate regression fixture.
                let a = amount.clamp(0.0, 1.0);
                let threshold = 0.002 + a * 0.058;
                let ratio = 2.0 + a * 7.0;
                s.push_str(&format!(
                    ",agate=threshold={}:ratio={}:attack=10:release=100",
                    fnum(threshold),
                    fnum(ratio)
                ));
            }
            _ => {}
        }
    }
    s
}

/// ffmpeg AUDIO filter snippet (comma-PREFIXED) for a clip's parametric EQ
/// (`edit.eq`) — the audio analog of the grade stage. Emits, in order: a high-pass
/// (low-cut, removes rumble) → each peaking band (constant-Q `equalizer` bell:
/// `t=q:w={Q}:g={dB}`) → a low-pass (high-cut, tames hiss). Returns "" when there
/// is no EQ (None) so an un-EQ'd clip's audio graph is byte-identical. Runs on the
/// conformed audio AFTER denoise/compressor and BEFORE gain/fade. Measured: a
/// highpass=120 + equalizer=1000:g=6 + lowpass=6000 chain moved the 80Hz/1k/8k
/// bands by −6.5/+6.0/−6.6 dB in the EQ regression fixture.
fn eq_filter(eq: Option<&cut_core::ClipEq>) -> String {
    let Some(eq) = eq else {
        return String::new();
    };
    let mut s = String::new();
    if let Some(hp) = eq.high_pass_hz {
        s.push_str(&format!(",highpass=f={}", fnum(hp as f64)));
    }
    for b in &eq.bands {
        s.push_str(&format!(
            ",equalizer=f={}:t=q:w={}:g={}",
            fnum(b.freq_hz as f64),
            fnum(b.q as f64),
            fnum(b.gain_db as f64),
        ));
    }
    if let Some(lp) = eq.low_pass_hz {
        s.push_str(&format!(",lowpass=f={}", fnum(lp as f64)));
    }
    s
}

/// Deterministic cache path for a clip's vidstab motion file (`.trf`), keyed by the
/// asset CONTENT hash + source range — so the detect is skipped across renders and
/// the path is reproducible from both `build_graph` (which only REFERENCES it) and
/// `prepare_stabilization` (which GENERATES it). Lives under `<project>/stab/`.
fn stab_trf_path(
    project_dir: &Path,
    asset_hash: &str,
    src_in_ms: u64,
    src_out_ms: u64,
) -> std::path::PathBuf {
    let safe: String = asset_hash
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    // Strip the Windows `\\?\` verbatim prefix so the path ffmpeg's vidstab filter
    // sees is plain (`C:\…`) — the prefix breaks the filtergraph. No-op on POSIX.
    strip_verbatim_prefix(project_dir)
        .join("stab")
        .join(format!("{safe}_{src_in_ms}_{src_out_ms}.trf"))
}

/// ffmpeg VIDEO filter snippet (comma-PREFIXED) applying a clip's stabilization
/// (`edit.stabilize`) — `vidstabtransform` reading the per-clip `.trf` the detect
/// pre-pass cached. Emitted right after the PTS reset (the same source frames the
/// detect saw), before reverse/crop/conform. Returns "" when the clip isn't
/// stabilized, is a frozen single frame (nothing to stabilize), OR the `.trf` is
/// not cached yet — a fast preview/scrub that skipped `prepare_stabilization`
/// degrades to UNstabilized instead of failing the render.
fn stab_filter(
    stabilize: Option<&cut_core::ClipStabilize>,
    frozen: bool,
    asset_hash: &str,
    src_in_ms: u64,
    src_out_ms: u64,
    project_dir: &Path,
) -> String {
    let Some(st) = stabilize else {
        return String::new();
    };
    if frozen {
        return String::new();
    }
    let trf = stab_trf_path(project_dir, asset_hash, src_in_ms, src_out_ms);
    if !trf.exists() {
        return String::new();
    }
    let smoothing = st.smoothing.clamp(1.0, 100.0).round() as u64;
    let (_, transform_fileformat) = crate::ffmpeg::vidstab_fileformat_support();
    stab_transform_filter(&trf, smoothing, transform_fileformat)
}

fn stab_transform_filter(trf: &Path, smoothing: u64, transform_fileformat: bool) -> String {
    let fileformat = if transform_fileformat {
        ":fileformat=ascii"
    } else {
        ""
    };
    format!(
        ",vidstabtransform=input={}:smoothing={}:crop=black{}",
        escape_filter_path(trf),
        smoothing,
        fileformat,
    )
}

fn stab_detect_filter(
    src_in_ms: u64,
    src_out_ms: u64,
    trf: &Path,
    detect_fileformat: bool,
) -> String {
    let fileformat = if detect_fileformat {
        ":fileformat=ascii"
    } else {
        ""
    };
    format!(
        "trim=start={}:end={},setpts=PTS-STARTPTS,\
         vidstabdetect=shakiness=8:accuracy=15{}:result={}",
        secs(src_in_ms),
        secs(src_out_ms),
        fileformat,
        escape_filter_path(trf),
    )
}

/// Run the `vidstabdetect` analysis PRE-PASS for every stabilized VIDEO segment that
/// has no cached `.trf` yet (idempotent — existing files are skipped). MUST run
/// before `build_graph` on any path that wants stabilization APPLIED (render_final,
/// render.frame{compose}, render.range); fast preview/scrub skip it and render
/// unstabilized. Detect operates on the SAME trimmed source range the transform
/// will, so the per-frame motion entries line up. Deterministic from source content.
pub fn prepare_stabilization(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
) -> Result<(), CutError> {
    use std::collections::HashSet;
    let mut done: HashSet<std::path::PathBuf> = HashSet::new();
    for seg in &edl.segments {
        if seg.stabilize.is_none() || seg.freeze.is_some() || seg.track_kind != TrackKind::Video {
            continue;
        }
        let (Some(asset_id), Some(src_in), Some(src_out)) =
            (&seg.asset, seg.src_in_ms, seg.src_out_ms)
        else {
            continue;
        };
        let Some(asset) = project.assets.get(asset_id) else {
            continue;
        };
        let trf = stab_trf_path(project_dir, &asset.hash, src_in, src_out);
        if trf.exists() || !done.insert(trf.clone()) {
            continue;
        }
        if let Some(parent) = trf.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let (detect_fileformat, _) = crate::ffmpeg::vidstab_fileformat_support();
        let vf = stab_detect_filter(src_in, src_out, &trf, detect_fileformat);
        let mut input_path = PathBuf::from(&asset.path);
        if input_path.is_relative() {
            input_path = project_dir.join(input_path);
        }
        let input_path = strip_verbatim_prefix(&input_path);
        crate::ffmpeg::run_ffmpeg(&[
            "-i".to_string(),
            input_path.display().to_string(),
            "-vf".to_string(),
            vf,
            "-f".to_string(),
            "null".to_string(),
            "-".to_string(),
        ])?;
    }
    Ok(())
}

/// Encode a [`crate::title::TitleSpec`] to a TRANSPARENT overlay video (qtrle
/// `.mov` — keeps a real alpha channel) at the spec's geometry + fps. Authored
/// at the PROJECT geometry so the overlay conform is a 1:1 no-op and the alpha
/// survives `format=yuva420p` — the existing overlay pipeline then composites it
/// like any video clip, so motion titles need ZERO renderer changes. Frames
/// come from the pure resvg `title::render_frame`; we stream them as raw RGBA
/// through one temp file into ffmpeg (one input, no PNG round-trip).
pub fn encode_title_overlay(spec: &crate::title::TitleSpec, out: &Path) -> Result<(), CutError> {
    use std::io::Write as _;
    let n = crate::title::frame_count(spec);
    // Backstop against a pathological spec (the verb layer caps duration at 10 min,
    // but ANY caller that builds a TitleSpec directly is bounded here too): each
    // frame writes W·H·4 raw bytes to the temp file, so an unbounded frame count
    // would fill the disk + never return. 100k frames is far past any real overlay
    // (10 min @ 120 fps) and keeps the temp file finite.
    const MAX_OVERLAY_FRAMES: u32 = 100_000;
    if n > MAX_OVERLAY_FRAMES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "title/shape overlay has too many frames",
            format!("{n} frames (cap {MAX_OVERLAY_FRAMES}); shorten the span or lower the fps"),
        ));
    }
    let raw = tempfile::Builder::new()
        .suffix(".rgba")
        .tempfile()
        .map_err(|e| CutError::new(error_codes::IO, "title temp file", e.to_string()))?;
    {
        let mut w = std::io::BufWriter::new(raw.as_file());
        for i in 0..n {
            let frame = crate::title::render_frame(spec, i).map_err(|e| {
                CutError::new(
                    error_codes::FFMPEG,
                    "title frame render failed",
                    e.to_string(),
                )
            })?;
            w.write_all(&frame)
                .map_err(|e| CutError::new(error_codes::IO, "title frame write", e.to_string()))?;
        }
        w.flush()
            .map_err(|e| CutError::new(error_codes::IO, "title flush", e.to_string()))?;
    }
    let size = format!("{}x{}", spec.width, spec.height);
    let fps = spec.fps.to_string();
    let inpath = raw.path().to_string_lossy().into_owned();
    let outpath = out.to_string_lossy().into_owned();
    let args = vec![
        "-f".to_string(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "rgba".into(),
        "-s".into(),
        size,
        "-r".into(),
        fps,
        "-i".into(),
        inpath,
        "-c:v".into(),
        "qtrle".into(),
        outpath,
    ];
    run_ffmpeg(&args)?;
    Ok(())
}

/// Format a speed/tempo factor cleanly for filter syntax: integers without a
/// trailing ".0" ("2"), fractions to ≤6 sig-fig with trailing zeros trimmed
/// ("0.5", "1.75"). Deterministic for a given f64 (the EDL stores the exact
/// factor, so the same project always emits the same string).
fn fnum(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// The `setpts` expression for a video segment at clip `speed` (edit.speed).
/// At 1.0 the plain reset (byte-identical pre-speed renders); otherwise divide
/// the reset PTS by speed — 2× ⇒ `/2` plays twice as fast, 0.5× ⇒ `/0.5` is
/// slow-motion. Frame handling is ffmpeg's default (drop on speed-up, hold on
/// slow-down); smooth interpolated slow-mo (minterpolate) is a deliberate v2.
fn video_setpts(speed: f64) -> String {
    if speed == 1.0 {
        "setpts=PTS-STARTPTS".into()
    } else {
        format!("setpts=(PTS-STARTPTS)/{}", fnum(speed))
    }
}

/// The trim/setpts/tpad pieces for ONE video segment, accounting for edit.freeze.
/// Returns `(trim_start_s, trim_end_s, setpts, tpad_suffix)`:
/// - FROZEN clip → a single source frame at Tf (offset `at_ms` into
///   [src_in,src_out), clamped to the last frame) + `tpad=stop_mode=clone` cloning
///   it to the slot's frame count, with a PLAIN setpts (a held frame has no speed).
/// - normal clip → `(secs(src_in), secs(src_out), video_setpts(speed), "")`.
/// The caller emits `trim=start={s}:end={e},{setpts}{tpad}...`, so freeze stays a
/// pure source-space splice (crop/conform/grade/effects follow as usual).
fn freeze_chain(
    freeze: Option<&cut_core::ClipFreeze>,
    speed: f64,
    src_in: u64,
    src_out: u64,
    seg_dur: u64,
    fps: f64,
) -> (String, String, String, String) {
    match freeze {
        Some(fz) => {
            let one = 1.0 / fps; // one frame, seconds
            let lo = src_in as f64 / 1000.0;
            let hi = (src_out as f64 / 1000.0 - one).max(lo);
            let tf = (lo + fz.at_ms as f64 / 1000.0).clamp(lo, hi);
            // Frame count of the slot (the held frame fills the existing duration).
            let nf = ((seg_dur as f64 / 1000.0) * fps).round().max(1.0) as i64;
            (
                format!("{tf:.3}"),
                format!("{:.3}", tf + one),
                "setpts=PTS-STARTPTS".to_string(),
                format!(",tpad=stop_mode=clone:stop={}", nf - 1),
            )
        }
        None => (
            secs(src_in),
            secs(src_out),
            video_setpts(speed),
            String::new(),
        ),
    }
}

/// Ken Burns pan/zoom filter (edit.animate) for ONE video segment, injected AFTER
/// the conform stage. Returns a leading-comma block or "" when not animated.
///
/// The emitted chain keeps the output frame count exact while applying visible
/// motion across the clip:
///   `,fps={fps},zoompan=z='…':d=1:x='…':y='…':s=WxH:fps={fps},setpts=N/(fps*TB)`
/// - the leading `fps` puts zoompan's input at the PROJECT rate so `on` (output
///   frame index) runs 0..nf-1 matching the slot's frame count;
/// - z/x/y are `on`-indexed LINEAR interpolations from `from` to `to` (NOT the
///   `zoom+step` form — `zoom` resets to 1.0 each input frame on video, so it
///   never accumulates; `on` is the cumulative counter that does);
/// - the trailing `setpts=N/(fps*TB)` REBUILDS clean integer PTS — this is the fix
///   for the 512× frame explosion (zoompan emits duration:0 frames on tbn=15360; a
///   later `fps` would otherwise re-expand each by 15360/fps). With the rebuild the
///   chain survives the renderer's own trailing `fps,format` 1:1.
/// Commas inside the `'…'` expressions are filtergraph-quoted.
/// CPU-only → the clip opts out of the GPU fast-track gate.
fn animate_filter(
    anim: Option<&cut_core::ClipAnimation>,
    w: u32,
    h: u32,
    fps: f64,
    seg_dur_ms: u64,
) -> String {
    let Some(a) = anim else {
        return String::new();
    };
    let nf = ((seg_dur_ms as f64 / 1000.0) * fps).round().max(1.0) as i64;
    let nm1 = (nf - 1).max(1); // denominator; nf=1 → constant from-state
    let z0 = a.from.zoom.clamp(1.0, 10.0);
    let z1 = a.to.zoom.clamp(1.0, 10.0);
    let dz = z1 - z0;
    let cx0 = a.from.x.clamp(0.0, 1.0);
    let cy0 = a.from.y.clamp(0.0, 1.0);
    let dcx = a.to.x.clamp(0.0, 1.0) - cx0;
    let dcy = a.to.y.clamp(0.0, 1.0) - cy0;
    let fps_s = fps_str(fps);
    // z >= 1 (zoompan requires it); x/y are the zoom window's top-left, clamped
    // on-frame (centre = focal·dim − halfwindow). Plain commas inside '…' parse.
    let z = format!("max(1,{z0:.5}+{dz:.5}*on/{nm1})");
    let x = format!("max(0,min(iw-iw/zoom,iw*({cx0:.5}+{dcx:.5}*on/{nm1})-iw/zoom/2))");
    let y = format!("max(0,min(ih-ih/zoom,ih*({cy0:.5}+{dcy:.5}*on/{nm1})-ih/zoom/2))");
    format!(
        ",fps={fps_s},zoompan=z='{z}':d=1:x='{x}':y='{y}':s={w}x{h}:fps={fps_s},setpts=N/({fps_s}*TB)"
    )
}

/// Find a keyframe param's control points as (t_seconds, value) + the hold flag,
/// or None when the clip has no keyframe for that param (edit.keyframe).
fn kf_points(
    keyframes: &[cut_core::Keyframe],
    param: cut_core::KfParam,
) -> Option<(Vec<(f64, f64)>, cut_core::KfInterp)> {
    keyframes.iter().find(|k| k.param == param).map(|k| {
        let pts: Vec<(f64, f64)> = k
            .points
            .iter()
            .map(|p| (p.t_ms as f64 / 1000.0, p.value))
            .collect();
        (pts, k.interp)
    })
}

/// An ffmpeg eval expression for the EASED fraction, given a raw-fraction expression
/// `frac` (a string in the time var, expected in `[0,1]`). The output expression's
/// VALUE is `KfInterp::sample(frac)` — this is the exact ffmpeg mirror of the pure-
/// Rust reference in `cut_core` (the live render proof checks they agree). `linear`/
/// `hold` never reach here (the caller emits the legacy form). The fraction is stored
/// once in eval slot 0 (`st(0, clip(frac,0,1))`) so an expensive `frac` sub-expression
/// is evaluated a single time; `ld(0)` reads it back. The `st(…)*0+…` idiom relies on
/// ffmpeg's left-to-right evaluation of `+` (the store runs before the body reads it),
/// which holds whether or not `if(…)` evaluates branches lazily — each branch is a
/// self-contained sub-expression. bounce reuses slot 1 for its inner argument.
fn ease_frac_expr(interp: cut_core::KfInterp, frac: &str) -> String {
    use cut_core::KfInterp as K;
    use std::f64::consts::PI;
    let f = "ld(0)";
    // Penner constants — IDENTICAL to cut_core::KfInterp::sample (kept in lockstep).
    let c1 = 1.701_58_f64;
    let c3 = c1 + 1.0;
    let c2 = c1 * 1.525;
    let c4 = (2.0 * PI) / 3.0;
    let c5 = (2.0 * PI) / 4.5;
    let n1 = 7.5625_f64;
    let d1 = 2.75_f64;
    // out_bounce(x) with x stored in slot 1 so it is evaluated once across the 4 hops.
    let bounce_out = |xexpr: &str| -> String {
        let x = "ld(1)";
        format!(
            "(st(1,{xexpr})*0+if(lt({x},{a}),{n1}*{x}*{x},if(lt({x},{b}),{n1}*pow({x}-{e},2)+0.75,if(lt({x},{c}),{n1}*pow({x}-{g},2)+0.9375,{n1}*pow({x}-{h},2)+0.984375))))",
            a = 1.0 / d1,
            b = 2.0 / d1,
            c = 2.5 / d1,
            e = 1.5 / d1,
            g = 2.25 / d1,
            h = 2.625 / d1,
        )
    };
    let body = match interp {
        K::Linear | K::Hold => return frac.to_string(), // never called; defensive
        K::EaseInQuad => format!("{f}*{f}"),
        K::EaseOutQuad => format!("1-(1-{f})*(1-{f})"),
        K::EaseInOutQuad => {
            format!("if(lt({f},0.5),2*{f}*{f},1-pow(-2*{f}+2,2)/2)")
        }
        K::EaseInCubic => format!("{f}*{f}*{f}"),
        K::EaseOutCubic => format!("1-pow(1-{f},3)"),
        K::EaseInOutCubic => {
            format!("if(lt({f},0.5),4*{f}*{f}*{f},1-pow(-2*{f}+2,3)/2)")
        }
        K::EaseInExpo => format!("if(lte({f},0),0,pow(2,10*{f}-10))"),
        K::EaseOutExpo => format!("if(gte({f},1),1,1-pow(2,-10*{f}))"),
        K::EaseInOutExpo => format!(
            "if(lte({f},0),0,if(gte({f},1),1,if(lt({f},0.5),pow(2,20*{f}-10)/2,(2-pow(2,-20*{f}+10))/2)))"
        ),
        K::EaseInBack => format!("{c3}*{f}*{f}*{f}-{c1}*{f}*{f}"),
        K::EaseOutBack => format!("1+{c3}*pow({f}-1,3)+{c1}*pow({f}-1,2)"),
        K::EaseInOutBack => format!(
            "if(lt({f},0.5),(pow(2*{f},2)*(({c2}+1)*2*{f}-{c2}))/2,(pow(2*{f}-2,2)*(({c2}+1)*(2*{f}-2)+{c2})+2)/2)"
        ),
        K::EaseInElastic => format!(
            "if(lte({f},0),0,if(gte({f},1),1,-pow(2,10*{f}-10)*sin((10*{f}-10.75)*{c4})))"
        ),
        K::EaseOutElastic => format!(
            "if(lte({f},0),0,if(gte({f},1),1,pow(2,-10*{f})*sin((10*{f}-0.75)*{c4})+1))"
        ),
        K::EaseInOutElastic => format!(
            "if(lte({f},0),0,if(gte({f},1),1,if(lt({f},0.5),-(pow(2,20*{f}-10)*sin((20*{f}-11.125)*{c5}))/2,(pow(2,-20*{f}+10)*sin((20*{f}-11.125)*{c5}))/2+1)))"
        ),
        K::EaseOutBounce => bounce_out(f),
        K::EaseInBounce => format!("1-{}", bounce_out(&format!("1-{f}"))),
        K::EaseInOutBounce => format!(
            "if(lt({f},0.5),(1-{lo})/2,(1+{hi})/2)",
            lo = bounce_out(&format!("1-2*{f}")),
            hi = bounce_out(&format!("2*{f}-1")),
        ),
    };
    format!("(st(0,clip(({frac}),0,1))*0+{body})")
}

/// Compile keyframe control points into a piecewise ffmpeg time-EXPRESSION in the
/// time variable `var` (`t` for eq/volume/scale, `T` for geq), seconds (or `on`
/// frame-index for zoompan). LINEAR lerps between adjacent points; `hold` steps;
/// `ease_*` reshapes the inter-keyframe fraction through a Penner curve (see
/// [`ease_frac_expr`]); all CLAMP to the first/last value outside the point range.
/// Built inner-to-outer as nested `if(lt(var,t),…,…)`. Plain commas inside the
/// expression parse fine when the whole thing is wrapped in the filter arg's single
/// quotes (proven). LINEAR/HOLD emit the EXACT pre-easing string (byte-identical
/// replay for every project predating the easing channel).
fn kf_expr(points: &[(f64, f64)], var: &str, interp: cut_core::KfInterp) -> String {
    let hold = matches!(interp, cut_core::KfInterp::Hold);
    let linear = matches!(interp, cut_core::KfInterp::Linear);
    match points.len() {
        0 => "0".to_string(),
        1 => fnum(points[0].1),
        n => {
            // var >= last t → the last value (the innermost else).
            let mut expr = fnum(points[n - 1].1);
            for i in (0..n - 1).rev() {
                let (t0, v0) = points[i];
                let (t1, v1) = points[i + 1];
                let seg = if hold || (t1 - t0).abs() < 1e-9 {
                    fnum(v0)
                } else if linear {
                    // v0 + (v1-v0)*(var-t0)/(t1-t0) — the EXACT legacy string.
                    format!(
                        "({}+({})*({}-{})/{})",
                        fnum(v0),
                        fnum(v1 - v0),
                        var,
                        fnum(t0),
                        fnum(t1 - t0)
                    )
                } else {
                    // v0 + (v1-v0)*ease( (var-t0)/(t1-t0) )
                    let frac = format!("(({}-{})/{})", var, fnum(t0), fnum(t1 - t0));
                    format!(
                        "({}+({})*{})",
                        fnum(v0),
                        fnum(v1 - v0),
                        ease_frac_expr(interp, &frac)
                    )
                };
                expr = format!("if(lt({},{}),{},{})", var, fnum(t1), seg, expr);
            }
            // Clamp before the first point to v0.
            format!(
                "if(lt({},{}),{},{})",
                var,
                fnum(points[0].0),
                fnum(points[0].1),
                expr
            )
        }
    }
}

/// The animated-ZOOM filter for a scale-keyframed clip (edit.keyframe param=scale),
/// or `None` when the clip has no scale keyframes. Lowers to the SAME proven
/// `zoompan` chain as [`animate_filter`] (the multi-point, eased generalization of
/// `edit.animate`'s 2-state Ken Burns): a centred zoom whose `z` is the keyframe
/// expression in the `on` (output-frame) index, clamped to the zoompan-legal
/// `[1,10]`. Injected at the SAME post-conform slot as `animate_filter`; a clip can
/// never have both (validated mutually-exclusive in `edit::keyframe`). CPU-only — a
/// scale-keyframed clip is already forced onto the software path (the keyframes gate).
fn scale_kf_zoompan(
    keyframes: &[cut_core::Keyframe],
    w: u32,
    h: u32,
    fps: f64,
    seg_dur_ms: u64,
) -> Option<String> {
    let (pts_s, interp) = kf_points(keyframes, cut_core::KfParam::Scale)?;
    if pts_s.is_empty() {
        return None;
    }
    // Convert the (seconds,value) points to (output-frame-index, value) so the
    // expression is in `on` (0..nf-1), matching animate_filter's frame-indexed form.
    let pts_f: Vec<(f64, f64)> = pts_s
        .iter()
        .map(|(t, v)| ((t * fps).round(), v.clamp(1.0, 10.0)))
        .collect();
    let fps_s = fps_str(fps);
    let zexpr = kf_expr(&pts_f, "on", interp);
    // z clamped ≥1 (zoompan requires it) and ≤10 (the edit.animate ceiling). x/y are
    // the zoom window's top-left for a CENTRED zoom (focal = frame centre); `zoom` is
    // zoompan's evaluated current z. (Focal-point zoom — driven by pos_x/pos_y — is a
    // record-integration refinement; v1 centres, which is right for most screen zooms.)
    let z = format!("max(1,min(10,{zexpr}))");
    let x = "iw/2-(iw/zoom)/2";
    let y = "ih/2-(ih/zoom)/2";
    let _ = seg_dur_ms; // duration is encoded in the points' frame indices already.
    Some(format!(
        ",fps={fps_s},zoompan=z='{z}':d=1:x='{x}':y='{y}':s={w}x{h}:fps={fps_s},setpts=N/({fps_s}*TB)"
    ))
}

/// The keyframed-OPACITY filter for an overlay segment (edit.keyframe param=opacity)
/// or "" when not keyframed. Sets the yuva alpha plane to the time-expression
/// (Y/Cb/Cr pass through), animating the overlay's transparency. Injected right
/// after `format=yuva420p` (so it runs on the conformed full frame, before the PiP
/// scale/pad — the pad border keeps its alpha=0). Var = `T` (geq time).
fn opacity_kf_filter(keyframes: &[cut_core::Keyframe]) -> String {
    match kf_points(keyframes, cut_core::KfParam::Opacity) {
        Some((pts, interp)) if !pts.is_empty() => format!(
            // `clip(…,0,1)` is load-bearing: an overshoot/undershoot easing (back/
            // elastic) can drive the eased alpha just past [0,1], and geq writes the
            // alpha plane WITHOUT clamping — a raw -1 wraps to 255 (a full-opacity
            // flash). Clamping keeps alpha a legal [0,1] (you cannot be more than
            // opaque). Linear/hold never leave [0,1] so this is a no-op for them.
            ",geq=lum='lum(X,Y)':cb='cb(X,Y)':cr='cr(X,Y)':a='255*clip(({}),0,1)'",
            kf_expr(&pts, "T", interp)
        ),
        _ => String::new(),
    }
}

/// The keyframed-VOLUME filter for an audio segment or "" when not keyframed.
/// The clamp is load-bearing: back/elastic easings can overshoot outside the
/// authored multiplier range; negative volume phase-inverts audio, and extreme
/// positive spikes are not useful automation.
fn volume_kf_filter(keyframes: &[cut_core::Keyframe]) -> String {
    match kf_points(keyframes, cut_core::KfParam::Volume) {
        Some((pts, interp)) if !pts.is_empty() => format!(
            ",volume='max(0,min(16,{}))':eval=frame",
            kf_expr(&pts, "t", interp)
        ),
        _ => String::new(),
    }
}

/// The audio MUTE-RANGE gate (edit.mute_range / transcript.mute_words) for a
/// segment, or "" when it carries none. One `volume` expression forcing 0 over
/// each SOURCE-time range's overlap with the segment's visible window, mapped
/// to POST-SPEED segment-local seconds (the same clock afade/keyframed volume
/// live on — the filter sits after atempo). Reverse mirrors the window: the
/// stream is reversed before atempo, so source offset `o` plays at
/// `(src_out - o)` from the segment start. Ranges wholly outside the window
/// emit nothing; an empty result keeps graphs byte-identical.
fn mute_gate_filter(seg: &cut_core::EdlSegment) -> String {
    let (Some(src_in), Some(src_out)) = (seg.src_in_ms, seg.src_out_ms) else {
        return String::new();
    };
    let mut gates: Vec<String> = Vec::new();
    for r in &seg.mute_ranges {
        let lo = r[0].max(src_in);
        let hi = r[1].min(src_out);
        if hi <= lo {
            continue; // no overlap with the visible source window
        }
        let (a_ms, b_ms) = if seg.reverse {
            (src_out - hi, src_out - lo)
        } else {
            (lo - src_in, hi - src_in)
        };
        let a = a_ms as f64 / seg.speed / 1000.0;
        let b = b_ms as f64 / seg.speed / 1000.0;
        gates.push(format!("between(t,{a:.6},{b:.6})"));
    }
    if gates.is_empty() {
        return String::new();
    }
    format!(",volume='if({},0,1)':eval=frame", gates.join("+"))
}

/// The audio speed filter chain for a segment at clip `speed`, terminated with
/// a trailing comma so it slots straight into the chain after
/// `asetpts=PTS-STARTPTS,` (empty string at unit speed → byte-identical).
///
/// - `preserve_pitch` (DEFAULT true): `atempo` — pitch-preserved time-stretch,
///   always present in stock ffmpeg (no librubberband dependency, so the render
///   never fails for lack of an optional filter). atempo is valid in [0.5, 2.0]
///   per instance; for the full 0.25–4.0 range it is sqrt-split into two stages
///   (√0.25 = 0.5 and √4 = 2.0 both land in range), which composes to the exact
///   factor. Sped-up speech stays human, not chipmunk.
/// - `preserve_pitch = false`: `asetrate`/`aresample` varispeed — the sample
///   rate is reinterpreted at `rate*speed` then resampled back, so PITCH FOLLOWS
///   SPEED (the classic tape/"varispeed" sound). Occasionally wanted as an effect.
fn audio_speed_filter(speed: f64, preserve_pitch: bool, rate: u32) -> String {
    if speed == 1.0 {
        return String::new();
    }
    if preserve_pitch {
        if (0.5..=2.0).contains(&speed) {
            format!("atempo={},", fnum(speed))
        } else {
            let r = fnum(speed.sqrt());
            format!("atempo={r},atempo={r},")
        }
    } else {
        let rs = (rate as f64 * speed).round() as u64;
        format!("asetrate={rs},aresample={rate},")
    }
}

/// True when this fade kind applies to video pixels.
fn fade_has_video(kind: cut_core::FadeKind) -> bool {
    matches!(kind, cut_core::FadeKind::Video | cut_core::FadeKind::Both)
}

/// True when this fade kind applies to audio samples.
fn fade_has_audio(kind: cut_core::FadeKind) -> bool {
    matches!(kind, cut_core::FadeKind::Audio | cut_core::FadeKind::Both)
}

/// Fade filter suffix for one segment chain under the `edit.fade` contract.
/// Times are SEGMENT-LOCAL (the chain has just reset PTS), durations clamped
/// to the segment length — a trim after the fade was set must degrade
/// gracefully, never emit an out-of-range filter. `video=true` emits `fade`
/// (with `:alpha=1` when `alpha` — overlay tracks fade their transparency
/// instead of dipping to black over the base), `video=false` emits `afade`.
/// LINEAR ramps only; crossfades between adjacent clips are v2.
fn fade_suffix(fade: Option<&cut_core::ClipFade>, seg_ms: u64, video: bool, alpha: bool) -> String {
    let Some(f) = fade else { return String::new() };
    let applies = if video {
        fade_has_video(f.kind)
    } else {
        fade_has_audio(f.kind)
    };
    if !applies || seg_ms == 0 {
        return String::new();
    }
    let name = if video { "fade" } else { "afade" };
    let a = if video && alpha { ":alpha=1" } else { "" };
    let mut s = String::new();
    let in_ms = f.in_ms.min(seg_ms);
    let out_ms = f.out_ms.min(seg_ms.saturating_sub(in_ms));
    if in_ms > 0 {
        write!(s, ",{name}=t=in:st=0:d={}{a}", secs(in_ms)).unwrap();
    }
    if out_ms > 0 {
        write!(
            s,
            ",{name}=t=out:st={}:d={}{a}",
            secs(seg_ms - out_ms),
            secs(out_ms)
        )
        .unwrap();
    }
    s
}

/// One built segment stream ready to be concatenated/crossfaded: its filter
/// label, its rendered duration (ms), and the crossfade-IN overlap (ms)
/// that dissolves it from the PREVIOUS segment on the same track (0 = hard
/// cut). Used by `fold_video`/`fold_audio` to chain segments with `xfade`/
/// `acrossfade` at flagged seams and plain `concat` everywhere else.
struct SegStream {
    label: String,
    dur_ms: u64,
    xfade_in_ms: u64,
    /// Crossfade transition style for this seam (None = "fade" dissolve). From the
    /// segment's `xfade_kind`; consumed by `fold_video` as `xfade=transition=`.
    xfade_kind: Option<String>,
}

/// Fold a track's video segment streams into ONE output label, dissolving at
/// every seam whose right segment carries `xfade_in_ms > 0` and
/// hard-cutting (pairwise `concat`) elsewhere. Returns the final label and the
/// REALIZED total duration (the sum of segment durations minus every applied
/// overlap — matches the EDL's crossfade-shortened duration).
///
/// WHY PAIRWISE: `xfade` is a two-input filter (transition over an `offset`
/// into the accumulator), so the chain is built left→right, threading the
/// running duration as the next xfade offset. `concat` of the whole list can't
/// express a mid-list dissolve, so we fold pairwise throughout — identical
/// output to a single `concat` when no seam crossfades (verified by tests).
/// `out_label` is the label the final stream is bound to.
///
/// TIMEBASE: `xfade` requires both input legs to share a timebase and fps, or
/// it refuses to configure ("First input link main
/// timebase (1/1000000) do not match the corresponding second input link xfade
/// timebase (1/60)"). The accumulator side is the trap: a `concat` (and even a
/// prior `xfade`) emits a MICROSECOND timebase (1/1000000), while a fresh
/// segment chain ends `fps={fps}` carrying the FRAME timebase (1/fps). At 30fps
/// both legs happened to negotiate to 1/1000000 and the bug hid; at 60fps the
/// frame leg stayed 1/60 and the mismatch surfaced — and because the failed
/// xfade poisons graph configuration, the WHOLE compose render died, not just
/// the seam. Fix per the documented ffmpeg recipe: normalise BOTH legs through
/// `settb=AVTB,fps={fps}` immediately before every `xfade` (a tiny no-op pad
/// chain `[leg]settb=AVTB,fps=..[legN]`), so the timebase is identical on both
/// sides regardless of project fps. Applied only on the xfade path — the hard-
/// cut `concat` branch is byte-identical to the older graph (replay invariant).
fn fold_video(f: &mut String, segs: &[SegStream], out_label: &str, fps: f64) -> u64 {
    debug_assert!(!segs.is_empty());
    let mut acc = segs[0].label.clone();
    let mut acc_dur = segs[0].dur_ms;
    let fps = fps_str(fps);
    for (i, s) in segs.iter().enumerate().skip(1) {
        let next = format!("{out_label}_x{i}");
        if s.xfade_in_ms > 0 {
            // Normalise BOTH legs to a common timebase + fps before xfade
            // (the crossfade-timebase contract). The accumulator (concat/xfade output) carries the
            // microsecond timebase; the segment chain carries the frame
            // timebase — settb=AVTB on each reconciles them so xfade can
            // configure its output pad at any project fps (30/60/arbitrary).
            let acc_n = format!("{out_label}_xa{i}");
            let seg_n = format!("{out_label}_xb{i}");
            writeln!(f, "[{acc}]settb=AVTB,fps={fps}[{acc_n}];").unwrap();
            writeln!(f, "[{l}]settb=AVTB,fps={fps}[{seg_n}];", l = s.label).unwrap();
            // Dissolve: offset = where in the accumulator the transition starts
            // (its end minus the overlap). Total length = acc + seg - overlap.
            let offset = acc_dur.saturating_sub(s.xfade_in_ms);
            // Transition style (edit.crossfade `transition`): the stored ffmpeg
            // xfade name, or "fade" (classic dissolve) when unset — byte-identical
            // to the pre-transitions graph for every existing crossfade.
            let kind = s.xfade_kind.as_deref().unwrap_or("fade");
            writeln!(
                f,
                "[{acc_n}][{seg_n}]xfade=transition={kind}:duration={d}:offset={o}[{next}];",
                d = secs(s.xfade_in_ms),
                o = secs(offset),
            )
            .unwrap();
            acc_dur = acc_dur + s.dur_ms - s.xfade_in_ms;
        } else {
            // Hard cut: pairwise concat keeps the chain uniform with xfade.
            writeln!(f, "[{acc}][{l}]concat=n=2:v=1:a=0[{next}];", l = s.label).unwrap();
            acc_dur += s.dur_ms;
        }
        acc = next;
    }
    // Bind the accumulator to the requested output label (a no-op copy when the
    // single-segment case never entered the loop).
    if acc != out_label {
        writeln!(f, "[{acc}]null[{out_label}];").unwrap();
    }
    acc_dur
}

/// Audio analog of `fold_video`: dissolve seams with `acrossfade`, hard-
/// cut with pairwise `concat`. Same left→right fold; `acrossfade=d=` consumes
/// `d` ms of the tail of the accumulator and the head of the next stream
/// (total length = acc + seg - d), matching the video xfade and the EDL.
fn fold_audio(f: &mut String, segs: &[SegStream], out_label: &str) {
    debug_assert!(!segs.is_empty());
    let mut acc = segs[0].label.clone();
    for (i, s) in segs.iter().enumerate().skip(1) {
        let next = format!("{out_label}_x{i}");
        if s.xfade_in_ms > 0 {
            writeln!(
                f,
                "[{acc}][{l}]acrossfade=d={d}[{next}];",
                l = s.label,
                d = secs(s.xfade_in_ms),
            )
            .unwrap();
        } else {
            writeln!(f, "[{acc}][{l}]concat=n=2:v=0:a=1[{next}];", l = s.label).unwrap();
        }
        acc = next;
    }
    if acc != out_label {
        writeln!(f, "[{acc}]anull[{out_label}];").unwrap();
    }
}

/// Build the deterministic `volume` expression for a track's duck windows
/// (cut_core::GainWindow — see its header for the windowed-gain-vs-sidechain
/// honesty note). Per window: a trapezoid weight w(t) that ramps 0→1 over
/// [start-attack, start], holds 1 across [start, end], ramps 1→0 over
/// [end, end+attack]; the factor is 1+(g-1)·w(t) with g = 10^(db/20).
/// Window factors MULTIPLY, so overlapping windows stack (deepest wins
/// naturally). Evaluated per audio frame (~21 ms @48 kHz) — smooth for the
/// 250 ms default attack.
fn duck_volume_expr(windows: &[cut_core::GainWindow]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for w in windows {
        let g = 10f64.powf(w.db / 20.0);
        // Guard the ramp length: attack 0 would divide by zero — 1 ms keeps
        // the trapezoid well-defined and is inaudibly short.
        let attack_s = (w.attack_ms.max(1)) as f64 / 1000.0;
        let start_s = w.range_ms[0] as f64 / 1000.0 - attack_s;
        let end_s = w.range_ms[1] as f64 / 1000.0 + attack_s;
        parts.push(format!(
            "(1+({g:.6}-1)*max(0,min(1,min((t-{start_s:.6})/{attack_s:.6},({end_s:.6}-t)/{attack_s:.6}))))"
        ));
    }
    parts.join("*")
}

/// Build the composition graph from the EDL (media-engine contract: trim/atrim + concat
/// per track, gain via volume, caption burn-in via ass filter).
/// `project_dir` resolves project-relative asset paths. `with_captions=false`
/// skips burn-in (seam for caption-less previews); `with_audio=false` skips
/// audio chains entirely — every emitted label must be mapped or consumed,
/// ffmpeg hard-errors on unconnected outputs (frame extraction is video-only).
/// Composition model (multi-track compositing regression): the FIRST video track with
/// clips is the base canvas; every later video track is composited above it
/// in track order via full-length transparent-backed overlay streams
/// (per-clip ClipTransform = PiP geometry; gaps stay transparent). Multicam
/// angle-switching remains out of scope. Audio tracks all mix (amix).
/// Resolve the distinct media inputs for a graph: one entry per asset referenced
/// by any media segment, in first-use order, plus the asset→`-i` index map the
/// filter chains reference. Shared by the software [`build_graph`] and the GPU
/// [`build_graph_gpu`] so both number their inputs identically. Stills are flagged
/// (looped at the input). Errors if an asset id or its file is missing.
/// Absolute path to a clip's baked matte alpha under the project cache. The
/// renderer (reader) and the server bake step (writer) both route through
/// `ClipMatte::cache_filename` so they always agree.
fn matte_alpha_path(project_dir: &Path, asset_hash: &str, m: &cut_core::ClipMatte) -> PathBuf {
    project_dir
        .join("cache")
        .join("matte")
        .join(m.cache_filename(asset_hash))
}

/// Graph-input map key for a matte alpha file (distinct from asset-id keys so a
/// matte and a same-named asset never collide).
fn matte_input_key(alpha_path: &Path) -> String {
    format!("matte::{}", alpha_path.display())
}

/// Absolute path to a clip's baked mask alpha PNG under the project cache. Content-
/// addressed by the mask geometry + frame size (`ClipMask::cache_tag`), so identical
/// masks share one baked file and a render reuses it.
fn mask_alpha_path(project_dir: &Path, mask: &cut_core::ClipMask, w: u32, h: u32) -> PathBuf {
    project_dir
        .join("cache")
        .join("mask")
        .join(format!("{}.png", mask.cache_tag(w, h)))
}

/// Graph-input map key for a mask alpha file (distinct from asset/matte keys).
fn mask_input_key(alpha_path: &Path) -> String {
    format!("mask::{}", alpha_path.display())
}

/// Absolute path to a power-window's baked shape-alpha PNG (edit.grade_window). The
/// window's geometry is lowered to an ephemeral [`cut_core::ClipMask`]
/// (`WindowShape::to_mask`), so the alpha bake + content-address reuse the proven mask
/// path verbatim — identical window shapes (across windows or clips) share one baked file.
/// Stored under a distinct `gwindow/` cache dir so a window alpha and a same-shaped mask
/// alpha never collide.
fn window_alpha_path(project_dir: &Path, win: &cut_core::WindowShape, w: u32, h: u32) -> PathBuf {
    project_dir
        .join("cache")
        .join("gwindow")
        .join(format!("{}.png", win.to_mask().cache_tag(w, h)))
}

/// Graph-input map key for a power-window alpha file (distinct from asset/matte/mask keys).
fn window_input_key(alpha_path: &Path) -> String {
    format!("gwindow::{}", alpha_path.display())
}

fn segment_video_track_visible(project: &Project, seg: &EdlSegment) -> bool {
    if seg.track_kind != TrackKind::Video {
        return true;
    }
    match project.track(&seg.track) {
        Some(track) => track.visible,
        // EDL segments normally reference live project tracks. If an imported or
        // legacy EDL lacks that track, preserve the historical render behavior.
        None => true,
    }
}

static ALPHA_BAKE_TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn bake_mask_png_atomic(
    mask: &cut_core::ClipMask,
    w: u32,
    h: u32,
    alpha_path: &Path,
) -> Result<(), CutError> {
    if alpha_path.exists() {
        return Ok(());
    }
    if let Some(parent) = alpha_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let seq = ALPHA_BAKE_TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file_name = alpha_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("alpha.png");
    let tmp = alpha_path.with_file_name(format!(".{file_name}.{}.{}.tmp", std::process::id(), seq));
    crate::mask::bake_mask_png(mask, w, h, &tmp)?;
    match std::fs::rename(&tmp, alpha_path) {
        Ok(()) => Ok(()),
        Err(e) if alpha_path.exists() => {
            let _ = std::fs::remove_file(&tmp);
            drop(e);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(CutError::new(
                error_codes::FFMPEG,
                "mask PNG publish failed",
                e.to_string(),
            ))
        }
    }
}

/// Preview output geometry: scale `(w,h)` to height `ph`, preserving aspect and
/// even yuv420 dimensions. No-op when `ph >= h`; previews never upscale.
fn preview_geometry(w: u32, h: u32, ph: u32) -> (u32, u32) {
    if h == 0 || ph == 0 || ph >= h {
        return (w & !1, h & !1);
    }
    let pw = ((w as u64 * ph as u64 + (h as u64) / 2) / h as u64) as u32;
    ((pw.max(2)) & !1, (ph.max(2)) & !1)
}

/// Whether an asset's proxy can safely replace its raw source in a preview.
/// Requires an existing clean downscale with matching aspect and no source-pixel
/// geometry such as crop or stabilization. Coordinate-free effects remain valid
/// at proxy resolution; ineligible assets fall back to the raw source.
fn asset_proxy_ok(project: &Project, edl: &Edl, asset_id: &str, asset: &cut_core::Asset) -> bool {
    let _ = project;
    if asset.proxy.is_none() {
        return false;
    }
    let Some((sw, sh)) = source_dims(asset) else {
        return false;
    };
    let src_ar = sw as f64 / sh as f64;
    let proxy_ar = crate::proxy::PROXY_WIDTH as f64 / crate::proxy::PROXY_HEIGHT as f64;
    if (src_ar - proxy_ar).abs() > 0.01 {
        return false; // letterboxed proxy — bars would composite as content
    }
    !edl.segments.iter().any(|s| {
        s.asset.as_deref() == Some(asset_id) && (s.crop.is_some() || s.stabilize.is_some())
    })
}

fn collect_graph_inputs(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
    use_proxy: bool,
    with_video: bool,
    output_w: u32,
    output_h: u32,
) -> Result<(Vec<GraphInput>, BTreeMap<String, usize>), CutError> {
    let mut input_idx: BTreeMap<String, usize> = BTreeMap::new();
    let mut inputs: Vec<GraphInput> = Vec::new();
    for seg in edl.segments.iter().filter(|s| s.asset.is_some()) {
        if !segment_video_track_visible(project, seg) {
            continue;
        }
        let asset_id = seg.asset.as_deref().unwrap();
        if input_idx.contains_key(asset_id) {
            continue;
        }
        let asset = project.assets.get(asset_id).ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                format!("asset {asset_id} referenced by the timeline does not exist"),
                "EDL references an asset id missing from project.assets",
            )
            .with_clip(seg.clip_id.clone().unwrap_or_default())
        })?;
        // Decode the proxy when it is coordinate-safe. `export.frame` passes
        // `use_proxy=false` for full-resolution source; a missing proxy falls back.
        let proxy_rel = if use_proxy && asset_proxy_ok(project, edl, asset_id, asset) {
            asset.proxy.clone()
        } else {
            None
        };
        let mut path = PathBuf::from(proxy_rel.as_deref().unwrap_or(asset.path.as_str()));
        if path.is_relative() {
            path = project_dir.join(path); // Asset (and proxy) paths may be project-relative.
        }
        if proxy_rel.is_some() && !path.exists() {
            // Proxy vanished — fall back to the raw source so the preview still renders.
            path = PathBuf::from(&asset.path);
            if path.is_relative() {
                path = project_dir.join(path);
            }
        }
        // Rust canonicalization stamps Windows paths with `\\?\`, but the
        // shipped FFmpeg build cannot open that extended form. Keep canonical
        // paths for ownership checks and persistence, then hand external media
        // tools the equivalent plain drive/UNC path.
        path = strip_verbatim_prefix(&path);
        if !path.exists() {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                format!(
                    "media file for asset {asset_id} not found: {}",
                    path.display()
                ),
                "source file moved or deleted since import",
            )
            .with_suggested_action(
                "repoint the asset at the file's new location via media.relink {asset, path} \
                 (media.check lists offline assets), or restore the file",
            ));
        }
        // Stills are looped at the input (-loop 1) so the trim chain can cut
        // the clip's duration out of an infinite single-frame stream.
        let image = asset
            .probe
            .as_ref()
            .and_then(|p| p.get("kind"))
            .and_then(|k| k.as_str())
            == Some("image");
        input_idx.insert(asset_id.to_string(), inputs.len());
        // gpu_decode defaults false (CPU decode); build_graph_gpu flips it on for
        // BASE-track inputs only (NVDEC). The software path leaves it false.
        inputs.push(GraphInput {
            path,
            image,
            gpu_decode: false,
        });
    }
    // Matte alpha mattes (edit.matte): each matted segment needs its baked alpha
    // as a PARALLEL input so the overlay chain can alphamerge it onto the clip.
    // Keyed by the cache path so clips sharing one baked matte share one `-i`.
    // The alpha is baked by edit.matte (content-addressed); a missing file is a
    // clear error, never a silent un-matted render.
    if with_video {
        for seg in edl.segments.iter().filter(|s| {
            s.matte.is_some() && s.asset.is_some() && segment_video_track_visible(project, s)
        }) {
            let m = seg.matte.as_ref().unwrap();
            let asset_id = seg.asset.as_deref().unwrap();
            let asset = project.assets.get(asset_id).ok_or_else(|| {
                CutError::new(
                    error_codes::NOT_FOUND,
                    format!("asset {asset_id} referenced by a matte segment does not exist"),
                    "EDL references an asset id missing from project.assets",
                )
            })?;
            let alpha_path = matte_alpha_path(project_dir, &asset.hash, m);
            let key = matte_input_key(&alpha_path);
            if input_idx.contains_key(&key) {
                continue;
            }
            if !alpha_path.exists() {
                return Err(CutError::new(
                    error_codes::NOT_FOUND,
                    format!(
                        "baked matte alpha missing for clip {}: {}",
                        seg.clip_id.as_deref().unwrap_or(asset_id),
                        alpha_path.display()
                    ),
                    "the matte alpha is baked by edit.matte (content-addressed); it is absent",
                )
                .with_suggested_action(
                    "re-apply edit.matte on the clip to bake its alpha (the matting sidecar must be reachable), then render",
                ));
            }
            input_idx.insert(key, inputs.len());
            inputs.push(GraphInput {
                path: alpha_path,
                image: false,
                gpu_decode: false,
            });
        }
        // Vector/freeform masks (edit.add_mask): each masked segment needs its baked
        // GRAY alpha PNG as a PARALLEL input so the chain can maskedmerge the region
        // effect. Unlike mattes (baked by a slow sidecar), a mask is rasterized HERE
        // (resvg, ~ms) and cached content-addressed — so it bakes on first render and
        // reuses thereafter. Keyed by the cache path → clips sharing one mask share `-i`.
        let (mw, mh) = (output_w, output_h);
        for seg in edl.segments.iter().filter(|s| {
            // A TRACKED rect/ellipse mask paints its alpha procedurally (geq) — no PNG
            // to bake. Only STATIC (or polygon) masks need a baked shape input here.
            s.mask.as_ref().is_some_and(|m| !mask_uses_geq(m))
                && s.asset.is_some()
                && segment_video_track_visible(project, s)
        }) {
            let m = seg.mask.as_ref().unwrap();
            let alpha_path = mask_alpha_path(project_dir, m, mw, mh);
            let key = mask_input_key(&alpha_path);
            if input_idx.contains_key(&key) {
                continue;
            }
            bake_mask_png_atomic(m, mw, mh, &alpha_path)?;
            input_idx.insert(key, inputs.len());
            inputs.push(GraphInput {
                path: alpha_path,
                image: true, // a single still PNG (looped at the input like other stills)
                gpu_decode: false,
            });
        }
        // Power windows (edit.grade_window): each window needs its baked shape-alpha PNG as a
        // PARALLEL input so the chain can alphamerge the GRADED copy into the region. The
        // alpha is rasterized exactly like a mask (resvg, ~ms) and cached content-addressed by
        // the window geometry — identical window shapes (within a clip, across clips) share one
        // `-i`. Windows are STATIC (no geq path), so every window bakes a PNG here.
        for seg in edl.segments.iter().filter(|s| {
            !s.grade_windows.is_empty()
                && s.asset.is_some()
                && segment_video_track_visible(project, s)
        }) {
            for gw in &seg.grade_windows {
                let alpha_path = window_alpha_path(project_dir, &gw.window, mw, mh);
                let key = window_input_key(&alpha_path);
                if input_idx.contains_key(&key) {
                    continue;
                }
                bake_mask_png_atomic(&gw.window.to_mask(), mw, mh, &alpha_path)?;
                input_idx.insert(key, inputs.len());
                inputs.push(GraphInput {
                    path: alpha_path,
                    image: true, // a single still PNG (looped at the input like other stills)
                    gpu_decode: false,
                });
            }
        }
    }
    Ok((inputs, input_idx))
}

#[derive(Debug, Clone, Copy)]
struct VideoTrackPlan<'a> {
    id: &'a str,
    visible: bool,
}

/// Video compositing order with hidden visual overlays removed. The first video
/// track with clips still reserves the BASE canvas slot even when hidden, so
/// visible overlays remain overlays over black instead of being promoted to base.
fn planned_video_tracks(project: &Project) -> Vec<VideoTrackPlan<'_>> {
    let mut out = Vec::new();
    let mut seen_base = false;
    for track in project
        .tracks
        .iter()
        .filter(|t| t.kind == TrackKind::Video && !t.clips.is_empty())
    {
        if !seen_base {
            out.push(VideoTrackPlan {
                id: track.id.as_str(),
                visible: track.visible,
            });
            seen_base = true;
        } else if track.visible {
            out.push(VideoTrackPlan {
                id: track.id.as_str(),
                visible: true,
            });
        }
    }
    out
}

fn build_graph(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
    with_captions: bool,
    with_audio: bool,
    with_video: bool,
    opts: RenderOptions,
    preview_height: Option<u32>,
) -> Result<Graph, CutError> {
    let s = &project.settings;
    // Output geometry: project settings by default, or the largest source
    // (match_source). fps/audio_rate always come from project settings. The
    // Fit mode selects the conform stage (contain pad vs cover crop).
    // Preview frames use reduced geometry and proxies. Keep this out of
    // RenderOptions so export replay determinism and full geometry stay unchanged.
    let (w, h) = opts.output_geometry(project, edl);
    let (w, h) = match preview_height {
        Some(ph) => preview_geometry(w, h, ph),
        None => (w, h),
    };
    let (fps, rate) = (fps_str(s.fps), s.audio_rate);

    // --- inputs: one -i per distinct asset used by any media segment -------
    let (inputs, input_idx) = collect_graph_inputs(
        project,
        edl,
        project_dir,
        preview_height.is_some(),
        with_video,
        w,
        h,
    )?;

    let mut f = String::new();
    // Mark where the VIDEO chains begin so an audio-only build (export.audio,
    // with_video=false) can truncate them back out of the filter string. The
    // video chains are built in-memory either way (cheap string ops) but are
    // DISCARDED before ffmpeg runs, so an audio export pays NO video-filter cost.
    let video_start = f.len();

    // --- video: first track with clips = BASE; later video tracks OVERLAY --
    // Compositing model (multi-track compositing regression): the base track is the
    // canvas; every subsequent video track is composited above it in track
    // order (later tracks on top). Overlay clips default to full-frame; a
    // ClipTransform (edit.transform) scales/positions them (PiP). Multicam
    // angle-switching stays out of scope — this is compositing only.
    let video_tracks = if with_video {
        planned_video_tracks(project)
    } else {
        Vec::new()
    };
    // Collect per-segment streams (label + duration + crossfade overlap)
    // so the seam fold can dissolve flagged cuts (`xfade`) and hard-cut the
    // rest (`concat`). A gap or the black tail pad carries no crossfade.
    let mut vsegs: Vec<SegStream> = Vec::new();
    if let Some(track_plan) = video_tracks.first() {
        let track_id = track_plan.id;
        if !track_plan.visible {
            writeln!(
                f,
                "color=c=black:s={w}x{h}:r={fps}:d={d},format=yuv420p[v0];",
                d = secs(edl.duration_ms.max(1)),
            )
            .unwrap();
            vsegs.push(SegStream {
                label: "v0".to_string(),
                dur_ms: edl.duration_ms.max(1),
                xfade_in_ms: 0,
                xfade_kind: None,
            });
        } else {
            for (n, seg) in edl.track_segments(track_id).enumerate() {
                let label = format!("v{n}");
                let seg_dur = seg.timeline_out_ms - seg.timeline_in_ms;
                match (&seg.asset, seg.src_in_ms, seg.src_out_ms) {
                    (Some(asset), Some(src_in), Some(src_out)) => {
                        // Conform every segment to project geometry/fps/SAR so
                        // concat never sees mismatched streams (warn-and-conform
                        // preflight is the server's job; here we just conform).
                        // Base-track video fades (edit.fade) dip to/from black.
                        let vfade = fade_suffix(seg.fade.as_ref(), seg_dur, true, false);
                        // Source crop (`edit.crop`): runs BEFORE scale/pad, in
                        // source space, so the conform fills the frame with the
                        // CROPPED picture — the way baked-in letterbox bands are
                        // removed before they reach the rendered frame.
                        let vcrop = crop_filter(seg.crop.as_ref());
                        let asset_meta = project.assets.get(asset);
                        let conform = asset_meta
                            .map(|a| conform_filter_for_asset(a, seg.crop.as_ref(), w, h, opts.fit))
                            .unwrap_or_else(|| conform_filter(w, h, opts.fit));
                        let vfps = asset_meta
                            .map(|a| {
                                fps_filter_for_asset(a, s.fps, seg.speed, seg.freeze.is_some())
                            })
                            .unwrap_or_else(|| format!("fps={fps}"));
                        // Speed retime (edit.speed): the setpts reset divides by the
                        // clip's speed so the trimmed source span plays in its
                        // (shorter/longer) timeline slot. Identity at 1.0.
                        // Freeze (edit.freeze) rewrites the trim into a single held
                        // frame + tpad clone (and forces a plain setpts); a normal clip
                        // gets the speed setpts and the full trim. Source-space splice.
                        let (trim_in, trim_out, vsetpts, vfreeze) = freeze_chain(
                            seg.freeze.as_ref(),
                            seg.speed,
                            src_in,
                            src_out,
                            seg_dur,
                            s.fps,
                        );
                        // Reverse playback (edit.reverse): emitted right after the PTS
                        // reset (source space, before crop/conform) so the clip plays
                        // backward; ffmpeg `reverse` buffers the whole clip in RAM (the
                        // verb fences clip size). Freeze takes precedence (reversing a
                        // held frame is a no-op, and avoids buffering N identical frames).
                        let vreverse = if seg.reverse && seg.freeze.is_none() {
                            ",reverse"
                        } else {
                            ""
                        };
                        // Ken Burns pan/zoom (edit.animate): zoompan AFTER conform so it
                        // animates the conformed full frame; empty when not animated. A
                        // SCALE keyframe (edit.keyframe param=scale) is the multi-point eased
                        // generalization → the same zoompan slot, mutually exclusive with
                        // edit.animate (so prefer it when present).
                        let vanim = scale_kf_zoompan(&seg.keyframes, w, h, s.fps, seg_dur)
                            .unwrap_or_else(|| {
                                animate_filter(seg.animation.as_ref(), w, h, s.fps, seg_dur)
                            });
                        // Color grade (edit.grade): per-pixel, AFTER conform.
                        let vgrade = grade_stack_filter(seg.grade.as_ref(), &seg.grade_stack);
                        // Color management (project.color + edit.color_space): convert the
                        // clip INPUT → WORKING → OUTPUT space as the FINAL per-clip pixel
                        // stage. "" when default (rec709 all round, no input tag) → the
                        // chain is byte-identical to a pre-color-management render.
                        let vcolor = colorspace_filter(seg.input_color_space.as_ref(), &s.color);
                        // Visual effects (edit.effect): after grade. overlay=false on the
                        // base (chroma key is skipped — nothing below the base to reveal).
                        let veffects = effect_filter(&seg.effects, false);
                        // Base-clip transform (edit.transform on the base / single clip): scale +
                        // position within the frame, padded with black. "" when identity (the
                        // common case → byte-identical). The final region output receives
                        // this transform too when a mask or power window is active.
                        let base_transform = project
                            .find_clip(seg.clip_id.as_deref().unwrap_or_default())
                            .and_then(|(tid, i)| match &project.track(tid)?.clips[i] {
                                cut_core::Clip::Media(c) => c.transform.clone(),
                                _ => None,
                            })
                            .unwrap_or_else(cut_core::ClipTransform::identity);
                        let vtransform = base_transform_filter(&base_transform, w, h);
                        let vopacity = base_opacity_filter(&base_transform, &seg.keyframes);
                        // Stabilization (edit.stabilize): vidstabtransform on the source
                        // frames (right after the PTS reset, before reverse/crop) using the
                        // detect pre-pass's .trf. Empty when not stabilized / no .trf yet.
                        let vstab = stab_filter(
                            seg.stabilize.as_ref(),
                            seg.freeze.is_some(),
                            project
                                .assets
                                .get(asset)
                                .map(|a| a.hash.as_str())
                                .unwrap_or(""),
                            src_in,
                            src_out,
                            project_dir,
                        );
                        // edit.add_mask: a region effect (blur/pixelate/black) scoped via
                        // a baked shape alpha. When present, the conformed clip is split,
                        // the effect runs on one copy, and it's alphamerge+overlay'd back
                        // inside the shape — so the rest of the frame is untouched. The
                        // un-masked path stays the byte-identical single-line chain.
                        let mask_idx = seg.mask.as_ref().and_then(|m| {
                            let p = mask_alpha_path(project_dir, m, w, h);
                            input_idx.get(&mask_input_key(&p)).copied()
                        });
                        // A mask renders iff it's TRACKED (procedural geq, no PNG input) OR
                        // it has a baked alpha input. Both first conform the clip into `pre`,
                        // then composite the region effect back over it.
                        let masked = seg
                            .mask
                            .as_ref()
                            .filter(|m| mask_uses_geq(m) || mask_idx.is_some());
                        // edit.grade_window: region-scoped grades. Like a mask, each window
                        // splits the conformed clip, grades one copy, and alphamerge+overlays
                        // it back inside the window shape. Windows STACK (composited in order)
                        // and chain AFTER the whole-frame grade (so they grade the graded
                        // frame, matching downstream-node order), and BEFORE any mask.
                        let has_windows = !seg.grade_windows.is_empty();
                        if masked.is_some() || has_windows {
                            // Region effects first produce one full-frame stream. Base
                            // transform/opacity are applied to that result below so combining
                            // a power window or mask with a transform never drops either edit.
                            let pre = format!("{label}pre");
                            let post_region = if vtransform.is_empty() && vopacity.is_empty() {
                                label.clone()
                            } else {
                                format!("{label}region")
                            };
                            writeln!(
                            f,
                            "[{idx}:v]trim=start={trim_in}:end={trim_out},{vsetpts}{vstab}{vreverse}{vfreeze}{vcrop},\
                             {conform}{vanim}{vgrade}{veffects}{vcolor},{vfps},format=yuv420p[{pre}];",
                            idx = input_idx[asset],
                        )
                        .unwrap();
                            // Composite each power window in order: pre → gw0 → gw1 → … The
                            // LAST composite on the clip (final window when unmasked, else the
                            // mask) carries vfade; earlier blocks carry "".
                            let mut cur = pre;
                            let n_windows = seg.grade_windows.len();
                            for (wi, gw) in seg.grade_windows.iter().enumerate() {
                                let is_final = masked.is_none() && wi + 1 == n_windows;
                                let out = if is_final {
                                    post_region.clone()
                                } else {
                                    format!("{label}gw{wi}")
                                };
                                let this_vfade = if is_final { vfade.as_str() } else { "" };
                                let uniq = format!("{label}w{wi}");
                                let wp = window_alpha_path(project_dir, &gw.window, w, h);
                                let widx = input_idx[&window_input_key(&wp)];
                                f.push_str(&grade_window_block(
                                    gw, widx, &cur, &out, &uniq, w, h, this_vfade,
                                ));
                                f.push('\n');
                                cur = out;
                            }
                            if let Some(m) = masked {
                                if mask_uses_geq(m) {
                                    f.push_str(&mask_block_tracked(
                                        m,
                                        &cur,
                                        &post_region,
                                        &post_region,
                                        w,
                                        h,
                                        &vfade,
                                    ));
                                } else {
                                    f.push_str(&mask_block(
                                        m,
                                        mask_idx.unwrap(),
                                        &cur,
                                        &post_region,
                                        &post_region,
                                        w,
                                        h,
                                        &vfade,
                                    ));
                                }
                                f.push('\n');
                            }
                            if post_region != label {
                                writeln!(
                                    f,
                                    "{}",
                                    base_region_transform_block(
                                        &post_region,
                                        &label,
                                        &vtransform,
                                        &vopacity,
                                    )
                                )
                                .unwrap();
                            }
                        } else {
                            writeln!(
                            f,
                            "[{idx}:v]trim=start={trim_in}:end={trim_out},{vsetpts}{vstab}{vreverse}{vfreeze}{vcrop},\
                             {conform}{vanim}{vgrade}{veffects}{vtransform}{vopacity}{vcolor},{vfps},format=yuv420p{vfade}[{label}];",
                            idx = input_idx[asset],
                        )
                        .unwrap();
                        }
                        vsegs.push(SegStream {
                            label,
                            dur_ms: seg_dur,
                            xfade_in_ms: seg.xfade_in_ms,
                            xfade_kind: seg.xfade_kind.clone(),
                        });
                    }
                    _ => {
                        // Gap on the video track = black (public contract: gaps occupy time).
                        writeln!(
                            f,
                            "color=c=black:s={w}x{h}:r={fps}:d={d},format=yuv420p[{label}];",
                            d = secs(seg_dur),
                        )
                        .unwrap();
                        vsegs.push(SegStream {
                            label,
                            dur_ms: seg_dur,
                            xfade_in_ms: 0,
                            xfade_kind: None,
                        });
                    }
                }
            }
            // Pad the base to the full composition length with black when an
            // overlay extends past it — overlays need a canvas under them.
            let base_end = edl
                .track_segments(track_id)
                .map(|s| s.timeline_out_ms)
                .max()
                .unwrap_or(0);
            if base_end < edl.duration_ms {
                let label = format!("v{}", vsegs.len());
                let pad = edl.duration_ms - base_end;
                writeln!(
                    f,
                    "color=c=black:s={w}x{h}:r={fps}:d={d},format=yuv420p[{label}];",
                    d = secs(pad),
                )
                .unwrap();
                vsegs.push(SegStream {
                    label,
                    dur_ms: pad,
                    xfade_in_ms: 0,
                    xfade_kind: None,
                });
            }
        }
    }
    // True when ANY base-track seam crossfades — only THEN do we switch to the
    // pairwise seam fold. With no crossfade we keep the ORIGINAL single N-way
    // `concat` so older op logs render byte-identical (the pairwise fold is a
    // different filter graph and is NOT guaranteed to match a single concat
    // bit-for-bit; the replay-determinism invariant forbids changing it here).
    let base_has_xfade = vsegs.iter().any(|s| s.xfade_in_ms > 0);
    let mut vcat = if vsegs.is_empty() {
        // No video content at all → black for the whole composition (audio-only edit).
        writeln!(
            f,
            "color=c=black:s={w}x{h}:r={fps}:d={d},format=yuv420p[vcat];",
            d = secs(edl.duration_ms.max(1))
        )
        .unwrap();
        "vcat".to_string()
    } else if vsegs.len() == 1 {
        vsegs[0].label.clone()
    } else if base_has_xfade {
        // Seam fold: dissolve (xfade) flagged cuts, hard-cut (pairwise concat)
        // the rest. `s.fps` (raw f64) drives the per-leg fps normalisation the
        // xfade timebase fix (the crossfade-timebase contract) needs.
        fold_video(&mut f, &vsegs, "vcat", s.fps);
        "vcat".to_string()
    } else {
        // Unchanged hard-cut path: one N-way concat (byte-identical replay).
        for s in &vsegs {
            write!(f, "[{l}]", l = s.label).unwrap();
        }
        writeln!(f, "concat=n={}:v=1:a=0[vcat];", vsegs.len()).unwrap();
        "vcat".to_string()
    };

    // --- overlay tracks: full-length transparent-backed streams, composited
    // above the base in track order. Each media segment is conformed to the
    // project frame, scaled/positioned per its ClipTransform, and padded back
    // to full frame with TRANSPARENT fill; gaps become transparent filler.
    // Building a continuous alpha stream (instead of overlay enable=…)
    // keeps framesync from buffering the whole base while waiting for a
    // late-starting overlay — that buffering OOMs on long timelines.
    for (ti, track_plan) in video_tracks.iter().enumerate().skip(1) {
        let track_id = track_plan.id;
        let track = project.track(track_id).expect("listed track exists");
        let mut olabels: Vec<String> = Vec::new();
        let mut cursor: u64 = 0; // timeline position covered so far
        let filler = |from: u64, to: u64, f: &mut String, olabels: &mut Vec<String>| {
            if to > from {
                let label = format!("o{ti}_f{}", olabels.len());
                writeln!(
                    f,
                    "color=c=black@0.0:s={w}x{h}:r={fps}:d={d},format=yuva420p[{label}];",
                    d = secs(to - from),
                )
                .unwrap();
                olabels.push(label);
            }
        };
        for seg in edl.track_segments(track_id) {
            let (Some(asset), Some(src_in), Some(src_out)) =
                (&seg.asset, seg.src_in_ms, seg.src_out_ms)
            else {
                continue; // gaps: transparent (handled by filler below)
            };
            filler(cursor, seg.timeline_in_ms, &mut f, &mut olabels);
            // Per-clip transform: even-rounded pixel geometry for yuv420.
            let t = project
                .find_clip(seg.clip_id.as_deref().unwrap_or_default())
                .and_then(|(tid, i)| match &project.track(tid)?.clips[i] {
                    cut_core::Clip::Media(c) => c.transform.clone(),
                    _ => None,
                })
                .unwrap_or_else(cut_core::ClipTransform::identity);
            // Even-rounded pixel geometry (yuv420 chroma alignment): sizes
            // floor at 2px and cap at the frame; positions clamp so the
            // overlay stays fully inside (pad refuses out-of-bounds).
            let (ow, oh) = (
                even_size_px(f64::from(w) * t.scale, w),
                even_size_px(f64::from(h) * t.scale, h),
            );
            let (ox, oy) = (
                even_pos_px(f64::from(w) * t.x, w.saturating_sub(ow)),
                even_pos_px(f64::from(h) * t.y, h.saturating_sub(oh)),
            );
            // Keyframed OPACITY (edit.keyframe param=opacity): a `geq` alpha
            // expression on the yuva frame, injected after format=yuva420p (before
            // the PiP scale/pad). When present it OVERRIDES the static opacity.
            let vopac = opacity_kf_filter(&seg.keyframes);
            let place = if t.is_identity() {
                String::new() // full-frame, fully opaque: no extra stage
            } else {
                // setsar=1 RIGHT AFTER the transform scale (non-exact-scale guard):
                // even-rounding ow/oh drifts the aspect ratio a sub-pixel,
                // and ffmpeg's scale compensates by setting a non-1:1 SAR
                // to preserve DAR (e.g. scale:0.62 → 1190x668 → SAR
                // 5344:5355). The conform stage's setsar=1 sits BEFORE this
                // suffix, so the drift survived to concat, which refuses
                // mixed-SAR inputs ("SAR 5344:5355 do not match SAR 1:1")
                // — one odd PiP scale bricked every render of the project.
                // Forcing square pixels here costs at most a sub-pixel of
                // geometric distortion (invisible), never a failed render.
                let mut s =
                    format!(",scale={ow}:{oh},setsar=1,pad={w}:{h}:{ox}:{oy}:color=black@0.0");
                // Overlay OPACITY (edit.transform.opacity): scale the alpha plane
                // (the stream is yuva420p) so the overlay blends over the base —
                // 1.0 = opaque (no filter), <1 = ghost/blend. colorchannelmixer
                // aa multiplies alpha; the transparent pad border stays 0. SKIPPED
                // when opacity is KEYFRAMED (the geq above is the animated source).
                if t.opacity < 1.0 && vopac.is_empty() {
                    let o = (t.opacity.clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
                    s.push_str(&format!(",colorchannelmixer=aa={o}"));
                }
                s
            };
            // Overlay video fades (edit.fade) ramp the ALPHA plane — the
            // overlay dissolves over the base instead of dipping to black.
            let vfade = fade_suffix(
                seg.fade.as_ref(),
                seg.timeline_out_ms - seg.timeline_in_ms,
                true,
                true,
            );
            // Source crop (`edit.crop`) on overlays too: crop the source
            // BEFORE the conform scale, then `place` applies the PiP transform
            // (crop → conform → transform). An overlay clip can be both
            // cropped (remove its own bands) and PiP-placed.
            let vcrop = crop_filter(seg.crop.as_ref());
            // Overlay conform respects the same Fit mode as the base; the
            // `place` transform then scales/positions the conformed full-frame
            // overlay (PiP). yuva420p keeps the alpha plane for compositing.
            let asset_meta = project.assets.get(asset);
            let conform = asset_meta
                .map(|a| conform_filter_for_asset(a, seg.crop.as_ref(), w, h, opts.fit))
                .unwrap_or_else(|| conform_filter(w, h, opts.fit));
            // Speed/freeze (edit.speed/edit.freeze) apply to overlays too.
            let (trim_in, trim_out, vsetpts, vfreeze) = freeze_chain(
                seg.freeze.as_ref(),
                seg.speed,
                src_in,
                src_out,
                seg.timeline_out_ms - seg.timeline_in_ms,
                s.fps,
            );
            let vfps = asset_meta
                .map(|a| fps_filter_for_asset(a, s.fps, seg.speed, seg.freeze.is_some()))
                .unwrap_or_else(|| format!("fps={fps}"));
            // Reverse playback (edit.reverse) applies to overlays too; freeze wins.
            let vreverse = if seg.reverse && seg.freeze.is_none() {
                ",reverse"
            } else {
                ""
            };
            // Ken Burns pan/zoom (edit.animate) applies to overlays too, after conform;
            // a SCALE keyframe takes the same slot (the eased multi-point form).
            let seg_dur_ms = seg.timeline_out_ms - seg.timeline_in_ms;
            let vanim = scale_kf_zoompan(&seg.keyframes, w, h, s.fps, seg_dur_ms)
                .unwrap_or_else(|| animate_filter(seg.animation.as_ref(), w, h, s.fps, seg_dur_ms));
            // Color grade (edit.grade) applies to overlay clips too, after conform.
            let vgrade = grade_stack_filter(seg.grade.as_ref(), &seg.grade_stack);
            // Color management (project.color + edit.color_space) on overlays too —
            // the final per-clip pixel stage (input → working → output). "" when
            // default → byte-identical. (On the matte path it tags the FOREGROUND
            // copy only; the gray alpha matte skips all color stages.)
            let vcolor = colorspace_filter(seg.input_color_space.as_ref(), &s.color);
            // Visual effects (edit.effect): overlay=true so chroma key keys the
            // background to transparent BEFORE format=yuva420p (revealing the
            // layer below); placed after conform/grade, before the PiP transform.
            let veffects = effect_filter(&seg.effects, true);
            // Stabilization (edit.stabilize) on overlays too: vidstabtransform on the
            // source frames (after the PTS reset), using the detect pre-pass's .trf.
            let vstab = stab_filter(
                seg.stabilize.as_ref(),
                seg.freeze.is_some(),
                project
                    .assets
                    .get(asset)
                    .map(|a| a.hash.as_str())
                    .unwrap_or(""),
                src_in,
                src_out,
                project_dir,
            );
            let label = format!("o{ti}_s{}", olabels.len());
            // Keyframed POSITION (edit.keyframe param=pos_x/pos_y): animate the PiP
            // placement. The static-PiP `pad` placement takes no time-expression, so
            // an animated overlay is placed by OVERLAYING the scaled clip onto a
            // per-segment TRANSPARENT canvas at an animated x/y (`overlay` DOES accept
            // time-expressions). The canvas keeps the segment a full-frame yuva stream
            // so it still concats into the continuous overlay track — the OOM-avoidance
            // the static path relies on is PRESERVED. Static overlays are untouched:
            // they keep the exact `pad` graph (byte-identical replay).
            let posx = kf_points(&seg.keyframes, cut_core::KfParam::PosX);
            let posy = kf_points(&seg.keyframes, cut_core::KfParam::PosY);
            // edit.matte: SET the overlay's alpha plane from the baked straight-alpha
            // (RVM) instead of the clip's own opaque alpha — the background is removed
            // (this overlay reveals the track below) / replaced. The matte is a
            // PARALLEL gray input trimmed/conformed IDENTICALLY to the clip (so it
            // stays frame-aligned), alphamerged onto the yuva clip; the usual
            // opacity/PiP/fade suffixes then apply unchanged. Software path only (the
            // GPU gate excludes matte). v1 composites with the STATIC `place` form
            // (full-frame or static PiP); the alpha skips the colour stages
            // (grade/effects don't change a matte) but keeps every GEOMETRY stage.
            let matte_alpha_idx = seg.matte.as_ref().and_then(|m| {
                let hash = project
                    .assets
                    .get(asset.as_str())
                    .map(|a| a.hash.as_str())
                    .unwrap_or("");
                input_idx
                    .get(&matte_input_key(&matte_alpha_path(project_dir, hash, m)))
                    .copied()
            });
            if let Some(am) = matte_alpha_idx {
                let fgpre = format!("ofp{ti}_{}", olabels.len());
                let amatte = format!("oam{ti}_{}", olabels.len());
                writeln!(
                    f,
                    "[{idx}:v]trim=start={trim_in}:end={trim_out},{vsetpts}{vstab}{vreverse}{vfreeze}{vcrop},\
                     {conform}{vanim}{vgrade}{veffects}{vcolor},{vfps},format=yuva420p[{fgpre}];\n\
                     [{am}:v]trim=start={trim_in}:end={trim_out},{vsetpts}{vstab}{vreverse}{vfreeze}{vcrop},\
                     {conform}{vanim},{vfps},format=gray[{amatte}];\n\
                     [{fgpre}][{amatte}]alphamerge{vopac}{place}{vfade}[{label}];",
                    idx = input_idx[asset.as_str()],
                )
                .unwrap();
            } else if posx.is_some() || posy.is_some() {
                // overlay x/y in pixels: an animated `frame_dim * fraction` when that
                // axis is keyframed, else the static ox/oy. `t` is segment-local (the
                // canvas and the PTS-reset clip both start at 0). Values are NOT
                // clamped, so the overlay can slide in from / out to off-screen.
                let xexpr = match posx {
                    Some((pts, interp)) => format!("{w}*({})", kf_expr(&pts, "t", interp)),
                    None => ox.to_string(),
                };
                let yexpr = match posy {
                    Some((pts, interp)) => format!("{h}*({})", kf_expr(&pts, "t", interp)),
                    None => oy.to_string(),
                };
                // Static opacity on the scaled overlay (same condition as the pad
                // path; skipped when opacity is keyframed — vopac drives the alpha).
                let opac = if t.opacity < 1.0 && vopac.is_empty() {
                    let o = (t.opacity.clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
                    format!(",colorchannelmixer=aa={o}")
                } else {
                    String::new()
                };
                let small = format!("o{ti}_sm{}", olabels.len());
                let canvas = format!("o{ti}_cv{}", olabels.len());
                writeln!(
                    f,
                    "[{idx}:v]trim=start={trim_in}:end={trim_out},{vsetpts}{vstab}{vreverse}{vfreeze}{vcrop},\
                     {conform}{vanim}{vgrade}{veffects}{vcolor},{vfps},format=yuva420p{vopac},\
                     scale={ow}:{oh},setsar=1{opac}[{small}];\n\
                     color=c=black@0.0:s={w}x{h}:r={fps}:d={d},format=yuva420p[{canvas}];\n\
                     [{canvas}][{small}]overlay=x='{xexpr}':y='{yexpr}':eof_action=pass{vfade}[{label}];",
                    idx = input_idx[asset.as_str()],
                    d = secs(seg.timeline_out_ms - seg.timeline_in_ms),
                )
                .unwrap();
            } else {
                writeln!(
                    f,
                    "[{idx}:v]trim=start={trim_in}:end={trim_out},{vsetpts}{vstab}{vreverse}{vfreeze}{vcrop},\
                     {conform}{vanim}{vgrade}{veffects}{vcolor},{vfps},format=yuva420p{vopac}{place}{vfade}[{label}];",
                    idx = input_idx[asset.as_str()],
                )
                .unwrap();
            }
            olabels.push(label);
            cursor = seg.timeline_out_ms;
        }
        filler(cursor, edl.duration_ms, &mut f, &mut olabels);
        if olabels.is_empty() {
            continue; // track listed but contributed nothing visible
        }
        let ostream = if olabels.len() == 1 {
            olabels[0].clone()
        } else {
            for l in &olabels {
                write!(f, "[{l}]").unwrap();
            }
            let cat = format!("o{ti}");
            writeln!(f, "concat=n={}:v=1:a=0[{cat}];", olabels.len()).unwrap();
            cat
        };
        let composed = format!("vo{ti}");
        // LAYER blend mode (edit.blend): when set, this track composites onto the
        // base with a blend (multiply/screen/…) — but ONLY where the overlay has
        // content. `blend` ignores alpha, so we blend the whole frame, then keep
        // only the layer's own region via its alpha mask (alphamerge), then overlay
        // that onto the base. Verified recipe. None/"normal" → plain alpha-over.
        match track
            .blend_mode
            .as_deref()
            .filter(|m| !m.is_empty() && *m != "normal")
        {
            Some(mode) => {
                let (vca, vcb) = (format!("bvc{ti}a"), format!("bvc{ti}b"));
                let (osa, osb, osc) = (
                    format!("bos{ti}a"),
                    format!("bos{ti}b"),
                    format!("bos{ti}c"),
                );
                let (mask, bl, bla) = (
                    format!("bmask{ti}"),
                    format!("bbl{ti}"),
                    format!("bbla{ti}"),
                );
                // Explicit formats keep the negotiation valid: alphaextract needs a
                // yuva input; `blend` needs both inputs the SAME format (the base is
                // yuv420p, so drop the overlay's alpha for the color blend), then the
                // overlay's alpha (mask) re-limits the blend to the layer's region.
                writeln!(
                    f,
                    "[{vcat}]split[{vca}][{vcb}];\n\
                     [{ostream}]format=yuva420p,split[{osa}][{osb}];\n\
                     [{osa}]alphaextract[{mask}];\n\
                     [{osb}]format=yuv420p[{osc}];\n\
                     [{vca}][{osc}]blend=all_mode={mode}:shortest=1[{bl}];\n\
                     [{bl}][{mask}]alphamerge[{bla}];\n\
                     [{vcb}][{bla}]overlay=0:0:eof_action=pass[{composed}];"
                )
                .unwrap();
            }
            None => {
                // eof_action=pass: frame-count rounding must never stall the base.
                writeln!(
                    f,
                    "[{vcat}][{ostream}]overlay=0:0:eof_action=pass[{composed}];"
                )
                .unwrap();
            }
        }
        vcat = composed;
    }

    // --- adjustment layers (edit.adjustment): non-destructive grade/effect bands
    // applied to the COMPOSITE of every track beneath them, each gated to its span.
    // v1 applies them as the TOP-MOST layer (after all overlays, before captions —
    // captions/titles sit above the grade, the editor convention), in list order.
    // OFF-PATH ENTIRELY when there is no adjustment: `edl.adjustments` is empty for
    // any timeline without one, so this loop emits nothing and the filtergraph is
    // byte-identical to before this feature (the determinism invariant).
    for (ai, adj) in edl.adjustments.iter().enumerate() {
        let out = format!("vadj{ai}");
        f.push_str(&adjustment_block(adj, &vcat, &out, &ai.to_string()));
        f.push('\n');
        vcat = out;
    }

    // --- captions: burn in via libass when any caption clip exists ---------
    let has_captions = edl.segments.iter().any(|s| s.caption_text.is_some());
    let (video_out, ass_dir) = if with_captions && has_captions {
        // Captions come from the EDL (which Edl::window has already rebased for
        // a windowed render), so segmented renders burn the right captions
        // in-pass. A full EDL reproduces the project's caption times verbatim →
        // byte-identical ASS.
        let ass = crate::captions::captions_to_ass_for_edl(project, edl)?;
        // Content is deterministic (pure function of the project), so the
        // temp file's PATH varying run-to-run does not affect output bytes.
        let dir = tempfile::tempdir().map_err(CutError::from)?;
        let ass_path = dir.path().join("burnin.ass");
        std::fs::write(&ass_path, ass)?;
        writeln!(
            f,
            "[{vcat}]ass=filename={}[vout];",
            escape_filter_path(&ass_path)
        )
        .unwrap();
        ("vout".to_string(), Some(dir))
    } else {
        (vcat, None)
    };
    // Audio-only build (export.audio): drop the video output label, so the final
    // graph + ffmpeg run carry ONLY audio. with_video=false keeps video_tracks
    // empty above, so mask/window/matte alpha inputs are not baked just to be
    // discarded here.
    let (video_out, ass_dir) = if with_video {
        (video_out, ass_dir)
    } else {
        f.truncate(video_start);
        (String::new(), None)
    };

    // --- audio: per-track atrim+concat chains, then amix across tracks -----
    // MUTE/SOLO (edit.mute / edit.solo): a non-audible track is DROPPED from the
    // mix → it contributes SILENCE, without touching its gain. Audibility is the
    // model's single source of truth (Project::audio_track_audible): muted ⇒ out;
    // if any track is soloed, only soloed tracks stay. Honored here, so render,
    // preview (render_preview reuses build_graph), and export.audio all agree.
    let mut track_outs: Vec<String> = Vec::new();
    for (t, track) in project
        .tracks
        .iter()
        .filter(|t| {
            with_audio
                && t.kind == TrackKind::Audio
                && !t.clips.is_empty()
                && project.audio_track_audible(t)
        })
        .enumerate()
    {
        let mut asegs: Vec<SegStream> = Vec::new();
        for (n, seg) in edl.track_segments(&track.id).enumerate() {
            let label = format!("a{t}_{n}");
            let seg_dur = seg.timeline_out_ms - seg.timeline_in_ms;
            match (&seg.asset, seg.src_in_ms, seg.src_out_ms) {
                (Some(asset), Some(src_in), Some(src_out)) => {
                    // Gain (clip+track, already summed into the EDL) via the
                    // volume filter; skipped at unity to keep graphs minimal.
                    // Keyframed VOLUME (edit.keyframe param=volume) OVERRIDES the
                    // static gain: a per-sample-window `volume` expression (linear
                    // multiplier) in clip-local time `t`. Else the static gain dB.
                    let gain = if seg
                        .keyframes
                        .iter()
                        .any(|kf| kf.param == cut_core::KfParam::Volume)
                    {
                        volume_kf_filter(&seg.keyframes)
                    } else if seg.gain_db != 0.0 {
                        format!(",volume={:.2}dB", seg.gain_db)
                    } else {
                        String::new()
                    };
                    // Audio fades (edit.fade): linear afade in segment time.
                    let afade = fade_suffix(seg.fade.as_ref(), seg_dur, false, false);
                    // Speed retime (edit.speed): pitch-preserved tempo change
                    // (atempo, sqrt-split for the full range). Emitted right
                    // after the timestamp reset and BEFORE the fade so the
                    // afade — computed against the TIMELINE (post-speed) seg_dur
                    // — lands on the already-stretched stream. Empty at 1.0.
                    let aspeed = audio_speed_filter(seg.speed, true, rate);
                    // Reverse playback (edit.reverse): `areverse` right after the
                    // timestamp reset and BEFORE atempo (so a reversed+sped clip
                    // reverses then time-stretches — the proven order). Empty when
                    // not reversed → byte-identical.
                    let areverse = if seg.reverse { "areverse," } else { "" };
                    // Audio effects (edit.effect, currently denoise/afftdn): on the
                    // conformed source audio, BEFORE gain + fade.
                    let adenoise = audio_effect_filter(&seg.effects);
                    // Parametric EQ (edit.eq): high-pass + peaking bands + low-pass,
                    // AFTER denoise/compressor and BEFORE gain/fade. Empty when no EQ
                    // → byte-identical.
                    let aeq = eq_filter(seg.eq.as_ref());
                    // Non-destructive MUTE ranges (edit.mute_range / transcript.
                    // mute_words): volume forced to 0 over each SOURCE-range's
                    // overlap with the window, in post-speed segment time — AFTER
                    // gain (a muted word stays silent whatever the gain) and BEFORE
                    // the fade. Empty when the clip has none → byte-identical.
                    let amute = mute_gate_filter(seg);
                    writeln!(
                        f,
                        "[{idx}:a]atrim=start={in_s}:end={out_s},asetpts=PTS-STARTPTS,{areverse}{aspeed}\
                         aformat=sample_fmts=fltp:sample_rates={rate}:channel_layouts=stereo{adenoise}{aeq}{gain}{amute}{afade}[{label}];",
                        idx = input_idx[asset],
                        in_s = secs(src_in),
                        out_s = secs(src_out),
                    )
                    .unwrap();
                    asegs.push(SegStream {
                        label,
                        dur_ms: seg_dur,
                        xfade_in_ms: seg.xfade_in_ms,
                        xfade_kind: seg.xfade_kind.clone(),
                    });
                }
                _ => {
                    // Audio gap = silence of the gap's duration.
                    writeln!(
                        f,
                        "anullsrc=r={rate}:cl=stereo,aformat=sample_fmts=fltp:sample_rates={rate}:\
                         channel_layouts=stereo,atrim=end={d}[{label}];",
                        d = secs(seg_dur),
                    )
                    .unwrap();
                    asegs.push(SegStream {
                        label,
                        dur_ms: seg_dur,
                        xfade_in_ms: 0,
                        xfade_kind: None,
                    });
                }
            }
        }
        // Crossfade fold only when a seam dissolves; otherwise the original N-way
        // concat remains byte-identical for older logs.
        let track_has_xfade = asegs.iter().any(|s| s.xfade_in_ms > 0);
        let mut out = if asegs.len() == 1 {
            asegs[0].label.clone()
        } else if track_has_xfade {
            let cat = format!("at{t}");
            fold_audio(&mut f, &asegs, &cat);
            cat
        } else {
            let cat = format!("at{t}");
            for s in &asegs {
                write!(f, "[{l}]", l = s.label).unwrap();
            }
            writeln!(f, "concat=n={}:v=0:a=1[{cat}];", asegs.len()).unwrap();
            cat
        };
        // edit.duck windows: deterministic per-sample volume expression over
        // the whole track chain (types.rs GainWindow — windowed gain, not a
        // sidechain). Applied AFTER concat so window times are timeline times.
        if !track.gain_windows.is_empty() {
            let ducked = format!("at{t}d");
            let expr = duck_volume_expr(&track.gain_windows);
            writeln!(f, "[{out}]volume=volume='{expr}':eval=frame[{ducked}];").unwrap();
            out = ducked;
        }
        // `edit.pan`: per-track stereo BALANCE on the whole
        // chain — center (0.0) emits NOTHING (byte-identical mix), off-center
        // ATTENUATES the opposite channel on a cosine taper and never boosts.
        // pan>0 (right): L' = L·cos(pan·π/2), R' = R; pan<0 mirrors. Fixed
        // 6-decimal coefficients keep the graph deterministic.
        if track.pan != 0.0 {
            let panned = format!("at{t}p");
            let theta = track.pan.abs() * std::f64::consts::FRAC_PI_2;
            let att = theta.cos();
            let (lg, rg) = if track.pan > 0.0 {
                (att, 1.0)
            } else {
                (1.0, att)
            };
            writeln!(
                f,
                "[{out}]pan=stereo|c0={lg:.6}*c0|c1={rg:.6}*c1[{panned}];"
            )
            .unwrap();
            out = panned;
        }
        track_outs.push(out);
    }
    let mut audio_out = match track_outs.len() {
        0 => None,
        1 => Some(track_outs[0].clone()),
        n => {
            for l in &track_outs {
                write!(f, "[{l}]").unwrap();
            }
            // normalize=0: mixing must not rescale levels — gain is explicit.
            writeln!(f, "amix=inputs={n}:duration=longest:normalize=0[aout];").unwrap();
            Some("aout".to_string())
        }
    };

    // Loudness normalization (render.final `normalize_loudness`): single-pass
    // ffmpeg loudnorm to the target LUFS, true-peak −1 dBTP, LRA 11 (EBU R128
    // defaults). Single-pass is deterministic (no measured-* two-pass) and only
    // runs when a target is set, so unnormalized renders stay byte-identical.
    // Closes the measure(lufs check)→target loop.
    if let (Some(t), Some(a)) = (opts.loudness_target, audio_out.clone()) {
        writeln!(f, "[{a}]loudnorm=I={t}:TP=-1.0:LRA=11[anorm];").unwrap();
        audio_out = Some("anorm".to_string());
    }

    Ok(Graph {
        inputs,
        filter: f,
        video_out,
        audio_out,
        _ass_dir: ass_dir,
    })
}

/// Assemble the common ffmpeg arg list for a graph: inputs, filter_complex,
/// and -map flags for the requested labels. On a GPU graph each `-i` is preceded
/// by `-hwaccel cuda -hwaccel_output_format cuda` so the input is NVDEC-decoded
/// straight to CUDA frames (no per-frame PCIe round-trip; scale_cuda/overlay_cuda
/// then run on those frames and nvenc encodes them in place).
fn graph_args(graph: &Graph, map_video: &str, map_audio: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    for input in &graph.inputs {
        if input.gpu_decode && !input.image {
            // NVDEC decode to VRAM for the BASE track. (Stills are not on the GPU
            // base-track path in v1 — the gate excludes them — but guard anyway: a
            // looped still would need format,hwupload instead of hwaccel decode.)
            // Per-input (not graph.gpu): OVERLAY inputs stay on the CPU so their
            // pad/colorchannelmixer/filler filters run, then a single hwupload feeds
            // overlay_cuda (2b-ii).
            args.push("-hwaccel".into());
            args.push("cuda".into());
            args.push("-hwaccel_output_format".into());
            args.push("cuda".into());
        }
        if input.image {
            // Input-level loop: the still becomes an infinite stream; every
            // segment chain trims it to the clip's exact duration.
            args.push("-loop".into());
            args.push("1".into());
        }
        args.push("-i".into());
        args.push(input.path.display().to_string());
    }
    args.push("-filter_complex".into());
    // Chains are emitted line-per-chain ending in ";\n" — strip the trailing
    // separator, ffmpeg rejects an empty final chain.
    args.push(graph.filter.trim_end_matches(['\n', ';']).to_string());
    args.push("-map".into());
    args.push(format!("[{map_video}]"));
    if let Some(a) = map_audio {
        args.push("-map".into());
        args.push(format!("[{a}]"));
    }
    args
}

/// sha256 a file → "sha256:<hex>" (determinism evidence on RenderOutput).
fn sha256_file(path: &Path) -> Result<String, CutError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

// === GPU render fast-track gate ===============================================
//
// Dual-path render: the SOFTWARE filtergraph is the default — deterministic
// (libx264 bit-exact), receipt-exact, works on any PC. The CUDA path is an opt-in
// GPU fast-track (NVDEC -> scale_cuda/overlay_cuda -> NVENC, frames in VRAM):
// ~1.5x AND it frees the CPU on real CPU-decode-bound 4K, but GPU output is NOT
// bit-reproducible, so it is a SEPARATE opt-in mode, never the default.
//
// The decision + capability probe + the CUDA graph are all wired: the gate
// ([`render_target`]) ANDs opt-in + the CUDA probe + a v1-scope timeline
// ([`timeline_is_gpu_friendly`]) + a VRAM-fitting estimate ([`gpu_vram_fits`]),
// and [`build_graph_gpu`] emits the CUDA graph (base track NVDEC→scale_cuda→concat→
// nvenc + overlay_cuda PiP; audio stays software). The default path is byte-
// identical to a pre-fast-track render (untouched below).

/// Which filtergraph backend a render uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphTarget {
    /// libx264 software filtergraph — default, deterministic, byte-identical replay.
    Software,
    /// CUDA GPU-resident fast-track — opt-in, probe-gated, VRAM-bounded,
    /// non-deterministic mode ([`build_graph_gpu`] emits the graph).
    Cuda,
}

/// Environment opt-in for the GPU fast-track (`SHELLX_CUT_RENDER_GPU`). Truthy is
/// `1`/`true`/`yes`/`on`; anything else is off.
fn gpu_opt_in() -> bool {
    std::env::var("SHELLX_CUT_RENDER_GPU")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// GPU fast-track scope gate: true only for timelines the narrow CUDA graph
/// (NVDEC + scale_cuda conform + hard-cut concat + nvenc — **base track only**)
/// can render FAITHFULLY. Everything else uses operations with NO CUDA filter or
/// not yet built on the GPU path — color grade (eq/curves/lut3d), fade
/// (black-dip / alpha-ramp), source crop, xfade dissolves, caption ASS burn-in,
/// alpha title overlays, OVERLAYS/PiP (overlay_cuda has no `enable=`; timed
/// overlays need the alpha-filler path), and aspect-mismatched
/// conform (scale_cuda=W:H would distort; letterbox/crop have no CUDA filter) —
/// and MUST take the software path, so opt-in transparently falls back instead of
/// producing a broken GPU render.
///
/// CONSERVATIVE BY DESIGN: any feature we are not certain the CUDA graph handles
/// → false (software). A false negative (software when GPU would have worked) just
/// forgoes speed; a false positive (GPU on a timeline it can't render) corrupts the
/// output. Audio is always a software single-pass, so it never affects this gate.
fn timeline_is_gpu_friendly(project: &Project, edl: &Edl, opts: &RenderOptions) -> bool {
    // Color management (project.color working/output ≠ rec709) is a CPU `zscale`
    // conversion the v1 CUDA graph does not emit → software path (which also keeps
    // the conversion bit-exact). A non-default project.color forces software.
    if !project.settings.color.is_default() {
        return false;
    }
    // An adjustment layer (edit.adjustment) is a CPU-only gated composite pass
    // (split + grade/effects + timed overlay) → software path, like edit.effect.
    if !edl.adjustments.is_empty() {
        return false;
    }
    // No caption track or alpha title-overlay track with content (both CPU-only
    // in v1: ASS burn-in and per-pixel-alpha titles).
    for t in &project.tracks {
        if t.clips.is_empty() {
            continue;
        }
        if t.kind == TrackKind::Caption && t.visible {
            return false;
        }
        // title.add places alpha overlays on a video track id-prefixed "title".
        if t.kind == TrackKind::Video && t.visible && t.id.starts_with("title") {
            return false;
        }
        // A layer blend mode (edit.blend) uses the masked-blend recipe (split +
        // blend + alphamerge), which overlay_cuda can't express → software.
        if t.visible && t.blend_mode.is_some() {
            return false;
        }
    }
    // No per-segment op without a CUDA filter. (speed = setpts, a timestamp-only
    // filter that works on CUDA frames, so it is allowed.) edit.effect filters
    // (vignette/sharpen/blur/grain/chroma) are all CPU-only → software.
    for seg in &edl.segments {
        if seg.grade.is_some()
            || !seg.grade_stack.is_empty() // edit.grade_stack — N CPU eq layers, software path
            || seg.input_color_space.is_some() // edit.color_space — CPU zscale convert
            || seg.matte.is_some() // baked-alpha composite (alphamerge) — software path
            || seg.mask.is_some() // edit.add_mask region composite — software path
            || !seg.grade_windows.is_empty() // edit.grade_window region grade composite — software path
            || seg.fade.is_some()
            || seg.crop.is_some()
            || seg.xfade_in_ms > 0
            || !seg.effects.is_empty()
            || seg.reverse
            || seg.freeze.is_some()
            || seg.animation.is_some()
            || !seg.keyframes.is_empty()
            || seg.stabilize.is_some()
        {
            return false;
        }
    }
    // Only one video track is currently safe. On FFmpeg 6.1, overlay_cuda drops
    // NVDEC crop metadata and exposes the aligned hardware surface (for example
    // 320x240 becomes 320x256). A post-scale restores geometry by squashing the
    // padded rows and visibly distorts the frame, so overlays must use software
    // until a CUDA crop path is available and live parity is proven.
    let video_tracks = planned_video_tracks(project);
    let Some(base_plan) = video_tracks.first() else {
        return false;
    };
    if video_tracks.len() != 1 {
        return false;
    }
    let base_id = base_plan.id;
    // The CUDA graph currently conforms the base frame but does not apply
    // ClipTransform geometry or opacity. Keep any non-identity base transform
    // on the software graph so GPU opt-in cannot silently drop an edit.
    if base_plan.visible
        && project.track(base_id).is_some_and(|track| {
            track.clips.iter().any(|clip| match clip {
                cut_core::Clip::Media(media) => media
                    .transform
                    .as_ref()
                    .is_some_and(|transform| !transform.is_identity()),
                _ => false,
            })
        })
    {
        return false;
    }
    // Conform on the BASE is scale_cuda=W:H (exact), faithful ONLY when the source
    // aspect matches the output's; a mismatch would distort (letterbox/crop have no
    // CUDA filter in v1). Require a probed, aspect-matching geometry for every
    // BASE-track VIDEO media segment. (Overlays use the CPU conform = scale+pad,
    // which handles any source aspect, so they are NOT aspect-constrained.)
    let (ow, oh) = opts.output_geometry(project, edl);
    if ow == 0 || oh == 0 {
        return false;
    }
    if base_plan.visible {
        for seg in edl
            .track_segments(base_id)
            .filter(|s| s.asset.is_some() && s.track_kind == TrackKind::Video)
        {
            let Some(asset) = project.assets.get(seg.asset.as_deref().unwrap()) else {
                return false;
            };
            let (Some(w), Some(h)) = (
                asset
                    .probe
                    .as_ref()
                    .and_then(|p| p.get("width"))
                    .and_then(|v| v.as_u64()),
                asset
                    .probe
                    .as_ref()
                    .and_then(|p| p.get("height"))
                    .and_then(|v| v.as_u64()),
            ) else {
                return false; // unknown source geometry — can't guarantee a faithful resize
            };
            if w == 0 || h == 0 {
                return false;
            }
            // aspect match within ~1% (cross-multiply to avoid float division).
            let diff = (ow as i64 * h as i64 - oh as i64 * w as i64).unsigned_abs();
            if diff as f64 > 0.01 * (oh as f64) * (w as f64) {
                return false;
            }
        }
    }
    true
}

// === GPU VRAM bound ============================================================
//
// GPU frames live in VRAM, which has NO cgroup backstop (the render_command cgroup
// caps system RAM only) — NVDEC/NVENC fail HARD on a VRAM OOM. The GPU graph is
// single-pass (not windowed), so the bound is a per-render PEAK estimate compared
// to the device's VRAM ([`crate::hwencode::cuda_total_vram_bytes`]); over budget →
// fall back to software (never wedge the GPU). The estimate is a deliberate
// OVER-estimate: a false "fits" must never OOM the GPU, while a false "too big"
// only forgoes the fast path.

/// Bytes per pixel for a VRAM frame estimate. yuva420p is 4 planes; NV12 is 1.5,
/// but 4 keeps generous headroom (driver overhead, alignment, intermediate
/// surfaces) so the estimate over-counts rather than under-counts.
const VRAM_BYTES_PER_PX: u64 = 4;
/// NVDEC decode-surface pool per hardware-decoded BASE input (DPB + extra
/// surfaces). 4K HEVC can need ~16–20; 24 over-estimates safely.
const VRAM_DECODE_POOL_FRAMES: u64 = 24;
/// In-flight CUDA surfaces an overlay track adds (its hwupload'd full-frame
/// yuva420p stream + the overlay_cuda working set).
const VRAM_OVERLAY_FRAMES: u64 = 4;
/// NVENC working set (DPB + lookahead), in OUTPUT frames.
const VRAM_NVENC_FRAMES: u64 = 16;

/// Estimated PEAK VRAM (bytes) the GPU graph holds for this timeline: NVDEC pools
/// for the distinct base-track inputs (sized at SOURCE geometry — the pool holds
/// source-resolution frames before scale_cuda conforms them) + one full
/// output-frame surface set per overlay track + the NVENC working set. Used by the
/// render gate to fall back to software before a VRAM OOM. Over-estimates by
/// design (see the module note above).
fn gpu_vram_estimate_bytes(project: &Project, edl: &Edl, opts: &RenderOptions) -> u64 {
    let (ow, oh) = opts.output_geometry(project, edl);
    let out_frame = (ow as u64) * (oh as u64) * VRAM_BYTES_PER_PX;

    // Base canvas = the FIRST video track with clips (NVDEC-decoded; stays on GPU).
    let video_tracks = planned_video_tracks(project);
    let base_plan = video_tracks.first().copied();
    let base_id = base_plan.map(|p| p.id);

    // Decode pools: one per DISTINCT base-track NVDEC input, at source geometry.
    let mut decode = 0u64;
    if let Some(bid) = base_id.filter(|_| base_plan.is_some_and(|p| p.visible)) {
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for seg in edl
            .track_segments(bid)
            .filter(|s| s.track_kind == TrackKind::Video)
        {
            let Some(asset_id) = seg.asset.as_deref() else {
                continue;
            };
            if !seen.insert(asset_id) {
                continue;
            }
            let (sw, sh) = project
                .assets
                .get(asset_id)
                .and_then(|a| a.probe.as_ref())
                .and_then(|p| Some((p.get("width")?.as_u64()?, p.get("height")?.as_u64()?)))
                .filter(|(w, h)| *w > 0 && *h > 0)
                .unwrap_or((ow as u64, oh as u64));
            decode += VRAM_DECODE_POOL_FRAMES * sw * sh * VRAM_BYTES_PER_PX;
        }
    }

    // Overlay tracks = every video track with clips BEYOND the base; each is a
    // CPU-built full-frame yuva420p stream hwupload'd into overlay_cuda.
    let overlay_tracks = video_tracks.len().saturating_sub(1) as u64;
    let overlays = overlay_tracks * VRAM_OVERLAY_FRAMES * out_frame;

    decode + overlays + VRAM_NVENC_FRAMES * out_frame
}

/// VRAM budget (bytes) for a GPU render. `SHELLX_CUT_GPU_VRAM_BUDGET_MB` overrides
/// (tests + a manual hard cap); else 85% of the probed device VRAM (headroom for
/// the driver/display + fragmentation); else a conservative 4 GiB when the device
/// size can't be read (no nvidia-smi) — so a small GPU is never assumed huge.
fn gpu_vram_budget_bytes() -> u64 {
    if let Some(mb) = std::env::var("SHELLX_CUT_GPU_VRAM_BUDGET_MB")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
    {
        return mb * 1024 * 1024;
    }
    match crate::hwencode::cuda_total_vram_bytes() {
        Some(total) => total * 85 / 100,
        None => 4 * 1024 * 1024 * 1024,
    }
}

/// True when the GPU graph's estimated peak VRAM fits the budget. The render gate
/// ANDs this in so a heavy timeline (many 4K overlay layers / a small GPU) falls
/// back to software instead of OOMing the GPU.
fn gpu_vram_fits(project: &Project, edl: &Edl, opts: &RenderOptions) -> bool {
    gpu_vram_estimate_bytes(project, edl, opts) <= gpu_vram_budget_bytes()
}

/// The render-backend gate. GPU is chosen ONLY when ALL hold: the user opted
/// in, the CUDA filter chain actually runs on this box (probe), the timeline is
/// within the v1 GPU scope ([`timeline_is_gpu_friendly`]), AND the GPU graph's
/// estimated peak VRAM fits the device budget ([`gpu_vram_fits`]). Otherwise the
/// software default — so opt-in never breaks a grade/caption/xfade render or OOMs
/// the GPU, it just forgoes the fast path.
///
/// GPU selection currently considers capability, timeline scope, and VRAM. It
/// does not estimate whether a trivial or short render would be faster on the
/// CPU-filter‖GPU-encode path.
fn render_target(project: &Project, edl: &Edl, opts: &RenderOptions) -> GraphTarget {
    if gpu_opt_in()
        && crate::hwencode::gpu_filters_available()
        && timeline_is_gpu_friendly(project, edl, opts)
        && gpu_vram_fits(project, edl, opts)
    {
        GraphTarget::Cuda
    } else {
        GraphTarget::Software
    }
}

/// Build the GPU fast-track filtergraph for a v1-scope timeline (the gate
/// [`timeline_is_gpu_friendly`] guarantees: one base video track, no overlays /
/// captions / grade / fade / crop / xfade, opaque, matching source aspect).
///
/// Inputs are NVDEC-decoded to CUDA frames (`graph.gpu = true` → `graph_args`
/// emits `-hwaccel cuda`); each base segment is trimmed, retimed (`setpts`), and
/// conformed with `scale_cuda` to the exact output geometry (faithful because the
/// gate requires matching aspect — no letterbox/crop, which have no CUDA filter);
/// `:format=yuv420p` pins every segment to one CUDA pixfmt so `concat` (hard cuts)
/// and the black gap frames match; `nvenc` encodes the CUDA frames in place. The
/// AUDIO chain is the SOFTWARE one, reused VERBATIM from `build_graph(with_video=
/// false)` — audio never touches the GPU. Output is perceptually equal to the
/// software render but NOT bit-identical (GPU is its own mode).
fn build_graph_gpu(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
    opts: RenderOptions,
) -> Result<Graph, CutError> {
    let s = &project.settings;
    let (w, h) = opts.output_geometry(project, edl);
    let fps = fps_str(s.fps);

    // Reuse the SOFTWARE audio chain verbatim: build_graph with with_video=false
    // truncates the video chains and keeps only the audio filter + inputs +
    // audio_out — exactly what a software render of this timeline would build.
    let audio = build_graph(project, edl, project_dir, false, true, false, opts, None)?;
    let mut inputs = audio.inputs;
    // Same asset->input index map the audio graph numbered its inputs with, so the
    // CUDA video chain references the same `-i` streams.
    let (_, input_idx) = collect_graph_inputs(project, edl, project_dir, false, false, w, h)?;

    // Video tracks WITH clips, in compositing order: [0] = the base canvas (stays
    // fully on the GPU: NVDEC → scale_cuda → nvenc), [1..] = overlays/PiP (2b-ii:
    // CPU-built full-length yuva420p streams hwupload'd into overlay_cuda).
    let video_tracks = planned_video_tracks(project);

    let mut f = String::new();
    let mut vsegs: Vec<String> = Vec::new();
    if let Some(track_plan) = video_tracks.first() {
        let track_id = track_plan.id;
        // NVDEC only the BASE-track inputs; overlay-only assets stay CPU-decoded so
        // their pad/colorchannelmixer/transparent-filler chain runs before a single
        // hwupload into overlay_cuda. (The gate guarantees no asset is shared across
        // base+overlay, so this base/overlay split is unambiguous per input.)
        if track_plan.visible {
            for seg in edl.track_segments(track_id) {
                if let Some(asset) = &seg.asset {
                    if let Some(&i) = input_idx.get(asset.as_str()) {
                        inputs[i].gpu_decode = true;
                    }
                }
            }
        }
        if !track_plan.visible {
            writeln!(
                f,
                "color=c=black:s={w}x{h}:r={fps}:d={d},format=yuv420p,hwupload[g0];",
                d = secs(edl.duration_ms.max(1)),
            )
            .unwrap();
            vsegs.push("g0".to_string());
        } else {
            for (n, seg) in edl.track_segments(track_id).enumerate() {
                let label = format!("g{n}");
                let seg_dur = seg.timeline_out_ms - seg.timeline_in_ms;
                match (&seg.asset, seg.src_in_ms, seg.src_out_ms) {
                    (Some(asset), Some(src_in), Some(src_out)) => {
                        // NVDEC frames -> trim -> retime -> scale_cuda to exact WxH ->
                        // `fps` normalize to the project rate. The fps filter is
                        // PTS-based (drops/dups whole frames; no pixel access), so it
                        // runs on CUDA hwframes — verified NVDEC->scale_cuda->fps->nvenc.
                        // REQUIRED: without it a source whose fps != the project fps
                        // (e.g. a 60fps clip in a 30fps project) passes straight through
                        // at the SOURCE rate — wrong framerate/duration AND a frame-count
                        // mismatch with the project-rate black-gap frames at concat. This
                        // mirrors the software base chain (`,fps={fps}`); the gate checks
                        // aspect, not fps, so the graph itself must conform the rate.
                        let vsetpts = video_setpts(seg.speed);
                        writeln!(
                            f,
                            "[{idx}:v]trim=start={in_s}:end={out_s},{vsetpts},\
                         scale_cuda={w}:{h}:format=yuv420p,fps={fps}[{label}];",
                            idx = input_idx[asset.as_str()],
                            in_s = secs(src_in),
                            out_s = secs(src_out),
                        )
                        .unwrap();
                    }
                    _ => {
                        // Gap = black, built in system memory then hwupload'd so it is a
                        // CUDA yuv420p frame that concats with the scale_cuda output.
                        writeln!(
                        f,
                        "color=c=black:s={w}x{h}:r={fps}:d={d},format=yuv420p,hwupload[{label}];",
                        d = secs(seg_dur),
                    )
                        .unwrap();
                    }
                }
                vsegs.push(label);
            }
            // Pad the base to full length with black when it ends early.
            let base_end = edl
                .track_segments(track_id)
                .map(|s| s.timeline_out_ms)
                .max()
                .unwrap_or(0);
            if base_end < edl.duration_ms {
                let label = format!("g{}", vsegs.len());
                let pad = edl.duration_ms - base_end;
                writeln!(
                    f,
                    "color=c=black:s={w}x{h}:r={fps}:d={d},format=yuv420p,hwupload[{label}];",
                    d = secs(pad),
                )
                .unwrap();
                vsegs.push(label);
            }
        }
    }

    let mut video_out = if vsegs.is_empty() {
        writeln!(
            f,
            "color=c=black:s={w}x{h}:r={fps}:d={d},format=yuv420p,hwupload[gvout];",
            d = secs(edl.duration_ms.max(1)),
        )
        .unwrap();
        "gvout".to_string()
    } else if vsegs.len() == 1 {
        vsegs[0].clone()
    } else {
        for l in &vsegs {
            write!(f, "[{l}]").unwrap();
        }
        writeln!(f, "concat=n={}:v=1:a=0[gvout];", vsegs.len()).unwrap();
        "gvout".to_string()
    };

    // --- overlay tracks (2b-ii): mirror the SOFTWARE overlay build (full-length
    // transparent-backed yuva420p streams — same construction as build_graph) but
    // composite with overlay_cuda over the GPU base. Each overlay is built on the
    // CPU (it is small: conform + PiP transform + opacity via colorchannelmixer +
    // transparent fillers for gaps/edges), then ONE hwupload per track feeds
    // overlay_cuda=0:0 (the PiP position is already baked into the full-frame
    // stream by `pad`). The 4K base never leaves the GPU — that is the win.
    for (ti, track_plan) in video_tracks.iter().enumerate().skip(1) {
        let track_id = track_plan.id;
        let mut olabels: Vec<String> = Vec::new();
        let mut cursor: u64 = 0; // timeline position covered so far
        let filler = |from: u64, to: u64, f: &mut String, olabels: &mut Vec<String>| {
            if to > from {
                // INLINE color source keeps the alpha (a `-i` color input would be
                // pre-converted to yuv420p, dropping it) — verified on overlay_cuda.
                let label = format!("og{ti}_f{}", olabels.len());
                writeln!(
                    f,
                    "color=c=black@0.0:s={w}x{h}:r={fps}:d={d},format=yuva420p[{label}];",
                    d = secs(to - from),
                )
                .unwrap();
                olabels.push(label);
            }
        };
        for seg in edl.track_segments(track_id) {
            let (Some(asset), Some(src_in), Some(src_out)) =
                (&seg.asset, seg.src_in_ms, seg.src_out_ms)
            else {
                continue; // gaps: transparent (handled by filler)
            };
            filler(cursor, seg.timeline_in_ms, &mut f, &mut olabels);
            let t = project
                .find_clip(seg.clip_id.as_deref().unwrap_or_default())
                .and_then(|(tid, i)| match &project.track(tid)?.clips[i] {
                    cut_core::Clip::Media(c) => c.transform.clone(),
                    _ => None,
                })
                .unwrap_or_else(cut_core::ClipTransform::identity);
            let (ow, oh) = (
                even_size_px(f64::from(w) * t.scale, w),
                even_size_px(f64::from(h) * t.scale, h),
            );
            let (ox, oy) = (
                even_pos_px(f64::from(w) * t.x, w.saturating_sub(ow)),
                even_pos_px(f64::from(h) * t.y, h.saturating_sub(oh)),
            );
            // Keyframed OPACITY (edit.keyframe param=opacity) on the GPU-path overlay
            // (CPU-built) — same geq alpha as the software overlay; overrides static.
            let vopac = opacity_kf_filter(&seg.keyframes);
            let place = if t.is_identity() {
                String::new()
            } else {
                let mut sp =
                    format!(",scale={ow}:{oh},setsar=1,pad={w}:{h}:{ox}:{oy}:color=black@0.0");
                // opacity<1 scales the alpha plane; skipped when opacity is keyframed.
                if t.opacity < 1.0 && vopac.is_empty() {
                    let o = (t.opacity.clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
                    sp.push_str(&format!(",colorchannelmixer=aa={o}"));
                }
                sp
            };
            let vfade = fade_suffix(
                seg.fade.as_ref(),
                seg.timeline_out_ms - seg.timeline_in_ms,
                true,
                true,
            );
            let vcrop = crop_filter(seg.crop.as_ref());
            let conform = conform_filter(w, h, opts.fit);
            // Speed/freeze on a GPU-path overlay (CPU-built) — kept identical to the
            // software overlay (the gate routes any reversed/frozen timeline to
            // software, so this is forward-compat today, like effects above).
            let (trim_in, trim_out, vsetpts, vfreeze) = freeze_chain(
                seg.freeze.as_ref(),
                seg.speed,
                src_in,
                src_out,
                seg.timeline_out_ms - seg.timeline_in_ms,
                s.fps,
            );
            let vreverse = if seg.reverse && seg.freeze.is_none() {
                ",reverse"
            } else {
                ""
            };
            // Ken Burns (edit.animate) on a GPU-path overlay — forward-compat,
            // kept identical to the software overlay (gate routes it to software). A
            // SCALE keyframe takes the same slot (the eased multi-point form).
            let seg_dur_ms = seg.timeline_out_ms - seg.timeline_in_ms;
            let vanim = scale_kf_zoompan(&seg.keyframes, w, h, s.fps, seg_dur_ms)
                .unwrap_or_else(|| animate_filter(seg.animation.as_ref(), w, h, s.fps, seg_dur_ms));
            let vgrade = grade_stack_filter(seg.grade.as_ref(), &seg.grade_stack);
            // Effects on a GPU-path overlay (CPU-built): overlay=true. (The gate
            // currently routes any effects timeline to software, so this is a
            // forward-compat no-op today — kept identical to the software overlay.)
            let veffects = effect_filter(&seg.effects, true);
            let label = format!("og{ti}_s{}", olabels.len());
            writeln!(
                f,
                "[{idx}:v]trim=start={trim_in}:end={trim_out},{vsetpts}{vreverse}{vfreeze}{vcrop},\
                 {conform}{vanim}{vgrade}{veffects},fps={fps},format=yuva420p{vopac}{place}{vfade}[{label}];",
                idx = input_idx[asset.as_str()],
            )
            .unwrap();
            olabels.push(label);
            cursor = seg.timeline_out_ms;
        }
        filler(cursor, edl.duration_ms, &mut f, &mut olabels);
        if olabels.is_empty() {
            continue; // track listed but contributed nothing visible
        }
        let ostream = if olabels.len() == 1 {
            olabels[0].clone()
        } else {
            for l in &olabels {
                write!(f, "[{l}]").unwrap();
            }
            let cat = format!("og{ti}");
            writeln!(f, "concat=n={}:v=1:a=0[{cat}];", olabels.len()).unwrap();
            cat
        };
        // Normalize to yuva420p, then hwupload onto the same CUDA device as the
        // base so overlay_cuda composites (alpha-honored). The `format=yuva420p`
        // is REQUIRED: an opacity<1 segment ends in `colorchannelmixer=aa`, which
        // emits rgba — software `overlay` accepts that, but `overlay_cuda` rejects
        // any non-yuva overlay ("Unsupported overlay input format: rgba"). It also
        // unifies any format concat negotiated across mixed segments. (rgba→yuva420p
        // preserves the scaled alpha — verified.) eof_action=pass: frame-count
        // rounding must never stall the base.
        let oup = format!("og{ti}u");
        writeln!(f, "[{ostream}]format=yuva420p,hwupload[{oup}];").unwrap();
        let composed = format!("gvo{ti}");
        writeln!(
            f,
            "[{video_out}][{oup}]overlay_cuda=0:0:eof_action=pass[{composed}];"
        )
        .unwrap();
        video_out = composed;
    }

    // Prepend the CUDA video chain to the (software) audio chain.
    let filter = format!("{f}{}", audio.filter);
    Ok(Graph {
        inputs,
        filter,
        video_out,
        audio_out: audio.audio_out,
        _ass_dir: None,
    })
}

/// NVENC encode args for the GPU fast-track, mapped from the quality preset.
/// No `-pix_fmt` (the input is already a CUDA surface; forcing a pixfmt would
/// trigger a hwdownload). cq tiers mirror the HW-encoder probe (draft/standard/
/// high = 32/27/23).
fn gpu_h264_args(preset: &RenderPreset) -> Vec<String> {
    let cq = match preset.name.as_str() {
        "draft" => "32",
        "high" => "23",
        _ => "27", // standard (default)
    };
    [
        "-c:v",
        "h264_nvenc",
        "-preset",
        "p5",
        "-tune",
        "hq",
        "-rc",
        "vbr",
        "-cq",
        cq,
        "-b:v",
        "0",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// GPU fast-track render (the CUDA branch of render.final). Builds the GPU
/// filtergraph (NVDEC + scale_cuda + concat), encodes with nvenc, and records
/// `pipeline: "gpu"` on the output. The gate ([`render_target`]) has already
/// confirmed opt-in + probe + a v1-scope timeline, so this path is reached only
/// when build_graph_gpu can render it. NOT deterministic (GPU output varies by
/// driver/hardware) — so it does NOT carry the software path's byte-identical
/// replay guarantee. `out` is already fenced by the caller.
fn render_gpu(
    project: &Project,
    edl: &Edl,
    fence: &PathFence,
    out: &Path,
    preset: &RenderPreset,
    opts: RenderOptions,
    on_progress: Option<ProgressFn>,
) -> Result<RenderOutput, CutError> {
    let graph = build_graph_gpu(project, edl, fence.project_dir(), opts)?;
    let mut args = graph_args(&graph, &graph.video_out, graph.audio_out.as_deref());
    args.extend(gpu_h264_args(preset)); // nvenc — NOT the software x264 preset
    if graph.audio_out.is_some() {
        args.extend(preset.audio_args.iter().cloned()); // audio stays software
    }
    args.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    args.push(out.display().to_string());

    // Progress streamed via -progress pipe:1; no-op closure when not wanted.
    let cb: ProgressFn = on_progress.unwrap_or_else(|| Box::new(|_| {}));
    run_ffmpeg_with_progress(&args, edl.duration_ms, cb.as_ref())?;

    let (gw, gh) = opts.output_geometry(project, edl);
    let non_default = opts != RenderOptions::default();
    Ok(RenderOutput {
        pipeline: Some("gpu".into()),
        width: non_default.then_some(gw),
        height: non_default.then_some(gh),
        fit: non_default.then(|| opts.fit.as_str().to_string()),
        hash: sha256_file(out)?,
        duration_ms: crate::probe::probe(out)?.duration_ms.ok_or_else(|| {
            CutError::new(
                error_codes::FFMPEG,
                "GPU render output has no measurable duration",
                "ffprobe reported no duration on the encoded file — encode is broken",
            )
        })?,
        path: out.to_path_buf(),
        preset: preset.name.clone(),
    })
}

/// Render the full composition to `out_path` with `preset` (public verb contract
/// `render.final`). `fence` enforces the output-path contract and
/// provides the project dir for asset resolution. Long-running: the server
/// wraps this in a job and passes `on_progress` to stream job_progress
/// events. Returns measured facts; the SERVER then runs verify.checks and
/// assembles the RenderReceipt.
pub fn render_final(
    project: &Project,
    edl: &Edl,
    fence: &PathFence,
    out_path: &Path,
    preset: &RenderPreset,
    opts: RenderOptions,
    on_progress: Option<ProgressFn>,
) -> Result<RenderOutput, CutError> {
    let out = fence.fence_output_path(out_path)?; // the output-fencing contract — before any work
    if edl.duration_ms == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "timeline is empty — nothing to render",
            "EDL duration is 0 ms",
        )
        .with_suggested_action("insert at least one clip before render.final"));
    }
    // GPU fast-track gate. Checked BEFORE the segmentation branch so the
    // opt-in is honored on every render path. Selecting CUDA means the gate has
    // confirmed opt-in + the CUDA probe + a v1-scope timeline + a VRAM-fitting
    // estimate, so the GPU graph (build_graph_gpu) renders it; the default +
    // non-GPU boxes + out-of-scope/over-VRAM timelines fall through to the software
    // path below, byte-identical to a pre-fast-track render.
    if render_target(project, edl, &opts) == GraphTarget::Cuda {
        return render_gpu(project, edl, fence, &out, preset, opts, on_progress);
    }
    // Heavy composites render SEGMENTED (Stage 3): the single-pass graph below
    // builds overlay tracks as continuous full-length alpha streams + hands the
    // whole composition to one ffmpeg, so memory grows with timeline LENGTH ×
    // resolution × overlays and OOMs the box. render_segmented renders in
    // time-windows (bounded memory) and concats. Light composites keep the
    // single pass so their output stays byte-identical to a pre-Stage-3 render.
    let (gw, gh) = opts.output_geometry(project, edl);
    if should_segment(project, edl, gw, gh) {
        return render_segmented(project, edl, fence, &out, preset, opts, on_progress);
    }
    // Stabilization detect PRE-PASS (edit.stabilize): cache each stabilized clip's
    // `.trf` so build_graph can reference it (segmented renders prepare per-window,
    // since a window's source sub-range has its own .trf).
    prepare_stabilization(project, edl, fence.project_dir())?;
    let graph = build_graph(
        project,
        edl,
        fence.project_dir(),
        true,
        true,
        true,
        opts,
        None,
    )?;

    let mut args = graph_args(&graph, &graph.video_out, graph.audio_out.as_deref());
    args.extend(preset.video_args.iter().cloned());
    // Output color tagging (project.color output ≠ rec709): make the delivered file's
    // color space explicit in the container. Empty (no flags) on the default rec709
    // output, so a default render stays byte-identical.
    args.extend(output_color_args(&project.settings.color));
    if graph.audio_out.is_some() {
        args.extend(preset.audio_args.iter().cloned());
    }
    args.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    args.push(out.display().to_string());

    // Progress streamed via -progress pipe:1; no-op closure when not wanted.
    let cb: ProgressFn = on_progress.unwrap_or_else(|| Box::new(|_| {}));
    run_ffmpeg_with_progress(&args, edl.duration_ms, cb.as_ref())?;

    // Record geometry/fit only when NON-default — a default render
    // (contain + project geometry) leaves these None so its receipt stays
    // byte-identical to an older receipt without crop metadata.
    let (gw, gh) = opts.output_geometry(project, edl);
    let non_default = opts != RenderOptions::default();
    Ok(RenderOutput {
        pipeline: None,
        width: non_default.then_some(gw),
        height: non_default.then_some(gh),
        fit: non_default.then(|| opts.fit.as_str().to_string()),
        hash: sha256_file(&out)?,
        // A rendered mp4 always has a measurable duration; None would mean a
        // broken encode — surface it, never report 0 as fact.
        duration_ms: crate::probe::probe(&out)?.duration_ms.ok_or_else(|| {
            CutError::new(
                error_codes::FFMPEG,
                "render output has no measurable duration",
                "ffprobe reported no duration on the encoded file — encode is broken",
            )
        })?,
        path: out,
        preset: preset.name.clone(),
    })
}

// === Segmented rendering (Stage 3) — the real memory bound ===================
//
// A long composite OOMs because build_graph builds overlay tracks as continuous
// FULL-LENGTH alpha streams and hands the whole timeline to ONE ffmpeg, so peak
// memory grows with timeline length. render_segmented renders the VIDEO in
// time-windows (each window's sub-EDL → build_graph holds only that window's
// clips → bounded memory), concats the windowed video, then renders the AUDIO in
// ONE cheap audio-only pass (no overlay cost) and muxes. Splitting video vs
// audio this way bounds the heavy part AND sidesteps AAC-gapless-concat clicks
// (audio is never cut at a window seam). Captions ride inside each window's video
// (build_graph reads them from the EDL, which Edl::window rebased), so there is
// no separate caption encode.

/// Overlay video tracks with clips (every video track ABOVE the base canvas).
/// These are the memory driver: build_graph builds each as a continuous
/// full-length alpha stream, so peak RSS ≈ window_len × frame_size × overlays.
fn overlay_track_count(project: &Project) -> usize {
    planned_video_tracks(project).len().saturating_sub(1) // first planned track is the non-empty base canvas
}

/// Adaptive segment window size so each window's peak RSS stays near a fixed
/// budget REGARDLESS of resolution / overlay count — the safety that protects a
/// small box rendering 4K. Measured: a 720p window with 2 overlays costs
/// ~32 MB per (window-second × overlay × 720p-frame-unit); peak scales with
/// window_len × resolution × overlays. So window_sec = budget / (32 × res_factor
/// × overlays), clamped 2..=60 s. At 4K (9× the pixels) with 2 overlays this
/// picks ~2 s windows (~1.2 GB) instead of 30 s (~17 GB → OOM on a 16 GB Mac).
///
/// `SHELLX_CUT_RENDER_WINDOW_SEC` hard-overrides the result (tests + manual
/// control); `SHELLX_CUT_RENDER_WINDOW_BUDGET_MB` tunes the per-window budget
/// (default 1500, clamp 256..=8192).
fn render_window_ms(gw: u32, gh: u32, overlays: usize) -> u64 {
    if let Some(secs) = std::env::var("SHELLX_CUT_RENDER_WINDOW_SEC")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| (2..=300).contains(n))
    {
        return secs * 1000;
    }
    let budget_mb = std::env::var("SHELLX_CUT_RENDER_WINDOW_BUDGET_MB")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|n| (256.0..=8192.0).contains(n))
        .unwrap_or(1500.0);
    let res_factor = ((gw as f64 * gh as f64) / (1280.0 * 720.0)).max(0.1);
    let ov = overlays.max(1) as f64;
    let per_sec = 32.0 * res_factor * ov; // MB per window-second (calibrated)
    let win_sec = (budget_mb / per_sec).floor().clamp(2.0, 60.0);
    (win_sec as u64) * 1000
}

/// Whether render.final should take the SEGMENTED path. The memory blow-up is
/// driven by OVERLAY tracks (continuous full-length alpha streams), so we
/// segment when the composite has overlays AND runs longer than one adaptive
/// window — this engages early at 4K (tiny windows) and never for short clips.
/// Base-only timelines don't OOM (no alpha streams; preview proved 20 min in
/// 78 s), so they segment only past a long-form safety ceiling (10 min).
///
/// `SHELLX_CUT_RENDER_SEGMENT_SEC` overrides: `0` forces segmentation (tests
/// prove seam-correctness on a short timeline), a large value disables it.
fn should_segment(project: &Project, edl: &Edl, gw: u32, gh: u32) -> bool {
    if let Some(thresh_sec) = std::env::var("SHELLX_CUT_RENDER_SEGMENT_SEC")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
    {
        return edl.duration_ms > thresh_sec * 1000;
    }
    let overlays = overlay_track_count(project);
    let win_ms = render_window_ms(gw, gh, overlays);
    (overlays >= 1 && edl.duration_ms > win_ms) || edl.duration_ms > 600_000
}

/// Plan contiguous render windows over `[0, duration)`: ~`target_ms` each,
/// snapped to the frame grid (a mid-clip split is then frame-accurate → no
/// duplicated/dropped frame at the seam) and nudged so no boundary lands inside
/// a base-track crossfade DISSOLVE region (which needs both sides in one window
/// to render). Returns `[(w0,w1), …]` tiling the timeline with no gap/overlap.
fn plan_windows(edl: &Edl, target_ms: u64, fps: f64) -> Vec<(u64, u64)> {
    let dur = edl.duration_ms;
    if dur == 0 {
        return vec![];
    }
    if dur <= target_ms {
        return vec![(0, dur)]; // single window — nothing to split
    }
    let frame_ms = (1000.0 / fps).max(1.0);
    let snap = |t: u64| -> u64 {
        let f = (t as f64 / frame_ms).round() * frame_ms;
        (f.round() as u64).min(dur)
    };
    // Dissolve regions [tl_in, tl_in+xfade) a boundary must not split.
    let mut xfades: Vec<(u64, u64)> = edl
        .segments
        .iter()
        .filter(|s| s.xfade_in_ms > 0)
        .map(|s| {
            (
                s.timeline_in_ms,
                s.timeline_in_ms.saturating_add(s.xfade_in_ms).min(dur),
            )
        })
        .filter(|(x0, x1)| x1 > x0)
        .collect();
    xfades.sort_unstable();
    let nudge = |mut b: u64| -> u64 {
        // Push a boundary that fell strictly inside a dissolve just past it.
        // Re-check until stable: a nudge can land inside a later or overlapping
        // dissolve range that was already inspected in the original segment order.
        b = b.min(dur);
        loop {
            let mut moved = false;
            for (x0, x1) in &xfades {
                if b > *x0 && b < *x1 {
                    b = *x1;
                    moved = true;
                }
            }
            if !moved {
                return b;
            }
        }
    };
    let mut windows = Vec::new();
    let mut start = 0u64;
    while start < dur {
        let mut end = nudge(snap(start + target_ms));
        if end <= start {
            end = dur; // always make progress (degenerate fps / tiny tail)
        }
        windows.push((start, end));
        start = end;
    }
    windows
}

/// Content hash for a window's video segment — folds in everything that changes
/// the rendered bytes (the windowed sub-EDL, the referenced assets' content
/// hashes, output geometry, fps, and the video preset). An unchanged window
/// hashes identically → its cached `seg_<hash>.mp4` is reused, so re-rendering
/// after an edit only re-renders the touched windows.
fn window_segment_key(
    project: &Project,
    wedl: &Edl,
    gw: u32,
    gh: u32,
    fps: f64,
    preset: &RenderPreset,
) -> String {
    let mut h = Sha256::new();
    h.update(serde_json::to_vec(wedl).unwrap_or_default());
    h.update(format!("{gw}x{gh}@{fps}").as_bytes());
    h.update(format!("{:?}", preset.video_args).as_bytes());
    // Referenced assets by content hash (a re-imported / changed source busts it).
    let mut ids: Vec<&str> = wedl
        .segments
        .iter()
        .filter_map(|s| s.asset.as_deref())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        if let Some(a) = project.assets.get(id) {
            h.update(id.as_bytes());
            h.update(a.hash.as_bytes());
        }
    }
    format!("seg_{:x}", h.finalize())[..20].to_string()
}

/// Render ONE window's VIDEO (captions burned, overlays composited, NO audio) to
/// `seg_path`, governed (cgroup MemoryHigh). `opts` already pins explicit
/// geometry so every window encodes identically (the concat demuxer needs it).
#[allow(clippy::too_many_arguments)]
fn render_window_video(
    project: &Project,
    wedl: &Edl,
    project_dir: &Path,
    seg_path: &Path,
    preset: &RenderPreset,
    opts: RenderOptions,
    caps: WindowCaps,
    on_progress: &dyn Fn(f32),
) -> Result<(), CutError> {
    // with_captions=true (from the EDL), with_audio=false (audio is single-pass),
    // with_video=true.
    // Stabilization detect for THIS window's source sub-ranges (a clip split across
    // windows has a distinct .trf per window; idempotent + cached).
    prepare_stabilization(project, wedl, project_dir)?;
    let graph = build_graph(project, wedl, project_dir, true, false, true, opts, None)?;
    let mut args = graph_args(&graph, &graph.video_out, None);
    args.extend(preset.video_args.iter().cloned());
    // Tag each segmented-render window with the project output color space so the
    // concat (stream-copy) carries it into the final file. Empty on default rec709.
    args.extend(output_color_args(&project.settings.color));
    args.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    args.push(seg_path.display().to_string());
    // Per-window cgroup cap + thread share so N windows run concurrently without
    // collectively exceeding the RAM budget. See plan_seg_resources.
    crate::ffmpeg::run_render_window(
        &args,
        wedl.duration_ms,
        on_progress,
        caps.high,
        caps.max,
        caps.threads,
    )
}

/// Per-window governance caps for a parallel segmented render.
#[derive(Clone, Copy)]
struct WindowCaps {
    high: u64,
    max: u64,
    threads: u32,
}

/// Two-term per-window peak-RSS model in MB: a fixed per-stream buffer plus a
/// per-second term, scaled by resolution and overlays. It sizes the cgroup cap
/// and parallel fan-out; the cgroup soft limit remains the backstop.
fn per_window_mem_mb(window_ms: u64, gw: u32, gh: u32, overlays: usize) -> f64 {
    let rf = ((gw as f64 * gh as f64) / (1280.0 * 720.0)).max(0.1);
    let ov = overlays.max(1) as f64;
    let sec = (window_ms as f64) / 1000.0;
    11.1 * sec * rf * ov + 222.0 * rf * ov
}

/// How many windows to render CONCURRENTLY + how to cap each.
///
/// DEFAULT = 1 (serial), and that is deliberate. The intuition "spare RAM → run
/// many windows → faster" does NOT hold: rendering the same total frames is bound
/// by the SHARED GPU NVENC encoder + memory bandwidth, not RAM. Measured on a
/// 5080 box, fanning out to a RAM budget gave only **1.1–1.25× for 3–10× the RAM**
/// (1080p/90s: 1.24× @ 10 GB; 4K/48s: 1.12× @ 17 GB) — a poor trade, and aggressive
/// RAM use would starve the desktop app that shares the box. The real win was the
/// memory BOUND (Stage 3), not consuming the slack.
///
/// So parallelism is OPT-IN for users with an idle big box who want the modest
/// gain: `SHELLX_CUT_RENDER_PARALLEL=N` (explicit count) or
/// `SHELLX_CUT_RENDER_RAM_PCT=P` (size N to ~P% of RAM). Bounding each window
/// (its own cgroup cap = budget/N) is what keeps the opt-in path SAFE. Linux only
/// (needs the cgroup soft-limit to bound the collective); else serial.
fn plan_seg_resources(window_ms: u64, gw: u32, gh: u32, overlays: usize) -> (usize, WindowCaps) {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let per_window_mb = per_window_mem_mb(window_ms, gw, gh, overlays).max(256.0);
    let ram = crate::ffmpeg::total_ram_bytes();
    let governed = crate::ffmpeg::cgroup_governance_available();

    let env_par = std::env::var("SHELLX_CUT_RENDER_PARALLEL")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1);
    let env_pct = std::env::var("SHELLX_CUT_RENDER_RAM_PCT")
        .ok()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|n| (10.0..=90.0).contains(n));

    let parallelism = if let Some(p) = env_par {
        p.min(cores).max(1)
    } else if let Some(pct) = env_pct {
        // Opt-in: size the fan-out to ~pct% of RAM (Linux + cgroup only).
        if ram == 0 || !governed {
            1
        } else {
            let budget_mb = (ram as f64 / (1024.0 * 1024.0)) * (pct / 100.0);
            let by_ram = (budget_mb / per_window_mb).floor() as usize;
            by_ram.clamp(1, cores.min(8))
        }
    } else {
        1 // DEFAULT: serial — the bounded render. Parallelism is opt-in (see doc).
    };

    // Per-window cgroup cap = the budget share with headroom over the estimate so
    // an accurate window does NOT throttle; a mis-estimate spills (safe). Floor at
    // 512 MB. N windows each ≤ high ⇒ collective ≤ N×high ≈ the budget.
    let one_mb = 1024 * 1024;
    let high = ((per_window_mb * 1.3) as u64)
        .saturating_mul(one_mb)
        .max(512 * one_mb);
    let max = high + 512 * one_mb;
    let threads = (cores / parallelism).max(1) as u32;
    (parallelism, WindowCaps { high, max, threads })
}

/// Render a long composition in time-windows so peak memory is bounded by the
/// window size, not the timeline length (Stage 3). `out` is already fenced by
/// the caller (render.final). See the module comment above for the pipeline.
pub fn render_segmented(
    project: &Project,
    edl: &Edl,
    fence: &PathFence,
    out: &Path,
    preset: &RenderPreset,
    opts: RenderOptions,
    on_progress: Option<ProgressFn>,
) -> Result<RenderOutput, CutError> {
    let cb: ProgressFn = on_progress.unwrap_or_else(|| Box::new(|_| {}));
    let project_dir = fence.project_dir();
    let dur = edl.duration_ms;
    let fps = project.settings.fps;

    // Geometry resolved ONCE from the full EDL and pinned explicitly so every
    // window encodes at identical w×h (match_source must not pick a different
    // size per window — that would break concat). Loudness is dropped here and
    // applied on the single audio pass instead.
    let (gw, gh) = opts.output_geometry(project, edl);
    let win_opts = RenderOptions {
        fit: opts.fit,
        resolution: Resolution::Explicit {
            width: gw,
            height: gh,
        },
        loudness_target: None,
    };

    // Per-render working dir (segment cache + intermediates) under the project.
    let work = project_dir.join(".cache").join("segrender");
    std::fs::create_dir_all(&work)?;

    // --- 1. video windows (governed, content-hash cached, rendered in PARALLEL)
    let overlays = overlay_track_count(project);
    let win_ms = render_window_ms(gw, gh, overlays);
    let windows = plan_windows(edl, win_ms, fps);
    let (parallelism, caps) = plan_seg_resources(win_ms, gw, gh, overlays);
    let video_weight = 0.9_f32; // reserve 0.9..1.0 for the audio/mux pass

    // Deterministic per-window cache paths (independent of render ORDER) so the
    // concat below reads them in TIMELINE order no matter which finished first.
    let seg_files: Vec<PathBuf> = windows
        .iter()
        .map(|(w0, w1)| {
            let wedl = edl.window(*w0, *w1);
            let key = window_segment_key(project, &wedl, gw, gh, fps, preset);
            work.join(format!("{key}.mp4"))
        })
        .collect();
    let todo: Vec<usize> = (0..windows.len())
        .filter(|&i| !seg_files[i].exists())
        .collect();

    // Fan out: `parallelism` workers pull window indices off a shared cursor. Each
    // window is independent (own sub-EDL, own output, own cgroup share = budget/N),
    // so this is embarrassingly parallel — it's what turns bounded-per-window memory
    // into "use the whole box". Errors are collected; the first aborts the render.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let total_todo = todo.len().max(1);
    let errors: std::sync::Mutex<Vec<CutError>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..parallelism {
            s.spawn(|| {
                use std::sync::atomic::Ordering::SeqCst;
                loop {
                    let k = next.fetch_add(1, SeqCst);
                    if k >= todo.len() {
                        break;
                    }
                    let i = todo[k];
                    let (w0, w1) = windows[i];
                    let wedl = edl.window(w0, w1);
                    if let Err(e) = render_window_video(
                        project,
                        &wedl,
                        project_dir,
                        &seg_files[i],
                        preset,
                        win_opts,
                        caps,
                        &|_| {},
                    ) {
                        errors.lock().unwrap().push(e);
                    }
                    let d = done.fetch_add(1, SeqCst) + 1;
                    cb((d as f32 / total_todo as f32) * video_weight);
                }
            });
        }
    });
    if let Some(e) = errors.into_inner().unwrap().into_iter().next() {
        return Err(e);
    }

    // --- 2. concat the windowed video (stream-copy) ------------------------
    let list_path = work.join("concat.txt");
    let mut list = String::new();
    for p in &seg_files {
        writeln!(list, "{}", concat_demuxer_file_line(p)).unwrap();
    }
    std::fs::write(&list_path, &list)?;
    let video_only = work.join("video.mp4");
    let mut cargs: Vec<String> = vec![
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list_path.display().to_string(),
        "-c".into(),
        "copy".into(),
    ];
    cargs.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    cargs.push(video_only.display().to_string());
    run_ffmpeg(&cargs)?;

    // --- 3. audio single-pass over the FULL EDL + mux ----------------------
    // Build the audio-only graph (no overlay cost). If the timeline has audio,
    // mux it onto the concatenated video (video stream-copied, audio encoded) in
    // one pass; otherwise the concatenated video IS the output.
    let agraph = build_graph(project, edl, project_dir, false, true, false, opts, None)?;
    if let Some(aout) = agraph.audio_out.clone() {
        // Inputs: the audio graph's assets (indices match its [i:a] refs), then
        // the concatenated video LAST so the audio indices are unshifted.
        let mut margs: Vec<String> = Vec::new();
        for input in &agraph.inputs {
            if input.image {
                margs.push("-loop".into());
                margs.push("1".into());
            }
            margs.push("-i".into());
            margs.push(input.path.display().to_string());
        }
        let vid_idx = agraph.inputs.len();
        margs.push("-i".into());
        margs.push(video_only.display().to_string());
        margs.push("-filter_complex".into());
        margs.push(agraph.filter.trim_end_matches(['\n', ';']).to_string());
        margs.push("-map".into());
        margs.push(format!("[{aout}]"));
        margs.push("-map".into());
        margs.push(format!("{vid_idx}:v"));
        margs.push("-c:v".into());
        margs.push("copy".into());
        margs.extend(preset.audio_args.iter().cloned());
        margs.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
        margs.push(out.display().to_string());
        run_ffmpeg_with_progress(
            &margs,
            dur,
            &|p| cb(video_weight + p * (1.0 - video_weight)),
        )?;
    } else {
        // No audio tracks → the concatenated video is the deliverable. Re-mux to
        // the fenced output path (stream-copy, near-instant).
        let mut mv: Vec<String> = vec![
            "-i".into(),
            video_only.display().to_string(),
            "-c".into(),
            "copy".into(),
        ];
        mv.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
        mv.push(out.display().to_string());
        run_ffmpeg(&mv)?;
    }
    cb(1.0);

    // --- 4. measured facts for the receipt ---------------------------------
    let (gw2, gh2) = opts.output_geometry(project, edl);
    let non_default = opts != RenderOptions::default();
    Ok(RenderOutput {
        pipeline: None,
        width: non_default.then_some(gw2),
        height: non_default.then_some(gh2),
        fit: non_default.then(|| opts.fit.as_str().to_string()),
        hash: sha256_file(out)?,
        duration_ms: crate::probe::probe(out)?.duration_ms.ok_or_else(|| {
            CutError::new(
                error_codes::FFMPEG,
                "segmented render output has no measurable duration",
                "ffprobe reported no duration on the muxed file — encode is broken",
            )
        })?,
        path: out.to_path_buf(),
        preset: preset.name.clone(),
    })
}

/// Auto-reframe a rendered video to a target aspect via a subject-tracked moving
/// crop — the reframe POST-PASS (perception contract reframe rework, the industry "new sequence
/// from the finished edit" model).
///
/// `input` is the already-rendered edit (full project aspect); `frames` are the
/// per-frame subject observations (mapped from the `subject` instrument's
/// SubjectTrack — see cut_perception). This walks the deterministic damped-spring
/// (`reframe::crop_path`), writes an ffmpeg `sendcmd` script, and runs ONE
/// crop+scale pass. AUDIO IS COPIED — the input already carries the final mix, so
/// reframing is video-only and lossless on audio. The original `input` is never
/// touched; the output is a derived artifact.
///
/// CONTRACT: `frames`/`scene_starts` MUST come from analyzing THIS `input` (same
/// fps + frame count) so the per-frame sendcmd timing (`f / fps`) aligns with the
/// video. The crop runs in INPUT pixels; the subject track's normalized coords map
/// regardless of the resolution it was analyzed at (e.g. a proxy of `input`).
/// `out_path` must already be fenced by the caller (the verb).
#[allow(clippy::too_many_arguments)]
pub fn reframe_video(
    input: &Path,
    out_path: &Path,
    frames: &[crate::reframe::FrameObs],
    aspect_w: u32,
    aspect_h: u32,
    out_w: u32,
    out_h: u32,
    params: &crate::reframe::ReframeParams,
    scene_starts: &[u32],
    preset: &RenderPreset,
    on_progress: Option<ProgressFn>,
) -> Result<RenderOutput, CutError> {
    let p = crate::probe::probe(input)?;
    let (in_w, in_h) = (p.width.unwrap_or(0), p.height.unwrap_or(0));
    if in_w == 0 || in_h == 0 {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "reframe input has no video geometry",
            "ffprobe reported no width/height on the input",
        ));
    }
    let fps = p.fps.unwrap_or(30.0);
    let dur_ms = p.duration_ms.unwrap_or(0);

    // Deterministic spring → per-frame crop rects (input px) → sendcmd script.
    let rects =
        crate::reframe::crop_path(frames, in_w, in_h, aspect_w, aspect_h, params, scene_starts);
    // crop_path emits a CONSTANT-size window (only x/y pan per frame — a per-frame
    // crop SIZE change stalls the downstream scale). Take the fixed size + initial
    // position from the first rect (all rects share w/h). Empty timeline is already
    // guarded by the caller; fall back to the full frame if somehow empty.
    let (cw, ch, x0, y0) = rects
        .first()
        .map(|r| (r.w, r.h, r.x, r.y))
        .unwrap_or((in_w, in_h, 0, 0));
    let script = crate::reframe::sendcmd_script(&rects, fps);

    // sendcmd staged on disk; clean up after.
    let cmds_path = out_path.with_extension("reframe.sendcmd.txt");
    std::fs::write(&cmds_path, script)?;

    // The crop window is a FIXED size (cw×ch); sendcmd only pans its x/y each frame.
    // scale conforms it to the fixed output; setsar=1 keeps square pixels.
    // Lanczos for the downscale.
    let vf = format!(
        "sendcmd=f={},crop={cw}:{ch}:{x0}:{y0},scale={out_w}:{out_h}:flags=lanczos,setsar=1",
        escape_filter_path(&cmds_path)
    );
    let mut args: Vec<String> = vec![
        "-i".into(),
        input.display().to_string(),
        "-vf".into(),
        vf,
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a?".into(),
        "-c:a".into(),
        "copy".into(), // final mix already encoded — copy, don't re-encode
    ];
    args.extend(preset.video_args.iter().cloned());
    args.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    args.push(out_path.display().to_string());

    let cb: ProgressFn = on_progress.unwrap_or_else(|| Box::new(|_| {}));
    let result = run_ffmpeg_with_progress(&args, dur_ms, cb.as_ref());
    let _ = std::fs::remove_file(&cmds_path); // best-effort cleanup either way
    result?;

    Ok(RenderOutput {
        pipeline: None,
        width: Some(out_w),
        height: Some(out_h),
        fit: Some(format!("reframe:{aspect_w}:{aspect_h}")),
        hash: sha256_file(out_path)?,
        duration_ms: crate::probe::probe(out_path)?.duration_ms.ok_or_else(|| {
            CutError::new(
                error_codes::FFMPEG,
                "reframe output has no measurable duration",
                "ffprobe reported no duration on the encoded file — encode is broken",
            )
        })?,
        path: out_path.to_path_buf(),
        preset: preset.name.clone(),
    })
}

/// Render the timeline's MIXED AUDIO ONLY (no video) to `out_path` with the given
/// audio codec args — the engine behind export.audio (mp3/m4a/wav/flac/opus).
/// It reuses the EXACT audio graph render_final builds (per-track atrim+concat →
/// amix, with gain / fades / speed) via build_graph(with_video=false), which drops
/// the video chains entirely — so an audio export pays no video-filter cost.
/// Long-running on a big timeline → `on_progress` streams job_progress. `out_path`
/// must already be fenced by the caller (the output-fencing contract).
pub fn render_audio(
    project: &Project,
    edl: &Edl,
    fence: &PathFence,
    out_path: &Path,
    audio_args: &[String],
    on_progress: Option<ProgressFn>,
) -> Result<RenderOutput, CutError> {
    let out = fence.fence_output_path(out_path)?; // the output-fencing contract — before any work
    if edl.duration_ms == 0 {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "timeline is empty — nothing to export",
            "EDL duration is 0 ms",
        )
        .with_suggested_action("insert at least one clip before export.audio"));
    }
    // with_captions=false, with_audio=true, with_video=false → audio-only graph.
    let graph = build_graph(
        project,
        edl,
        fence.project_dir(),
        false,
        true,
        false,
        RenderOptions::default(),
        None,
    )?;
    let aout = graph.audio_out.clone().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "timeline has no audio to export",
            "no audio-bearing clip produced an audio stream",
        )
        .with_suggested_action("place a clip with sound on an audio track, then export.audio")
    })?;
    // inputs (indices must match the filter — keep ALL, no -loop since the audio
    // graph never consumes an image's video) + filter_complex + map ONLY [aout] +
    // -vn + the chosen audio codec.
    let mut args: Vec<String> = Vec::new();
    for input in &graph.inputs {
        args.push("-i".into());
        args.push(input.path.display().to_string());
    }
    args.push("-filter_complex".into());
    args.push(graph.filter.trim_end_matches(['\n', ';']).to_string());
    args.push("-map".into());
    args.push(format!("[{aout}]"));
    args.push("-vn".into());
    args.extend(audio_args.iter().cloned());
    // Clean, reproducible export: strip metadata + bitexact audio (no video flags).
    args.extend(
        [
            "-map_metadata",
            "-1",
            "-fflags",
            "+bitexact",
            "-flags:a",
            "+bitexact",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args.push(out.display().to_string());

    let cb: ProgressFn = on_progress.unwrap_or_else(|| Box::new(|_| {}));
    run_ffmpeg_with_progress(&args, edl.duration_ms, cb.as_ref())?;
    Ok(RenderOutput {
        pipeline: None,
        width: None,
        height: None,
        fit: None,
        hash: sha256_file(&out)?,
        duration_ms: crate::probe::probe(&out)?
            .duration_ms
            .unwrap_or(edl.duration_ms),
        path: out,
        preset: "audio".into(),
    })
}

/// Fast low-res preview of `[at_ms, at_ms+duration_ms)` (public verb contract
/// render.preview). Proxy-grade encode into `out_dir` (server-owned temp
/// space, NOT a user path — no fence needed); returns the file path.
pub fn render_preview(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
    at_ms: u64,
    duration_ms: u64,
    out_dir: &Path,
) -> Result<PathBuf, CutError> {
    if at_ms >= edl.duration_ms {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("preview start {at_ms} ms is past the composition end"),
            format!("composition is {} ms long", edl.duration_ms),
        )
        .with_at_ms(at_ms));
    }
    std::fs::create_dir_all(out_dir)?;
    let out = out_dir.join(format!("preview_{at_ms}_{duration_ms}.mp4"));
    // Preview always uses default framing (contain + project geometry) — it is
    // a fast proxy-grade view, not the final framing decision.
    let mut graph = build_graph(
        project,
        edl,
        project_dir,
        true,
        true,
        true,
        RenderOptions::default(),
        None,
    )?;
    let end_ms = (at_ms + duration_ms).min(edl.duration_ms);

    // Window the composed streams, then downscale to proxy geometry.
    let v = graph.video_out.clone();
    write!(
        graph.filter,
        "[{v}]trim=start={s}:end={e},setpts=PTS-STARTPTS,\
         scale=960:540:force_original_aspect_ratio=decrease,pad=960:540:(ow-iw)/2:(oh-ih)/2[vprev];",
        s = secs(at_ms),
        e = secs(end_ms),
    )
    .unwrap();
    let audio_label = if let Some(a) = graph.audio_out.clone() {
        write!(
            graph.filter,
            "[{a}]atrim=start={s}:end={e},asetpts=PTS-STARTPTS[aprev];",
            s = secs(at_ms),
            e = secs(end_ms)
        )
        .unwrap();
        Some("aprev")
    } else {
        None
    };

    let mut args = graph_args(&graph, "vprev", audio_label);
    args.extend(
        [
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "28",
            "-pix_fmt",
            "yuv420p",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    if audio_label.is_some() {
        args.extend(
            ["-c:a", "aac", "-b:a", "128k"]
                .iter()
                .map(|s| s.to_string()),
        );
    }
    args.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    args.push(out.display().to_string());
    run_ffmpeg(&args)?;
    Ok(out)
}

/// Default scrub-frame height (px) when a caller omits `?h=`. 540 = the proxy
/// height, a natural preview size; the human scrubs at this, the agent can ask
/// for more via `h`. The width follows the proxy's 16:9 box (960×540).
pub const SCRUB_DEFAULT_HEIGHT: u32 = 540;

/// A resolved fast-scrub plan: seek THIS proxy file at THIS source
/// position to get the composed frame at the requested timeline time, WITHOUT
/// building the full composition graph. Valid only when the base-track segment
/// under the playhead is a plain media segment with a built proxy and no
/// per-position compositing that the fast path can't reproduce (overlay/PiP,
/// captions burn-in). The dispatch layer resolves this from the project+EDL;
/// `extract_scrub_frame` consumes it. When `plan_scrub_frame` returns None the
/// caller falls back to `extract_frame` (the exact composed path).
#[derive(Debug, Clone)]
pub struct ScrubPlan {
    /// Absolute path to the proxy file to seek.
    pub proxy_path: PathBuf,
    /// Source position (ms) to seek to inside the proxy — the proxy preserves
    /// source timestamps, so this is the clip's `src_in_ms` plus the offset of
    /// `at_ms` into the segment.
    pub src_pos_ms: u64,
    /// PROXY-space crop rect `[x,y,w,h]` to apply on the fast path (the proxy-crop contract), if
    /// the segment is cropped. Already mapped from the source-space edit.crop
    /// onto the letterboxed proxy grid (see `map_crop_to_proxy`) and made even
    /// for yuv420 — so the cropped clip scrubs the SAME framing the final render
    /// shows, without the slow composed path. None = no crop on this segment.
    pub proxy_crop: Option<[u32; 4]>,
}

/// Decide whether `at_ms` can be served by the FAST scrub path,
/// and if so resolve the proxy + source position. Returns None to signal "use
/// the exact composed path" (`extract_frame`).
///
/// FAST-PATH ELIGIBILITY (must ALL hold — anything else is correctness-unsafe
/// to approximate, so we fall back rather than show a wrong frame):
/// - the base video track has a MEDIA segment covering `at_ms` (not a gap),
/// - that segment's asset has a built proxy on disk,
/// - NO overlay video track contributes a visible segment at `at_ms` (PiP
///   compositing changes the pixels — the fast single-input seek can't show it),
/// - NO caption is burned in at `at_ms` (the human scrub omits captions by
///   design; an agent that needs the exact composed frame passes compose=1
///   which never calls this).
///
/// `proxy_rel_path` maps an asset id to its proxy path RELATIVE to project_dir
/// (the server reads `asset.proxy`); None when the asset has no proxy yet.
pub fn plan_scrub_frame(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
    at_ms: u64,
) -> Option<ScrubPlan> {
    if at_ms >= edl.duration_ms {
        return None;
    }
    let base = edl.base_video_track()?;
    let base_visible = match project.track(base) {
        Some(track) => track.visible,
        // Keep legacy/proxy scrub behavior for external EDLs without project
        // track metadata: only an explicit hidden track disables the fast path.
        None => true,
    };
    if !base_visible {
        return None;
    }
    // The base-track media segment covering at_ms (half-open [in, out)).
    let seg = edl
        .track_segments(base)
        .find(|s| s.asset.is_some() && s.timeline_in_ms <= at_ms && at_ms < s.timeline_out_ms)?;
    let asset_id = seg.asset.as_deref()?;
    let src_in = seg.src_in_ms?;
    // A caption burned in at this time → fall back (scrub-fast omits captions).
    let caption_here = edl.segments.iter().any(|s| {
        s.caption_text.is_some() && s.timeline_in_ms <= at_ms && at_ms < s.timeline_out_ms
    });
    if caption_here {
        return None;
    }
    // An overlay (non-base) video track with a visible media segment here →
    // fall back (the fast single-input seek can't composite a PiP overlay).
    let overlay_here = edl.segments.iter().any(|s| {
        s.track_kind == TrackKind::Video
            && s.track != base
            && segment_video_track_visible(project, s)
            && s.asset.is_some()
            && s.timeline_in_ms <= at_ms
            && at_ms < s.timeline_out_ms
    });
    if overlay_here {
        return None;
    }
    // Per-pixel effects the fast single-input proxy seek CANNOT reproduce → fall
    // back to the composed path (extract_frame), which applies them. A color
    // grade (eq / colortemperature / lut3d) or a fade changes the visible pixels;
    // the raw proxy seek would show the UNGRADED / un-faded frame, so a graded
    // clip's preview/frame looked identical to ungraded — a silent miss. Crop is
    // still handled below by mapping into proxy space; captions + PiP overlays
    // fell back above. Correctness over speed (same philosophy as the crop case).
    // ...and a retimed clip: the proxy is NOT retimed, so the fast path's
    // src_pos (below) would seek the wrong source moment at non-1.0 speed.
    if seg.grade.is_some()
        || !seg.grade_stack.is_empty() // a layered grade stack changes pixels too
        || !seg.grade_windows.is_empty() // a power window changes a REGION's pixels too
        || seg.fade.is_some()
        || (seg.speed - 1.0).abs() > 1e-6
    {
        return None;
    }
    let asset = project.assets.get(asset_id)?;
    let proxy_rel = asset.proxy.as_ref()?;
    let mut proxy_path = PathBuf::from(proxy_rel);
    if proxy_path.is_relative() {
        proxy_path = project_dir.join(proxy_path);
    }
    if !proxy_path.exists() {
        return None;
    }
    // A cropped clip (the proxy-crop contract): the crop rect is in SOURCE pixel space (e.g. 4K
    // coords) and does NOT map 1:1 onto the letterboxed 960×540 proxy. Rather
    // than fall back to the slow composed path (the proxy-crop regression
    // is the recommended fix for the most common real source — OBS bars — so a
    // cropped clip is exactly when fast scrub matters most), MAP the crop into
    // proxy coordinates and apply it on the fast path. The mapping needs the
    // source dimensions (from the asset probe). If those are unavailable we keep
    // the composed fallback — correctness over speed for the rare unprobed case.
    let proxy_crop = match seg.crop.as_ref() {
        None => None,
        Some(c) => {
            let (sw, sh) = source_dims(asset)?; // None → composed fallback
            Some(map_crop_to_proxy(c, sw, sh)?) // out-of-bounds → composed fallback
        }
    };
    // Source position = clip's src_in + how far at_ms is into the segment.
    let src_pos_ms = src_in + (at_ms - seg.timeline_in_ms);
    Some(ScrubPlan {
        proxy_path,
        src_pos_ms,
        proxy_crop,
    })
}

/// Source video dimensions from the asset's probe (`width`/`height`), if both
/// present. Returns None when the asset was imported without a video probe — the
/// scrub-crop mapping then can't be computed, and `plan_scrub_frame` keeps the
/// composed fallback.
fn source_dims(asset: &cut_core::Asset) -> Option<(u32, u32)> {
    let p = asset.probe.as_ref()?;
    let w = p.get("width").and_then(|v| v.as_u64())? as u32;
    let h = p.get("height").and_then(|v| v.as_u64())? as u32;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// Source video fps from the normalized probe. Missing/non-positive/NaN fps means
/// "unknown", so render keeps the explicit `fps=` normalizer.
fn source_fps(asset: &cut_core::Asset) -> Option<f64> {
    let fps = asset.probe.as_ref()?.get("fps")?.as_f64()?;
    (fps.is_finite() && fps > 0.0).then_some(fps)
}

/// Map a SOURCE-space crop rect onto the letterboxed proxy's pixel grid (the proxy-crop contract).
///
/// The proxy is built (proxy.rs) as
///   `scale=960:540:force_original_aspect_ratio=decrease, pad=960:540:center`
/// so the source `sw×sh` is scaled by `f = min(960/sw, 540/sh)` into a centered
/// `round(sw·f) × round(sh·f)` box with letterbox offsets
/// `pad_x = (960−round(sw·f))/2`, `pad_y = (540−round(sh·f))/2`. A source crop
/// `{x,y,w,h}` therefore maps to proxy coords:
///   `px = pad_x + round(x·f)`, `py = pad_y + round(y·f)`,
///   `pw = round(w·f)`,         `ph = round(h·f)`
/// clamped to the proxy frame and forced even (yuv420 chroma alignment). Returns
/// `[px,py,pw,ph]`, or None if the mapped rect is degenerate/out of bounds (then
/// the caller keeps the composed fallback rather than emit a bad crop).
fn map_crop_to_proxy(c: &cut_core::ClipCrop, sw: u32, sh: u32) -> Option<[u32; 4]> {
    let (pw_box, ph_box) = (
        crate::proxy::PROXY_WIDTH as f64,
        crate::proxy::PROXY_HEIGHT as f64,
    );
    let f = (pw_box / sw as f64).min(ph_box / sh as f64);
    let scaled_w = (sw as f64 * f).round();
    let scaled_h = (sh as f64 * f).round();
    let pad_x = ((pw_box - scaled_w) / 2.0).max(0.0);
    let pad_y = ((ph_box - scaled_h) / 2.0).max(0.0);
    // Map + round, then force even (crop filter on yuv420 needs even x/y/w/h).
    let even = |v: f64| ((v.round() as i64).max(0) as u32) & !1u32;
    let px = even(pad_x + c.x as f64 * f);
    let py = even(pad_y + c.y as f64 * f);
    let mut pw = even(c.w as f64 * f);
    let mut ph = even(c.h as f64 * f);
    // Clamp the rect to the proxy frame so crop= never exceeds the input.
    if px + pw > crate::proxy::PROXY_WIDTH {
        pw = (crate::proxy::PROXY_WIDTH.saturating_sub(px)) & !1u32;
    }
    if py + ph > crate::proxy::PROXY_HEIGHT {
        ph = (crate::proxy::PROXY_HEIGHT.saturating_sub(py)) & !1u32;
    }
    // Degenerate result → bail to the composed path (correctness over a bad frame).
    if pw < 2 || ph < 2 {
        return None;
    }
    Some([px, py, pw, ph])
}

/// FAST scrub frame: input-side fast-seek the proxy at the EDL-
/// mapped source position and emit one scaled JPEG. The `-ss` BEFORE `-i`
/// (input-side seek) is the whole point — ffmpeg jumps to the nearest keyframe
/// and decodes forward only to `src_pos`, which on the short-GOP proxy
/// (PROXY_GOP_FRAMES) is ≤1 s of frames. Contrast `extract_frame`, which
/// builds the full composition and decodes from t=0 (latency grows with the
/// playhead). Output is letterboxed proxy pixels scaled to height `h` (width
/// even-rounded to keep the proxy's aspect, yuv420 chroma alignment).
///
/// This is the HUMAN scrub path: captions/overlays are intentionally NOT
/// composited (the dispatch layer only routes here when `plan_scrub_frame`
/// confirmed none apply at this position). The agent's exact composed frame
/// stays `extract_frame` (compose=1).
pub fn extract_scrub_frame(plan: &ScrubPlan, h: u32) -> Result<Vec<u8>, CutError> {
    let dir = tempfile::tempdir().map_err(CutError::from)?;
    let out = dir.path().join("scrub.jpg");
    // Even target height; scale keeps aspect (width even-rounded for yuv420).
    let h = (h.max(2)) & !1u32;
    // the proxy-crop contract: a cropped clip now stays on the fast path. The crop is applied in
    // PROXY pixel space (already mapped from the source-space edit.crop by
    // plan_scrub_frame → map_crop_to_proxy), THEN the cropped frame is scaled to
    // the requested height `h`. So `h` is honored for cropped clips too (it was
    // silently ignored before, when crops forced the composed fallback). For an
    // UNCROPPED clip the filter is just the scale (the common, byte-identical
    // path). Transform/overlay/caption cases never reach here — plan_scrub_frame
    // returns None for those, keeping the composed fallback.
    let vf = match plan.proxy_crop {
        Some([x, y, cw, ch]) => format!("crop={cw}:{ch}:{x}:{y},scale=-2:{h}"),
        None => format!("scale=-2:{h}"),
    };
    let args: Vec<String> = vec![
        "-ss".into(),
        secs(plan.src_pos_ms),
        "-i".into(),
        plan.proxy_path.display().to_string(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        vf,
        "-c:v".into(),
        "mjpeg".into(),
        "-q:v".into(),
        "3".into(),
        "-f".into(),
        "image2".into(),
        out.display().to_string(),
    ];
    run_ffmpeg(&args)?;
    Ok(std::fs::read(&out)?)
}

/// Extract ONE composed frame at `at_ms` as JPEG bytes (public verb contract render.frame
/// — "agent's eyes without rendering"). Implemented as a 1-frame render of
/// the composed timeline (media-engine contract), captions included. Decodes the timeline
/// up to `at_ms` (no input-side seek — the composition, not an asset, is the
/// target); suitable for ordinary project lengths. This exact path currently
/// decodes from the beginning rather than seeking to a local segment.
/// This is the EXACT composed path (the agent's compose=1 / verify eyes); the
/// fast human-scrub path is `extract_scrub_frame` above.
/// Render a TIME WINDOW `[range_ms[0], range_ms[1])` of the composed timeline to
/// `out_path` at the chosen preset — the "save a part of the timeline as a new
/// asset" path (#export.range). Same composition as render_final (all effects
/// baked) but the final video/audio streams are trimmed to the window before
/// encode (mirrors extract_frame's single-frame trim, widened to a span).
#[allow(clippy::too_many_arguments)]
pub fn render_range(
    project: &Project,
    edl: &Edl,
    fence: &PathFence,
    out_path: &Path,
    preset: &RenderPreset,
    range_ms: [u64; 2],
    opts: RenderOptions,
    on_progress: Option<ProgressFn>,
) -> Result<RenderOutput, CutError> {
    let out = fence.fence_output_path(out_path)?; // the output-fencing contract — before any work
    let in_ms = range_ms[0];
    let out_ms = range_ms[1].min(edl.duration_ms);
    if out_ms <= in_ms {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "range is empty",
            format!(
                "[{in_ms}, {out_ms}) within composition {} ms",
                edl.duration_ms
            ),
        )
        .with_suggested_action("pick range_ms = [start, end) inside the timeline"));
    }
    prepare_stabilization(project, edl, fence.project_dir())?;
    let mut graph = build_graph(
        project,
        edl,
        fence.project_dir(),
        true,
        true,
        true,
        opts,
        None,
    )?;
    let (in_s, out_s) = (secs(in_ms), secs(out_ms));
    // Trim the composed video (+ audio) to the window, resetting PTS so the
    // output starts at 0.
    let v = graph.video_out.clone();
    write!(
        graph.filter,
        "[{v}]trim=start={in_s}:end={out_s},setpts=PTS-STARTPTS[vrange];"
    )
    .unwrap();
    let a_map = if let Some(a) = graph.audio_out.clone() {
        write!(
            graph.filter,
            "[{a}]atrim=start={in_s}:end={out_s},asetpts=PTS-STARTPTS[arange];"
        )
        .unwrap();
        Some("arange".to_string())
    } else {
        None
    };
    let mut args = graph_args(&graph, "vrange", a_map.as_deref());
    args.extend(preset.video_args.iter().cloned());
    if a_map.is_some() {
        args.extend(preset.audio_args.iter().cloned());
    }
    args.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    args.push(out.display().to_string());
    let cb: ProgressFn = on_progress.unwrap_or_else(|| Box::new(|_| {}));
    run_ffmpeg_with_progress(&args, out_ms - in_ms, cb.as_ref())?;
    let (gw, gh) = opts.output_geometry(project, edl);
    let non_default = opts != RenderOptions::default();
    Ok(RenderOutput {
        pipeline: None,
        width: non_default.then_some(gw),
        height: non_default.then_some(gh),
        fit: non_default.then(|| opts.fit.as_str().to_string()),
        hash: sha256_file(&out)?,
        duration_ms: crate::probe::probe(&out)?
            .duration_ms
            .unwrap_or(out_ms - in_ms),
        path: out,
        preset: preset.name.clone(),
    })
}

/// A solid-black JPEG at `width`×`height`. Shown when scrubbing PAST the
/// composition end (every NLE shows black in the empty region; render.frame used
/// to return a 422 there, which left the preview poster broken and its loading
/// spinner endless — a reported "endless rendering" bug on a short image timeline
/// whose ruler extends past the content).
pub fn black_frame_jpeg(width: u32, height: u32) -> Result<Vec<u8>, CutError> {
    let dir = tempfile::tempdir().map_err(CutError::from)?;
    let out = dir.path().join("black.jpg");
    let args: Vec<String> = vec![
        "-f".into(),
        "lavfi".into(),
        "-i".into(),
        format!("color=c=black:s={}x{}", width.max(2), height.max(2)),
        "-frames:v".into(),
        "1".into(),
        "-c:v".into(),
        "mjpeg".into(),
        "-q:v".into(),
        "7".into(),
        "-f".into(),
        "image2".into(),
        out.display().to_string(),
    ];
    run_ffmpeg(&args)?;
    Ok(std::fs::read(&out)?)
}

pub fn extract_frame(
    project: &Project,
    edl: &Edl,
    project_dir: &Path,
    at_ms: u64,
    preview_height: Option<u32>,
) -> Result<Vec<u8>, CutError> {
    if at_ms >= edl.duration_ms {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("frame position {at_ms} ms is past the composition end"),
            format!("composition is {} ms long", edl.duration_ms),
        )
        .with_at_ms(at_ms)
        .with_suggested_action("pick at_ms within [0, duration_ms)"));
    }
    // Compose only the WINDOW containing at_ms. Building the whole graph for one
    // deep frame processes 0..at_ms (the trim drops earlier frames but the
    // upstream overlay/concat still produce them) → O(at_ms) memory = the 22GB
    // compose-frame OOM on a multi-minute timeline. The windowed frame is
    // IDENTICAL (same clips/overlays/captions, rebased). A timeline that fits in
    // ONE window (short — the common scrub case) yields the whole EDL back, so
    // the frame is byte-identical to the pre-windowing path. Window boundaries
    // come from the same xfade-safe planner as the segmented render.
    let (fgw, fgh) = RenderOptions::default().output_geometry(project, edl);
    let windows = plan_windows(
        edl,
        render_window_ms(fgw, fgh, overlay_track_count(project)),
        project.settings.fps,
    );
    let (w0, w1) = windows
        .iter()
        .find(|(a, b)| at_ms >= *a && at_ms < *b)
        .copied()
        .unwrap_or((0, edl.duration_ms));
    let windowed = ((w0, w1) != (0, edl.duration_ms)).then(|| (edl.window(w0, w1), w0));
    let (render_edl, frame_at) = match &windowed {
        Some((wedl, w0)) => (wedl, at_ms - w0),
        None => (edl, at_ms),
    };

    // Frame extraction uses default framing (project geometry) — render.frame
    // is the agent's eyes at the composition geometry, independent of the
    // final render's fit/resolution choice.
    prepare_stabilization(project, render_edl, project_dir)?;
    let mut graph = build_graph(
        project,
        render_edl,
        project_dir,
        true,
        false,
        true,
        RenderOptions::default(),
        preview_height,
    )?;
    let v = graph.video_out.clone();
    write!(
        graph.filter,
        "[{v}]trim=start={s},setpts=PTS-STARTPTS[vframe];",
        s = secs(frame_at)
    )
    .unwrap();

    let dir = tempfile::tempdir().map_err(CutError::from)?;
    let out = dir.path().join("frame.jpg");
    let mut args = graph_args(&graph, "vframe", None);
    args.extend(
        [
            "-frames:v",
            "1",
            "-c:v",
            "mjpeg",
            "-q:v",
            "2",
            "-f",
            "image2",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args.extend(DETERMINISM_FLAGS.iter().map(|s| s.to_string()));
    args.push(out.display().to_string());
    run_ffmpeg(&args)?;
    Ok(std::fs::read(&out)?)
}

#[cfg(test)]
mod tests;
