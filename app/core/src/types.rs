//! types.rs — the timeline data model (timeline/op-log contract "Timeline JSON").
//!
//! Role: serde-faithful Rust mirror of `project.json` (schema "shellx-cut/1").
//! All times are MILLISECONDS (u64). Clips on a track are non-overlapping and
//! ordered; timeline position is cumulative (gap clips occupy time).
//! Dependencies: serde, serde_json. Primary callers: every other crate —
//! cut-media (EDL→render), cut-perception (checks), server (verb handlers).

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Current project schema tag. Bump only with a migration path.
pub const PROJECT_SCHEMA: &str = "shellx-cut/1";

pub const DEFAULT_SEQUENCE_ID: &str = "seq1";

fn default_active_sequence() -> String {
    DEFAULT_SEQUENCE_ID.to_string()
}

fn is_default_active_sequence(id: &String) -> bool {
    id == DEFAULT_SEQUENCE_ID
}

// ── Source↔timeline time-remap (the per-clip speed invariant) ──────────────
//
// The single place the source↔timeline conversion lives. Before edit.speed the
// mapping was 1:1: a clip
// occupied `src_out − src_in` ms of timeline. With a speed factor `s` a clip
// plays `s`× as fast, so it occupies `src_span / s` ms of timeline (2× ⇒ half
// the span; 0.5× slow-mo ⇒ double). Every feature that maps source↔timeline
// (cut_on_word, remove_*, assemble, scene split, beat map, captions) routes
// through these two functions, so speed support is one rule, not 15 scattered
// edits.
//
// INVARIANT: both functions are the EXACT IDENTITY at s = 1.0 (early return, no
// float math) — every op log / EDL recorded before edit.speed replays
// byte-identical. They are inverses modulo ms rounding.

/// serde default for the `speed` field: a missing key replays as 1.0 (normal
/// speed) so pre-speed op logs / projects deserialize unchanged.
pub fn default_speed() -> f64 {
    1.0
}

/// serde skip-predicate: speed 1.0 is the default and is omitted from JSON so
/// pre-speed op logs / projects / EDLs round-trip byte-identical.
pub fn is_unit_speed(v: &f64) -> bool {
    *v == 1.0
}

fn usable_speed(speed: f64) -> Option<f64> {
    if speed.is_finite() && speed > 0.0 {
        Some(speed)
    } else {
        None
    }
}

/// A SOURCE-time offset (ms inside the asset) → the TIMELINE offset it occupies
/// at clip speed `speed`. Identity at speed 1.0.
pub fn src_off_to_tl(src_off: u64, speed: f64) -> u64 {
    if speed == 1.0 {
        return src_off; // exact identity — byte-identical pre-speed replay
    }
    let Some(speed) = usable_speed(speed) else {
        return src_off;
    };
    (src_off as f64 / speed).round() as u64
}

/// A TIMELINE offset → the SOURCE span (ms) consumed across it at clip speed
/// `speed`. Inverse of [`src_off_to_tl`]; identity at speed 1.0.
pub fn tl_off_to_src(tl_off: u64, speed: f64) -> u64 {
    if speed == 1.0 {
        return tl_off;
    }
    let Some(speed) = usable_speed(speed) else {
        return tl_off;
    };
    (tl_off as f64 * speed).round() as u64
}

/// A color space ShellX Cut's color management understands (`project.color`
/// working/output + `edit.color_space` clip input). LIGHTWEIGHT, ffmpeg-backed —
/// NOT a full ACES engine: each space is a fixed (primaries, transfer, matrix)
/// triple the renderer feeds to the `zscale` filter to convert between spaces.
///
/// SUPPORTED CONVERSIONS: every pair among these four is supported (zscale converts
/// input→working→output directly); unknown space names are rejected at the verb
/// boundary with an actionable error. Serialized lowercase so the verb arg strings
/// ("rec709", "rec2020", "srgb", "linear") are the on-disk + on-wire form.
///
/// NOTE (honest scope): `srgb` shares Rec.709 PRIMARIES — it differs only in the
/// TRANSFER (sRGB gamma vs the BT.709 OETF) — and `linear` is scene-linear light in
/// the 709 gamut. The 8-bit yuv420p render pipeline linearizes/de-linearizes through
/// zscale's float core but stores 8-bit at the boundary, so `linear` as a WORKING
/// space trades precision for simplicity (documented, not hidden).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorSpace {
    /// HD video standard (BT.709 primaries/transfer/matrix). The DEFAULT — a
    /// project that never touches color management is rec709 in and out, so its
    /// render is byte-identical to a pre-color-management render.
    #[default]
    Rec709,
    /// UHD / wide-gamut (BT.2020 primaries, BT.2020-10 transfer, non-constant
    /// luminance matrix).
    Rec2020,
    /// sRGB display (BT.709 primaries, IEC 61966-2-1 transfer) — typical for
    /// screen-capture / graphics sources.
    Srgb,
    /// Scene-linear light in the 709 gamut (linear transfer) — a working space for
    /// compositing/grade math in linear light.
    Linear,
}

impl ColorSpace {
    /// Parse a verb-arg space name; None for an unknown name (the verb turns this
    /// into an actionable INVALID_ARGS error listing the supported set).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "rec709" | "bt709" | "709" => Some(Self::Rec709),
            "rec2020" | "bt2020" | "2020" => Some(Self::Rec2020),
            "srgb" => Some(Self::Srgb),
            "linear" => Some(Self::Linear),
            _ => None,
        }
    }

    /// Canonical lowercase name (round-trips with [`ColorSpace::parse`]).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rec709 => "rec709",
            Self::Rec2020 => "rec2020",
            Self::Srgb => "srgb",
            Self::Linear => "linear",
        }
    }

    /// The comma-separated list of accepted space names (for error messages).
    pub const SUPPORTED: &'static str = "rec709, rec2020, srgb, linear";

    /// zscale `primaries`/`p` token for this space.
    pub fn zs_primaries(&self) -> &'static str {
        match self {
            Self::Rec2020 => "bt2020",
            // rec709 / srgb / linear all use the BT.709 gamut.
            _ => "bt709",
        }
    }

    /// zscale `transfer`/`t` token (also the ffmpeg `-color_trc` output token).
    pub fn zs_transfer(&self) -> &'static str {
        match self {
            Self::Rec709 => "bt709",
            Self::Rec2020 => "bt2020-10",
            Self::Srgb => "iec61966-2-1",
            Self::Linear => "linear",
        }
    }

    /// zscale `matrix`/`m` token (also the ffmpeg `-colorspace` output token).
    pub fn zs_matrix(&self) -> &'static str {
        match self {
            Self::Rec2020 => "bt2020nc",
            // rec709 / srgb / linear store YCbCr via the BT.709 matrix.
            _ => "bt709",
        }
    }
}

/// The project's color-management configuration (`project.color`): the WORKING
/// space (the renderer composites/grades in it) and the OUTPUT space (the delivered
/// file's tagging + final pixels). DEFAULT = rec709/rec709 = today's behavior; a
/// default config emits NO color transform so the render is byte-identical to a
/// pre-color-management render. Serde-skipped when default (see `ProjectSettings`),
/// so existing project.json round-trips unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorConfig {
    /// The space the renderer converts each clip INTO before grade/effects.
    #[serde(default)]
    pub working: ColorSpace,
    /// The space the delivered file is tagged + encoded in (working→output transform
    /// at the end of each clip chain).
    #[serde(default)]
    pub output: ColorSpace,
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self {
            working: ColorSpace::Rec709,
            output: ColorSpace::Rec709,
        }
    }
}

impl ColorConfig {
    /// True when this is the rec709/rec709 default — the byte-identical baseline.
    /// Drives serde-skip (project.json round-trips) AND the renderer's "emit no
    /// transform" fast path.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Output/composition settings for the project (timeline/op-log contract `settings`).
/// Each field is serde-defaulted (to the documented default) so a PARTIAL settings
/// object fills the rest — e.g. project.create{settings:{width,height,fps}} no
/// longer errors on the omitted `audio_rate` (it defaults to 48 kHz). The whole
/// object still defaults via [`Default`] when settings is omitted entirely.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectSettings {
    #[serde(default = "ProjectSettings::default_width")]
    pub width: u32,
    #[serde(default = "ProjectSettings::default_height")]
    pub height: u32,
    #[serde(default = "ProjectSettings::default_fps")]
    pub fps: f64,
    #[serde(default = "ProjectSettings::default_audio_rate")]
    pub audio_rate: u32,
    /// Color management (`project.color`): working + output color space. Default
    /// rec709/rec709 (today's behavior). Serde-skipped when default so existing
    /// project.json round-trips byte-identical AND a default render is byte-identical.
    #[serde(default, skip_serializing_if = "ColorConfig::is_default")]
    pub color: ColorConfig,
}

impl ProjectSettings {
    fn default_width() -> u32 {
        1920
    }
    fn default_height() -> u32 {
        1080
    }
    fn default_fps() -> f64 {
        30.0
    }
    fn default_audio_rate() -> u32 {
        48_000
    }
}

impl Default for ProjectSettings {
    /// documented default: 1920x1080 @ 30fps, 48 kHz audio.
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30.0,
            audio_rate: 48_000,
            color: ColorConfig::default(),
        }
    }
}

/// An imported media asset. `probe` holds the normalized ffprobe JSON
/// (cut-media owns its shape); `transcript`/`perception` are RELATIVE paths
/// inside the project dir (e.g. "receipts/a1.words.json") — kept as paths so
/// project.json stays small and receipts stay individually inspectable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    /// Absolute (or project-relative) path to the source media file.
    pub path: String,
    /// "sha256:<hex>" content hash — cache key for proxies/perception.
    pub hash: String,
    /// Normalized probe JSON (duration_ms, streams, ...). None until probed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<serde_json::Value>,
    /// Relative path to word-level transcript JSON, once transcribed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    /// Relative path to perception.json (instrument facts), once analyzed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perception: Option<String>,
    /// Relative path to the generated proxy file (proxies/<id>.mp4), if built.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// Relative path to the timeline thumbnail strip (filmstrip/<id>.jpg), once
    /// built — a horizontal tile of frames the UI slices per clip ("frames in the
    /// time bar"). Video assets only; set by the import chain / media.filmstrip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filmstrip: Option<String>,
}

/// Track kind discriminator (timeline/op-log contract `tracks[].kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Video,
    Audio,
    Caption,
}

/// A clip slot on a track. Serialized UNTAGGED to match timeline/op-log contract JSON exactly:
/// media clips have `asset`+`src_in_ms`+`src_out_ms`, gaps have
/// `kind:"gap"`+`duration_ms`, caption clips have `text`+`range_ms`.
/// Field sets are disjoint, so untagged deserialization is unambiguous —
/// Gap is tried first because its `kind` literal is the strongest signal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
// `Media` is the dominant variant by far — a real timeline is overwhelmingly
// MediaClip, with only occasional Gap/Caption — so a `Vec<Clip>` wastes almost
// no space to the size delta, and boxing `MediaClip` would instead add a heap
// indirection to the HOT path (every clip access). The lint's memory win doesn't
// apply to this distribution; allow it deliberately rather than pessimize the
// common case. (If MediaClip keeps growing, box its RARELY-set sub-structs —
// grade/matte/eq/stabilize — internally instead.)
#[allow(clippy::large_enum_variant)]
pub enum Clip {
    Gap(GapClip),
    Media(MediaClip),
    Caption(CaptionClip),
}

impl Clip {
    /// Clip id if the variant carries one (gaps are anonymous).
    pub fn id(&self) -> Option<&str> {
        match self {
            Clip::Gap(_) => None,
            Clip::Media(c) => Some(&c.id),
            Clip::Caption(c) => Some(&c.id),
        }
    }

    /// Duration this clip occupies on the timeline, in ms. For media clips this
    /// is the SOURCE span remapped through the clip's speed factor
    /// (`src_span / speed`) — the one place clip speed turns into timeline
    /// length, so `edl_from_project`'s cursor (which advances by this) places
    /// every later clip correctly for free. Identity at speed 1.0.
    pub fn timeline_duration_ms(&self) -> u64 {
        match self {
            Clip::Gap(g) => g.duration_ms,
            Clip::Media(c) => match &c.speed_ramp {
                // Variable speed (edit.speed_ramp): the realized length is the
                // integral of (1/speed) over the source = the sum of the
                // expanded constant-speed sub-segment durations. Routes through
                // the SAME `speed_ramp_segments` the EDL emits, so the cursor
                // placement and the rendered segments agree exactly.
                Some(ramp) => speed_ramp_segments(c.src_in_ms, c.src_out_ms, ramp)
                    .iter()
                    .map(|s| s.dur_ms)
                    .sum(),
                // Constant speed (or normal): the one place clip speed turns
                // into timeline length. Identity at speed 1.0.
                None => src_off_to_tl(c.src_out_ms.saturating_sub(c.src_in_ms), c.speed),
            },
            Clip::Caption(c) => c.range_ms[1].saturating_sub(c.range_ms[0]),
        }
    }
}

/// Per-clip overlay geometry (the `edit.transform` verb's storage), used by
/// clips on OVERLAY video tracks (any video track after the first). All
/// values are NORMALIZED to the project frame so they survive resolution
/// changes: `x`/`y` = top-left position as a fraction of frame width/height,
/// `scale` = overlay width as a fraction of frame width (height scales
/// proportionally — the overlay is the full conformed frame, scaled).
/// `opacity` = 0..1 overlay alpha multiplier (1 = fully opaque) for blend/ghost
/// looks. None/identity (0, 0, 1, 1) = full-frame, fully opaque.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipTransform {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
    /// Overlay opacity 0..1 (1 = opaque). Multiplies the clip's alpha on overlay
    /// video tracks. `#[serde(default)] = 1.0` so projects saved before this
    /// field load unchanged (a missing opacity = fully opaque).
    #[serde(default = "default_opacity")]
    pub opacity: f64,
}

/// Default overlay opacity (fully opaque) — for the serde default + old projects.
pub fn default_opacity() -> f64 {
    1.0
}

impl ClipTransform {
    /// The do-nothing transform (full-frame at origin, fully opaque).
    pub fn identity() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            scale: 1.0,
            opacity: 1.0,
        }
    }

    /// True when this transform changes nothing (geometry AND opacity).
    pub fn is_identity(&self) -> bool {
        self.x == 0.0 && self.y == 0.0 && self.scale == 1.0 && self.opacity == 1.0
    }
}

/// Per-clip source crop rectangle (the `edit.crop` verb's storage;
/// "framing correctness"). All values are SOURCE PIXELS in the asset's own
/// coordinate space — NOT normalized — because crop happens in source space
/// BEFORE any conform/transform (a crop is a property of the footage, not of
/// the composition geometry, so it must survive resolution changes by being
/// expressed in source px and stays valid as long as the source does).
///
/// COMPOSE ORDER (documented contract, render.rs): crop → conform
/// (scale/pad to project geometry) → overlay transform (PiP). The renderer
/// inserts `crop=w:h:x:y` as the FIRST filter on the segment chain, right
/// after the source trim, then the existing scale/pad/setsar conform runs on
/// the cropped picture. This is why crop is source-px and transform is
/// normalized: they live in different coordinate spaces and stack in this
/// fixed order.
///
/// The canonical use: an import's `content_bbox` perception fact reports a
/// baked-in letterbox/pillarbox; `edit.crop` to that bbox removes the bands
/// from the SOURCE before it is conformed, so the rendered frame is filled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipCrop {
    /// Left edge of the kept rectangle in source px.
    pub x: u32,
    /// Top edge of the kept rectangle in source px.
    pub y: u32,
    /// Width of the kept rectangle in source px (must be > 0).
    pub w: u32,
    /// Height of the kept rectangle in source px (must be > 0).
    pub h: u32,
}

impl ClipCrop {
    /// True when this crop keeps the whole frame given the source geometry —
    /// an identity crop (origin, full size) is stored as None by `edit.crop`.
    pub fn is_full_frame(&self, src_w: u32, src_h: u32) -> bool {
        self.x == 0 && self.y == 0 && self.w == src_w && self.h == src_h
    }
}

/// Which streams a clip fade applies to (the `edit.fade` verb's `kind`).
/// "both" = whatever streams the clip's track contributes (video tracks
/// render video only, audio tracks audio only — so "both" is the safe
/// default everywhere; an explicit kind that can do NOTHING on the target
/// track is refused at verb time rather than silently no-opping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FadeKind {
    Audio,
    Video,
    Both,
}

/// Per-clip linear fade stored under the `edit.fade` contract.
/// `in_ms` ramps from silence/black (or transparent, on overlay tracks) at
/// the clip's timeline start; `out_ms` ramps to it at the clip's end. Times
/// are CLIP-LOCAL durations, applied by the renderer in segment-local time —
/// they survive ripples (the fade travels with the clip, not the timeline).
/// LINEAR ONLY; crossfades between adjacent clips are an explicit v2 feature
/// (this struct fades each clip independently). The renderer clamps a fade
/// longer than the clip to the clip duration (trims can shrink clips after
/// the fade was set).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipFade {
    /// Fade-in length, ms (0 = none).
    #[serde(default)]
    pub in_ms: u64,
    /// Fade-out length, ms (0 = none).
    #[serde(default)]
    pub out_ms: u64,
    /// Stream selector; one kind per clip fade (setting a fade replaces the
    /// kind too — split fades per stream on one clip are out of scope).
    pub kind: FadeKind,
}

/// Per-clip color grade (the `edit.grade` verb's storage; native color
/// grading — the "hero look" pass). Two stages, both optional, applied in this
/// order by the renderer: PARAMETRIC (ffmpeg `eq` + `colortemperature`) then a
/// 3D LUT (`lut3d`). Identity (all parametric defaults + no lut) is stored as
/// None by `edit.grade` and skipped in JSON so pre-grade op logs / projects
/// replay byte-identical.
///
/// COMPOSE ORDER (render.rs): crop → conform → GRADE → transform/overlay. Grade
/// is per-pixel (commutes with scale), applied on the conformed frame for
/// speed. The parametric knobs map directly to ffmpeg `eq`: `contrast` (1 =
/// none), `brightness` (0 = none), `saturation` (1 = none), `gamma` (1 = none);
/// `temperature_k` maps to `colortemperature=temperature=K` (None = none; 6500K
/// ≈ neutral, lower = warmer). `lut` is a path to a user `.cube` 3D LUT (e.g. an
/// S-Log3→Rec.709 conversion), applied LAST via `lut3d`; the path is fenced at
/// verb time (must exist + end in `.cube`) — we ship no LUT files (keeps the
/// artifact license-clean), the user supplies pro LUTs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipGrade {
    /// ffmpeg `eq` contrast (1.0 = unchanged).
    #[serde(default = "default_one")]
    pub contrast: f64,
    /// ffmpeg `eq` brightness (0.0 = unchanged; range ~ -1..1).
    #[serde(default)]
    pub brightness: f64,
    /// ffmpeg `eq` saturation (1.0 = unchanged; 0 = greyscale).
    #[serde(default = "default_one")]
    pub saturation: f64,
    /// ffmpeg `eq` gamma (1.0 = unchanged).
    #[serde(default = "default_one")]
    pub gamma: f64,
    /// White balance, Kelvin (None = unchanged; ~6500 neutral, lower = warmer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_k: Option<u32>,
    /// Path to a user `.cube` 3D LUT, applied last (None = none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lut: Option<String>,
}

impl ClipGrade {
    /// True when the grade changes nothing (parametric identity + no LUT).
    pub fn is_identity(&self) -> bool {
        self.contrast == 1.0
            && self.brightness == 0.0
            && self.saturation == 1.0
            && self.gamma == 1.0
            && self.temperature_k.is_none()
            && self.lut.is_none()
    }
}

/// serde/grade default: the multiplicative-identity 1.0 (contrast/saturation/
/// gamma are 1 when unchanged; brightness defaults to 0 via `Default`).
fn default_one() -> f64 {
    1.0
}

