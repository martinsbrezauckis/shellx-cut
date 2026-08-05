//! color_match.rs — per-frame colour-stat sampling + two colour-correction
//! derivations that both lower onto a plain [`cut_core::ClipGrade`]:
//!   1. `edit.color_match` — match one clip's colour/tonality to a REFERENCE
//!      clip (the colorist "make this shot match that shot" tool).
//!   2. `edit.auto_balance` — REFERENCE-FREE one-click auto white-balance +
//!      exposure (the "Auto Color" / "Balance Color" every NLE has: Resolve,
//!      common one-click balance tools). Instead of matching another clip it neutralises the
//!      image's OWN colour cast and sets exposure toward a mid target.
//! Both DERIVE the grade from a single decoded frame's per-channel statistics
//! and apply it through the normal `edit.grade` path, so the result composes /
//! undoes / replays like any per-clip grade (this module only PRODUCES the
//! grade — it never touches the EDL).
//!
//! Role: SAMPLE a representative frame from a target asset and a reference asset,
//! compute per-channel statistics (MEAN + STD), and DERIVE an [`cut_core::ClipGrade`]
//! that moves the target's colour toward the reference's. The derived grade is
//! applied through the normal `edit.grade` storage, so it composes / undoes /
//! replays like any other per-clip grade (this module only PRODUCES the grade —
//! it never touches the EDL).
//!
//! Colour space / method (v1): plain RGB Reinhard-style transfer
//! (Reinhard et al. 2001, "Color Transfer between Images") — shift the target's
//! mean toward the reference's, scale spread by the std/level ratio. RGB (not
//! LAB) is the deliberate v1 baseline: it is dependency-free (we decode the
//! frame to raw `rgb24` via ffmpeg and accumulate in Rust — no image crate),
//! and the `edit.grade` knobs it maps onto (ffmpeg `eq` brightness/contrast/
//! saturation + `colortemperature`) themselves operate in RGB/gamma space, so
//! deriving the correction in the SAME space keeps the mapping internally
//! consistent. A perceptual LAB space (separating luma from the green-magenta /
//! blue-yellow chroma axes) is the documented quality fast-follow; note that
//! `edit.grade` has no green-magenta (tint) knob, so the a*-axis can't be
//! expressed today regardless of the sampling space.
//!
//! Determinism: ffmpeg single-frame decode at a fixed source time → identical
//! scaled bytes → identical stats → identical derived grade. The derivation is a
//! PURE function of the two stat sets + strength ([`derive_grade`]), unit-tested
//! on synthetic stats with no ffmpeg in the loop.
//!
//! Dependencies: ffmpeg subprocess (crate::ffmpeg), cut-core (ClipGrade).
//! Caller: dispatch `edit.color_match` (which resolves the two clips → asset
//! paths + a representative source time, samples both, derives the grade, then
//! commits it via the normal `edit.grade` path).

use crate::ffmpeg::ffmpeg_bin;
use cut_core::{error_codes, ClipGrade, CutError};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

/// Height (px) the sampled frame is scaled to before stats are accumulated.
/// Small enough to be fast (~tens of thousands of pixels), large enough to be a
/// faithful colour sample. BOTH clips are scaled to the same height, so the
/// std/level RATIOS the transfer uses are comparable. Width is `-2` (kept even,
/// aspect-preserving) — we never need the geometry, only the colour triplets.
const SAMPLE_HEIGHT: u32 = 144;

/// Per-channel colour statistics of one sampled frame, on the 8-bit `0..255`
/// scale. `mean`/`std` are population statistics over every pixel. Luma is
/// Rec.601 (`0.299 R + 0.587 G + 0.114 B`); chroma is the per-pixel distance of
/// the RGB triplet from its own luma (a saturation/colourfulness magnitude).
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct ChannelStats {
    pub mean_r: f64,
    pub mean_g: f64,
    pub mean_b: f64,
    pub std_r: f64,
    pub std_g: f64,
    pub std_b: f64,
    /// Rec.601 luma mean / std (brightness level + tonal spread → contrast).
    pub mean_luma: f64,
    pub std_luma: f64,
    /// Chroma magnitude mean / std (colourfulness level → saturation).
    pub mean_chroma: f64,
    pub std_chroma: f64,
}

impl ChannelStats {
    /// Warm/cool axis on the 0..255 scale: positive = warmer (more red than
    /// blue), negative = cooler. Drives the `temperature_k` derivation.
    pub fn warmth(&self) -> f64 {
        self.mean_r - self.mean_b
    }