/// A named GRADE PRESET in the project's grade GALLERY (`grade.save`/`grade.apply`/
/// `grade.list`). A snapshot of one clip's [`ClipGrade`] params under a `name`, stored
/// project-level (`Project::grade_presets`) so a "look" can be COPIED between shots —
/// a grade-gallery workflow (save a graded shot's look, drop it onto other
/// shots). Pure DATA: `grade.save` snapshots the source clip's current single grade;
/// `grade.apply` re-applies a preset's params to a target clip by LOWERING to a plain
/// `edit.grade` op (concrete params recorded → replay-safe, independent of whether the
/// preset still exists). Additive serde-default so pre-gallery projects round-trip
/// byte-identical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradePreset {
    /// Preset name (the gallery key; a re-save under the same name REPLACES it).
    pub name: String,
    /// The captured grade params (the same `ClipGrade` shape `edit.grade` stores).
    pub grade: ClipGrade,
}

/// A CAPTION STYLE PRESET (`captions.save_style` / `captions.apply_style` /
/// `captions.list_styles`) — a named [`CaptionStyle`] snapshot, the caption
/// analog of [`GradePreset`]. `apply_style` LOWERS to plain
/// `captions.set_style` ops (concrete style recorded → replay-safe,
/// independent of whether the preset still exists). Name-keyed: re-save under
/// the same name REPLACES; built-in catalog names are reserved. Persisted via
/// the grade-gallery op-log metadata pattern (off the undo cursor).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionStylePreset {
    /// Preset name (the key; re-save replaces; built-in names refused).
    pub name: String,
    /// The captured style (the same shape `captions.set_style` stores).
    pub style: CaptionStyle,
}

/// A SMART BIN (`media.bin_save` / `media.bin_delete` / `media.bin_list`) — a
/// NAMED SAVED SEARCH over the project's asset tray, the NLE
/// convention (saved smart bins): membership is COMPUTED at list
/// time from the query, never stored, so bins can't go stale as assets come
/// and go. Query fields are AND-combined; at least one must be set (an
/// everything-bin is refused at the verb layer). Name-keyed like
/// [`GradePreset`]: a re-save under the same name REPLACES the query.
/// Persisted via the grade-gallery op-log metadata pattern (off the undo
/// cursor; replay re-derives the bin list).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmartBin {
    /// Bin name (the key; re-save replaces).
    pub name: String,
    /// Asset kind filter: "video" | "audio" | "image" (probe kind).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Case-insensitive substring match on the source file's basename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// true = only assets NOT referenced by any timeline clip (unused media);
    /// false = only assets that ARE on the timeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unused: Option<bool>,
    /// Minimum source width in pixels. Assets without a probe do not match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_width: Option<u32>,
    /// Minimum source height in pixels. Assets without a probe do not match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_height: Option<u32>,
    /// true = missing/offline source files; false = files currently present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline: Option<bool>,
    /// Inclusive lower bound for source file modification time (Unix epoch ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_after_ms: Option<i64>,
    /// Inclusive upper bound for source file modification time (Unix epoch ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_before_ms: Option<i64>,
}

/// Non-destructive transcript ignore span. Word indices are SOURCE transcript
/// indices for one asset; consumers decide whether to skip them for captions,
/// assembly, and other transcript-derived outputs. The source transcript remains
/// intact so the UI can show ignored words quietly and undo can restore them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptIgnore {
    pub asset: String,
    pub word_range: [usize; 2],
}

/// AI background MATTE (`edit.matte`) — a straight-alpha subject cutout WITHOUT a
/// green screen, baked by the matting sidecar (RVM by default). `None` on a clip
/// = no matte; serde-skip-None keeps pre-matte op logs / projects replaying
/// byte-identical.
///
/// Stored as a per-clip ATTRIBUTE: the source is NEVER mutated. The alpha itself
/// is a content-addressed CACHE artifact (keyed by asset content + model +
/// quality, see [`ClipMatte::cache_tag`]) — regenerated on demand, never carried
/// in the EDL, so replay stays pure + offline. `remove` drops the background to
/// reveal the LOWER track (overlay-only, exactly like chroma key); `replace`
/// fills behind the subject with `bg`. v1 ships `remove` + `rvm`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipMatte {
    /// `remove` = drop the background (reveal the track below); `replace` = fill
    /// behind the subject with `bg`.
    #[serde(default)]
    pub mode: MatteMode,
    /// Matting model. `rvm` = default (portable, automatic, license-clean as a
    /// user-side runtime); `matanyone` = opt-in NVIDIA/non-commercial upgrade.
    #[serde(default)]
    pub model: MatteModel,
    /// Replace-mode background (mode = `replace` only): a solid colour or an
    /// asset id painted behind the subject. `None` for `remove`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<MatteBg>,
    /// Quality/speed tradeoff hint passed to the sidecar (it sets the RVM
    /// downsample ratio). Changes the alpha → part of the cache key.
    #[serde(default)]
    pub quality: MatteQuality,
    /// PREMIUM target-selection seed (`model = matanyone` only): the user's CLICK
    /// (or box) on one frame → SAM2 turns it into the first-frame mask MatAnyone2
    /// propagates ("pick WHICH subject"). `None` = the RVM auto-seed (zero-click,
    /// finds "the human"). Replay-safe DATA — the prompt is recorded; SAM2
    /// regenerates the same mask deterministically at bake time. Changes the alpha
    /// → part of the cache key (a different pick re-bakes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<MatteSeed>,
}

/// A first-frame subject seed for the premium matte (`edit.matte{model:matanyone,
/// seed}`). A click (or box) on one source frame; SAM2 (`sam2_runner.py`) turns it
/// into the binary first-frame mask that seeds MatAnyone2's propagation. This is
/// the "pick which subject" path — RVM seeds automatically (no choice), SAM2 lets
/// the user select the target. Stored as DATA (replay-safe; SAM2 is deterministic).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatteSeed {
    /// Source time (ms) of the frame the click was made on.
    #[serde(default)]
    pub at_ms: u64,
    /// Positive click `[x, y]` in SOURCE pixels — the subject to KEEP.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<[i64; 2]>,
    /// Subject box `[x, y, w, h]` in SOURCE pixels (alternative to `point`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[i64; 4]>,
}

impl MatteSeed {
    /// A stable short hash of the prompt, folded into the matte cache tag so a
    /// different pick re-bakes. FNV-1a is used explicitly so the cache tag does
    /// not depend on std's DefaultHasher implementation.
    pub fn short_hash(&self) -> String {
        fn mix(h: &mut u64, bytes: &[u8]) {
            const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
            for b in bytes {
                *h ^= u64::from(*b);
                *h = h.wrapping_mul(FNV_PRIME);
            }
        }
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        mix(&mut h, &self.at_ms.to_le_bytes());
        match self.point {
            Some(p) => {
                mix(&mut h, &[1]);
                mix(&mut h, &p[0].to_le_bytes());
                mix(&mut h, &p[1].to_le_bytes());
            }
            None => mix(&mut h, &[0]),
        }
        match self.bbox {
            Some(b) => {
                mix(&mut h, &[1]);
                for v in b {
                    mix(&mut h, &v.to_le_bytes());
                }
            }
            None => mix(&mut h, &[0]),
        }
        format!("{h:016x}")
    }
}

/// Matte composite mode. `remove` reveals the lower track; `replace` fills `bg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatteMode {
    #[default]
    Remove,
    Replace,
}

/// Matting model. The alpha differs per model, so this is part of the cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatteModel {
    #[default]
    Rvm,
    Matanyone,
}

impl MatteModel {
    pub fn as_str(&self) -> &'static str {
        match self {
            MatteModel::Rvm => "rvm",
            MatteModel::Matanyone => "matanyone",
        }
    }
}

/// Quality/speed hint. `good` = native downsample (best edges); `fast` = coarser
/// downsample (quicker, softer edges). Changes the alpha → part of the cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatteQuality {
    Fast,
    #[default]
    Good,
}

impl MatteQuality {
    pub fn as_str(&self) -> &'static str {
        match self {
            MatteQuality::Fast => "fast",
            MatteQuality::Good => "good",
        }
    }
}

/// Replace-mode background fill. A solid colour or an imported asset behind the
/// matted subject.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MatteBg {
    /// Solid colour fill — an ffmpeg colour name ("black") or "0xRRGGBB".
    Color { color: String },
    /// An imported asset (image/video) id painted behind the subject.
    Asset { asset: String },
}

impl ClipMatte {
    /// The content-address tag for this matte's baked alpha: the alpha depends
    /// ONLY on the source pixels (`asset_hash`) + model + quality. `mode`/`bg`
    /// are composite-time choices that REUSE the same alpha, so they are
    /// deliberately excluded from the key.
    pub fn cache_tag(&self, asset_hash: &str) -> String {
        let base = format!(
            "{asset_hash}.{}.{}",
            self.model.as_str(),
            self.quality.as_str()
        );
        // A SAM2 seed changes the alpha → fold it into the key (a different pick
        // re-bakes). No seed (RVM auto) leaves the tag unchanged → pre-seed caches
        // + op-logs stay byte-identical.
        match &self.seed {
            Some(s) => format!("{base}.s{}", s.short_hash()),
            None => base,
        }
    }

    /// Filename of the baked alpha matte under the project's `cache/matte/` dir.
    /// The SAME helper is used by the server bake step (writer) and the renderer
    /// (reader) so they always agree on the path. `.mkv` = the FFV1 lossless gray
    /// matte the sidecar returns.
    pub fn cache_filename(&self, asset_hash: &str) -> String {
        format!("{}.mkv", self.cache_tag(asset_hash))
    }
}

/// A vector/freeform MASK on a clip (`edit.add_mask`): a SHAPE region + an EFFECT
/// applied inside it (blur / pixelate / black-out). The renderer bakes a feathered
/// GRAY alpha (white inside the shape) and `maskedmerge`s the effected frame over
/// the original, so the effect is scoped to the region; `invert` scopes OUTSIDE.
/// This is the region-blur / privacy-redaction primitive; `edit.redact` adds OCR
/// auto-points and optional motion tracking on top. CPU-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipMask {
    /// The region shape.
    pub shape: MaskShape,
    /// Normalized points (fractions of frame W/H, 0..1). `rect`: two opposite
    /// corners `[[x0,y0],[x1,y1]]`; `ellipse`: `[[cx,cy],[rx,ry]]` (centre + radii);
    /// `polygon`/freeform: the vertices (≥3), drawn closed.
    pub points: Vec<[f64; 2]>,
    /// Edge feather as a fraction of frame HEIGHT (0 = hard edge). Softens the
    /// shape boundary (a gaussian on the alpha).
    #[serde(default)]
    pub feather: f64,
    /// Scope the effect OUTSIDE the shape instead of inside (a "hole" / surround).
    #[serde(default)]
    pub invert: bool,
    /// What to do inside the masked region.
    #[serde(default)]
    pub effect: MaskEffect,
    /// Effect strength: blur → gaussian sigma (px); pixelate → block size (px);
    /// black → ignored. `None` uses the effect's sensible default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<f64>,
    /// TIME-BOUNDING (`edit.redact`): `[start_ms, end_ms]` clip-local — the effect
    /// is active ONLY in this window (the renderer gates the overlay with
    /// `enable='between(t,…)'`). `None` = the WHOLE clip (the `edit.add_mask`
    /// default). This is what makes redaction practical: a password/key/face is on
    /// screen only briefly, so blur just that range and keep the rest sharp.
    /// serde-skip-None keeps pre-range masks replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_ms: Option<[u64; 2]>,
    /// MOTION-TRACKED region (`edit.redact{track}`): the region CENTRE over
    /// time (clip-local), so the redaction FOLLOWS a moving subject (a scrolling
    /// box, a walking face). When set, the renderer animates the alpha PROCEDURALLY
    /// via a time-varying `geq` (cx(t)/cy(t) lowered like keyframes) instead of the
    /// static baked PNG — so NO PNG is baked for a tracked mask. The `shape`/`points`
    /// still give the region SIZE/form; the track overrides its centre. v1 supports
    /// `rect`/`ellipse` only (procedural geq); `polygon`-follow is a follow-up.
    /// Typically fed from `edit.track`'s `points`. None = a STATIC region (the
    /// baked-PNG path). serde-skip-None keeps static masks replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<Vec<MaskTrackPoint>>,
    /// ADDITIONAL regions beyond the primary `shape`/`points` for face blur:
    /// the redaction covers the UNION of the primary region + every region here, all
    /// sharing this mask's `effect`/`strength`/`feather`/`range_ms`. This is how a
    /// single `edit.redact` blurs N faces (or a face + a licence plate) at once. Each
    /// region carries its own `shape`/`points` (+ optional per-region `track`). Empty
    /// ⇒ a single-region mask that replays byte-identically (serde-skip-empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<MaskRegion>,
}

/// One extra region of a multi-region face-blur mask (`ClipMask.regions`).
/// `shape`/`points` mirror the primary region's geometry (normalized fractions);
/// `track` optionally makes THIS region follow a moving subject independently.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskRegion {
    pub shape: MaskShape,
    pub points: Vec<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track: Option<Vec<MaskTrackPoint>>,
}

/// One point on a mask's motion track (`edit.redact{track}`): the region CENTRE
/// `(cx, cy)` as FRACTIONS of frame W/H at clip-local time `t_ms`. The renderer
/// lowers the series to piecewise-linear `geq` centre expressions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MaskTrackPoint {
    pub t_ms: u64,
    pub cx: f64,
    pub cy: f64,
}

/// Mask region shapes (`edit.add_mask`). `polygon` covers freeform (a hand-drawn
/// path is just many vertices).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskShape {
    Rect,
    Ellipse,
    Polygon,
}

/// What a mask DOES inside its region. `blur` (default) = gaussian (privacy /
/// cleanup); `pixelate` = mosaic; `black` = solid box (hard censor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskEffect {
    #[default]
    Blur,
    Pixelate,
    Black,
}

impl MaskEffect {
    pub fn as_str(self) -> &'static str {
        match self {
            MaskEffect::Blur => "blur",
            MaskEffect::Pixelate => "pixelate",
            MaskEffect::Black => "black",
        }
    }
}

impl ClipMask {
    /// A short stable content-address hash of the mask geometry+effect, for the
    /// baked-alpha cache filename. Depends on every field that changes the alpha
    /// (shape/points/feather) — the EFFECT and invert change the composite, not the
    /// alpha shape, but are folded in too so a re-render after an effect change is
    /// unambiguous. Cheap FNV-1a over the canonical debug form (deterministic).
    pub fn cache_tag(&self, w: u32, h: u32) -> String {
        let mut hasher: u64 = 0xcbf29ce484222325;
        let mut feed = |b: &[u8]| {
            for &x in b {
                hasher ^= x as u64;
                hasher = hasher.wrapping_mul(0x100000001b3);
            }
        };
        feed(self.shape.as_str().as_bytes());
        feed(self.effect.as_str().as_bytes());
        feed(&[self.invert as u8]);
        feed(&self.feather.to_le_bytes());
        for p in &self.points {
            feed(&p[0].to_le_bytes());
            feed(&p[1].to_le_bytes());
        }
        if !self.regions.is_empty() {
            feed(b"regions");
            feed(&(self.regions.len() as u64).to_le_bytes());
            for region in &self.regions {
                feed(region.shape.as_str().as_bytes());
                for p in &region.points {
                    feed(&p[0].to_le_bytes());
                    feed(&p[1].to_le_bytes());
                }
                if let Some(track) = &region.track {
                    feed(b"track");
                    feed(&(track.len() as u64).to_le_bytes());
                    for point in track {
                        feed(&point.t_ms.to_le_bytes());
                        feed(&point.cx.to_le_bytes());
                        feed(&point.cy.to_le_bytes());
                    }
                }
            }
        }
        feed(&w.to_le_bytes());
        feed(&h.to_le_bytes());
        format!("mask_{hasher:016x}")
    }
}

impl MaskShape {
    pub fn as_str(self) -> &'static str {
        match self {
            MaskShape::Rect => "rect",
            MaskShape::Ellipse => "ellipse",
            MaskShape::Polygon => "polygon",
        }
    }
}

/// One GEOMETRIC POWER WINDOW on a clip (`edit.grade_window`): a region of the frame
/// plus the [`ClipGrade`] applied ONLY inside it. This is a geometric grade window — a
/// region-scoped grade — the one color gap vs `edit.grade`'s whole-frame look. The region
/// reuses the `edit.add_mask` geometry vocabulary ([`WindowShape`]: rect / ellipse /
/// polygon, optional feathered edge, optional `invert`), so any shape a redaction mask
/// supports is a valid window. Multiple windows STACK on a clip
/// ([`MediaClip::grade_windows`]) — each composites over the previous result, so you can
/// warm a face in one window and cool the background in another, independently. The
/// renderer bakes the window's shape alpha (the proven mask-PNG path) and `alphamerge`+
/// `overlay`s the GRADED copy inside the region, leaving the rest of the frame untouched.
/// HSL/luma QUALIFIERS (key by colour, not region) are a documented follow-up — this is
/// the GEOMETRIC window only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GradeWindow {
    /// The region the grade is scoped to.
    pub window: WindowShape,
    /// The grade applied INSIDE the window (the same params `edit.grade` takes).
    pub grade: ClipGrade,
}

/// The GEOMETRIC region of a power window (`edit.grade_window`). Mirrors the GEOMETRY
/// subset of [`ClipMask`] (shape / points / feather / invert) — the part that defines
/// WHERE, not WHAT (the "what" is [`GradeWindow::grade`]). Kept as its own type so a
/// stored window carries none of the mask's irrelevant `effect`/`strength` fields. The
/// renderer builds an ephemeral [`ClipMask`] from this (via [`WindowShape::to_mask`]) to
/// reuse the proven shape-alpha bake. v1 is a STATIC region (no motion track) — a
/// tracked / animated window is a follow-up.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowShape {
    /// The region shape (rect / ellipse / polygon — the same vocabulary as `edit.add_mask`).
    pub shape: MaskShape,
    /// Normalized points (fractions of frame W/H, 0..1). `rect`: two opposite corners
    /// `[[x0,y0],[x1,y1]]`; `ellipse`: `[[cx,cy],[rx,ry]]` (centre + radii); `polygon`:
    /// the vertices (≥3), drawn closed.
    pub points: Vec<[f64; 2]>,
    /// Edge feather as a fraction of frame HEIGHT (0 = hard edge), softening the grade's
    /// boundary so it BLENDS rather than cuts.
    #[serde(default)]
    pub feather: f64,
    /// Scope the grade OUTSIDE the shape instead of inside (grade the surround, keep the
    /// region untouched).
    #[serde(default)]
    pub invert: bool,
}

impl WindowShape {
    /// Build the ephemeral [`ClipMask`] the renderer feeds to `bake_mask_png` to
    /// rasterize this window's shape alpha. The mask's `effect`/`strength`/`range_ms`/
    /// `track`/`regions` are inert (the bake reads only shape/points/feather). `invert`
    /// is applied by the renderer's `,negate`, NOT here — so it is left `false`, which
    /// also lets two windows differing only in `invert` SHARE one baked alpha. The fixed
    /// dummy `effect` keeps the content-address (`ClipMask::cache_tag`) stable.
    pub fn to_mask(&self) -> ClipMask {
        ClipMask {
            shape: self.shape,
            points: self.points.clone(),
            feather: self.feather,
            invert: false,
            effect: MaskEffect::Black,
            strength: None,
            range_ms: None,
            track: None,
            regions: Vec::new(),
        }
    }
}

/// One peaking (bell) band of a parametric EQ (`edit.eq`). Maps to ffmpeg
/// `equalizer=f={freq_hz}:t=q:w={q}:g={gain_db}` — a constant-Q peaking filter
/// centred at `freq_hz` that boosts/cuts by `gain_db` with bandwidth set by `q`
/// (higher Q = narrower). A 0 dB band is a no-op (dropped at identity check).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EqBand {
    /// Centre frequency, Hz.
    pub freq_hz: f32,
    /// Gain at the centre, dB (positive = boost, negative = cut).
    pub gain_db: f32,
    /// Quality factor / bandwidth (higher = narrower). Defaults to 1.0.
    #[serde(default = "eq_default_q")]
    pub q: f32,
}

/// serde/EQ-band default Q (1.0 = a musically-moderate one-octave-ish bell).
fn eq_default_q() -> f32 {
    1.0
}

/// Parametric audio EQ for one clip (`edit.eq`) — the audio analog of
/// [`ClipGrade`]. A high-pass (low-cut, removes rumble), any number of peaking
/// bands (presence boost / mud or de-ess cut), and a low-pass (high-cut, tames
/// hiss). Emitted on the conformed clip audio as a chain of ffmpeg
/// `highpass` → `equalizer`(per band) → `lowpass` filters. None = no EQ. Audio
/// only — applies on BOTH the software and GPU render paths (the GPU fast-track
/// reuses the software audio chain verbatim), so it does NOT gate the video GPU
/// path. serde-skip-None keeps pre-EQ op logs / projects replaying byte-identical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipEq {
    /// High-pass (low-cut) corner frequency, Hz. None = no high-pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_pass_hz: Option<f32>,
    /// Low-pass (high-cut) corner frequency, Hz. None = no low-pass.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low_pass_hz: Option<f32>,
    /// Peaking bands (each a boost/cut bell). Empty = none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bands: Vec<EqBand>,
}

impl ClipEq {
    /// True when the EQ changes nothing: no high/low-pass and every band is ~0 dB.
    pub fn is_identity(&self) -> bool {
        self.high_pass_hz.is_none()
            && self.low_pass_hz.is_none()
            && self.bands.iter().all(|b| b.gain_db.abs() < 1e-3)
    }
}

/// Video stabilization for one clip (`edit.stabilize`) — smooths out camera shake.
/// Renderer uses the 2-pass ffmpeg `vidstab` (a `vidstabdetect` analysis PRE-PASS
/// writes a per-clip `.trf` motion file, then `vidstabtransform` applies the
/// smoothed correction in the render graph). `smoothing` = the look-ahead/behind
/// window in FRAMES (higher = steadier but more "locked-down"; ~10–30 typical).
/// CPU-only + needs the detect pre-pass, so a stabilized clip opts the timeline out
/// of the GPU fast-track and is applied only on render paths that ran the pre-pass.
/// None = no stabilization. serde-skip-None keeps pre-stabilize logs/projects
/// replaying byte-identical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipStabilize {
    /// Smoothing window in frames (clamped sane at the verb layer). Default 15.
    #[serde(default = "default_stab_smoothing")]
    pub smoothing: f64,
}

/// serde/stabilize default smoothing window (frames).
fn default_stab_smoothing() -> f64 {
    15.0
}

/// The `xfade` transition styles ShellX Cut exposes via `edit.crossfade`
/// (`transition` arg). A CURATED subset of ffmpeg's xfade names that is stable
/// across every supported ffmpeg build (all present since xfade landed in 4.3),
/// so a project renders the same style on Linux/Windows/macOS. "fade" (a classic
/// dissolve) is the default and the pre-existing behavior. Newer/less-portable
/// styles (zoomin, squeeze*, cover*, reveal*, *wind) are intentionally omitted.
/// Keep in sync with schema/verbs.json edit.crossfade `transition` + reference.md.
pub const TRANSITIONS: &[&str] = &[
    "fade",
    "fadeblack",
    "fadewhite",
    "dissolve", // dissolves
    "wipeleft",
    "wiperight",
    "wipeup",
    "wipedown", // wipes
    "slideleft",
    "slideright",
    "slideup",
    "slidedown", // pushes
    "smoothleft",
    "smoothright",
    "smoothup",
    "smoothdown", // soft wipes
    "circleopen",
    "circleclose",
    "circlecrop",
    "rectcrop", // shapes
    "horzopen",
    "horzclose",
    "vertopen",
    "vertclose", // barn-door
    "diagtl",
    "diagtr",
    "diagbl",
    "diagbr", // diagonals
    "hlslice",
    "hrslice",
    "vuslice",
    "vdslice", // slices
    "radial",
    "pixelize",
    "hblur", // stylized
    // --- Full ffmpeg xfade set (drift-guarded vs transition_specs) ---
    "fadegrays",
    "fadefast",
    "fadeslow",
    "distance", // more dissolves
    "wipetl",
    "wipetr",
    "wipebl",
    "wipebr", // corner wipes
    "coverleft",
    "coverright",
    "coverup",
    "coverdown", // covers (B covers A)
    "revealleft",
    "revealright",
    "revealup",
    "revealdown", // reveals (A uncovers B)
    "squeezeh",
    "squeezev", // squeezes
    "zoomin",   // zoom
    "hlwind",
    "hrwind",
    "vuwind",
    "vdwind", // wind-streak wipes
];

/// True when `name` is an exposed `edit.crossfade` transition style (the exact
/// ffmpeg `xfade` name; case-sensitive). Drives the verb's validation.
pub fn is_valid_transition(name: &str) -> bool {
    TRANSITIONS.contains(&name)
}

/// Layer blend modes ShellX Cut exposes via `edit.blend` (`mode` arg) — a CURATED
/// subset of ffmpeg's `blend=all_mode=` values that are stable across builds and
/// musically/visually well-known. `"normal"` = the default alpha-over composite
/// (clears the blend). Keep in sync with schema/verbs.json edit.blend + reference.md.
pub const BLEND_MODES: &[&str] = &[
    "normal",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "difference",
    "addition",
    "subtract",
    "softlight",
    "hardlight",
];

/// True when `name` is an exposed `edit.blend` mode (see [`BLEND_MODES`]).
pub fn is_valid_blend_mode(name: &str) -> bool {
    BLEND_MODES.contains(&name)
}

/// True when `s` is a SAFE ffmpeg chroma-key color literal — either a bare color
/// NAME (ASCII letters only, e.g. "green", "DarkGreen") or a hex literal
/// `0xRRGGBB` / `0xRRGGBBAA`.
///
/// SECURITY: the chroma color is interpolated into an ffmpeg `-filter_complex`
/// graph (`chromakey=color={color}:…`). The graph is passed as an argv argument,
/// not through a shell — so this is FILTERGRAPH injection, not shell injection —
/// but a crafted color containing a filtergraph metacharacter (`,` `;` `[` `]`
/// `:` `'` `=` space) could still break out of the `chromakey` filter and inject
/// arbitrary filters (e.g. `,movie=/etc/passwd` to read a file). A bare color
/// name or `0x…` hex literal contains none of those characters, so this strict
/// allowlist closes the hole at the type's edge. Mirrors the schema contract
/// ("name (green) or 0xRRGGBB").
pub fn is_valid_chroma_color(s: &str) -> bool {
    // Bare color name: one-or-more ASCII letters, nothing else.
    let is_name = !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphabetic());
    // Hex literal: 0xRRGGBB (8 chars) or 0xRRGGBBAA (10 chars).
    let is_hex = (s.len() == 8 || s.len() == 10)
        && s.starts_with("0x")
        && s.as_bytes()[2..].iter().all(|b| b.is_ascii_hexdigit());
    is_name || is_hex
}

// serde defaults for ClipEffect params (a partial JSON fills sensible values).
fn eff_half() -> f64 {
    0.5
}
fn eff_one() -> f64 {
    1.0
}
fn eff_five() -> f64 {
    5.0
}
fn eff_six() -> f64 {
    6.0
}
fn eff_sixteen() -> f64 {
    16.0
}
fn eff_eight() -> f64 {
    8.0
}
fn eff_auto() -> f64 {
    0.7
}
fn eff_twenty() -> f64 {
    20.0
}
fn eff_similarity() -> f64 {
    0.15
}
fn eff_blend() -> f64 {
    0.1
}

/// A per-clip visual EFFECT (the `edit.effect` verb's storage), applied IN ORDER
/// after the clip's conform/grade stage. Serialized tagged by `type` (snake_case)
/// in the clip's `effects` list. Every variant maps to a CPU-only ffmpeg filter
/// (no CUDA equivalent), so a BASE-track clip carrying ANY effect falls the
/// timeline back to the software renderer; on an OVERLAY they run in the
/// already-CPU-built overlay chain and stay compatible with the GPU base path.
/// Extend the editor's effect set by adding a variant + a render arm in
/// `effect_filter` + an `edit.effect` schema enum entry — no model churn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClipEffect {
    /// Darkened corners. `amount` 0..1 (0 ≈ none, 1 = strong). ffmpeg `vignette`.
    Vignette {
        #[serde(default = "eff_half")]
        amount: f64,
    },
    /// Unsharp-mask sharpen. `amount` 0..3 (luma strength). ffmpeg `unsharp`.
    Sharpen {
        #[serde(default = "eff_one")]
        amount: f64,
    },
    /// Gaussian blur. `radius` = sigma px (> 0). ffmpeg `gblur`.
    Blur {
        #[serde(default = "eff_five")]
        radius: f64,
    },
    /// Film grain (additive temporal noise). `amount` 0..100. ffmpeg `noise`.
    Grain {
        #[serde(default = "eff_twenty")]
        amount: f64,
    },
    /// Chroma key / greenscreen: make `color` (e.g. "green", "0x00FF00")
    /// transparent so a LOWER track shows through — OVERLAY clips only (the verb
    /// refuses a base-track clip; there is nothing under the base to reveal).
    /// `similarity`/`blend` 0..1. ffmpeg `chromakey`.
    ChromaKey {
        color: String,
        #[serde(default = "eff_similarity")]
        similarity: f64,
        #[serde(default = "eff_blend")]
        blend: f64,
    },
    /// Voice/noise reduction (the ONE AUDIO effect) — cleans hiss/hum/room tone
    /// from a clip's audio. `amount` 0..1 (0 ≈ off, 1 = strong). Maps to ffmpeg
    /// `afftdn` (adaptive FFT denoiser). AUDIO-track clips only (it runs in the
    /// audio chain, not the video chain).
    Denoise {
        #[serde(default = "eff_half")]
        amount: f64,
    },
    /// Mirror — flip horizontally (ffmpeg `hflip`). The "un-mirror my webcam /
    /// selfie" one-click, and a creative flip.
    Mirror,
    /// Flip vertically (ffmpeg `vflip`).
    Flip,
    /// Hue rotation — shift all colors around the wheel by `degrees` (ffmpeg
    /// `hue=h=`). 0 = unchanged; 180 = complementary.
    HueShift {
        #[serde(default)]
        degrees: f64,
    },
    /// RGB-split / chromatic aberration (the glitch / retro-VHS look): offset the
    /// red and blue channels horizontally by `amount` px (ffmpeg `rgbashift`).
    RgbSplit {
        #[serde(default = "eff_six")]
        amount: f64,
    },
    /// Pixelize / mosaic (retro 8-bit or censor look): average `size`×`size` px
    /// blocks (ffmpeg `pixelize`).
    Pixelize {
        #[serde(default = "eff_sixteen")]
        size: f64,
    },
    /// Sepia tone (vintage / old-photo warmth) — a fixed sepia colour matrix
    /// (ffmpeg `colorchannelmixer`).
    Sepia,
    /// One-click AUTO-COLOR / auto-enhance — per-channel auto contrast + white
    /// balance (ffmpeg `normalize`). `amount` 0..1 blends toward the corrected
    /// look (1 = full). The "fix my dull footage" button.
    AutoColor {
        #[serde(default = "eff_auto")]
        amount: f64,
    },
    /// VHS / retro-tape look (a preset CHAIN): chroma shift + tape grain + a soft
    /// blur, scaled by `amount` 0..1 (ffmpeg `rgbashift`+`noise`+`gblur`).
    Vhs {
        #[serde(default = "eff_half")]
        amount: f64,
    },
    /// Posterize — reduce each channel to `levels` steps (retro / poster look;
    /// ffmpeg `lutrgb`). `levels` 2..64 (fewer = more banded). Default 8.
    Posterize {
        #[serde(default = "eff_eight")]
        levels: f64,
    },
    /// Invert / negative — flip all colours (ffmpeg `negate`). No params.
    Invert,
    /// Emboss — a relief / engraved look (ffmpeg `convolution` emboss kernel). No params.
    Emboss,
    /// AUDIO dynamics COMPRESSOR — evens out loud/quiet speech (ffmpeg
    /// `acompressor`). `amount` 0..1 sets how hard it compresses (0 ≈ off → ratio 1;
    /// 1 = strong → ratio ~8) with auto makeup gain. The talking-head / podcast
    /// "even out my voice" button. AUDIO-track clips only.
    Compressor {
        #[serde(default = "eff_half")]
        amount: f64,
    },
    /// AUDIO noise GATE — silences the clip BELOW a threshold so room tone, hum,
    /// and breaths BETWEEN phrases drop out while speech passes (ffmpeg `agate`).
    /// Complements `Denoise` (steady-state hiss) and `Compressor` (dynamics): the
    /// gate handles the gaps. `amount` 0..1 sets how aggressively it gates (0 ≈
    /// gentle → low threshold/ratio; 1 = strong → higher threshold + hard ratio).
    /// The talking-head / podcast "kill the dead-air noise" button. AUDIO-track
    /// clips only.
    Gate {
        #[serde(default = "eff_half")]
        amount: f64,
    },
}

impl ClipEffect {
    /// The effect's tag (`"vignette"`, `"chroma_key"`, …) — for receipts + the
    /// base-track chroma-key guard without matching every variant.
    pub fn kind(&self) -> &'static str {
        match self {
            ClipEffect::Vignette { .. } => "vignette",
            ClipEffect::Sharpen { .. } => "sharpen",
            ClipEffect::Blur { .. } => "blur",
            ClipEffect::Grain { .. } => "grain",
            ClipEffect::ChromaKey { .. } => "chroma_key",
            ClipEffect::Denoise { .. } => "denoise",
            ClipEffect::Mirror => "mirror",
            ClipEffect::Flip => "flip",
            ClipEffect::HueShift { .. } => "hue_shift",
            ClipEffect::RgbSplit { .. } => "rgb_split",
            ClipEffect::Pixelize { .. } => "pixelize",
            ClipEffect::Sepia => "sepia",
            ClipEffect::AutoColor { .. } => "auto_color",
            ClipEffect::Vhs { .. } => "vhs",
            ClipEffect::Posterize { .. } => "posterize",
            ClipEffect::Invert => "invert",
            ClipEffect::Emboss => "emboss",
            ClipEffect::Compressor { .. } => "compressor",
            ClipEffect::Gate { .. } => "gate",
        }
    }
    /// True for effects that need an alpha-capable composite (a LOWER track to
    /// reveal) and so are valid only on overlay clips — currently chroma key.
    pub fn is_overlay_only(&self) -> bool {
        matches!(self, ClipEffect::ChromaKey { .. })
    }
    /// True for AUDIO effects (run in the audio chain, on audio-track clips) vs
    /// VISUAL effects (video chain): denoise + compressor.
    pub fn is_audio(&self) -> bool {
        matches!(
            self,
            ClipEffect::Denoise { .. } | ClipEffect::Compressor { .. } | ClipEffect::Gate { .. }
        )
    }
}

// ---------------------------------------------------------------------------
// Effects-as-data registry — a DISCOVERY catalog: every `edit.effect`
// effect with its track (video/audio), one-line description, and parameter
// schema (name/type/range/default). The `effects.list` verb returns this so a
// UI/agent can enumerate effects + their params WITHOUT hardcoding. The render
// LOWERING (cut-media render.rs `effect_filter`) is intentionally NOT generated
// from this table: those filters carry per-effect computed ffmpeg logic, and the
// plan's hard constraint is "keep every existing effect byte-identical" — so the
// proven lowering stays Rust, and this registry is the validation/discovery
// spine beside it (kept in lockstep with [`ClipEffect`]).
// ---------------------------------------------------------------------------

/// One parameter of an effect, for the discovery catalog.
#[derive(Debug, Clone, Serialize)]
pub struct EffectParam {
    /// Parameter name (the JSON key under `edit.effect {effect:{type,..}}`).
    pub name: &'static str,
    /// `number` | `color` — the value kind.
    pub kind: &'static str,
    /// Inclusive minimum for a numeric param (None = unbounded / n/a).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Inclusive maximum for a numeric param (None = unbounded / n/a).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Default value when omitted (numeric params only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<f64>,
    /// True when the param has no default and must be supplied.
    pub required: bool,
}

/// One effect's discovery spec (catalog entry).
#[derive(Debug, Clone, Serialize)]
pub struct EffectSpec {
    /// The effect key (the `type` value: `vignette`, `chroma_key`, …).
    pub key: &'static str,
    /// `video` or `audio` — which chain it runs in (the clip track it applies to).
    pub track: &'static str,
    /// One-line human description.
    pub description: &'static str,
    /// True for effects valid ONLY on overlay clips (need a lower track to reveal).
    pub overlay_only: bool,
    /// The effect's parameters (empty for no-param effects like invert/mirror).
    pub params: Vec<EffectParam>,
}

/// A numeric param helper.
fn num(name: &'static str, min: f64, max: f64, default: f64) -> EffectParam {
    EffectParam {
        name,
        kind: "number",
        min: Some(min),
        max: Some(max),
        default: Some(default),
        required: false,
    }
}

/// The full `edit.effect` catalog — kept in lockstep with [`ClipEffect`].
/// Order = a sensible UI grouping (visual looks, then color, then audio).
pub fn effect_specs() -> Vec<EffectSpec> {
    let v = |key, description, overlay_only, params| EffectSpec {
        key,
        track: "video",
        description,
        overlay_only,
        params,
    };
    let a = |key, description, params| EffectSpec {
        key,
        track: "audio",
        description,
        overlay_only: false,
        params,
    };
    vec![
        v("vignette", "Darkened corners.", false, vec![num("amount", 0.0, 1.0, 0.5)]),
        v("sharpen", "Unsharp-mask sharpen (luma).", false, vec![num("amount", 0.0, 3.0, 1.0)]),
        v("blur", "Gaussian blur (sigma px).", false, vec![num("radius", 0.0, 100.0, 5.0)]),
        v("grain", "Film grain (additive temporal noise).", false, vec![num("amount", 0.0, 100.0, 20.0)]),
        v("chroma_key", "Greenscreen: make a color transparent so a lower track shows through (overlay clips only).", true, vec![
            EffectParam { name: "color", kind: "color", min: None, max: None, default: None, required: true },
            num("similarity", 0.0, 1.0, 0.15),
            num("blend", 0.0, 1.0, 0.1),
        ]),
        v("mirror", "Flip horizontally (un-mirror a webcam / creative flip).", false, vec![]),
        v("flip", "Flip vertically.", false, vec![]),
        v("hue_shift", "Rotate all colors around the wheel by `degrees`.", false, vec![num("degrees", 0.0, 360.0, 0.0)]),
        v("rgb_split", "RGB-split / chromatic aberration (glitch look) — offset red/blue by `amount` px.", false, vec![num("amount", 0.0, 64.0, 6.0)]),
        v("pixelize", "Pixelize / mosaic (retro 8-bit or censor) — `size`×`size` px blocks.", false, vec![num("size", 2.0, 256.0, 16.0)]),
        v("sepia", "Sepia tone (vintage / old-photo warmth).", false, vec![]),
        v("auto_color", "One-click auto-color / auto-enhance (per-channel contrast + white balance).", false, vec![num("amount", 0.0, 1.0, 0.7)]),
        v("vhs", "VHS / retro-tape look (chroma shift + tape grain + soft blur).", false, vec![num("amount", 0.0, 1.0, 0.5)]),
        v("posterize", "Posterize — reduce each channel to `levels` steps (banded).", false, vec![num("levels", 2.0, 64.0, 8.0)]),
        v("invert", "Invert / negative — flip all colors.", false, vec![]),
        v("emboss", "Emboss — a relief / engraved look.", false, vec![]),
        a("denoise", "Voice/noise reduction — clean hiss/hum/room tone (adaptive FFT denoiser).", vec![num("amount", 0.0, 1.0, 0.5)]),
        a("compressor", "Dynamics compressor — even out loud/quiet speech (auto makeup gain).", vec![num("amount", 0.0, 1.0, 0.5)]),
        a("gate", "Noise gate — silence below a threshold (kills dead-air room tone/breaths).", vec![num("amount", 0.0, 1.0, 0.5)]),
    ]
}

/// One transition's discovery spec (catalog entry for `transitions.list`). The
/// `name` is the exact `transition` value `edit.crossfade` accepts (an ffmpeg
/// `xfade` style); `category` groups the family for UI/agent picking; `direction`
/// is the motion axis where it applies (None for non-directional styles).
#[derive(Debug, Clone, Serialize)]
pub struct TransitionSpec {
    /// The transition key (the `edit.crossfade {transition}` value, == ffmpeg xfade).
    pub name: &'static str,
    /// Family: dissolve | wipe | slide | smooth | cover | reveal | shape | slice |
    /// diagonal | squeeze | zoom | wind.
    pub category: &'static str,
    /// Motion direction where meaningful: left|right|up|down|in|tl|tr|bl|br, else None.
    pub direction: Option<&'static str>,
    /// One-line human description (what the seam looks like).
    pub description: &'static str,
}