    /// Accumulate stats over a tightly-packed `rgb24` buffer (`len % 3 == 0`).
    /// Returns an error for an empty/odd buffer (a decode that produced nothing).
    pub fn from_rgb24(buf: &[u8]) -> Result<ChannelStats, CutError> {
        if buf.is_empty() || !buf.len().is_multiple_of(3) {
            return Err(CutError::new(
                error_codes::FFMPEG,
                "sampled frame had no decodable rgb24 pixels",
                format!(
                    "rgb24 buffer length {} is empty or not a multiple of 3",
                    buf.len()
                ),
            ));
        }
        let n = (buf.len() / 3) as f64;
        let (mut sr, mut sg, mut sb) = (0.0_f64, 0.0_f64, 0.0_f64);
        let (mut sr2, mut sg2, mut sb2) = (0.0_f64, 0.0_f64, 0.0_f64);
        let (mut sy, mut sy2) = (0.0_f64, 0.0_f64);
        let (mut sc, mut sc2) = (0.0_f64, 0.0_f64);
        for px in buf.chunks_exact(3) {
            let (r, g, b) = (px[0] as f64, px[1] as f64, px[2] as f64);
            sr += r;
            sg += g;
            sb += b;
            sr2 += r * r;
            sg2 += g * g;
            sb2 += b * b;
            let y = 0.299 * r + 0.587 * g + 0.114 * b;
            sy += y;
            sy2 += y * y;
            // Per-pixel chroma magnitude: distance of the triplet from its luma.
            let (dr, dg, db) = (r - y, g - y, b - y);
            let c = (dr * dr + dg * dg + db * db).sqrt();
            sc += c;
            sc2 += c * c;
        }
        // Population std = sqrt(E[x^2] - E[x]^2), clamped at 0 against fp noise.
        let std = |sum: f64, sumsq: f64| -> f64 {
            let mean = sum / n;
            (sumsq / n - mean * mean).max(0.0).sqrt()
        };
        Ok(ChannelStats {
            mean_r: sr / n,
            mean_g: sg / n,
            mean_b: sb / n,
            std_r: std(sr, sr2),
            std_g: std(sg, sg2),
            std_b: std(sb, sb2),
            mean_luma: sy / n,
            std_luma: std(sy, sy2),
            mean_chroma: sc / n,
            std_chroma: std(sc, sc2),
        })
    }
}

/// The grade derivation receipt: the ClipGrade that was derived plus the two
/// stat sets it came from, for an HONEST `edit.color_match` receipt.
#[derive(Debug, Clone, Serialize)]
pub struct ColorMatch {
    /// Colour space / method used (`"rgb"` Reinhard-style for v1).
    pub space: &'static str,
    /// Match strength actually applied (0 = identity, 1 = full match).
    pub strength: f64,
    /// The derived parametric grade (what gets stored on the clip).
    pub derived: ClipGrade,
    /// True when the derived grade is identity (no correction — e.g. strength 0
    /// or the clip matched to itself); applying it CLEARS any existing grade.
    pub identity: bool,
    pub target: ChannelStats,
    pub reference: ChannelStats,
}

/// Decode ONE representative frame from `asset_path` at `at_s` seconds to a
/// tightly-packed `rgb24` buffer via ffmpeg (fast seek before `-i`, scaled to
/// [`SAMPLE_HEIGHT`]) — no image-decode dependency. Shared by the colour-stat
/// sampler ([`sample_channel_stats`]) and the auto-balance sampler
/// ([`sample_for_auto_balance`]) so a frame is decoded exactly once per call.
/// Errors actionably if ffmpeg fails to run or the source yields no pixels.
fn decode_sample_rgb24(asset_path: &Path, at_s: f64) -> Result<Vec<u8>, CutError> {
    let at = format!("{:.3}", at_s.max(0.0));
    let out = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-ss", &at, "-i"])
        .arg(asset_path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale=-2:{SAMPLE_HEIGHT}"),
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .map_err(|e| {
            CutError::new(
                error_codes::FFMPEG,
                "ffmpeg failed to run for colour frame sample",
                e.to_string(),
            )
        })?;
    if !out.status.success() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            format!("could not sample a frame from '{}'", asset_path.display()),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        )
        .with_suggested_action("pass a clip whose source is a decodable video with pixels"));
    }
    Ok(out.stdout)
}

/// Sample a representative frame from `asset_path` at `at_s` seconds and return
/// its per-channel stats (decodes once via [`decode_sample_rgb24`], accumulates
/// in Rust). Errors actionably if ffmpeg fails or yields no pixels.
pub fn sample_channel_stats(asset_path: &Path, at_s: f64) -> Result<ChannelStats, CutError> {
    ChannelStats::from_rgb24(&decode_sample_rgb24(asset_path, at_s)?)
}

/// White-balance Kelvin mapping shared by `derive_grade` (reference match) and
/// `derive_auto_balance` (reference-free), so both produce the SAME warm/cool
/// feel. ffmpeg `colortemperature`: Kelvin BELOW neutral = warmer output, ABOVE
/// = cooler. `KELVIN_PER_WARMTH` converts a 0..255 warm/cool (R−B) delta into a
/// Kelvin offset; a `DEADBAND_K` band around neutral maps to `None` (a true
/// white-balance no-op, so an already-neutral frame stays untouched).
const NEUTRAL_K: f64 = 6500.0;
const KELVIN_PER_WARMTH: f64 = 70.0;
const DEADBAND_K: f64 = 80.0;