/// The full `edit.crossfade` VIDEO-transition catalog (the ffmpeg `xfade` styles
/// this build supports) — kept in LOCKSTEP with the schema's `edit.crossfade`
/// `transition` enum (drift-guarded by a server test). A pure read so a UI/agent
/// can DISCOVER transitions + pick by family/direction without hardcoding. Audio
/// always `acrossfade`s under any video style; this catalog is the VIDEO seam look.
pub fn transition_specs() -> Vec<TransitionSpec> {
    let t = |name, category, direction, description| TransitionSpec {
        name,
        category,
        direction,
        description,
    };
    vec![
        // --- dissolve / fade family (non-directional cross-blends) ---------------
        t("fade", "dissolve", None, "Straight cross-dissolve A→B."),
        t(
            "dissolve",
            "dissolve",
            None,
            "Noisy/grainy pixel dissolve A→B.",
        ),
        t("fadeblack", "dissolve", None, "Dip to black, then up to B."),
        t("fadewhite", "dissolve", None, "Dip to white, then up to B."),
        t(
            "fadegrays",
            "dissolve",
            None,
            "Desaturate to gray across the blend.",
        ),
        t(
            "fadefast",
            "dissolve",
            None,
            "Cross-dissolve with a fast (ease-in) curve.",
        ),
        t(
            "fadeslow",
            "dissolve",
            None,
            "Cross-dissolve with a slow (ease-out) curve.",
        ),
        t(
            "distance",
            "dissolve",
            None,
            "Color-distance morph between frames.",
        ),
        t(
            "pixelize",
            "dissolve",
            None,
            "Pixelate up then resolve into B (mosaic).",
        ),
        t(
            "hblur",
            "dissolve",
            None,
            "Blur out A, blur in B (horizontal).",
        ),
        t(
            "radial",
            "dissolve",
            None,
            "Radial clock-wipe sweep around the centre.",
        ),
        // --- wipe (a hard edge sweeps across) -----------------------------------
        t(
            "wipeleft",
            "wipe",
            Some("left"),
            "Hard edge wipes to the left.",
        ),
        t(
            "wiperight",
            "wipe",
            Some("right"),
            "Hard edge wipes to the right.",
        ),
        t("wipeup", "wipe", Some("up"), "Hard edge wipes upward."),
        t(
            "wipedown",
            "wipe",
            Some("down"),
            "Hard edge wipes downward.",
        ),
        t(
            "wipetl",
            "wipe",
            Some("tl"),
            "Wipe from the top-left corner.",
        ),
        t(
            "wipetr",
            "wipe",
            Some("tr"),
            "Wipe from the top-right corner.",
        ),
        t(
            "wipebl",
            "wipe",
            Some("bl"),
            "Wipe from the bottom-left corner.",
        ),
        t(
            "wipebr",
            "wipe",
            Some("br"),
            "Wipe from the bottom-right corner.",
        ),
        // --- slide (B pushes A out) ---------------------------------------------
        t(
            "slideleft",
            "slide",
            Some("left"),
            "B slides in pushing A out to the left.",
        ),
        t(
            "slideright",
            "slide",
            Some("right"),
            "B slides in pushing A out to the right.",
        ),
        t("slideup", "slide", Some("up"), "B slides in pushing A up."),
        t(
            "slidedown",
            "slide",
            Some("down"),
            "B slides in pushing A down.",
        ),
        // --- smooth (soft gradient wipe) ----------------------------------------
        t(
            "smoothleft",
            "smooth",
            Some("left"),
            "Soft gradient wipe to the left.",
        ),
        t(
            "smoothright",
            "smooth",
            Some("right"),
            "Soft gradient wipe to the right.",
        ),
        t(
            "smoothup",
            "smooth",
            Some("up"),
            "Soft gradient wipe upward.",
        ),
        t(
            "smoothdown",
            "smooth",
            Some("down"),
            "Soft gradient wipe downward.",
        ),
        // --- cover (B covers A, A stationary) -----------------------------------
        t(
            "coverleft",
            "cover",
            Some("left"),
            "B covers A moving in from the right→left.",
        ),
        t(
            "coverright",
            "cover",
            Some("right"),
            "B covers A moving in from the left→right.",
        ),
        t("coverup", "cover", Some("up"), "B covers A moving up."),
        t(
            "coverdown",
            "cover",
            Some("down"),
            "B covers A moving down.",
        ),
        // --- reveal (A uncovers B, B stationary) --------------------------------
        t(
            "revealleft",
            "reveal",
            Some("left"),
            "A slides off to the left, revealing B.",
        ),
        t(
            "revealright",
            "reveal",
            Some("right"),
            "A slides off to the right, revealing B.",
        ),
        t(
            "revealup",
            "reveal",
            Some("up"),
            "A slides off upward, revealing B.",
        ),
        t(
            "revealdown",
            "reveal",
            Some("down"),
            "A slides off downward, revealing B.",
        ),
        // --- shape (geometric mask grows/shrinks) -------------------------------
        t(
            "circleopen",
            "shape",
            None,
            "Circle opens out from the centre to reveal B.",
        ),
        t("circleclose", "shape", None, "Circle closes in to B."),
        t(
            "circlecrop",
            "shape",
            None,
            "Circular crop collapses then expands into B.",
        ),
        t(
            "rectcrop",
            "shape",
            None,
            "Rectangular crop collapses then expands into B.",
        ),
        t(
            "horzopen",
            "shape",
            None,
            "Horizontal barn-doors open to B.",
        ),
        t(
            "horzclose",
            "shape",
            None,
            "Horizontal barn-doors close to B.",
        ),
        t("vertopen", "shape", None, "Vertical barn-doors open to B."),
        t(
            "vertclose",
            "shape",
            None,
            "Vertical barn-doors close to B.",
        ),
        // --- slice (interleaved bars) -------------------------------------------
        t(
            "hlslice",
            "slice",
            Some("left"),
            "Horizontal slices wipe in from the left.",
        ),
        t(
            "hrslice",
            "slice",
            Some("right"),
            "Horizontal slices wipe in from the right.",
        ),
        t(
            "vuslice",
            "slice",
            Some("up"),
            "Vertical slices wipe in upward.",
        ),
        t(
            "vdslice",
            "slice",
            Some("down"),
            "Vertical slices wipe in downward.",
        ),
        // --- diagonal -----------------------------------------------------------
        t(
            "diagtl",
            "diagonal",
            Some("tl"),
            "Diagonal wipe from the top-left.",
        ),
        t(
            "diagtr",
            "diagonal",
            Some("tr"),
            "Diagonal wipe from the top-right.",
        ),
        t(
            "diagbl",
            "diagonal",
            Some("bl"),
            "Diagonal wipe from the bottom-left.",
        ),
        t(
            "diagbr",
            "diagonal",
            Some("br"),
            "Diagonal wipe from the bottom-right.",
        ),
        // --- squeeze ------------------------------------------------------------
        t(
            "squeezeh",
            "squeeze",
            None,
            "A squeezes horizontally to a line, B expands.",
        ),
        t(
            "squeezev",
            "squeeze",
            None,
            "A squeezes vertically to a line, B expands.",
        ),
        // --- zoom ---------------------------------------------------------------
        t(
            "zoomin",
            "zoom",
            Some("in"),
            "B zooms in over A (punch-in).",
        ),
        // --- wind (gusty streak wipe) -------------------------------------------
        t(
            "hlwind",
            "wind",
            Some("left"),
            "Wind-streak wipe to the left.",
        ),
        t(
            "hrwind",
            "wind",
            Some("right"),
            "Wind-streak wipe to the right.",
        ),
        t("vuwind", "wind", Some("up"), "Wind-streak wipe upward."),
        t("vdwind", "wind", Some("down"), "Wind-streak wipe downward."),
    ]
}

/// A media-backed clip: plays asset content [src_in_ms, src_out_ms).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaClip {
    pub id: String,
    /// Asset id (key into Project::assets).
    pub asset: String,
    pub src_in_ms: u64,
    pub src_out_ms: u64,
    /// Per-clip visual effects (edit.effect), applied in order. Empty = none;
    /// serde-default keeps pre-effects clips/logs deserializing unchanged.
    #[serde(default)]
    pub effects: Vec<ClipEffect>,
    /// Per-clip audio gain in dB (0 = unity).
    #[serde(default)]
    pub gain_db: f64,
    /// Overlay geometry (edit.transform); None = full-frame. Only read for
    /// clips on overlay video tracks (the renderer composites those above
    /// the base track).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<ClipTransform>,
    /// Source crop rectangle (`edit.crop`); None = no crop (whole source
    /// frame). Source px, applied BEFORE conform/transform (see ClipCrop).
    /// serde-skip-None keeps older op logs replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<ClipCrop>,
    /// Linear fade in/out (edit.fade); None = hard cuts on both ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade: Option<ClipFade>,
    /// Crossfade-IN length, ms (the `edit.crossfade` verb's storage).
    /// 0 = a hard cut into this clip (the default). When > 0 AND a MEDIA clip
    /// immediately precedes this one on the same track, the renderer dissolves
    /// the cut between them over this many ms (video `xfade`, audio
    /// `acrossfade`) instead of a hard concat — the overlap is taken from the
    /// preceding clip's TAIL and this clip's HEAD (both must carry ≥ this many
    /// ms of source on the relevant side), so the realized timeline SHORTENS by
    /// this amount across the crossfade (standard NLE centred-on-the-cut
    /// dissolve; verified: ffmpeg `xfade=duration=D` yields len_a+len_b-D).
    ///
    /// Stored on the RIGHT clip of the pair (the clip whose start IS the cut
    /// point) so it travels with that clip through ripples — the same doctrine
    /// as `fade`. A split of the right clip keeps the crossfade-in on the LEFT
    /// half (it owns the original beginning); ripple_delete that removes the
    /// preceding clip clears any now-dangling crossfade (no left neighbour to
    /// dissolve from). The renderer clamps an overlap longer than either
    /// adjacent clip. serde-skip-default keeps older op logs replaying
    /// byte-identical.
    ///
    /// INTERACTION WITH edit.fade: a crossfade OWNS the cut between two clips.
    /// A per-clip fade-out on the LEFT clip + a crossfade-in on the right would
    /// double-dip the boundary, so `edit.crossfade` clears the left clip's
    /// fade-out and this clip's fade-in when it sets the crossfade (recorded on
    /// the op). The two clips' OTHER ends keep their fades.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub xfade_in_ms: u64,
    /// Crossfade TRANSITION style (edit.crossfade `transition` arg). None / "fade"
    /// = the classic dissolve (the only style before this field, so serde-skip-None
    /// keeps every pre-existing log replaying byte-identical). Other values are
    /// ffmpeg `xfade` transition names — wipeleft, slideup, circleopen, fadeblack,
    /// pixelize, … — validated against TRANSITIONS at the verb boundary. Applies to
    /// the VIDEO seam only (audio is always a smooth `acrossfade`); ignored when
    /// `xfade_in_ms == 0`. Travels with the right clip like `xfade_in_ms`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xfade_kind: Option<String>,
    /// Playback speed factor (the `edit.speed` verb's storage; retime/slow-mo).
    /// 1.0 = normal. >1 plays the clip FASTER, so it occupies LESS timeline
    /// (`src_span / speed`); <1 is slow-motion (occupies more). Range enforced
    /// 0.25–4.0 at verb time. The source range [src_in,src_out) is UNCHANGED by
    /// speed — speed only remaps how that span lays onto the timeline
    /// (`Clip::timeline_duration_ms` divides by it; source↔timeline mapping
    /// routes through `src_off_to_tl`/`tl_off_to_src`) and how the renderer
    /// time-stretches it (video `setpts={1/speed}`, audio `rubberband`/`atempo`
    /// at `tempo=speed`). A split inherits the parent's speed onto both halves.
    /// serde-skip at 1.0 keeps pre-speed op logs / projects replaying
    /// byte-identical.
    #[serde(default = "default_speed", skip_serializing_if = "is_unit_speed")]
    pub speed: f64,
    /// Color grade (edit.grade); None = ungraded (footage colors as-is).
    /// serde-skip-None keeps pre-grade op logs / projects replaying
    /// byte-identical. Read for ALL media clips (base + overlay).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<ClipGrade>,
    /// LAYERED color grade — a node-stack of grades applied IN ORDER (edit.grade_stack);
    /// a serial grading workflow vs the single `grade` above. Empty = use the
    /// single `grade` field (the legacy byte-identical path). When NON-empty this is the
    /// authority: `grade` is cleared (set None) by edit.grade_stack, and the renderer
    /// emits each layer's grade filter in sequence (so layer 2 grades layer 1's output,
    /// etc.). A single-element stack emits the EXACT same filter as the equivalent
    /// single `grade`, so it renders byte-identical to edit.grade. serde-skip-empty keeps
    /// pre-stack op logs / projects replaying byte-identical. Read for ALL media clips
    /// (base + overlay).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grade_stack: Vec<ClipGrade>,
    /// GEOMETRIC POWER WINDOWS — region-scoped grades (edit.grade_window). Each entry
    /// grades ONLY inside its [`WindowShape`] region; multiple windows STACK (composited
    /// IN ORDER, each over the previous). Empty = no windows (the byte-identical default —
    /// the renderer emits NO window composite at all). A region-scoped grade, vs the
    /// whole-frame `grade` / `grade_stack` above. serde-skip-empty keeps pre-window op
    /// logs / projects replaying byte-identical. Read for BASE-track video clips (v1);
    /// routes the timeline to the SOFTWARE render path (region composite).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grade_windows: Vec<GradeWindow>,
    /// AI background matte (edit.matte); None = no matte. Some(..) cuts the
    /// subject out via a baked straight-alpha (RVM) so the background is removed
    /// (reveal the lower track) or replaced. CPU/external-service path — opts the
    /// timeline out of the GPU fast-track. Read on overlay clips for `remove`
    /// (needs a lower track to reveal). serde-skip-None keeps pre-matte op logs /
    /// projects replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matte: Option<ClipMatte>,
    /// Vector/freeform MASK (edit.add_mask); None = no mask. Some(..) applies an
    /// effect (blur/pixelate/black) inside a shape region via a baked feathered
    /// alpha + maskedmerge — the region-blur / privacy-redaction primitive. CPU-only
    /// (opts the timeline onto the software path). serde-skip-None keeps pre-mask op
    /// logs / projects replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<ClipMask>,
    /// Reverse playback (edit.reverse). false = normal. When true the renderer
    /// emits `reverse` (video) / `areverse` (audio) so the clip plays BACKWARD;
    /// the timeline duration is UNCHANGED (a reversed N-frame clip is still N
    /// frames). CPU-only (buffers the whole clip in RAM — the verb fences clip
    /// size at the dispatch layer), so a reversed clip opts the timeline out of
    /// the GPU fast-track. serde-skip at false keeps pre-reverse logs/projects
    /// replaying byte-identical.
    #[serde(default, skip_serializing_if = "is_false")]
    pub reverse: bool,
    /// Freeze-frame (edit.freeze); None = plays normally. Some(at_ms) HOLDS the
    /// single source frame at `at_ms` (offset into the clip's visible range) for
    /// the clip's WHOLE timeline slot — the renderer trims to that one frame and
    /// clones it (`tpad`). Audio plays through untouched (the common "freeze the
    /// picture, audio rolls" effect); a frozen clip ignores `speed` (a held frame
    /// has no speed). CPU-only → opts the timeline out of the GPU fast-track.
    /// serde-skip-None keeps pre-freeze logs/projects replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeze: Option<ClipFreeze>,
    /// Ken Burns pan/zoom animation (edit.animate); None = static. The renderer
    /// linearly interpolates the zoom window from `from` to `to` across the clip's
    /// frames (ffmpeg `zoompan`, with the measured `setpts=N/(fps*TB)` PTS rebuild
    /// that keeps the frame count exact). CPU-only → opts the timeline out of the
    /// GPU fast-track. serde-skip-None keeps pre-animate logs/projects replaying
    /// byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<ClipAnimation>,
    /// Parameter keyframes (edit.keyframe); empty = none. Each entry animates ONE
    /// parameter (opacity, volume, …) over time via an ffmpeg time-expression. A
    /// keyframed parameter OVERRIDES its static counterpart (a keyframed opacity
    /// supersedes transform.opacity; a keyframed volume supersedes gain_db).
    /// CPU-only → opts the timeline out of the GPU fast-track. serde-skip-empty
    /// keeps pre-keyframe logs/projects replaying byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyframes: Vec<Keyframe>,
    /// Parametric audio EQ (edit.eq); None = no EQ. High-pass + peaking bands +
    /// low-pass on the clip's audio. Read for ALL audio-bearing clips. Audio-only
    /// → does NOT opt the timeline out of the GPU fast-track (the GPU path reuses
    /// the software audio chain verbatim). serde-skip-None keeps pre-EQ op logs /
    /// projects replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<ClipEq>,
    /// Non-destructive MUTE ranges (edit.mute_range / transcript.mute_words);
    /// empty = none. Each `[start_ms, end_ms)` is in SOURCE-ASSET time (the same
    /// clock as src_in/src_out), so a mute stays glued to the spoken content
    /// through trims / slips / splits with NO op rewriting — deliberately unlike
    /// fades/keyframes (clip-local) and duck windows (timeline-time, which need
    /// remapping). The renderer gates the clip's audio volume to 0 over each
    /// range's overlap with the visible source window (post-speed mapping;
    /// reverse handled by mirroring; speed RAMPS are refused at edit time — the
    /// piecewise mapping would silently drift). Kept sorted + merged (normalize
    /// on every edit). Audio-only → does NOT opt the timeline out of the GPU
    /// fast-track (like eq). serde-skip-empty keeps pre-mute logs/projects
    /// replaying byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mute_ranges: Vec<[u64; 2]>,
    /// Video stabilization (edit.stabilize); None = not stabilized. CPU-only + needs
    /// a `vidstabdetect` pre-pass → opts the timeline out of the GPU fast-track.
    /// serde-skip-None keeps pre-stabilize logs/projects replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stabilize: Option<ClipStabilize>,
    /// VARIABLE speed / time remap (edit.speed_ramp); None = constant speed (the
    /// scalar `speed` field). Some(..) is a piecewise-LINEAR speed curve over the
    /// clip's source window — a speed ramp (normal → fast →
    /// normal for a dramatic beat). Unlike `speed` (one constant factor), the ramp
    /// is realized at EDL-DERIVATION time: `edl_from_project` EXPANDS the clip into
    /// `ramp.segments` contiguous CONSTANT-speed sub-segments sampled from the curve
    /// (see `speed_ramp_segments`), each rendered by the proven per-segment
    /// setpts/atempo path — so the ramp needs NO render change and NO new clip ids
    /// (one op, replay-safe like `keyframes`). The realized timeline length is the
    /// integral of (1/speed) over the source = the sum of the sub-segment durations
    /// (`timeline_duration_ms` routes through the same `speed_ramp_segments`). A
    /// ramped clip is treated as RETIMED (`is_retimed`) — verbs that map a timeline
    /// position back to source (split/trim/sub-range paste/detach/split_edit) refuse
    /// it (non-linear map), and it is mutually exclusive with the scalar `speed` and
    /// the time/baked features (reverse/freeze/animation/keyframes/matte/mask/
    /// stabilize) so the sub-segmentation stays correct. serde-skip-None keeps
    /// pre-ramp logs/projects replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_ramp: Option<SpeedRamp>,
    /// INPUT color space of this clip's source footage (`edit.color_space`). None =
    /// the source is assumed to already be in the project WORKING space (the common
    /// case → no input→working transform). Some(space) tags a log/sRGB/Rec.2020
    /// source so the renderer converts it INTO the working space before grade/effects
    /// (and on to the output space). serde-skip-None keeps pre-color-management op
    /// logs / projects replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_color_space: Option<ColorSpace>,
    /// COMPOUND CLIP / NEST reference (`edit.nest`); None = an ordinary media clip.
    /// Some(nest_id) marks this clip as a NEST: its content is the sub-timeline
    /// stored in `Project::nests` under that id (a group of clips collapsed into a
    /// single clip on the parent track, a nested compound clip).
    /// The clip still occupies `[src_in_ms, src_out_ms)` of the nest's BAKED render
    /// (src_in 0 .. the nest's combined span); `asset` carries the SAME nest id (the
    /// clip's "source" IS the nest) — a content-addressed bake replaces it with the
    /// rendered file at render time (server `nest::bake_and_flatten`, mirroring the
    /// matte bake), so the main renderer needs no nest awareness and a project with no
    /// nest renders byte-identical. serde-skip-None keeps pre-nest op logs / projects
    /// replaying byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nest: Option<String>,
}

impl MediaClip {
    /// True when this clip is a NEST (compound clip): its content is a sub-timeline
    /// in `Project::nests`, not a plain imported asset. The renderer bakes it first.
    pub fn is_nest(&self) -> bool {
        self.nest.is_some()
    }

    /// True when the clip is time-remapped: a constant `speed` ≠ 1, OR a variable
    /// `speed_ramp`. In both cases the timeline↔source mapping is non-identity (and
    /// non-LINEAR for a ramp), so any verb that maps a TIMELINE position back to a
    /// SOURCE position (sub-range paste, detach, split-edit roll) must refuse it.
    pub fn is_retimed(&self) -> bool {
        (self.speed - 1.0).abs() > f64::EPSILON || self.speed_ramp.is_some()
    }

    /// True when the clip carries a variable-speed ramp (edit.speed_ramp).
    pub fn has_speed_ramp(&self) -> bool {
        self.speed_ramp.is_some()
    }
}

/// Default number of constant-speed sub-segments a speed ramp is sampled into
/// when the verb omits `segments` — smooth enough for a multi-second dramatic
/// ramp (~one slice per ~100 ms on a 2–3 s clip) without an oversized filtergraph.
pub const DEFAULT_RAMP_SEGMENTS: usize = 24;
/// Minimum / maximum sub-segment count (the verb clamps `segments` here): ≥ 2 so a
/// ramp is at least two distinct speeds; ≤ 120 to bound the per-clip filtergraph.
pub const MIN_RAMP_SEGMENTS: usize = 2;
pub const MAX_RAMP_SEGMENTS: usize = 120;
/// Frame-aware cap floor: each constant-speed sub-segment must render to at least
/// this many OUTPUT frames at the ramp's FASTEST point. The per-segment trim+concat
/// quantises each sub-segment to whole video frames; if a sub-segment is sub-frame
/// (too many segments on a short/fast clip) the video concat drops it while the
/// sample-accurate audio atempo does not, accumulating into visible A/V drift
///. `edit.speed_ramp` clamps the
/// requested `segments` so this never happens — keeping the realized video and audio
/// lengths within a frame of each other.
pub const MIN_FRAMES_PER_SUBSEG: usize = 4;

/// One control point of a [`SpeedRamp`]: at clip-local SOURCE offset `at_ms` (ms
/// into the clip's `[src_in, src_out)` window, measured at NATURAL 1× speed —
/// 0 = the clip's start, `src_out − src_in` = its natural end), the playback
/// speed is `factor` (>1 = faster, <1 = slow-motion). The ramp interpolates the
/// factor LINEARLY between successive points and HOLDS the first/last factor
/// outside the points' range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeedRampPoint {
    /// Source offset into the clip's window, ms (at natural 1× speed).
    pub at_ms: u64,
    /// Playback speed factor at this point (0.25–4.0, enforced at verb time).
    pub factor: f64,
}

/// Variable-speed time remap (the `edit.speed_ramp` verb's storage). A piecewise-
/// LINEAR speed curve over the clip's source window, realized at EDL-derivation
/// time as `segments` contiguous CONSTANT-speed sub-segments (each a midpoint
/// sample of the curve over an equal slice of the source). More segments = a
/// smoother ramp at the cost of more filtergraph nodes. The points carry the
/// curve; `segments` the current grid-safe sampling granularity. New frame-aware
/// ramps also retain their bounded requested granularity so a temporary lower-FPS
/// format can reduce the effective filtergraph without permanently coarsening the
/// curve. Historic ramps have no retained preference and keep millisecond replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeedRamp {
    /// Control points, sorted strictly ascending by `at_ms` (≥ 2, enforced at verb
    /// time). The speed between points is linearly interpolated; before the first /
    /// after the last point the nearest factor is held.
    pub points: Vec<SpeedRampPoint>,
    /// Number of constant-speed sub-segments the curve is sampled into at render
    /// (2–120, after the active output grid's frame-safety cap).
    pub segments: usize,
    /// Original bounded `segments` request for a frame-aware ramp. The effective
    /// [`Self::segments`] is recomputed from this preference on each project-format
    /// regrid, so lowering and then restoring the FPS restores the requested curve
    /// detail where the safety cap allows it. Absent on historic ramps and older
    /// frame-aware project caches; their existing effective value is used as the
    /// backward-compatible preference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_segments: Option<usize>,
    /// Project frame rate resolved when the ramp is committed. New ramps use
    /// this frame grid for one authoritative duration; absent ramps preserve
    /// the historic millisecond interpretation on replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timebase_fps: Option<f64>,
    /// Project audio rate resolved with `timebase_fps`; keeps each ramp slice
    /// on the same sample budget as its frame budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timebase_audio_rate: Option<u32>,
}

/// One constant-speed sub-segment of an expanded speed ramp: the source span
/// `[src_in, src_out)` plays at constant `speed`, occupying `dur_ms` of timeline.
/// Produced by [`speed_ramp_segments`]; consumed by `Clip::timeline_duration_ms`
/// (sums `dur_ms`) and `edl_from_project` (emits one EDL segment per entry).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RampSeg {
    pub src_in: u64,
    pub src_out: u64,
    pub speed: f64,
    pub dur_ms: u64,
    /// Whole output frames assigned from the ramp's cumulative frame budget.
    /// `None` denotes the historical millisecond-only replay path.
    pub frame_count: Option<u64>,
    /// Audio samples allocated from the same cumulative timebase as frames.
    pub sample_count: Option<u64>,
}

/// Linearly interpolate a [`SpeedRamp`]'s speed factor at source offset `off`
/// (ms into the clip's window). Holds the first/last point's factor outside the
/// points' range. `points` is assumed sorted ascending by `at_ms` (the verb
/// enforces it) and non-empty (the verb enforces ≥ 2).
pub fn speed_ramp_factor_at(ramp: &SpeedRamp, off: u64) -> f64 {
    let pts = &ramp.points;
    // Empty is impossible after verb validation, but stay total for safety.
    let Some(first) = pts.first() else {
        return 1.0;
    };
    if off <= first.at_ms {
        return first.factor;
    }
    let last = pts.last().unwrap();
    if off >= last.at_ms {
        return last.factor;
    }
    // Find the bracketing pair [a, b) with a.at_ms <= off < b.at_ms.
    for w in pts.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if off >= a.at_ms && off < b.at_ms {
            let span = (b.at_ms - a.at_ms) as f64;
            if span <= 0.0 {
                return a.factor;
            }
            let frac = (off - a.at_ms) as f64 / span;
            return a.factor + (b.factor - a.factor) * frac;
        }
    }
    last.factor
}

/// Partition a ramped clip's source window `[src_in, src_out)` into
/// `ramp.segments` contiguous CONSTANT-speed sub-segments, each sampling the
/// curve at its source MIDPOINT. The source slices tile the window EXACTLY (u128
/// integer partition — no rounding gap or overlap). Ramps with a persisted
/// frame rate allocate their output from one cumulative frame/sample budget;
/// older ramps without that timebase retain their historical per-slice
/// millisecond rounding. SHARED by `Clip::timeline_duration_ms` (sum) and
/// `edl_from_project` (emit) so the cursor math and the rendered segments use
/// the same expansion. Empty source slices are dropped; non-empty slices that
/// round to zero output frames merge their source span into an adjacent emitted
/// segment so source coverage stays contiguous.
pub fn speed_ramp_segments(src_in: u64, src_out: u64, ramp: &SpeedRamp) -> Vec<RampSeg> {
    crate::speed_ramp_timing::speed_ramp_segments(src_in, src_out, ramp)
}

/// One animated parameter (edit.keyframe): a `param` and its time/value `points`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub param: KfParam,
    /// Control points; the renderer linearly interpolates between them (or steps,
    /// for `interp = "hold"`), clamping to the first/last value outside the range.
    pub points: Vec<KfPoint>,
    /// Interpolation between points. Default linear; "hold" = stepped.
    #[serde(default, skip_serializing_if = "KfInterp::is_default")]
    pub interp: KfInterp,
}

/// A keyframe control point: `value` at clip-local time `t_ms`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KfPoint {
    pub t_ms: u64,
    pub value: f64,
}

/// The animatable parameters (edit.keyframe). Extensible: a new param = a variant
/// + a render-emit arm. opacity (overlay alpha) and volume (audio gain) are the v1
/// set; grade/position keyframes reuse the same machinery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KfParam {
    /// Overlay alpha 0..1 (overlay video clips). Animated fade in/out.
    Opacity,
    /// Linear volume multiplier (audio-bearing clips; 1 = unity). Audio automation.
    Volume,
    /// Overlay top-left X as a fraction of frame width (video clips). Animated PiP
    /// position — slide a logo/webcam across the frame. Values are NOT clamped to
    /// [0,1] so the overlay can slide in from / out to off-screen (negative or >1).
    /// Effective on OVERLAY clips (the base ignores position, like a static transform).
    PosX,
    /// Overlay top-left Y as a fraction of frame height (video clips). Pairs with
    /// `PosX` for full 2-D animated PiP motion. Not clamped (allows off-screen).
    PosY,
    /// Uniform scale multiplier (video clips; 1 = native size). Animated zoom — the
    /// multi-keyframe, eased generalization of `edit.animate`'s 2-state Ken Burns.
    /// On a BASE clip it lowers to a `zoompan` (centred zoom, the picture is kept at
    /// frame size); on an OVERLAY clip it lowers to `scale=…:eval=frame` (the PiP box
    /// grows/shrinks). Values are clamped to [1, 10] at render (zoompan requires z≥1;
    /// the same ceiling as `edit.animate`). MUTUALLY EXCLUSIVE with `edit.animate` on
    /// a clip — the keyframe channel IS the richer form (validated in `edit::keyframe`).
    /// This is the native target the integrated recorder's eased auto-zoom lowers onto.
    Scale,
}

/// Keyframe interpolation mode. `linear`/`hold` are the originals (and render
/// byte-identical to before, so existing projects replay unchanged). The `ease_*`
/// variants are the Penner curve set — they reshape the linear inter-keyframe
/// fraction `f∈[0,1]` before the value lerp, so motion reads professional rather
/// than mechanical. `back`/`elastic`
/// overshoot past the endpoints (intentional — anticipation / spring feel); `bounce`
/// settles with decaying hops. [`KfInterp::sample`] is the pure-Rust reference the
/// render's ffmpeg expression mirrors exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KfInterp {
    #[default]
    Linear,
    Hold,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
    EaseInCubic,
    EaseOutCubic,
    EaseInOutCubic,
    EaseInExpo,
    EaseOutExpo,
    EaseInOutExpo,
    EaseInBack,
    EaseOutBack,
    EaseInOutBack,
    EaseInElastic,
    EaseOutElastic,
    EaseInOutElastic,
    EaseInBounce,
    EaseOutBounce,
    EaseInOutBounce,
}

impl KfInterp {
    fn is_default(&self) -> bool {
        matches!(self, KfInterp::Linear)
    }

    /// True for an `ease_*` variant (i.e. neither `linear` nor `hold`). The render
    /// uses this to decide whether to emit the eased-fraction expression branch.
    pub fn is_eased(self) -> bool {
        !matches!(self, KfInterp::Linear | KfInterp::Hold)
    }

    /// Map a linear inter-keyframe fraction `f` to its eased fraction (Penner set),
    /// `f` clamped to `[0,1]`. `linear` is the identity; `hold` returns 0 (the
    /// stepped value is the segment's start, handled by the caller — sampling hold
    /// here is only meaningful at the endpoints). This is the SOURCE OF TRUTH the
    /// ffmpeg lowering (`render::ease_frac_expr`) must reproduce; the live render
    /// proof checks the two agree. Endpoints are exact: `sample(0)=0`, `sample(1)=1`
    /// for every monotone curve (back/elastic/bounce may exceed `[0,1]` in between).
    pub fn sample(self, f: f64) -> f64 {
        use std::f64::consts::PI;
        let f = f.clamp(0.0, 1.0);
        // Penner constants (easings.org).
        let c1 = 1.701_58_f64; // back overshoot
        let c3 = c1 + 1.0;
        let c2 = c1 * 1.525;
        let c4 = (2.0 * PI) / 3.0; // elastic
        let c5 = (2.0 * PI) / 4.5;
        let n1 = 7.5625_f64; // bounce
        let d1 = 2.75_f64;
        let out_bounce = |x: f64| -> f64 {
            if x < 1.0 / d1 {
                n1 * x * x
            } else if x < 2.0 / d1 {
                let x = x - 1.5 / d1;
                n1 * x * x + 0.75
            } else if x < 2.5 / d1 {
                let x = x - 2.25 / d1;
                n1 * x * x + 0.9375
            } else {
                let x = x - 2.625 / d1;
                n1 * x * x + 0.984_375
            }
        };
        match self {
            KfInterp::Linear => f,
            KfInterp::Hold => 0.0,
            KfInterp::EaseInQuad => f * f,
            KfInterp::EaseOutQuad => 1.0 - (1.0 - f) * (1.0 - f),
            KfInterp::EaseInOutQuad => {
                if f < 0.5 {
                    2.0 * f * f
                } else {
                    1.0 - (-2.0 * f + 2.0).powi(2) / 2.0
                }
            }
            KfInterp::EaseInCubic => f * f * f,
            KfInterp::EaseOutCubic => 1.0 - (1.0 - f).powi(3),
            KfInterp::EaseInOutCubic => {
                if f < 0.5 {
                    4.0 * f * f * f
                } else {
                    1.0 - (-2.0 * f + 2.0).powi(3) / 2.0
                }
            }
            KfInterp::EaseInExpo => {
                if f <= 0.0 {
                    0.0
                } else {
                    (2.0_f64).powf(10.0 * f - 10.0)
                }
            }
            KfInterp::EaseOutExpo => {
                if f >= 1.0 {
                    1.0
                } else {
                    1.0 - (2.0_f64).powf(-10.0 * f)
                }
            }
            KfInterp::EaseInOutExpo => {
                if f <= 0.0 {
                    0.0
                } else if f >= 1.0 {
                    1.0
                } else if f < 0.5 {
                    (2.0_f64).powf(20.0 * f - 10.0) / 2.0
                } else {
                    (2.0 - (2.0_f64).powf(-20.0 * f + 10.0)) / 2.0
                }
            }
            KfInterp::EaseInBack => c3 * f * f * f - c1 * f * f,
            KfInterp::EaseOutBack => 1.0 + c3 * (f - 1.0).powi(3) + c1 * (f - 1.0).powi(2),
            KfInterp::EaseInOutBack => {
                if f < 0.5 {
                    let g = 2.0 * f;
                    (g * g * ((c2 + 1.0) * g - c2)) / 2.0
                } else {
                    let g = 2.0 * f - 2.0;
                    (g * g * ((c2 + 1.0) * g + c2) + 2.0) / 2.0
                }
            }
            KfInterp::EaseInElastic => {
                if f <= 0.0 {
                    0.0
                } else if f >= 1.0 {
                    1.0
                } else {
                    -(2.0_f64).powf(10.0 * f - 10.0) * ((10.0 * f - 10.75) * c4).sin()
                }
            }
            KfInterp::EaseOutElastic => {
                if f <= 0.0 {
                    0.0
                } else if f >= 1.0 {
                    1.0
                } else {
                    (2.0_f64).powf(-10.0 * f) * ((10.0 * f - 0.75) * c4).sin() + 1.0
                }
            }
            KfInterp::EaseInOutElastic => {
                if f <= 0.0 {
                    0.0
                } else if f >= 1.0 {
                    1.0
                } else if f < 0.5 {
                    -((2.0_f64).powf(20.0 * f - 10.0) * ((20.0 * f - 11.125) * c5).sin()) / 2.0
                } else {
                    ((2.0_f64).powf(-20.0 * f + 10.0) * ((20.0 * f - 11.125) * c5).sin()) / 2.0
                        + 1.0
                }
            }
            KfInterp::EaseInBounce => 1.0 - out_bounce(1.0 - f),
            KfInterp::EaseOutBounce => out_bounce(f),
            KfInterp::EaseInOutBounce => {
                if f < 0.5 {
                    (1.0 - out_bounce(1.0 - 2.0 * f)) / 2.0
                } else {
                    (1.0 + out_bounce(2.0 * f - 1.0)) / 2.0
                }
            }
        }
    }
}

/// Freeze-frame spec (edit.freeze). `at_ms` is the offset INTO the clip's visible
/// source range (0 = the clip's first frame) of the frame to hold; the renderer
/// clamps it to the last available frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipFreeze {
    pub at_ms: u64,
}

/// One end-state of a Ken Burns animation (edit.animate). `zoom` >= 1.0 (1.0 = no
/// zoom — the whole frame); `x`/`y` = the normalized focal CENTRE the zoom window
/// is centred on (0..1; 0.5,0.5 = frame centre). The renderer interpolates
/// linearly from a clip's `from` AnimState to its `to` over the clip's frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimState {
    #[serde(default = "anim_zoom_one")]
    pub zoom: f64,
    #[serde(default = "anim_half")]
    pub x: f64,
    #[serde(default = "anim_half")]
    pub y: f64,
}

fn anim_zoom_one() -> f64 {
    1.0
}
fn anim_half() -> f64 {
    0.5
}

impl Default for AnimState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            x: 0.5,
            y: 0.5,
        }
    }
}

/// Ken Burns pan/zoom animation for a clip (edit.animate): linear interpolation
/// from `from` to `to` across the clip's frames. CPU-only (ffmpeg `zoompan`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipAnimation {
    pub from: AnimState,
    pub to: AnimState,
}

impl ClipAnimation {
    /// True when the animation does NOTHING (both ends are the identity frame —
    /// no zoom, centred). Stored as None so a cleared animation replays
    /// byte-identical to never-animated.
    pub fn is_identity(&self) -> bool {
        let id = |s: &AnimState| s.zoom == 1.0 && s.x == 0.5 && s.y == 0.5;
        id(&self.from) && id(&self.to)
    }
}

/// serde skip-predicate: a zero crossfade is the default (no crossfade) and is
/// omitted from JSON so older op logs / projects round-trip byte-identical.
fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

/// serde skip-predicate: `false` is the default for a bool flag (the feature off)
/// and is omitted from JSON so pre-feature op logs / projects round-trip
/// byte-identical. `pub` so the EDL (cut_core::edl) shares the one predicate.
pub fn is_false(v: &bool) -> bool {
    !*v
}

/// serde default/skip helper: `true` is the default for positive feature flags
/// such as visual track visibility. Missing old-project fields replay visible.
pub fn default_true() -> bool {
    true
}

/// serde skip helper: omit a bool when it is the default `true`.
pub fn is_true(v: &bool) -> bool {
    *v
}

/// A gap (empty time) on a track. `kind` is the literal "gap" in JSON.
///
/// Gaps are ANONYMOUS by design (the anonymous-gap contract): no `id` field — they are inserted
/// automatically (ripple insert/move splices them into sibling tracks;
/// `ripple_delete{ripple:false}` leaves one) and addressed by POSITION, never by
/// id. `Clip::id()` returns `None` for a gap; agents iterating `track.clips`
/// must special-case `kind:"gap"` before keying on `id`. Documented in the skill
/// reference under "Clip shapes on a track".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GapClip {
    /// Always "gap" — kept as a field so untagged (de)serialization round-trips.
    pub kind: String,
    pub duration_ms: u64,
}

impl GapClip {
    pub fn new(duration_ms: u64) -> Self {
        Self {
            kind: "gap".into(),
            duration_ms,
        }
    }
}

/// A caption clip on a caption track. `range_ms` is [start, end) on the
/// TIMELINE (not source time) — captions belong to the composition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionClip {
    pub id: String,
    pub text: String,
    /// Key into Project::caption_styles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_ref: Option<String>,
    pub range_ms: [u64; 2],
}