/// DERIVE a [`ClipGrade`] that moves `target`'s colour toward `reference`'s,
/// scaled by `strength` (0 = identity, 1 = full match). PURE + deterministic —
/// this is the unit-tested core of `edit.color_match`.
///
/// Mapping (RGB Reinhard-style → `edit.grade` knobs):
/// - **brightness** ← luma-MEAN delta `(R.luma − T.luma)/255`, so a brighter
///   reference lifts the target. ffmpeg `eq` brightness adds to the normalised
///   pixel value; clamped to `[-1, 1]`.
/// - **contrast** ← luma-STD ratio `R.std / T.std` (the Reinhard spread scale),
///   interpolated from 1 by `strength`; clamped to `[0, 3]`.
/// - **saturation** ← chroma-MEAN ratio `R.chroma / T.chroma` (the colourfulness
///   level), interpolated from 1 by `strength`; clamped to `[0, 3]`.
/// - **temperature_k** ← warm/cool (R−B) mean delta: a warmer reference lowers
///   the Kelvin below 6500 (ffmpeg `colortemperature`: lower = warmer). A small
///   neutral deadband around 6500 maps to `None` (no white-balance filter), so an
///   identity match stays a true no-op.
///
/// std/level denominators are floored at 1.0 (on the 0..255 scale) so a flat /
/// near-grayscale target can't blow the ratio up.
pub fn derive_grade(target: &ChannelStats, reference: &ChannelStats, strength: f64) -> ClipGrade {
    let s = strength.clamp(0.0, 1.0);

    // Brightness: luma-mean delta, normalised to eq's [-1,1] range.
    let brightness = (s * (reference.mean_luma - target.mean_luma) / 255.0).clamp(-1.0, 1.0);

    // Contrast: luma-std ratio, eased from identity by strength.
    let contrast_ratio = reference.std_luma / target.std_luma.max(1.0);
    let contrast = (1.0 + s * (contrast_ratio - 1.0)).clamp(0.0, 3.0);

    // Saturation: chroma-magnitude-mean ratio, eased from identity by strength.
    let sat_ratio = reference.mean_chroma / target.mean_chroma.max(1.0);
    let saturation = (1.0 + s * (sat_ratio - 1.0)).clamp(0.0, 3.0);

    // White balance: warm/cool (R−B) delta → Kelvin offset from neutral 6500
    // (constants lifted to module scope, shared with derive_auto_balance).
    // Warmer reference (positive delta) ⇒ LOWER Kelvin (ffmpeg colortemperature).
    let warmth_delta = s * (reference.warmth() - target.warmth());
    let temp = (NEUTRAL_K - warmth_delta * KELVIN_PER_WARMTH).clamp(3000.0, 12000.0);
    let temperature_k = if (temp - NEUTRAL_K).abs() < DEADBAND_K {
        None
    } else {
        Some(temp.round() as u32)
    };

    ClipGrade {
        contrast,
        brightness,
        saturation,
        gamma: 1.0, // not derived in v1 (luma is handled by brightness + contrast)
        temperature_k,
        lut: None,
    }
}

/// Convenience: sample both assets, derive the grade, and bundle the receipt.
/// `target_at_s` / `reference_at_s` are representative source times (seconds).
pub fn match_color(
    target_path: &Path,
    target_at_s: f64,
    reference_path: &Path,
    reference_at_s: f64,
    strength: f64,
) -> Result<ColorMatch, CutError> {
    let target = sample_channel_stats(target_path, target_at_s)?;
    let reference = sample_channel_stats(reference_path, reference_at_s)?;
    let derived = derive_grade(&target, &reference, strength);
    Ok(ColorMatch {
        space: "rgb",
        strength: strength.clamp(0.0, 1.0),
        identity: derived.is_identity(),
        derived,
        target,
        reference,
    })
}

// ───────────────────────── reference-free auto-balance ──────────────────────
// edit.auto_balance: the one-click "Auto Color" / "Balance Color" every NLE has
// The SAME pipeline as color_match — sample one frame,
// derive a ClipGrade, commit a replay-safe grade — but REFERENCE-FREE: instead
// of matching another clip, it neutralises the image's OWN colour cast (white
// balance) and nudges exposure to a mid target. The derivation is a PURE
// function of one frame's stats (+ strength + mode), unit-tested with no ffmpeg.