/// A timed gain reduction on an audio track (the `edit.duck` verb's storage).
/// `range_ms` is ABSOLUTE timeline time ([start, end) of the full-depth
/// plateau); `db` is the reduction applied inside it (negative); `attack_ms`
/// is the linear ramp length on EACH side (gain ramps down over
/// [start-attack, start] and back up over [end, end+attack]).
///
/// HONEST IMPLEMENTATION NOTE: ducking is WINDOWED GAIN, not a sidechain
/// compressor — windows are computed ONCE (by edit.duck, from the
/// against-track's perception silences mapped through the EDL) and recorded
/// on the op, then applied at render time as a deterministic volume
/// expression. Same input + EDL ⇒ same output hash holds; the exact db
/// amount is honored; but audio that appears on the against-track AFTER the
/// duck was computed is not reacted to — re-run edit.duck after timeline
/// changes that add/move speech. Timeline ripples DO remap existing windows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GainWindow {
    pub range_ms: [u64; 2],
    pub db: f64,
    pub attack_ms: u64,
}

/// A track: ordered, non-overlapping clips of one kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub kind: TrackKind,
    pub clips: Vec<Clip>,
    /// Track-level gain in dB, applied on top of clip gain (audio tracks).
    #[serde(default)]
    pub gain_db: f64,
    /// Timed gain reductions (edit.duck). Empty = none; omitted in JSON.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gain_windows: Vec<GainWindow>,
    /// LAYER blend mode (edit.blend) for an OVERLAY video track — how this whole
    /// track composites onto everything below it (multiply/screen/overlay/…). None
    /// or "normal" = the default alpha-over composite. Only meaningful on overlay
    /// video tracks (the base canvas + audio/caption tracks ignore it). CPU-only →
    /// opts the timeline out of the GPU fast-track. serde-skip-None keeps pre-blend
    /// projects byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend_mode: Option<String>,
    /// VISUAL track visibility (verb `edit.track_visible`). False hides video or
    /// caption track output from preview/export while keeping the clips editable
    /// and in place. Audio tracks use `edit.mute` instead; visibility defaults
    /// true so older projects render exactly as before.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub visible: bool,
    /// UI edit lock (verb `edit.track_lock`). True blocks timeline editing
    /// gestures/drops on this track without mutating clips. The engine persists
    /// the flag so the app can keep accidental-edit protection across reloads.
    #[serde(default, skip_serializing_if = "is_false")]
    pub locked: bool,
    /// NON-DESTRUCTIVE AUDIO-track MUTE (verb `edit.mute`). True = this track
    /// contributes SILENCE to the audio mix; the track's `gain_db` is left
    /// UNTOUCHED, so a dialed-in level survives mute/unmute and a reload (mute is a
    /// flag, NOT a -100 dB gain write — that older mechanism destroyed the level on
    /// reload). Honored at the mix stage (the render/preview/export audio loop) via
    /// [`Project::audio_track_audible`]. serde-default + skip-when-false keeps
    /// pre-mute projects byte-identical on round-trip.
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
    /// NON-DESTRUCTIVE AUDIO-track SOLO (verb `edit.solo`). When ANY audio track
    /// has `solo == true`, ONLY soloed audio tracks are audible and every other
    /// audio track contributes silence — WITHOUT touching any gain. An explicit `muted` still
    /// wins over solo (a muted track is silent even if soloed). serde-default +
    /// skip-when-false → byte-identical pre-solo projects.
    #[serde(default, skip_serializing_if = "is_false")]
    pub solo: bool,
    /// NON-DESTRUCTIVE AUDIO-track stereo PAN/balance (verb `edit.pan`), −1.0
    /// (full left) … 0.0 (center) … +1.0 (full right). BALANCE semantics on the
    /// stereo-conformed track chain: center = unity (no filter emitted — byte-
    /// identical mix), panning ATTENUATES the opposite channel on a cosine
    /// taper and never boosts (no clipping headroom risk). Applied at the mix
    /// stage after concat/duck, so render, preview, and export.audio stems all
    /// agree. Like mute/solo: a flag, independent of `gain_db`; serde-default +
    /// skip-when-0 keeps pre-pan projects byte-identical.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub pan: f64,
}

/// serde skip helper: `0.0` = the field's default (center pan / no effect).
pub fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

impl Track {
    /// Timeline END of this track in ms.
    ///
    /// Two time models coexist (timeline/op-log contract, edit.rs header):
    /// - video/audio tracks are CUMULATIVE — clips occupy consecutive time,
    ///   so the track end is the sum of clip durations;
    /// - caption tracks carry ABSOLUTE `range_ms` — the track end is the max
    ///   range end, NOT the sum. Summing caption durations inflated
    ///   `Project::duration_ms()` past the real composition end, which made
    ///   `edl.duration_ms` disagree with rendered output. The mismatch equals the
    ///   caption-clip duration sum minus the real caption track end.
    pub fn duration_ms(&self) -> u64 {
        match self.kind {
            TrackKind::Caption => self
                .clips
                .iter()
                .filter_map(|c| match c {
                    Clip::Caption(cc) => Some(cc.range_ms[1]),
                    _ => None, // malformed caption track: do not mix duration with absolute ends
                })
                .max()
                .unwrap_or(0),
            _ => self.clips.iter().map(|c| c.timeline_duration_ms()).sum(),
        }
    }
}

/// A timeline marker (timeline/op-log contract `markers`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub id: String,
    pub at_ms: u64,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Display color: one of [`MARKER_COLORS`]. Absent =
    /// the default marker look. Optional + defaulted so pre-color projects
    /// load unchanged and non-colored markers serialize byte-identically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// The valid `Marker::color` names. A closed set — the UI maps each to a
/// theme swatch, and a typo'd color must fail the verb, not silently render
/// as the default.
pub const MARKER_COLORS: [&str; 8] = [
    "red", "orange", "yellow", "green", "teal", "blue", "purple", "pink",
];

/// A named caption style (timeline/op-log contract `caption_styles`). Free-form-ish but typed
/// on the fields the renderer needs; `extra` keeps unknown keys round-tripping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptionStyle {
    pub font: String,
    pub size: u32,
    /// CSS-style color, e.g. "#fff".
    pub color: String,
    /// Background color incl. alpha, e.g. "#000a".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    /// Position keyword: "bottom" | "top" | "center" (renderer maps to ASS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<String>,
    /// Forward-compat: unknown style keys survive load/save.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Project-owned brand constraints used by `verify.brand` and publish packages.
///
/// Every field is optional because a brand may pin only a subset of the editor's
/// choices. The kit itself must contain at least one constraint; callers pass the
/// full replacement snapshot through `project.brand`, which keeps replay simple
/// and makes clearing explicit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrandKit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fonts: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_size: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
}

impl BrandKit {
    pub fn has_constraints(&self) -> bool {
        self.fonts.is_some()
            || self.colors.is_some()
            || self.position.is_some()
            || self.min_size.is_some()
            || self.max_size.is_some()
            || self.aspect.is_some()
    }

    /// Validate and canonicalize a complete brand-kit snapshot before it is
    /// persisted or used as an explicit verification override.
    pub fn normalized(mut self) -> Result<Self, String> {
        if let Some(fonts) = self.fonts.take() {
            if fonts.is_empty() {
                return Err("fonts must contain at least one name when supplied".into());
            }
            if fonts.len() > 16 {
                return Err("fonts supports at most 16 names".into());
            }
            let mut seen = BTreeSet::new();
            let mut normalized = Vec::new();
            for font in fonts {
                let font = font.trim();
                if font.is_empty() || font.chars().count() > 80 {
                    return Err("each font must be 1..=80 characters after trimming".into());
                }
                if seen.insert(font.to_lowercase()) {
                    normalized.push(font.to_string());
                }
            }
            self.fonts = Some(normalized);
        }

        if let Some(colors) = self.colors.take() {
            if colors.is_empty() {
                return Err("colors must contain at least one hex color when supplied".into());
            }
            if colors.len() > 32 {
                return Err("colors supports at most 32 entries".into());
            }
            let mut seen = BTreeSet::new();
            let mut normalized = Vec::new();
            for color in colors {
                let raw = color.trim();
                let Some(hex) = raw.strip_prefix('#') else {
                    return Err(format!("brand color '{raw}' must start with #"));
                };
                if !matches!(hex.len(), 3 | 4 | 6 | 8)
                    || !hex.chars().all(|ch| ch.is_ascii_hexdigit())
                {
                    return Err(format!(
                        "brand color '{raw}' must be #rgb, #rgba, #rrggbb, or #rrggbbaa"
                    ));
                }
                let lower = hex.to_ascii_lowercase();
                let expanded = if matches!(lower.len(), 3 | 4) {
                    lower.chars().flat_map(|ch| [ch, ch]).collect::<String>()
                } else {
                    lower
                };
                let canonical = format!("#{expanded}");
                if seen.insert(canonical.clone()) {
                    normalized.push(canonical);
                }
            }
            self.colors = Some(normalized);
        }

        if let Some(position) = self.position.take() {
            let position = position.trim().to_ascii_lowercase();
            if !matches!(position.as_str(), "bottom" | "top" | "center") {
                return Err("position must be bottom, top, or center".into());
            }
            self.position = Some(position);
        }

        for (label, value) in [("min_size", self.min_size), ("max_size", self.max_size)] {
            if value.is_some_and(|size| !(1..=512).contains(&size)) {
                return Err(format!("{label} must be in 1..=512 px"));
            }
        }
        if let (Some(min), Some(max)) = (self.min_size, self.max_size) {
            if min > max {
                return Err("min_size must be less than or equal to max_size".into());
            }
        }

        if let Some(aspect) = self.aspect.take() {
            let raw = aspect.trim();
            let Some((width, height)) = raw.split_once(':') else {
                return Err(format!("aspect '{raw}' must use W:H, for example 16:9"));
            };
            let width: u32 = width
                .trim()
                .parse()
                .map_err(|_| format!("aspect '{raw}' must use positive integers"))?;
            let height: u32 = height
                .trim()
                .parse()
                .map_err(|_| format!("aspect '{raw}' must use positive integers"))?;
            if width == 0 || height == 0 || width > 10_000 || height > 10_000 {
                return Err("aspect components must be in 1..=10000".into());
            }
            fn gcd(a: u32, b: u32) -> u32 {
                if b == 0 {
                    a
                } else {
                    gcd(b, a % b)
                }
            }
            let divisor = gcd(width, height);
            self.aspect = Some(format!("{}:{}", width / divisor, height / divisor));
        }

        if !self.has_constraints() {
            return Err("brand kit must pin at least one constraint".into());
        }
        Ok(self)
    }

    pub fn aspect_ratio(&self) -> Option<(u32, u32)> {
        let (width, height) = self.aspect.as_deref()?.split_once(':')?;
        Some((width.parse().ok()?, height.parse().ok()?))
    }
}

/// Durable clip anchor for a review comment.
///
/// `at_ms` stays in the comment as the replay-stable absolute timestamp the
/// reviewer saw when leaving the note. `anchor` lets the UI/agents resolve the
/// note back to the same clip after upstream ripple edits. If the clip is gone,
/// callers can show the note as stale instead of silently seeking to unrelated
/// footage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentAnchor {
    pub track_id: String,
    pub clip_id: String,
    /// Timeline offset inside the clip's slot when the note was created.
    pub offset_ms: u64,
}

/// Provenance attached to feedback imported from a portable review package.
/// It binds the note to the exact rendered bytes and op-log head the reviewer
/// saw, so later edits cannot silently turn old feedback into current feedback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentReviewSource {
    pub source_op_id: String,
    pub render_id: String,
    pub render_hash: String,
}

/// A validated external-review note ready to be imported atomically.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewFeedbackNote {
    pub at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    pub text: String,
    pub author: String,
}

/// A timecoded review comment.
/// Anchors review feedback at a timeline position (`at_ms`, optional `end_ms`
/// for a range like "0:40–0:50 is slow") with a free-text note. New comments
/// also carry an optional `anchor` pointing to the clip under the playhead so
/// they can follow that clip through upstream ripple edits. Lifecycle:
/// `open` → `addressed` (the agent's drafted change was applied) | `dismissed`.
///
/// Comments are REVIEW METADATA, not timeline edits — they ride in the op-log
/// for receipts + replay but are NOT part of the timeline undo stack (managed
/// via comment.resolve, like checkpoints/imports — `OpRecord::mutates_timeline`
/// excludes the comment verbs). `draft` holds the agent's pending proposed
/// change set (`comment.draft`) until applied/dismissed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    /// Allocated id "cmN".
    pub id: String,
    /// Timeline anchor (ms).
    pub at_ms: u64,
    /// End of a range comment (ms); None = a point comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    /// Clip+offset anchor captured at creation time. Missing on old projects or
    /// comments created over gaps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<CommentAnchor>,
    /// The review note.
    pub text: String,
    /// Who left it (free-form, e.g. "client", "editor").
    pub author: String,
    /// "open" | "addressed" | "dismissed".
    pub status: String,
    /// RFC3339 timestamp.
    pub ts: String,
    /// Render/package provenance for externally imported feedback. Local and
    /// legacy comments omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_source: Option<CommentReviewSource>,
    /// The agent's pending drafted change (`comment.draft`): the proposed
    /// `{verbs:[{verb,args}], rationale, ...}`. None until drafted; cleared/kept
    /// for the audit trail on apply/dismiss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<serde_json::Value>,
}

/// A non-destructive ADJUSTMENT LAYER (the `edit.adjustment` verb's storage) —
/// a colour grade / look effect applied across a TIME SPAN to the COMPOSITE of
/// everything beneath it, NOT baked per-clip (the standard adjustment-layer
/// "adjustment layer"). Stored at PROJECT level (a list of layers) rather than as
/// a clip on a track: v1 renders every adjustment as the TOP-MOST composite layer
/// over its span (so "the tracks beneath it" = all video tracks), which is the
/// common adjustment-layer use; per-track-position layering between overlays is a
/// documented v2 upgrade. The render is a single TIME-GATED grade/effect pass on
/// the intermediate composite — kept OFF the filtergraph entirely when the list is
/// empty, so a timeline with NO adjustment renders byte-identical (determinism).
///
/// Reuses the per-clip `ClipGrade` / `ClipEffect` shapes verbatim, so the same
/// grade knobs and look effects an individual clip accepts apply to the span. At
/// least one of `grade` / `effects` is non-empty (the verb refuses an empty layer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Adjustment {
    /// Stable id (`adj1`, `adj2`, …) for toggling / removing the layer. Allocated
    /// deterministically (max existing index + 1) so replay re-derives it.
    pub id: String,
    /// Timeline span [start_ms, end_ms) the grade/effect is active over (absolute
    /// composition time). The renderer gates the pass to this window.
    pub range_ms: [u64; 2],
    /// The colour grade applied over the span (None = none; same shape as edit.grade).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grade: Option<ClipGrade>,
    /// The look effects applied over the span, in order (same shape as edit.effect's
    /// VISUAL effects; audio effects / chroma-key are refused at the verb boundary —
    /// an adjustment grades a composite, it has no audio and no layer below to key).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ClipEffect>,
}

/// A checkpoint: a named pointer into the op-log (timeline/op-log contract `checkpoints`).
/// Reverting = appending new inverse ops, never rewriting history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub id: String,
    pub name: String,
    /// Sequence whose composition state this checkpoint points into. Missing on
    /// legacy records means the implicit main sequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence_id: Option<String>,
    /// Last op id included in this checkpoint (e.g. "op_000041").
    pub at_op: String,
    /// RFC3339 timestamp.
    pub ts: String,
}

/// A NEST / COMPOUND CLIP (`edit.nest`) — a group of timeline clips collapsed into
/// a single clip on the parent track (a nested compound clip or
/// "nest"). It holds its own SUB-TIMELINE: `tracks` is a self-contained mini-project
/// timeline (the moved clips, rebased so the first starts at sub-time 0) whose
/// sub-EDL is derived by the SAME `edl_from_project` as the main timeline — so "a
/// nest is just a timeline like the main one" is literally true (same derivation,
/// same render). Each clip keeps EVERY per-clip attribute (grade/effects/fade/speed/
/// timing) losslessly inside the nest.
///
/// The nest is referenced from the parent track by a single MediaClip whose `nest`
/// field is this id (see [`MediaClip::nest`]). At RENDER time the server bakes the
/// sub-timeline to a content-addressed file (mirroring the matte bake) and feeds it
/// in as the nest clip's source, so the main renderer is nest-blind and a project
/// with no nest renders byte-identical.
///
/// v1 scope: CREATE + RENDER. Editing INSIDE a nest (mutating `tracks` after
/// creation) is a documented follow-up — the data model already supports it (the
/// content-addressed bake re-renders only when the sub-timeline changes), the verbs
/// to address a clip by `(nest_id, clip_id)` are not yet exposed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Nest {
    /// Stable id (`nest1`, `nest2`, …), allocated deterministically (max existing
    /// nest index + 1) so replay re-derives it. Also the key the parent nest clip's
    /// `nest`/`asset` fields reference, and the cache namespace for the bake.
    pub id: String,
    /// Optional human label (`edit.nest {name}`); None = unnamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The nest's sub-timeline: a self-contained set of tracks (the clips moved off
    /// the parent track, rebased to start at 0). Derived to a sub-EDL exactly like the
    /// main timeline. v1 holds one video track (the contiguous run that was nested).
    pub tracks: Vec<Track>,
}

impl Nest {
    /// Combined timeline span of the nest's sub-timeline, ms — the duration of the
    /// baked render and therefore the parent nest clip's slot length. The longest
    /// sub-track (mirrors `Project::duration_ms`).
    pub fn span_ms(&self) -> u64 {
        self.tracks
            .iter()
            .map(|t| t.duration_ms())
            .max()
            .unwrap_or(0)
    }
}

/// One independently editable timeline inside a project. Source assets and
/// reusable galleries stay project-wide; composition state lives here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub id: String,
    pub name: String,
    pub settings: ProjectSettings,
    pub tracks: Vec<Track>,
    pub markers: Vec<Marker>,
    pub caption_styles: BTreeMap<String, CaptionStyle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustments: Vec<Adjustment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nests: Vec<Nest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transcript_ignores: Vec<TranscriptIgnore>,
}

impl Sequence {
    pub fn duration_ms(&self) -> u64 {
        self.tracks
            .iter()
            .map(Track::duration_ms)
            .max()
            .unwrap_or(0)
    }

    pub fn clip_count(&self) -> usize {
        self.tracks.iter().map(|track| track.clips.len()).sum()
    }
}