/// How `edit.auto_balance` measures the colour cast to neutralise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoBalanceMode {
    /// GRAY-WORLD (default): assume the WHOLE-frame average should be neutral
    /// grey, so the warm/cool cast is the global `mean_r − mean_b`. Simple and
    /// robust for typical scenes; can be fooled by one large strongly-coloured
    /// object that drags the average off neutral.
    GrayWorld,
    /// WHITE-PATCH: assume the bright NEAR-NEUTRAL region (the highlights that
    /// "should be" white) is the neutral reference, so the cast is measured from
    /// those pixels only — more robust when a big colour object would skew the
    /// gray-world average. Falls back to gray-world when no such highlights exist.
    WhitePatch,
}

impl AutoBalanceMode {
    /// Parse the `mode` arg string; `None`/empty ⇒ the gray-world default.
    /// An unknown value is an actionable error (the schema enum is the same two
    /// snake_case values).
    pub fn parse(s: Option<&str>) -> Result<AutoBalanceMode, CutError> {
        match s.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("gray_world") => Ok(AutoBalanceMode::GrayWorld),
            Some("white_patch") => Ok(AutoBalanceMode::WhitePatch),
            Some(other) => Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("unknown auto_balance mode '{other}'"),
                "mode must be \"gray_world\" (default) or \"white_patch\"",
            )
            .with_suggested_action("pass mode:\"gray_world\" or mode:\"white_patch\"")),
        }
    }

    /// Stable snake_case name for receipts / rationale text.
    pub fn as_str(self) -> &'static str {
        match self {
            AutoBalanceMode::GrayWorld => "gray_world",
            AutoBalanceMode::WhitePatch => "white_patch",
        }
    }
}

/// Auto-EXPOSURE target: the luma MEAN (0..255) brightness nudges toward —
/// ≈0.46 of full scale, the photographic 18%-grey mid-tone in gamma space. A
/// frame already near this gets a near-zero brightness correction.
const TARGET_LUMA_MEAN: f64 = 117.0;

/// Auto-CONTRAST target: the luma STD (0..255) the GENTLE contrast nudge aims
/// for, with a deadband so a normally-exposed frame is left untouched. The pull
/// is HALVED (`CONTRAST_GAIN`) and clamped tight so auto-balance never wrecks
/// contrast; a frame whose spread is within `±CONTRAST_DEADBAND_RATIO` of the
/// target keeps contrast at exactly 1.0.
const TARGET_LUMA_STD: f64 = 52.0;
const CONTRAST_GAIN: f64 = 0.5;
const CONTRAST_DEADBAND_RATIO: f64 = 0.15;

/// DERIVE a [`ClipGrade`] that neutralises one frame's OWN colour cast (white
/// balance) and nudges its exposure toward a mid target — the reference-free
/// auto-balance. PURE + deterministic; the unit-tested core of
/// `edit.auto_balance`.
///
/// Mapping (one frame's [`ChannelStats`] → `edit.grade` knobs):
/// - **brightness** ← `(TARGET_LUMA_MEAN − mean_luma)/255` (eased by `strength`,
///   clamped to eq's `[-1,1]`): a dark frame lifts, a bright frame pulls down.
/// - **contrast** ← a GENTLE, DEADBANDED pull of the luma spread toward
///   `TARGET_LUMA_STD` (half-strength, clamped `[0.5, 1.8]`); a normally-exposed
///   frame stays at 1.0. A flat field (std→0) is floored, and contrast (which
///   pivots around the mean) leaves a flat field visually unchanged anyway.
/// - **temperature_k** ← the warm/cool cast pushed toward neutral. GRAY-WORLD
///   uses the whole-frame warmth (`mean_r − mean_b`); WHITE-PATCH uses
///   `highlight_warmth` (the bright near-neutral region). SAME Kelvin mapping as
///   [`derive_grade`], here against an implicit NEUTRAL reference (warmth 0): a
///   WARM cast ⇒ Kelvin ABOVE neutral = cooler output (the correction); a cool
///   cast ⇒ below neutral = warmer. A cast inside `DEADBAND_K` ⇒ `None`.
/// - **saturation / gamma** ← left at identity: there is no principled
///   reference-free saturation target in v1 (white balance already removes the
///   cast that most skews perceived saturation), so auto-balance does not touch
///   it — same conservative stance color_match documents for the tint axis.
///
/// `strength` 0 ⇒ a clean identity grade (clears any prior auto-balance). The
/// derivation is a per-channel-GAIN idea expressed through the `temperature_k`
/// (warm/cool) axis only: `edit.grade` has NO green-magenta tint knob, so the
/// green-magenta axis of the cast is left uncorrected (the documented v1 limit,
/// identical to color_match). `highlight_warmth` is `Some(R̄_hi − B̄_hi)` for
/// white-patch, `None` for gray-world (or white-patch with no qualifying
/// highlights → gray-world cast).
pub fn derive_auto_balance(
    stats: &ChannelStats,
    highlight_warmth: Option<f64>,
    strength: f64,
) -> ClipGrade {
    let s = strength.clamp(0.0, 1.0);

    // EXPOSURE: nudge the luma mean toward the mid target (eq brightness adds to
    // the normalised pixel value, so a positive delta lifts a dark frame).
    let brightness = (s * (TARGET_LUMA_MEAN - stats.mean_luma) / 255.0).clamp(-1.0, 1.0);

    // CONTRAST: gentle, deadbanded pull of the luma spread toward the target.
    // The std denominator is floored so a near-flat frame can't blow the ratio up.
    let std_ratio = TARGET_LUMA_STD / stats.std_luma.max(1.0);
    let contrast = if (std_ratio - 1.0).abs() < CONTRAST_DEADBAND_RATIO {
        1.0
    } else {
        (1.0 + s * CONTRAST_GAIN * (std_ratio - 1.0)).clamp(0.5, 1.8)
    };

    // WHITE BALANCE: push the warm/cool cast toward neutral (an implicit
    // reference of warmth 0). Equivalent to derive_grade with a neutral
    // reference: temp = NEUTRAL_K + s·cast·KELVIN_PER_WARMTH (a warm cast,
    // cast > 0, raises Kelvin = cooler output, undoing the warmth).
    let cast = highlight_warmth.unwrap_or_else(|| stats.warmth());
    let temp = (NEUTRAL_K + s * cast * KELVIN_PER_WARMTH).clamp(3000.0, 12000.0);
    let temperature_k = if (temp - NEUTRAL_K).abs() < DEADBAND_K {
        None
    } else {
        Some(temp.round() as u32)
    };

    ClipGrade {
        contrast,
        brightness,
        saturation: 1.0, // reference-free: no principled saturation target in v1
        gamma: 1.0,
        temperature_k,
        lut: None,
    }
}

/// Warm/cool cast (`R̄−B̄`, 0..255) of the bright NEAR-NEUTRAL pixels — the
/// highlights a WHITE-PATCH balance treats as the neutral reference. Two passes
/// over the `rgb24` buffer: find the frame's max luma, then average R,G,B over
/// pixels that are BRIGHT (luma ≥ 0.85·max) AND near-neutral (chroma magnitude
/// < `CHROMA_MAX`), so a strongly-coloured bright object is excluded. Returns
/// `None` when too few pixels qualify (the caller falls back to gray-world).
fn highlight_neutral_warmth(buf: &[u8]) -> Option<f64> {
    if buf.is_empty() || !buf.len().is_multiple_of(3) {
        return None;
    }
    let luma = |px: &[u8]| 0.299 * px[0] as f64 + 0.587 * px[1] as f64 + 0.114 * px[2] as f64;
    let max_luma = buf.chunks_exact(3).map(luma).fold(0.0_f64, f64::max);
    let luma_hi = max_luma * 0.85;
    const CHROMA_MAX: f64 = 25.0;
    const MIN_PIXELS: usize = 16;
    let (mut sr, mut sb, mut n) = (0.0_f64, 0.0_f64, 0usize);
    for px in buf.chunks_exact(3) {
        let (r, g, b) = (px[0] as f64, px[1] as f64, px[2] as f64);
        let y = 0.299 * r + 0.587 * g + 0.114 * b;
        if y < luma_hi {
            continue;
        }
        // Chroma magnitude: distance of the triplet from its own luma.
        let (dr, dg, db) = (r - y, g - y, b - y);
        if (dr * dr + dg * dg + db * db).sqrt() >= CHROMA_MAX {
            continue;
        }
        sr += r;
        sb += b;
        n += 1;
    }
    if n < MIN_PIXELS {
        return None;
    }
    Some((sr - sb) / n as f64)
}

/// Sample a representative frame for auto-balance: decode ONCE to `rgb24`, return
/// its [`ChannelStats`] plus — for WHITE-PATCH — the bright near-neutral
/// highlight warmth (`None` for gray-world, or white-patch with no qualifying
/// highlights → caller falls back to the whole-frame average).
pub fn sample_for_auto_balance(
    asset_path: &Path,
    at_s: f64,
    mode: AutoBalanceMode,
) -> Result<(ChannelStats, Option<f64>), CutError> {
    let buf = decode_sample_rgb24(asset_path, at_s)?;
    let stats = ChannelStats::from_rgb24(&buf)?;
    let highlight = match mode {
        AutoBalanceMode::GrayWorld => None,
        AutoBalanceMode::WhitePatch => highlight_neutral_warmth(&buf),
    };
    Ok((stats, highlight))
}