/// The materialized project state — `project.json`. This is a CACHE of the
/// op-log (`ops.jsonl` is the source of truth, timeline/op-log contract); it is rebuilt from
/// the log on demand and incrementally maintained on each applied op.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Schema tag, always PROJECT_SCHEMA for now.
    pub schema: String,
    /// Project display name (also the .cutproj dir stem).
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub settings: ProjectSettings,
    /// Asset id → Asset. BTreeMap for deterministic serialization order.
    #[serde(default)]
    pub assets: BTreeMap<String, Asset>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub markers: Vec<Marker>,
    #[serde(default)]
    pub caption_styles: BTreeMap<String, CaptionStyle>,
    /// Project-level brand constraints. Omitted for legacy/unbranded projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<BrandKit>,
    #[serde(default)]
    pub checkpoints: Vec<Checkpoint>,
    /// The composition currently materialized in the top-level timeline fields.
    /// Omitted for legacy single-sequence projects.
    #[serde(
        default = "default_active_sequence",
        skip_serializing_if = "is_default_active_sequence"
    )]
    pub active_sequence: String,
    /// Sequence snapshots are initialized lazily on the first sequence command,
    /// preserving the serialized shape of older single-sequence projects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequences: Vec<Sequence>,
    /// Timecoded review comments. Empty = none; omitted in JSON so
    /// older projects round-trip byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<Comment>,
    /// Non-destructive adjustment layers (edit.adjustment) — each a grade/effect
    /// over a time span applied to the composite of everything beneath it. Empty =
    /// none; omitted in JSON so pre-adjustment projects round-trip byte-identical
    /// (and renders without any adjustment stay byte-identical — the render path is
    /// off entirely when this is empty).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adjustments: Vec<Adjustment>,
    /// NEST / COMPOUND-CLIP sub-timelines (edit.nest) — each a group of clips
    /// collapsed into a single nest clip on a parent track (see [`Nest`]). Empty =
    /// none; omitted in JSON so pre-nest projects round-trip byte-identical (and a
    /// render with no nest stays byte-identical — the bake path is off entirely).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nests: Vec<Nest>,
    /// GRADE GALLERY — named [`GradePreset`] snapshots saved with `grade.save`, applied
    /// to other clips with `grade.apply`, listed with `grade.list` (a stills-style
    /// gallery / "copy a look between shots"). Empty = none; omitted in JSON so
    /// pre-gallery projects round-trip byte-identical (presets carry no render of their
    /// own — `grade.apply` lowers to `edit.grade` — so a project with presets renders
    /// byte-identical to one without).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grade_presets: Vec<GradePreset>,
    /// SMART BINS — named saved searches over the asset tray ([`SmartBin`];
    /// media.bin_save/bin_delete/bin_list). Membership is COMPUTED at list
    /// time, never stored. Empty = none; omitted in JSON so pre-bin projects
    /// round-trip byte-identical (bins carry no render of their own).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smart_bins: Vec<SmartBin>,
    /// CAPTION STYLE GALLERY — named [`CaptionStylePreset`] snapshots
    /// (captions.save_style/apply_style/list_styles), the caption analog of
    /// `grade_presets`. Empty = none; omitted in JSON so pre-gallery projects
    /// round-trip byte-identical (presets render nothing of their own —
    /// apply lowers to captions.set_style).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caption_style_presets: Vec<CaptionStylePreset>,
    /// Non-destructive ignored transcript ranges. Empty = none; omitted in JSON
    /// so older projects round-trip unchanged. Captions/assemble skip these
    /// words, while source transcript views can still render them quietly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transcript_ignores: Vec<TranscriptIgnore>,
}

impl Project {
    /// Fresh empty project with default v1/a1 track pair (video+audio),
    /// matching what `project.create` produces.
    pub fn new(name: &str, settings: ProjectSettings) -> Self {
        Self {
            schema: PROJECT_SCHEMA.to_string(),
            name: name.to_string(),
            settings,
            assets: BTreeMap::new(),
            tracks: vec![
                Track {
                    id: "v1".into(),
                    kind: TrackKind::Video,
                    clips: vec![],
                    gain_db: 0.0,
                    gain_windows: vec![],
                    blend_mode: None,
                    visible: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    pan: 0.0,
                },
                Track {
                    id: "a1t".into(),
                    kind: TrackKind::Audio,
                    clips: vec![],
                    gain_db: 0.0,
                    gain_windows: vec![],
                    blend_mode: None,
                    visible: true,
                    locked: false,
                    muted: false,
                    solo: false,
                    pan: 0.0,
                },
            ],
            markers: vec![],
            caption_styles: BTreeMap::new(),
            brand: None,
            checkpoints: vec![],
            active_sequence: default_active_sequence(),
            sequences: vec![],
            comments: vec![],
            adjustments: vec![],
            nests: vec![],
            grade_presets: vec![],
            smart_bins: vec![],
            caption_style_presets: vec![],
            transcript_ignores: vec![],
        }
    }

    fn current_sequence_snapshot(&self, id: String, name: String) -> Sequence {
        Sequence {
            id,
            name,
            settings: self.settings.clone(),
            tracks: self.tracks.clone(),
            markers: self.markers.clone(),
            caption_styles: self.caption_styles.clone(),
            comments: self.comments.clone(),
            adjustments: self.adjustments.clone(),
            nests: self.nests.clone(),
            transcript_ignores: self.transcript_ignores.clone(),
        }
    }

    fn blank_sequence(&self, id: String, name: String) -> Sequence {
        let blank = Project::new("", self.settings.clone());
        blank.current_sequence_snapshot(id, name)
    }

    fn load_sequence_snapshot(&mut self, sequence: &Sequence) {
        self.settings = sequence.settings.clone();
        self.tracks = sequence.tracks.clone();
        self.markers = sequence.markers.clone();
        self.caption_styles = sequence.caption_styles.clone();
        self.comments = sequence.comments.clone();
        self.adjustments = sequence.adjustments.clone();
        self.nests = sequence.nests.clone();
        self.transcript_ignores = sequence.transcript_ignores.clone();
        self.active_sequence = sequence.id.clone();
    }

    /// Materialize the implicit legacy timeline as `seq1` before a sequence
    /// command changes the project structure.
    pub fn ensure_sequence_bank(&mut self) {
        if self.sequences.is_empty() {
            let id = self.active_sequence.clone();
            let name = if id == DEFAULT_SEQUENCE_ID {
                "Main".to_string()
            } else {
                id.clone()
            };
            self.sequences
                .push(self.current_sequence_snapshot(id, name));
        }
    }

    /// Keep the active bank entry aligned with the materialized top-level state.
    pub fn sync_active_sequence(&mut self) {
        if self.sequences.is_empty() {
            return;
        }
        let Some(pos) = self
            .sequences
            .iter()
            .position(|sequence| sequence.id == self.active_sequence)
        else {
            return;
        };
        let name = self.sequences[pos].name.clone();
        self.sequences[pos] = self.current_sequence_snapshot(self.active_sequence.clone(), name);
    }

    pub fn create_sequence_snapshot(
        &mut self,
        id: String,
        name: String,
        duplicate_active: bool,
    ) -> Sequence {
        self.ensure_sequence_bank();
        self.sync_active_sequence();
        let sequence = if duplicate_active {
            self.current_sequence_snapshot(id, name)
        } else {
            self.blank_sequence(id, name)
        };
        self.sequences.push(sequence.clone());
        self.load_sequence_snapshot(&sequence);
        sequence
    }

    pub fn switch_sequence(&mut self, id: &str) -> bool {
        self.ensure_sequence_bank();
        self.sync_active_sequence();
        let Some(sequence) = self
            .sequences
            .iter()
            .find(|sequence| sequence.id == id)
            .cloned()
        else {
            return false;
        };
        self.load_sequence_snapshot(&sequence);
        true
    }

    /// Every track in every sequence. Before sequence management is activated,
    /// the materialized legacy timeline is the only sequence.
    pub fn all_sequence_tracks(&self) -> Box<dyn Iterator<Item = &Track> + '_> {
        if self.sequences.is_empty() {
            Box::new(self.tracks.iter())
        } else {
            Box::new(
                self.sequences
                    .iter()
                    .flat_map(|sequence| sequence.tracks.iter()),
            )
        }
    }

    /// Replay a recorded create payload without re-running id allocation.
    pub fn insert_and_activate_sequence(&mut self, sequence: Sequence) {
        self.ensure_sequence_bank();
        self.sync_active_sequence();
        if let Some(pos) = self
            .sequences
            .iter()
            .position(|existing| existing.id == sequence.id)
        {
            self.sequences[pos] = sequence.clone();
        } else {
            self.sequences.push(sequence.clone());
        }
        self.load_sequence_snapshot(&sequence);
    }

    /// AUDIO-MIX audibility (verbs `edit.mute` / `edit.solo`) — the single source
    /// of truth the render, preview, and per-track-stem audio mixes all honor.
    ///
    /// An AUDIO track is AUDIBLE iff it is **not explicitly muted** AND
    /// (**no audio track is soloed** OR **this track is soloed**). Equivalently:
    /// `!track.muted && (!any_solo || track.solo)`. So an explicit mute always
    /// silences; if ANY audio track is soloed only soloed audio tracks play;
    /// otherwise every non-muted audio track plays. Non-audio tracks always return
    /// false because they never enter the audio graph.
    ///
    /// Pure function of the boolean flags — `gain_db` is never consulted (mute/solo
    /// are flags, not levels), so the dialed gain is independent of mute state.
    pub fn audio_track_audible(&self, track: &Track) -> bool {
        if track.kind != TrackKind::Audio || track.muted {
            return false;
        }
        // Old builds accepted mute/solo on video tracks even though the render
        // graph has always sourced audio from TrackKind::Audio only. Ignore those
        // persisted no-op flags so a legacy video solo cannot silence every real
        // audio track after upgrade.
        let any_solo = self
            .tracks
            .iter()
            .any(|t| t.kind == TrackKind::Audio && t.solo);
        !any_solo || track.solo
    }

    /// True when the project has any nest (compound clip). Drives the render-time
    /// bake fast-path: a project with no nest flattens to itself → byte-identical.
    pub fn has_nests(&self) -> bool {
        !self.nests.is_empty()
    }

    /// Find a nest (compound clip) by id.
    pub fn nest(&self, id: &str) -> Option<&Nest> {
        self.nests.iter().find(|n| n.id == id)
    }

    /// Find a track by id.
    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// Find a track by id, mutable.
    pub fn track_mut(&mut self, id: &str) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// Locate a clip by id → (track_id, index in track.clips).
    pub fn find_clip(&self, clip_id: &str) -> Option<(&str, usize)> {
        for t in &self.tracks {
            for (i, c) in t.clips.iter().enumerate() {
                if c.id() == Some(clip_id) {
                    return Some((t.id.as_str(), i));
                }
            }
        }
        None
    }

    /// Capture the clip under an absolute timeline position for a new review
    /// comment. Video tracks win over audio/caption tracks because review notes
    /// normally refer to visible footage; gaps deliberately produce no anchor.
    pub fn comment_anchor_at(&self, at_ms: u64) -> Option<CommentAnchor> {
        for kind in [TrackKind::Video, TrackKind::Audio, TrackKind::Caption] {
            for track in self.tracks.iter().filter(|t| t.kind == kind) {
                if let Some(anchor) = comment_anchor_in_track(track, at_ms) {
                    return Some(anchor);
                }
            }
        }
        None
    }

    /// Resolve a stored review-comment clip anchor against the current timeline.
    /// Returns `None` when the comment has no anchor or the anchored clip no
    /// longer exists.
    pub fn resolve_comment_anchor_ms(&self, comment: &Comment) -> Option<u64> {
        let anchor = comment.anchor.as_ref()?;
        if let Some(track) = self.tracks.iter().find(|t| t.id == anchor.track_id) {
            if let Some(at_ms) = resolve_comment_anchor_in_track(track, anchor) {
                return Some(at_ms);
            }
        }
        self.tracks
            .iter()
            .filter(|t| t.id != anchor.track_id)
            .find_map(|track| resolve_comment_anchor_in_track(track, anchor))
    }

    /// Timeline duration = longest track, ms.
    pub fn duration_ms(&self) -> u64 {
        self.tracks
            .iter()
            .map(|t| t.duration_ms())
            .max()
            .unwrap_or(0)
    }

    /// Heal track grouping: stably partition `tracks` into `[Video…, Audio…,
    /// Caption…]` while preserving the relative order WITHIN each kind. Called at
    /// LOAD time (store::open) so projects saved before track grouping — whose
    /// lanes may be interleaved like `[v1, a1t, v2, a2t]` — come back grouped.
    ///
    /// INVARIANT preserved: VIDEO-track Vec order == compositing z-order (first
    /// video = base canvas; render.rs / edl.rs base_video_track rely on this).
    /// A STABLE sort keyed only on kind keeps the relative order of the video
    /// tracks (and of the audio tracks) exactly as it was — it merely groups
    /// them — so the base canvas and overlay stack are unchanged. Audio-track
    /// order is a render/EDL no-op regardless. Idempotent: re-running on an
    /// already-grouped project is a no-op (a stable sort of sorted-by-key data
    /// leaves it untouched).
    pub fn normalize_track_order(&mut self) {
        // Vec::sort_by_key is guaranteed STABLE → equal-rank (same-kind) tracks
        // keep their relative order; only cross-kind ordering is normalized.
        self.tracks.sort_by_key(|t| match t.kind {
            TrackKind::Video => 0u8,
            TrackKind::Audio => 1,
            TrackKind::Caption => 2,
        });
    }
}

fn comment_anchor_in_track(track: &Track, at_ms: u64) -> Option<CommentAnchor> {
    if track.kind == TrackKind::Caption {
        for clip in &track.clips {
            if let Clip::Caption(c) = clip {
                if at_ms >= c.range_ms[0] && at_ms < c.range_ms[1] {
                    return Some(CommentAnchor {
                        track_id: track.id.clone(),
                        clip_id: c.id.clone(),
                        offset_ms: at_ms.saturating_sub(c.range_ms[0]),
                    });
                }
            }
        }
        return None;
    }

    let mut cursor = 0u64;
    let mut prev_media_dur: Option<u64> = None;
    for clip in &track.clips {
        let dur = clip.timeline_duration_ms();
        let xfade = match (prev_media_dur, clip) {
            (Some(prev), Clip::Media(c)) if c.xfade_in_ms > 0 => c.xfade_in_ms.min(prev).min(dur),
            _ => 0,
        };
        let start = cursor.saturating_sub(xfade);
        let end = start.saturating_add(dur);
        if let Some(clip_id) = clip.id() {
            if at_ms >= start && at_ms < end {
                return Some(CommentAnchor {
                    track_id: track.id.clone(),
                    clip_id: clip_id.to_string(),
                    offset_ms: at_ms.saturating_sub(start),
                });
            }
        }
        cursor = end;
        prev_media_dur = match clip {
            Clip::Media(_) => Some(dur),
            _ => None,
        };
    }
    None
}

fn resolve_comment_anchor_in_track(track: &Track, anchor: &CommentAnchor) -> Option<u64> {
    if track.kind == TrackKind::Caption {
        return track.clips.iter().find_map(|clip| match clip {
            Clip::Caption(c) if c.id == anchor.clip_id => {
                Some(c.range_ms[0].saturating_add(anchor.offset_ms))
            }
            _ => None,
        });
    }

    let mut cursor = 0u64;
    let mut prev_media_dur: Option<u64> = None;
    for clip in &track.clips {
        let dur = clip.timeline_duration_ms();
        let xfade = match (prev_media_dur, clip) {
            (Some(prev), Clip::Media(c)) if c.xfade_in_ms > 0 => c.xfade_in_ms.min(prev).min(dur),
            _ => 0,
        };
        let start = cursor.saturating_sub(xfade);
        if clip.id() == Some(anchor.clip_id.as_str()) {
            return Some(start.saturating_add(anchor.offset_ms.min(dur)));
        }
        cursor = start.saturating_add(dur);
        prev_media_dur = match clip {
            Clip::Media(_) => Some(dur),
            _ => None,
        };
    }
    None
}

#[cfg(test)]
mod effect_catalog_tests {
    use super::*;

    /// the effects-as-data catalog covers EVERY ClipEffect, with no drift.
    /// Spot-construct one of each variant, collect its `kind()`, and assert the
    /// catalog has EXACTLY those keys (so adding a ClipEffect variant without a
    /// spec — or vice versa — fails here). Also pins the track/param shape.
    #[test]
    fn catalog_matches_every_clip_effect() {
        let all: Vec<ClipEffect> = vec![
            ClipEffect::Vignette { amount: 0.5 },
            ClipEffect::Sharpen { amount: 1.0 },
            ClipEffect::Blur { radius: 5.0 },
            ClipEffect::Grain { amount: 20.0 },
            ClipEffect::ChromaKey {
                color: "green".into(),
                similarity: 0.15,
                blend: 0.1,
            },
            ClipEffect::Denoise { amount: 0.5 },
            ClipEffect::Mirror,
            ClipEffect::Flip,
            ClipEffect::HueShift { degrees: 0.0 },
            ClipEffect::RgbSplit { amount: 6.0 },
            ClipEffect::Pixelize { size: 16.0 },
            ClipEffect::Sepia,
            ClipEffect::AutoColor { amount: 0.7 },
            ClipEffect::Vhs { amount: 0.5 },
            ClipEffect::Posterize { levels: 8.0 },
            ClipEffect::Invert,
            ClipEffect::Emboss,
            ClipEffect::Compressor { amount: 0.5 },
            ClipEffect::Gate { amount: 0.5 },
        ];
        let mut enum_keys: Vec<&str> = all.iter().map(|e| e.kind()).collect();
        enum_keys.sort_unstable();
        let specs = effect_specs();
        let mut spec_keys: Vec<&str> = specs.iter().map(|s| s.key).collect();
        spec_keys.sort_unstable();
        assert_eq!(
            spec_keys, enum_keys,
            "effect_specs() must cover every ClipEffect kind, no extras"
        );

        let hue = specs
            .iter()
            .find(|s| s.key == "hue_shift")
            .expect("hue spec exists");
        assert_eq!(
            hue.params
                .iter()
                .find(|p| p.name == "degrees")
                .and_then(|p| p.default),
            Some(0.0),
            "hue_shift catalog default must match ClipEffect serde default"
        );

        // Each spec's track/overlay flags agree with the enum's helpers.
        for e in &all {
            let spec = specs
                .iter()
                .find(|s| s.key == e.kind())
                .expect("spec exists");
            assert_eq!(
                spec.track == "audio",
                e.is_audio(),
                "track mismatch for {}",
                e.kind()
            );
            assert_eq!(
                spec.overlay_only,
                e.is_overlay_only(),
                "overlay flag mismatch for {}",
                e.kind()
            );
        }
        // chroma_key has a required `color` param; invert has none.
        let ck = specs.iter().find(|s| s.key == "chroma_key").unwrap();
        assert!(ck
            .params
            .iter()
            .any(|p| p.name == "color" && p.required && p.kind == "color"));
        assert!(specs
            .iter()
            .find(|s| s.key == "invert")
            .unwrap()
            .params
            .is_empty());
    }

    /// the transition catalog (transition_specs) must cover EXACTLY the
    /// canonical TRANSITIONS set that `is_valid_transition` (and the verb) accept —
    /// no drift between what crossfade allows and what transitions.list advertises.
    #[test]
    fn transition_specs_match_the_canonical_set() {
        let specs = transition_specs();
        let mut spec_names: Vec<&str> = specs.iter().map(|s| s.name).collect();
        spec_names.sort_unstable();
        let mut canon: Vec<&str> = TRANSITIONS.to_vec();
        canon.sort_unstable();
        assert_eq!(
            spec_names, canon,
            "transition_specs() must cover EXACTLY TRANSITIONS (no drift, no dupes)"
        );
        // No duplicate spec names; every name is_valid_transition; categories non-empty.
        let unique: std::collections::BTreeSet<&str> = specs.iter().map(|s| s.name).collect();
        assert_eq!(unique.len(), specs.len(), "duplicate transition spec name");
        for s in &specs {
            assert!(is_valid_transition(s.name), "{} not valid", s.name);
            assert!(!s.category.is_empty(), "{} has no category", s.name);
            assert!(!s.description.is_empty(), "{} has no description", s.name);
        }
    }
}

#[cfg(test)]
mod easing_tests {
    use super::*;

    /// The full Penner set whose endpoints must land EXACTLY on 0 and 1 (the value
    /// at a keyframe is the keyframe's value, regardless of curve). back/elastic/
    /// bounce overshoot in BETWEEN but still pin the endpoints.
    const ALL_EASED: &[KfInterp] = &[
        KfInterp::EaseInQuad,
        KfInterp::EaseOutQuad,
        KfInterp::EaseInOutQuad,
        KfInterp::EaseInCubic,
        KfInterp::EaseOutCubic,
        KfInterp::EaseInOutCubic,
        KfInterp::EaseInExpo,
        KfInterp::EaseOutExpo,
        KfInterp::EaseInOutExpo,
        KfInterp::EaseInBack,
        KfInterp::EaseOutBack,
        KfInterp::EaseInOutBack,
        KfInterp::EaseInElastic,
        KfInterp::EaseOutElastic,
        KfInterp::EaseInOutElastic,
        KfInterp::EaseInBounce,
        KfInterp::EaseOutBounce,
        KfInterp::EaseInOutBounce,
    ];

    /// Every curve pins its endpoints: f(0)=0, f(1)=1. This is the load-bearing
    /// property — the rendered value AT a keyframe equals that keyframe's value.
    #[test]
    fn endpoints_are_pinned() {
        for &e in ALL_EASED {
            assert!(e.is_eased(), "{e:?} should report eased");
            assert!(e.sample(0.0).abs() < 1e-9, "{e:?} f(0) != 0");
            assert!((e.sample(1.0) - 1.0).abs() < 1e-9, "{e:?} f(1) != 1");
        }
        // linear is identity; hold samples to 0 (the caller steps to the start value).
        assert_eq!(KfInterp::Linear.sample(0.37), 0.37);
        assert!(!KfInterp::Linear.is_eased());
        assert!(!KfInterp::Hold.is_eased());
    }

    /// The fraction is clamped to [0,1] before the curve — out-of-range inputs do
    /// not blow up (defensive; the renderer only ever feeds [0,1]).
    #[test]
    fn input_is_clamped() {
        for &e in ALL_EASED {
            assert_eq!(e.sample(-5.0), e.sample(0.0), "{e:?} under-clamp");
            assert_eq!(e.sample(5.0), e.sample(1.0), "{e:?} over-clamp");
            assert!(e.sample(0.5).is_finite(), "{e:?} midpoint not finite");
        }
    }

    /// The "non-overshooting" curves (quad/cubic/expo) stay monotone non-decreasing
    /// inside [0,1] — eased ≠ jittery. (back/elastic/bounce are deliberately excluded:
    /// they overshoot.)
    #[test]
    fn smooth_curves_are_monotonic() {
        let monotone = [
            KfInterp::EaseInQuad,
            KfInterp::EaseOutQuad,
            KfInterp::EaseInOutQuad,
            KfInterp::EaseInCubic,
            KfInterp::EaseOutCubic,
            KfInterp::EaseInOutCubic,
            KfInterp::EaseInExpo,
            KfInterp::EaseOutExpo,
            KfInterp::EaseInOutExpo,
        ];
        for e in monotone {
            let mut prev = e.sample(0.0);
            for i in 1..=100 {
                let f = i as f64 / 100.0;
                let v = e.sample(f);
                assert!(
                    v >= prev - 1e-9,
                    "{e:?} not monotone at f={f}: {v} < {prev}"
                );
                assert!((0.0..=1.0).contains(&v), "{e:?} left [0,1] at f={f}: {v}");
                prev = v;
            }
        }
    }

    /// Known-value spot checks against easings.org (the curves' identity — guards a
    /// silent formula typo). ease_in_quad(0.5)=0.25; ease_out_quad(0.5)=0.75;
    /// ease_in_out_cubic(0.5)=0.5 (symmetric); ease_in_cubic(0.5)=0.125.
    #[test]
    fn known_midpoints() {
        assert!((KfInterp::EaseInQuad.sample(0.5) - 0.25).abs() < 1e-9);
        assert!((KfInterp::EaseOutQuad.sample(0.5) - 0.75).abs() < 1e-9);
        assert!((KfInterp::EaseInCubic.sample(0.5) - 0.125).abs() < 1e-9);
        assert!((KfInterp::EaseInOutCubic.sample(0.5) - 0.5).abs() < 1e-9);
        assert!((KfInterp::EaseInOutQuad.sample(0.5) - 0.5).abs() < 1e-9);
        // ease_out_back overshoots above 1 before settling (anticipation feel).
        let peak = (0..=100)
            .map(|i| KfInterp::EaseOutBack.sample(i as f64 / 100.0))
            .fold(f64::MIN, f64::max);
        assert!(peak > 1.0, "ease_out_back should overshoot >1, got {peak}");
    }

    /// snake_case JSON round-trips (the wire form the agent / schema uses).
    #[test]
    fn serde_snake_case() {
        let j = serde_json::to_string(&KfInterp::EaseInOutCubic).unwrap();
        assert_eq!(j, "\"ease_in_out_cubic\"");
        let back: KfInterp = serde_json::from_str("\"ease_out_elastic\"").unwrap();
        assert_eq!(back, KfInterp::EaseOutElastic);
        // legacy values still parse (replay-safety for pre-easing projects).
        assert_eq!(
            serde_json::from_str::<KfInterp>("\"linear\"").unwrap(),
            KfInterp::Linear
        );
        assert_eq!(serde_json::to_string(&KfParam::Scale).unwrap(), "\"scale\"");
    }
}

#[cfg(test)]
mod speed_tests {
    use super::*;

    /// Identity at speed 1.0 — the load-bearing guarantee that pre-speed op
    /// logs / EDLs replay byte-identical (both helpers early-return the input).
    #[test]
    fn helpers_are_identity_at_unit_speed() {
        for off in [0u64, 1, 33, 1000, 62_176, u64::from(u32::MAX)] {
            assert_eq!(src_off_to_tl(off, 1.0), off);
            assert_eq!(tl_off_to_src(off, 1.0), off);
        }
    }

    /// 2× plays faster ⇒ a source span occupies HALF the timeline; 0.5× slow-mo
    /// ⇒ DOUBLE. tl_off_to_src is the inverse.
    #[test]
    fn helpers_remap_by_factor() {
        assert_eq!(src_off_to_tl(1000, 2.0), 500); // 2× → half the timeline
        assert_eq!(src_off_to_tl(1000, 0.5), 2000); // slow-mo → double
        assert_eq!(src_off_to_tl(1000, 4.0), 250);
        assert_eq!(tl_off_to_src(500, 2.0), 1000); // inverse of the 2× case
        assert_eq!(tl_off_to_src(2000, 0.5), 1000);
        // Round-trip src→tl→src is lossy by at most ~speed/2 ms: the timeline
        // offset is ms-quantized (≤0.5ms error), and converting back multiplies
        // that by `speed`. At 4× that's up to 2ms — still sub-frame (33ms/frame
        // @30fps), so imperceptible, but a real property of integer-ms timelines.
        for &speed in &[0.25f64, 0.5, 2.0, 3.0, 4.0] {
            let tol = (speed / 2.0).ceil() as u64 + 1;
            for &src in &[1000u64, 1234, 5000, 9999] {
                let back = tl_off_to_src(src_off_to_tl(src, speed), speed);
                assert!(
                    back.abs_diff(src) <= tol,
                    "round-trip {src}@{speed} → {back} (tol {tol})"
                );
            }
        }
    }

    #[test]
    fn invalid_speed_falls_back_to_identity_in_time_helpers() {
        for speed in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(src_off_to_tl(1000, speed), 1000);
            assert_eq!(tl_off_to_src(1000, speed), 1000);
        }
    }

    /// A media clip's timeline duration divides the source span by its speed;
    /// gaps/captions are unaffected (speed lives on media clips only).
    #[test]
    fn media_clip_timeline_duration_scales_with_speed() {
        let mut c = make_media_clip_speed("c1", "a1", 0, 4000, 1.0);
        assert_eq!(Clip::Media(c.clone()).timeline_duration_ms(), 4000);
        c.speed = 2.0;
        assert_eq!(Clip::Media(c.clone()).timeline_duration_ms(), 2000); // 2× → half
        c.speed = 0.5;
        assert_eq!(Clip::Media(c).timeline_duration_ms(), 8000); // slow-mo → double
    }

    #[test]
    fn caption_track_duration_ignores_stray_non_caption_spans() {
        let track = Track {
            id: "cap1".into(),
            kind: TrackKind::Caption,
            clips: vec![
                Clip::Caption(CaptionClip {
                    id: "cap_a".into(),
                    text: "a".into(),
                    style_ref: None,
                    range_ms: [1000, 5000],
                }),
                Clip::Gap(GapClip::new(60_000)),
            ],
            gain_db: 0.0,
            gain_windows: vec![],
            blend_mode: None,
            visible: true,
            locked: false,
            muted: false,
            solo: false,
            pan: 0.0,
        };
        assert_eq!(track.duration_ms(), 5000);
    }

    #[test]
    fn matte_seed_rejects_negative_source_time() {
        let err = serde_json::from_str::<MatteSeed>(r#"{"at_ms":-1,"point":[10,20]}"#)
            .expect_err("negative source time must not deserialize");
        assert!(
            err.to_string().contains("invalid value"),
            "serde error should explain the unsigned field: {err}"
        );
    }

    #[test]
    fn matte_seed_short_hash_is_stable_width_and_input_sensitive() {
        let a = MatteSeed {
            at_ms: 1000,
            point: Some([10, 20]),
            bbox: None,
        };
        let b = MatteSeed {
            at_ms: 1001,
            point: Some([10, 20]),
            bbox: None,
        };
        assert_eq!(a.short_hash().len(), 16);
        assert_eq!(a.short_hash(), a.short_hash());
        assert_ne!(a.short_hash(), b.short_hash());
    }

    #[test]
    fn invalid_media_clip_speed_does_not_overflow_duration() {
        let mut c = make_media_clip_speed("c1", "a1", 0, 4000, 0.0);
        assert_eq!(Clip::Media(c.clone()).timeline_duration_ms(), 4000);
        c.speed = f64::NAN;
        assert_eq!(Clip::Media(c).timeline_duration_ms(), 4000);
    }

    /// speed 1.0 is serde-skipped (byte-identical pre-speed JSON); a non-unit
    /// speed serializes and round-trips.
    #[test]
    fn speed_serde_skips_unit_and_roundtrips() {
        let unit = make_media_clip_speed("c1", "a1", 0, 1000, 1.0);
        let j = serde_json::to_string(&unit).unwrap();
        assert!(!j.contains("speed"), "unit speed must be omitted: {j}");
        let fast = make_media_clip_speed("c1", "a1", 0, 1000, 2.0);
        let j2 = serde_json::to_string(&fast).unwrap();
        assert!(
            j2.contains("\"speed\":2"),
            "non-unit speed must serialize: {j2}"
        );
        let back: MediaClip = serde_json::from_str(&j2).unwrap();
        assert_eq!(back.speed, 2.0);
        // A JSON without speed deserializes to 1.0 (the default).
        let legacy: MediaClip =
            serde_json::from_str(r#"{"id":"c1","asset":"a1","src_in_ms":0,"src_out_ms":1000}"#)
                .unwrap();
        assert_eq!(legacy.speed, 1.0);
    }

    /// Local test constructor (edit::make_media_clip is in another module).
    fn make_media_clip_speed(id: &str, asset: &str, si: u64, so: u64, speed: f64) -> MediaClip {
        MediaClip {
            id: id.into(),
            asset: asset.into(),
            src_in_ms: si,
            src_out_ms: so,
            effects: vec![],
            gain_db: 0.0,
            transform: None,
            crop: None,
            fade: None,
            xfade_in_ms: 0,
            xfade_kind: None,
            speed,
            grade: None,
            matte: None,
            mask: None,
            reverse: false,
            freeze: None,
            animation: None,
            keyframes: vec![],
            eq: None,
            mute_ranges: vec![],
            stabilize: None,
            speed_ramp: None,
            input_color_space: None,
            nest: None,
            grade_stack: vec![],
            grade_windows: vec![],
        }
    }

    /// Build a SpeedRamp from `(at_ms, factor)` pairs with a given segment count.
    fn ramp(pts: &[(u64, f64)], segments: usize) -> SpeedRamp {
        SpeedRamp {
            points: pts
                .iter()
                .map(|&(at_ms, factor)| SpeedRampPoint { at_ms, factor })
                .collect(),
            segments,
            preferred_segments: None,
            timebase_fps: None,
            timebase_audio_rate: None,
        }
    }

    /// A CONSTANT 2× ramp over the whole clip halves its duration — the speed-ramp
    /// integral reduces to the constant-speed result, and the sub-segments tile the
    /// source window exactly (no rounding gap).
    #[test]
    fn ramp_constant_factor_matches_constant_speed() {
        let r = ramp(&[(0, 2.0), (4000, 2.0)], 16);
        let segs = speed_ramp_segments(0, 4000, &r);
        // Sub-segments tile [0,4000) exactly: contiguous, covering the whole span.
        assert_eq!(segs.first().unwrap().src_in, 0);
        assert_eq!(segs.last().unwrap().src_out, 4000);
        for w in segs.windows(2) {
            assert_eq!(w[0].src_out, w[1].src_in, "sub-segments must be contiguous");
        }
        let total: u64 = segs.iter().map(|s| s.dur_ms).sum();
        assert_eq!(total, 2000, "constant 2× over the clip halves the duration");
        // timeline_duration_ms agrees with the sub-segment sum.
        let mut c = make_media_clip_speed("c1", "a1", 0, 4000, 1.0);
        c.speed_ramp = Some(r);
        assert_eq!(Clip::Media(c).timeline_duration_ms(), 2000);
    }

    #[test]
    fn ramp_zero_duration_slice_preserves_source_contiguity() {
        let r = ramp(&[(0, 1.0), (1, 1000.0), (2, 1.0), (4, 1.0)], 4);
        let segs = speed_ramp_segments(0, 4, &r);
        assert_eq!(segs.first().unwrap().src_in, 0);
        assert_eq!(segs.last().unwrap().src_out, 4);
        for w in segs.windows(2) {
            assert_eq!(w[0].src_out, w[1].src_in, "sub-segments must be contiguous");
        }
    }

    /// A symmetric speed-UP-then-down ramp: the realized duration equals the
    /// integral of (1/speed) over the source, approximated by the piecewise sum,
    /// and is SHORTER than the un-ramped clip (the fast middle removes more time
    /// than the slow ends add, for this curve). The duration also exactly equals
    /// the sum of the emitted sub-segments (the cursor invariant).
    #[test]
    fn ramp_up_then_down_duration_is_piecewise_integral() {
        // 1× → 4× at the middle → 1×, over a 4000 ms source window.
        let r = ramp(&[(0, 1.0), (2000, 4.0), (4000, 1.0)], 40);
        let segs = speed_ramp_segments(0, 4000, &r);
        let total: u64 = segs.iter().map(|s| s.dur_ms).sum();
        // Closed form: ∫₀^4000 1/speed(t) dt with speed linear 1→4 on [0,2000] and
        // 4→1 on [2000,4000]. On a linear ramp a→b over span L the time integral is
        // L*ln(b/a)/(b-a). Two symmetric halves (L=2000): 2 * 2000*ln(4)/3.
        let predicted = 2.0 * (2000.0 * (4.0f64).ln() / 3.0);
        let predicted = predicted.round() as u64; // ≈ 1848 ms
                                                  // Midpoint sampling with 40 segments lands within a few ms of the integral.
        assert!(
            total.abs_diff(predicted) <= 10,
            "ramp duration {total} ms vs integral {predicted} ms (±10)"
        );
        assert!(total < 4000, "the fast middle nets a shorter clip");
        // timeline_duration_ms == the sub-segment sum (cursor/render agreement).
        let mut c = make_media_clip_speed("c1", "a1", 0, 4000, 1.0);
        c.speed_ramp = Some(r);
        assert_eq!(Clip::Media(c).timeline_duration_ms(), total);
    }

    /// More segments → a closer approximation of the speed-ramp integral
    /// (monotone-ish convergence; midpoint rule). Sanity that segment count is the
    /// smoothness/accuracy knob the doc claims.
    #[test]
    fn ramp_more_segments_converges_to_integral() {
        let pts = [(0u64, 0.5), (3000, 3.0)]; // a single slow→fast linear ramp
        let predicted = 3000.0 * (3.0f64 / 0.5).ln() / (3.0 - 0.5); // L*ln(b/a)/(b-a)
        let err = |n: usize| -> f64 {
            let segs = speed_ramp_segments(0, 3000, &ramp(&pts, n));
            let total: u64 = segs.iter().map(|s| s.dur_ms).sum();
            (total as f64 - predicted).abs()
        };
        // 64 segments is at least as accurate as 4 (midpoint rule shrinks error).
        assert!(err(64) <= err(4) + 1.0, "more segments should not be worse");
        assert!(
            err(64) <= 5.0,
            "64-segment error within a few ms of the integral"
        );
    }

    /// Factor interpolation: linear between points, held outside the range.
    #[test]
    fn ramp_factor_interpolates_and_holds() {
        let r = ramp(&[(1000, 1.0), (3000, 3.0)], 8);
        assert_eq!(speed_ramp_factor_at(&r, 0), 1.0); // before first → hold
        assert_eq!(speed_ramp_factor_at(&r, 1000), 1.0);
        assert_eq!(speed_ramp_factor_at(&r, 2000), 2.0); // midpoint → linear
        assert_eq!(speed_ramp_factor_at(&r, 3000), 3.0);
        assert_eq!(speed_ramp_factor_at(&r, 9000), 3.0); // after last → hold
    }

    /// Multi-region redaction bakes one alpha PNG for the union of primary and
    /// extra regions. The cache key must therefore include every extra region;
    /// otherwise one face/plate set can reuse another region set's alpha.
    #[test]
    fn clip_mask_cache_tag_includes_extra_regions() {
        let base = ClipMask {
            shape: MaskShape::Rect,
            points: vec![[0.1, 0.1], [0.3, 0.3]],
            feather: 0.02,
            invert: false,
            effect: MaskEffect::Blur,
            strength: None,
            range_ms: None,
            track: None,
            regions: vec![MaskRegion {
                shape: MaskShape::Ellipse,
                points: vec![[0.5, 0.5], [0.1, 0.1]],
                track: None,
            }],
        };
        let mut changed = base.clone();
        changed.regions[0].points = vec![[0.7, 0.7], [0.1, 0.1]];

        assert_ne!(base.cache_tag(1920, 1080), changed.cache_tag(1920, 1080));
    }

    /// speed_ramp is serde-skipped when None (byte-identical pre-ramp JSON) and
    /// round-trips when present.
    #[test]
    fn speed_ramp_serde_skips_none_and_roundtrips() {
        let plain = make_media_clip_speed("c1", "a1", 0, 1000, 1.0);
        let j = serde_json::to_string(&plain).unwrap();
        assert!(!j.contains("speed_ramp"), "None ramp must be omitted: {j}");
        let mut ramped = make_media_clip_speed("c2", "a1", 0, 4000, 1.0);
        ramped.speed_ramp = Some(ramp(&[(0, 1.0), (4000, 2.0)], 12));
        let j2 = serde_json::to_string(&ramped).unwrap();
        assert!(j2.contains("speed_ramp"), "a ramp must serialize: {j2}");
        let back: MediaClip = serde_json::from_str(&j2).unwrap();
        assert_eq!(back.speed_ramp, ramped.speed_ramp);
        assert!(back.is_retimed() && back.has_speed_ramp());
    }

    #[test]
    fn frame_aware_ramp_serde_defaults_missing_preferred_segments() {
        let old_cache = serde_json::json!({
            "points": [{"at_ms": 0, "factor": 1.0}, {"at_ms": 4000, "factor": 2.0}],
            "segments": 25,
            "timebase_fps": 60.0,
            "timebase_audio_rate": 48_000
        });
        let restored: SpeedRamp = serde_json::from_value(old_cache).unwrap();
        assert_eq!(restored.preferred_segments, None);
        assert_eq!(restored.segments, 25);
        assert_eq!(restored.timebase_fps, Some(60.0));
    }
}

#[cfg(test)]
mod project_serde_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn project_settings_default_when_missing_from_legacy_json() {
        let project: Project = serde_json::from_value(json!({
            "schema": PROJECT_SCHEMA,
            "name": "legacy"
        }))
        .expect("legacy project without settings should deserialize");
        assert_eq!(project.settings, ProjectSettings::default());
        assert!(project.tracks.is_empty());
        assert!(project.assets.is_empty());
        assert!(project.brand.is_none());
    }

    #[test]
    fn brand_kit_normalizes_and_rejects_invalid_constraints() {
        let brand = BrandKit {
            fonts: Some(vec![" Inter ".into(), "inter".into(), "Arial".into()]),
            colors: Some(vec!["#FFF".into(), "#ffffff".into(), "#000A".into()]),
            position: Some(" Bottom ".into()),
            min_size: Some(28),
            max_size: Some(64),
            aspect: Some("1920:1080".into()),
        }
        .normalized()
        .unwrap();
        assert_eq!(brand.fonts.unwrap(), vec!["Inter", "Arial"]);
        assert_eq!(brand.colors.unwrap(), vec!["#ffffff", "#000000aa"]);
        assert_eq!(brand.position.as_deref(), Some("bottom"));
        assert_eq!(brand.aspect.as_deref(), Some("16:9"));

        let empty = BrandKit {
            fonts: None,
            colors: None,
            position: None,
            min_size: None,
            max_size: None,
            aspect: None,
        };
        assert!(empty.clone().normalized().is_err());
        let inverted = BrandKit {
            min_size: Some(80),
            max_size: Some(40),
            ..empty
        };
        assert!(inverted.normalized().is_err());
    }
}