/// The auto-balance receipt: the derived grade + the sampled frame stats + the
/// cast source, for an HONEST `edit.auto_balance` receipt.
#[derive(Debug, Clone, Serialize)]
pub struct AutoBalance {
    /// The mode the cast was measured with (gray-world / white-patch).
    pub mode: AutoBalanceMode,
    /// Strength actually applied (0 = identity, 1 = full correction).
    pub strength: f64,
    /// The derived parametric grade (what gets committed on the clip).
    pub derived: ClipGrade,
    /// True when the derived grade is identity (already-neutral well-exposed
    /// frame, or strength 0) — committing it CLEARS any existing grade.
    pub identity: bool,
    /// The sampled frame's per-channel statistics.
    pub stats: ChannelStats,
    /// White-patch only: the bright near-neutral region's warmth actually used
    /// (`None` ⇒ gray-world, OR white-patch found no qualifying highlights and
    /// fell back to the whole-frame average).
    pub highlight_warmth: Option<f64>,
}

/// Convenience: sample the clip's frame, derive the auto-balance grade, bundle
/// the receipt. `at_s` is a representative source time (seconds).
pub fn auto_balance(
    asset_path: &Path,
    at_s: f64,
    mode: AutoBalanceMode,
    strength: f64,
) -> Result<AutoBalance, CutError> {
    let (stats, highlight_warmth) = sample_for_auto_balance(asset_path, at_s, mode)?;
    let derived = derive_auto_balance(&stats, highlight_warmth, strength);
    Ok(AutoBalance {
        mode,
        strength: strength.clamp(0.0, 1.0),
        identity: derived.is_identity(),
        derived,
        stats,
        highlight_warmth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A neutral mid-tone baseline; tests tweak fields off this.
    fn base() -> ChannelStats {
        ChannelStats {
            mean_r: 120.0,
            mean_g: 120.0,
            mean_b: 120.0,
            std_r: 40.0,
            std_g: 40.0,
            std_b: 40.0,
            mean_luma: 120.0,
            std_luma: 40.0,
            mean_chroma: 20.0,
            std_chroma: 10.0,
        }
    }

    /// strength 0 ⇒ identity grade no matter how different the inputs are.
    #[test]
    fn zero_strength_is_identity() {
        let t = base();
        let mut r = base();
        r.mean_luma = 200.0;
        r.mean_r = 220.0;
        r.mean_b = 60.0;
        r.std_luma = 80.0;
        r.mean_chroma = 60.0;
        let g = derive_grade(&t, &r, 0.0);
        assert!(
            g.is_identity(),
            "strength 0 must derive identity, got {g:?}"
        );
    }

    /// A brighter + warmer reference ⇒ brightness lifted AND temperature toward
    /// warm (Kelvin < 6500).
    #[test]
    fn brighter_warmer_reference() {
        let t = base();
        let mut r = base();
        r.mean_luma = 170.0; // brighter
        r.mean_r = 160.0; // warmer: more red…
        r.mean_b = 80.0; // …less blue
        let g = derive_grade(&t, &r, 1.0);
        assert!(
            g.brightness > 0.0,
            "brighter ref ⇒ brightness>0, got {}",
            g.brightness
        );
        let k = g.temperature_k.expect("a warm shift sets temperature_k");
        assert!(k < 6500, "warmer ref ⇒ Kelvin<6500 (warm), got {k}");
    }

    /// A cooler reference (more blue than red) ⇒ Kelvin > 6500 (cool).
    #[test]
    fn cooler_reference_raises_kelvin() {
        let t = base();
        let mut r = base();
        r.mean_r = 80.0;
        r.mean_b = 160.0; // strongly blue
        let g = derive_grade(&t, &r, 1.0);
        let k = g.temperature_k.expect("a cool shift sets temperature_k");
        assert!(k > 6500, "cooler ref ⇒ Kelvin>6500 (cool), got {k}");
    }

    /// A more-saturated reference ⇒ saturation > 1; a flatter (lower-std) ⇒
    /// contrast < 1.
    #[test]
    fn saturation_and_contrast_track_ratios() {
        let t = base();
        let mut r = base();
        r.mean_chroma = 40.0; // 2× the target colourfulness
        r.std_luma = 20.0; // half the target tonal spread
        let g = derive_grade(&t, &r, 1.0);
        assert!(
            g.saturation > 1.0,
            "more colourful ref ⇒ saturation>1, got {}",
            g.saturation
        );
        assert!(
            g.contrast < 1.0,
            "flatter ref ⇒ contrast<1, got {}",
            g.contrast
        );
        // Full-strength chroma-mean ratio is 40/20 = 2.0.
        assert!(
            (g.saturation - 2.0).abs() < 1e-9,
            "sat ratio exact, got {}",
            g.saturation
        );
    }

    /// strength 0.5 yields exactly half the full-strength brightness correction.
    #[test]
    fn half_strength_is_halfway() {
        let t = base();
        let mut r = base();
        r.mean_luma = 220.0;
        let full = derive_grade(&t, &r, 1.0).brightness;
        let half = derive_grade(&t, &r, 0.5).brightness;
        assert!(
            (half - full / 2.0).abs() < 1e-9,
            "half: {half}, full/2: {}",
            full / 2.0
        );
    }

    /// Matching a clip to ITSELF (identical stats) is a clean no-op (identity).
    #[test]
    fn self_match_is_identity() {
        let t = base();
        let g = derive_grade(&t, &t, 1.0);
        assert!(g.is_identity(), "self-match must be identity, got {g:?}");
    }

    /// Denominator floor: a flat (zero-spread) target can't explode the ratios.
    #[test]
    fn flat_target_does_not_blow_up() {
        let mut t = base();
        t.std_luma = 0.0;
        t.mean_chroma = 0.0;
        let r = base();
        let g = derive_grade(&t, &r, 1.0);
        assert!(g.contrast.is_finite() && g.contrast <= 3.0);
        assert!(g.saturation.is_finite() && g.saturation <= 3.0);
    }

    /// rgb24 accumulation: a flat grey buffer → mean≈value, std≈0, chroma≈0.
    #[test]
    fn rgb24_flat_grey_stats() {
        let buf = vec![100u8; 3 * 256]; // 256 grey pixels
        let s = ChannelStats::from_rgb24(&buf).unwrap();
        assert!((s.mean_r - 100.0).abs() < 1e-9 && (s.mean_luma - 100.0).abs() < 1e-6);
        assert!(
            s.std_luma < 1e-6 && s.mean_chroma < 1e-6,
            "flat grey has no spread/chroma"
        );
    }

    /// rgb24 accumulation rejects a malformed (non-multiple-of-3) buffer.
    #[test]
    fn rgb24_rejects_bad_buffer() {
        assert!(ChannelStats::from_rgb24(&[1, 2]).is_err());
        assert!(ChannelStats::from_rgb24(&[]).is_err());
    }

    // ───────────────────────── auto-balance (reference-free) ───────────────

    /// A perfectly mid-exposed, colour-neutral frame: luma mean AT the exposure
    /// target, spread inside the contrast deadband, zero warm/cool cast.
    fn well_exposed_neutral() -> ChannelStats {
        ChannelStats {
            mean_r: TARGET_LUMA_MEAN,
            mean_g: TARGET_LUMA_MEAN,
            mean_b: TARGET_LUMA_MEAN,
            std_r: TARGET_LUMA_STD,
            std_g: TARGET_LUMA_STD,
            std_b: TARGET_LUMA_STD,
            mean_luma: TARGET_LUMA_MEAN,
            std_luma: TARGET_LUMA_STD,
            mean_chroma: 0.0,
            std_chroma: 0.0,
        }
    }

    /// An already-neutral, well-exposed frame derives an exact identity grade
    /// (the "auto-balance does nothing when nothing is wrong" contract).
    #[test]
    fn auto_balance_neutral_frame_is_identity() {
        let g = derive_auto_balance(&well_exposed_neutral(), None, 1.0);
        assert!(
            g.is_identity(),
            "neutral well-exposed frame ⇒ identity, got {g:?}"
        );
    }

    /// strength 0 ⇒ identity no matter how cast / mis-exposed the frame is.
    #[test]
    fn auto_balance_zero_strength_is_identity() {
        let mut s = well_exposed_neutral();
        s.mean_luma = 30.0;
        s.std_luma = 10.0;
        s.mean_r = 200.0;
        s.mean_b = 40.0;
        let g = derive_auto_balance(&s, Some(60.0), 0.0);
        assert!(g.is_identity(), "strength 0 ⇒ identity, got {g:?}");
    }

    /// A WARM-cast frame (R̄ ≫ B̄) ⇒ temperature pushed COOL (Kelvin > 6500),
    /// i.e. the correction neutralises the cast.
    #[test]
    fn auto_balance_warm_cast_pushes_cool() {
        let mut s = well_exposed_neutral();
        s.mean_r = 140.0; // more red…
        s.mean_b = 90.0; // …than blue ⇒ warmth +50
        let g = derive_auto_balance(&s, None, 1.0);
        let k = g
            .temperature_k
            .expect("a warm cast sets temperature_k (white balance)");
        assert!(
            k > 6500,
            "warm cast ⇒ Kelvin>6500 (cool correction), got {k}"
        );
        // Exposure untouched (luma at target) ⇒ brightness exactly 0, isolating WB.
        assert_eq!(g.brightness, 0.0, "luma at target ⇒ no exposure change");
    }

    /// A COOL-cast frame (B̄ ≫ R̄) ⇒ temperature pushed WARM (Kelvin < 6500).
    #[test]
    fn auto_balance_cool_cast_pushes_warm() {
        let mut s = well_exposed_neutral();
        s.mean_r = 90.0;
        s.mean_b = 140.0; // strongly blue ⇒ warmth −50
        let g = derive_auto_balance(&s, None, 1.0);
        let k = g.temperature_k.expect("a cool cast sets temperature_k");
        assert!(
            k < 6500,
            "cool cast ⇒ Kelvin<6500 (warm correction), got {k}"
        );
    }

    /// A too-DARK frame lifts brightness (> 0); a too-BRIGHT frame lowers it.
    #[test]
    fn auto_balance_exposure_corrects_both_ways() {
        let mut dark = well_exposed_neutral();
        dark.mean_luma = 50.0;
        assert!(
            derive_auto_balance(&dark, None, 1.0).brightness > 0.0,
            "dark frame ⇒ brightness>0"
        );
        let mut bright = well_exposed_neutral();
        bright.mean_luma = 210.0;
        assert!(
            derive_auto_balance(&bright, None, 1.0).brightness < 0.0,
            "bright frame ⇒ brightness<0"
        );
    }

    /// strength 0.5 yields exactly half the full-strength exposure correction.
    #[test]
    fn auto_balance_half_strength_is_halfway() {
        let mut s = well_exposed_neutral();
        s.mean_luma = 217.0;
        let full = derive_auto_balance(&s, None, 1.0).brightness;
        let half = derive_auto_balance(&s, None, 0.5).brightness;
        assert!(
            (half - full / 2.0).abs() < 1e-9,
            "half: {half}, full/2: {}",
            full / 2.0
        );
    }

    /// WHITE-PATCH uses the highlight warmth: a frame whose AVERAGE is neutral
    /// (gray-world ⇒ no WB) but whose HIGHLIGHTS are warm ⇒ white-patch cools.
    #[test]
    fn auto_balance_white_patch_uses_highlights() {
        let s = well_exposed_neutral(); // global average is neutral (warmth 0)
        let gw = derive_auto_balance(&s, None, 1.0);
        assert!(
            gw.temperature_k.is_none(),
            "gray-world neutral average ⇒ no white balance, got {:?}",
            gw.temperature_k
        );
        let wp = derive_auto_balance(&s, Some(40.0), 1.0); // warm highlights
        let k = wp
            .temperature_k
            .expect("warm highlights ⇒ white-patch sets temperature_k");
        assert!(k > 6500, "warm highlights ⇒ cool (Kelvin>6500), got {k}");
    }

    /// Contrast is GENTLE + deadbanded: a low-spread frame boosts a little, a
    /// high-spread frame tames a little, but stays inside the tight clamp.
    #[test]
    fn auto_balance_contrast_is_gentle() {
        let mut flat = well_exposed_neutral();
        flat.std_luma = 25.0; // well below target ⇒ boost
        let c = derive_auto_balance(&flat, None, 1.0).contrast;
        assert!(c > 1.0 && c <= 1.8, "flat frame ⇒ gentle boost, got {c}");
        let mut harsh = well_exposed_neutral();
        harsh.std_luma = 90.0; // well above target ⇒ tame
        let c2 = derive_auto_balance(&harsh, None, 1.0).contrast;
        assert!(
            (0.5..1.0).contains(&c2),
            "harsh frame ⇒ gentle tame, got {c2}"
        );
    }

    /// The highlight estimator reads the bright NEAR-NEUTRAL region's warmth and
    /// ignores darker pixels.
    #[test]
    fn highlight_warmth_reads_bright_neutral_region() {
        let mut buf = vec![100u8; 3 * 200]; // 200 mid-grey pixels (too dark → excluded)
        for _ in 0..64 {
            buf.extend_from_slice(&[230, 220, 210]); // bright, slightly-warm near-neutral
        }
        let w = highlight_neutral_warmth(&buf).expect("a bright neutral region is present");
        assert!(
            (w - 20.0).abs() < 1.0,
            "warm highlight warmth ≈ R−B = 20, got {w}"
        );
    }

    /// A saturated bright object is NOT a neutral highlight: it is excluded, and
    /// with no other highlights the estimator returns None (→ gray-world).
    #[test]
    fn highlight_warmth_excludes_coloured_object() {
        let mut buf = vec![40u8; 3 * 200]; // dark grey (excluded)
        for _ in 0..64 {
            buf.extend_from_slice(&[255, 0, 0]); // bright but pure-red (high chroma)
        }
        assert!(
            highlight_neutral_warmth(&buf).is_none(),
            "a saturated bright object must not be read as a neutral highlight"
        );
    }

    /// Mode parsing: default, the two valid modes, and an actionable error.
    #[test]
    fn auto_balance_mode_parse() {
        assert_eq!(
            AutoBalanceMode::parse(None).unwrap(),
            AutoBalanceMode::GrayWorld
        );
        assert_eq!(
            AutoBalanceMode::parse(Some("gray_world")).unwrap(),
            AutoBalanceMode::GrayWorld
        );
        assert_eq!(
            AutoBalanceMode::parse(Some("white_patch")).unwrap(),
            AutoBalanceMode::WhitePatch
        );
        assert!(AutoBalanceMode::parse(Some("sepia")).is_err());
    }
}
