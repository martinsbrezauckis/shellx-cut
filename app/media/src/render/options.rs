use cut_core::{Edl, Project, TrackKind};
use serde::{Deserialize, Serialize};

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

/// Encoder preset for final renders. Named presets keep encodes deterministic
/// and receipt-comparable; the receipt records the preset name verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderPreset {
    /// Preset name recorded in the RenderReceipt.
    pub name: String,
    /// Video codec args, e.g. ["-c:v","libx264","-preset","medium","-crf","18"].
    pub video_args: Vec<String>,
    /// Audio codec args, e.g. ["-c:a","aac","-b:a","192k"].
    pub audio_args: Vec<String>,
}

/// The named preset registry (quality regression: the single ≈550 kbps
/// default was too thin for a homepage hero asset). Three quality tiers; the
/// output geometry/fps always come from the PROJECT SETTINGS — presets only
/// pick encoder effort + rate-control + audio bitrate:
/// - "draft"    — the pre-tier default encode verbatim (x264 medium CRF 18,
///                AAC 192k): fine for review cuts, fast.
/// - "standard" — DEFAULT. x264 slow CRF 20, AAC 192k: better rate-efficiency
///                at slightly stronger compression (the slower preset is what
///                keeps it at draft-class quality with smaller files).
/// - "high"     — x264 slow CRF 17, AAC 256k: hero-asset tier (homepage).
pub const PRESET_NAMES: &[&str] = &["draft", "standard", "high"];

impl RenderPreset {
    /// Look up a preset by name; None for unknown names (callers turn that
    /// into an actionable invalid_args BEFORE any encode work starts).
    pub fn named(name: &str) -> Option<Self> {
        let s = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        match name {
            "draft" => Some(Self {
                name: "draft".into(),
                video_args: s(&[
                    "-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p",
                ]),
                audio_args: s(&["-c:a", "aac", "-b:a", "192k"]),
            }),
            "standard" => Some(Self {
                name: "standard".into(),
                video_args: s(&[
                    "-c:v", "libx264", "-preset", "slow", "-crf", "20", "-pix_fmt", "yuv420p",
                ]),
                audio_args: s(&["-c:a", "aac", "-b:a", "192k"]),
            }),
            "high" => Some(Self {
                name: "high".into(),
                video_args: s(&[
                    "-c:v", "libx264", "-preset", "slow", "-crf", "17", "-pix_fmt", "yuv420p",
                ]),
                audio_args: s(&["-c:a", "aac", "-b:a", "256k"]),
            }),
            _ => None,
        }
    }
}
impl Default for RenderPreset {
    /// Default tier: "standard" (quality contract: default must be presentable,
    /// draft is opt-in for speed, high is opt-in for hero assets).
    fn default() -> Self {
        Self::named("standard").expect("standard is a registered preset")
    }
}

/// Final-render output FORMATS (render.final `format` — "different file exports").
/// h264 is the DEFAULT and reuses the named quality preset verbatim, so omitting
/// `format` replays byte-identical. Researched against current editors + codecs
/// (2026-06): H.264+AAC/mp4 = the universal "golden combo" (YouTube-recommended,
/// every platform); HEVC/mp4 ≈30-50% smaller at equal quality (Apple `hvc1` tag
/// for QuickTime); VP9+Opus/webm = royalty-free web-native; ProRes 422/mov = the
/// pro editing-handoff intermediate; AV1/mp4 = the highest quality ceiling but
/// SOFTWARE-slow here (libsvtav1) — hardware av1_nvenc/qsv is a follow-up.
pub const FORMAT_NAMES: &[&str] = &["h264", "hevc", "vp9", "prores", "av1"];

/// Map a `format` id + quality tier (draft|standard|high) to the encoder args +
/// output extension. Returns None for an unknown format (the verb turns that into
/// an actionable error BEFORE any encode). The quality tier shifts the rate knob
/// per codec (their CRF scales differ); ProRes quality is fixed by its profile.
pub fn format_codec_args(
    format: &str,
    quality: &str,
) -> Option<(Vec<String>, Vec<String>, &'static str)> {
    let s = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let q = match quality {
        "draft" => 0usize,
        "high" => 2,
        _ => 1, // standard
    };
    Some(match format {
        // Default golden combo — reuse the named h264 preset for byte-identical
        // replay of a no-`format` render.
        "h264" | "mp4" => {
            let p = RenderPreset::named(quality).unwrap_or_default();
            (p.video_args, p.audio_args, "mp4")
        }
        "hevc" | "h265" => {
            let crf = ["28", "26", "23"][q];
            (
                s(&[
                    "-c:v", "libx265", "-preset", "medium", "-crf", crf, "-pix_fmt", "yuv420p",
                    "-tag:v", "hvc1",
                ]),
                s(&["-c:a", "aac", "-b:a", "192k"]),
                "mp4",
            )
        }
        "vp9" | "webm" => {
            let crf = ["36", "32", "28"][q];
            (
                s(&[
                    "-c:v",
                    "libvpx-vp9",
                    "-b:v",
                    "0",
                    "-crf",
                    crf,
                    "-pix_fmt",
                    "yuv420p",
                    "-row-mt",
                    "1",
                ]),
                s(&["-c:a", "libopus", "-b:a", "160k"]),
                "webm",
            )
        }
        "prores" | "mov" => (
            // ProRes 422 (profile 2): the editing-handoff standard; 10-bit 4:2:2,
            // PCM audio (the .mov convention). Quality fixed by profile, not CRF.
            s(&[
                "-c:v",
                "prores_ks",
                "-profile:v",
                "2",
                "-pix_fmt",
                "yuv422p10le",
                "-vendor",
                "apl0",
            ]),
            s(&["-c:a", "pcm_s16le"]),
            "mov",
        ),
        "av1" => {
            let crf = ["38", "32", "27"][q];
            (
                s(&[
                    "-c:v",
                    "libsvtav1",
                    "-preset",
                    "6",
                    "-crf",
                    crf,
                    "-pix_fmt",
                    "yuv420p",
                ]),
                s(&["-c:a", "aac", "-b:a", "192k"]),
                "mp4",
            )
        }
        _ => return None,
    })
}

/// Parse a human bitrate string into kbps. Accepts `"12M"` (12 Mbps → 12000
/// kbps), `"12000k"` (12000 kbps), `"0.5M"` (500 kbps), or a bare number
/// (kbps). Returns None for unparseable / out-of-sane-range (<50 kbps or
/// >500 Mbps) values — the verb turns that into an actionable error before any
/// > encode. Rate-targeted publishing (render.final `bitrate`) needs kbps; this
/// > is the single place the unit grammar lives so the UI and agents agree.
pub fn parse_bitrate_kbps(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let (num, mult) = if let Some(p) = t.strip_suffix(['M', 'm']) {
        (p, 1000.0) // Mbps → kbps
    } else if let Some(p) = t.strip_suffix(['k', 'K']) {
        (p, 1.0)
    } else {
        (t, 1.0)
    };
    let v: f64 = num.trim().parse().ok()?;
    if !v.is_finite() || v <= 0.0 {
        return None;
    }
    let kbps = (v * mult).round() as i64;
    if !(50..=500_000).contains(&kbps) {
        return None;
    }
    u32::try_from(kbps).ok()
}

/// Rewrite a codec's rate-control from quality-targeted (CRF/CQ — the default)
/// to BITRATE-targeted (VBR or CBR), for platform-spec publishing (render.final
/// `bitrate`/`rate_control`; export.publish). `encoder` is the resolved `-c:v`
/// value (libx264, libx265, h264_nvenc, hevc_qsv, …) so the right per-family
/// rate flags are emitted — software x264/x265/svtav1/vp9 use `-b:v/-maxrate/
/// -bufsize`; NVENC/QSV/AMF/VideoToolbox each take their own `-rc`/cap grammar.
/// The existing quality knob (`-crf`, `-cq`, `-q:v`, vp9's `-b:v 0`, …) is
/// STRIPPED first so the two rate-control regimes never both apply. ProRes is
/// profile-fixed (no bitrate target) → returned unchanged; the caller warns.
/// `kbps` is the target average; VBR caps at ~1.45× (headroom for motion), CBR
/// pins min=max=target. This is the genuinely-new encoding capability (the
/// pre-existing path was CRF-only).
pub fn apply_bitrate(video_args: Vec<String>, kbps: u32, cbr: bool, encoder: &str) -> Vec<String> {
    let s = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    // ProRes (and any future fixed-rate intra codec) ignores a bitrate target.
    if encoder.contains("prores") {
        return video_args;
    }
    // Quality / rate flags that conflict with an explicit bitrate target — drop
    // each flag AND its value before re-stating the rate control.
    const RATE_FLAGS: &[&str] = &[
        "-crf",
        "-cq",
        "-qp",
        "-qp_i",
        "-qp_p",
        "-qp_b",
        "-global_quality",
        "-q:v",
        "-b:v",
        "-maxrate",
        "-minrate",
        "-bufsize",
        "-rc",
        "-rc:v",
    ];
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < video_args.len() {
        if RATE_FLAGS.contains(&video_args[i].as_str()) {
            i += 2; // skip flag + its value
            continue;
        }
        out.push(video_args[i].clone());
        i += 1;
    }
    let b = format!("{kbps}k");
    let max_vbr = format!("{}k", rounded_u32_clamped(f64::from(kbps) * 1.45, u32::MAX));
    let buf_vbr = format!("{}k", kbps * 2);
    let cap = format!("{kbps}k"); // CBR cap = target
    if encoder.contains("nvenc") {
        if cbr {
            out.extend(s(&["-rc", "cbr", "-b:v"]));
            out.push(b);
            out.extend(s(&["-maxrate"]));
            out.push(cap.clone());
            out.extend(s(&["-bufsize"]));
            out.push(cap);
        } else {
            out.extend(s(&["-rc", "vbr", "-b:v"]));
            out.push(b);
            out.extend(s(&["-maxrate"]));
            out.push(max_vbr);
            out.extend(s(&["-bufsize"]));
            out.push(buf_vbr);
        }
    } else if encoder.contains("qsv") {
        out.push("-b:v".into());
        out.push(b);
        out.push("-maxrate".into());
        out.push(if cbr { cap } else { max_vbr });
    } else if encoder.contains("amf") {
        out.extend(s(if cbr {
            &["-rc", "cbr"]
        } else {
            &["-rc", "vbr_peak"]
        }));
        out.push("-b:v".into());
        out.push(b);
        if !cbr {
            out.push("-maxrate".into());
            out.push(max_vbr);
        }
    } else if encoder.contains("videotoolbox") {
        out.push("-b:v".into());
        out.push(b);
        out.push("-maxrate".into());
        out.push(if cbr { cap } else { max_vbr });
    } else {
        // Software (libx264 / libx265 / libsvtav1 / libvpx-vp9) + generic.
        out.push("-b:v".into());
        out.push(b);
        if cbr {
            out.push("-minrate".into());
            out.push(cap.clone());
            out.push("-maxrate".into());
            out.push(cap.clone());
            out.push("-bufsize".into());
            out.push(cap);
            // TRUE CBR: -minrate=-maxrate alone only CAPS the rate (x264 won't
            // pad up to it on easy content). The
            // codec-specific HRD/strict flag makes CBR honestly constant (pads to
            // target), which is the point of CBR (streaming / strict ingest).
            if encoder.contains("libx264") {
                out.push("-x264-params".into());
                out.push("nal-hrd=cbr".into());
            } else if encoder.contains("libx265") {
                out.push("-x265-params".into());
                out.push("strict-cbr=1".into());
            }
            // libsvtav1 / libvpx-vp9: best-effort capped CBR (no stable strict-CBR
            // flag across builds) — documented as approximate for those codecs.
        } else {
            out.push("-maxrate".into());
            out.push(max_vbr);
            out.push("-bufsize".into());
            out.push(buf_vbr);
        }
    }
    out
}

/// Override the audio bitrate (`-b:a`) in a codec's audio args — platform specs
/// want a specific AAC rate (YouTube 384k, social 192k). Rewrites an existing
/// `-b:a` value in place; leaves lossless codecs (PCM/wav/prores, which carry
/// no `-b:a`) untouched. `kbps` is the target audio bitrate.
pub fn set_audio_bitrate(audio_args: Vec<String>, kbps: u32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < audio_args.len() {
        if audio_args[i] == "-b:a" && i + 1 < audio_args.len() {
            out.push("-b:a".into());
            out.push(format!("{kbps}k"));
            i += 2;
            continue;
        }
        out.push(audio_args[i].clone());
        i += 1;
    }
    out
}

/// A platform publish target — the encoding spec for a one-click "Export for
/// YouTube/TikTok/…" (export.publish). Researched 2026-06 against current
/// platform/creator docs (YouTube Help upload settings, TikTok/Reels 2026 spec
/// guides). These DRIFT — re-verify before trusting. Geometry + bitrate + audio
/// rate are the high-value, well-documented knobs; fps comes from the project
/// settings (a publish does not resample fps in v1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformSpec {
    /// Human label for the receipt/result.
    pub label: &'static str,
    pub width: u32,
    pub height: u32,
    /// Target video bitrate (kbps) and rate control.
    pub video_kbps: u32,
    pub cbr: bool,
    /// Target audio bitrate (kbps).
    pub audio_kbps: u32,
    /// Output format id (always h264 today — universal platform combo).
    pub format: &'static str,
}

/// Canonical platform ids (+ a couple of aliases handled in `platform_spec`).
pub const PLATFORM_NAMES: &[&str] = &[
    "youtube",
    "youtube_4k",
    "tiktok",
    "reels",
    "instagram_feed",
    "x",
    "square",
];

/// Resolve a platform id → its publish spec. Aliases: `shorts`→tiktok geometry,
/// `instagram`/`ig`→reels, `twitter`→x. None for an unknown id (the verb lists
/// the valid names). 2026-researched values; H.264/AAC/mp4 everywhere (the one
/// combo every platform ingests cleanly).
pub fn platform_spec(p: &str) -> Option<PlatformSpec> {
    let spec = |label, width, height, video_kbps, cbr, audio_kbps| PlatformSpec {
        label,
        width,
        height,
        video_kbps,
        cbr,
        audio_kbps,
        format: "h264",
    };
    Some(match p {
        // 16:9 landscape
        "youtube" => spec("YouTube 1080p", 1920, 1080, 12_000, false, 384),
        "youtube_4k" | "youtube4k" => spec("YouTube 4K", 3840, 2160, 45_000, false, 384),
        "x" | "twitter" => spec("X (Twitter)", 1920, 1080, 20_000, false, 192),
        // 9:16 vertical
        "tiktok" | "shorts" | "youtube_shorts" => {
            spec("TikTok / Shorts", 1080, 1920, 12_000, false, 192)
        }
        "reels" | "instagram" | "ig" | "instagram_reels" => {
            spec("Instagram Reels", 1080, 1920, 10_000, false, 192)
        }
        // 4:5 portrait feed
        "instagram_feed" | "ig_feed" => spec("Instagram Feed", 1080, 1350, 10_000, false, 192),
        // 1:1 square
        "square" => spec("Square 1:1", 1080, 1080, 10_000, false, 192),
        _ => return None,
    })
}

/// How a segment that does not match the output aspect ratio is fitted to the
/// frame (render.final `fit`). DEFAULT = `Contain` — the original
/// behavior, so omitting `fit` replays byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Fit {
    /// Letterbox/pillarbox: scale to fit INSIDE the frame, pad the remainder
    /// with black. No source pixels are lost. This preserves legacy behavior.
    #[default]
    Contain,
    /// Crop-to-fill: scale to COVER the frame, centre-crop the overflow. Fills
    /// the frame edge-to-edge; pixels outside the frame aspect are lost.
    Cover,
}

impl Fit {
    /// Wire name (render.final `fit` arg / receipt string).
    pub fn as_str(self) -> &'static str {
        match self {
            Fit::Contain => "contain",
            Fit::Cover => "cover",
        }
    }
}

impl std::str::FromStr for Fit {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "contain" => Ok(Fit::Contain),
            "cover" => Ok(Fit::Cover),
            other => Err(format!("unknown fit '{other}' — valid: contain, cover")),
        }
    }
}

/// Output geometry resolution (render.final `resolution`). DEFAULT =
/// `Project` — the project settings geometry, the original behavior, so
/// omitting `resolution` replays byte-identical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Resolution {
    /// Use the project settings width×height (the default, unchanged behavior).
    #[default]
    Project,
    /// Derive the output geometry from the LARGEST source video used on the
    /// timeline (max width, max height across base-track media assets). Keeps a
    /// high-res source at its native resolution instead of conforming down to
    /// the project frame. fps/audio_rate still come from project settings.
    MatchSource,
    /// Explicit output geometry for THIS render only (render.final
    /// `aspect`/`width`/`height`) — multi-format publishing (e.g. reframe a
    /// 16:9 project to 9:16 for Shorts/Reels) WITHOUT mutating the project. The
    /// project settings are untouched; fps/audio_rate still come from them.
    /// Paired with `fit:cover`, the conform stage scales-to-cover and
    /// centre-crops each clip into the new frame — the reframe.
    Explicit { width: u32, height: u32 },
}

impl Resolution {
    /// Wire name (render.final `resolution` arg / receipt string).
    pub fn as_str(self) -> &'static str {
        match self {
            Resolution::Project => "project",
            Resolution::MatchSource => "match_source",
            Resolution::Explicit { .. } => "explicit",
        }
    }
}

impl std::str::FromStr for Resolution {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "project" => Ok(Resolution::Project),
            "match_source" => Ok(Resolution::MatchSource),
            other => Err(format!(
                "unknown resolution '{other}' — valid: project, match_source"
            )),
        }
    }
}

/// Output framing options for a render (render.final `fit` + `resolution`).
/// `Default` = contain + project geometry = the legacy render, so an
/// op log that never set these replays byte-identical (the registry-sync
/// default test depends on this).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RenderOptions {
    pub fit: Fit,
    pub resolution: Resolution,
    /// Target integrated loudness in LUFS (render.final `normalize_loudness`).
    /// `None` (default) = no normalization, byte-identical pre-feature replay.
    /// `Some(t)` applies single-pass ffmpeg `loudnorm` to the mixed audio so the
    /// output measures ≈ `t` LUFS — closing the measure(lufs check)→target loop
    /// for consistent published loudness (-16 long-form / -14 social / -23 EBU).
    /// i32 (not f64): LUFS targets are whole numbers and it keeps RenderOptions
    /// Copy + Eq for the registry-sync default test.
    pub loudness_target: Option<i32>,
}

impl RenderOptions {
    /// Resolve the OUTPUT (width, height) for a project + EDL under this
    /// options' resolution mode. `Project` → settings geometry; `MatchSource`
    /// → the largest source VIDEO geometry on the timeline (a cropped clip
    /// contributes its crop rect, not the full source frame, so cropping then
    /// match_source fills the frame). Falls back to project geometry when no
    /// video source carries a probed width/height (stills-only or unprobed
    /// timeline). Even-rounded for yuv420 chroma.
    ///
    /// ONLY VIDEO-track segments are considered: an audio segment referencing
    /// the same asset carries the asset's VIDEO probe geometry but no crop, so
    /// counting it would re-introduce the full (uncropped) source height and
    /// undo the crop. Geometry is a video concept;
    /// audio tracks never contribute frame size.
    pub fn output_geometry(&self, project: &Project, edl: &Edl) -> (u32, u32) {
        let (pw, ph) = (project.settings.width, project.settings.height);
        let even = |v: u32| v & !1u32;
        match self.resolution {
            Resolution::Project => (pw, ph),
            Resolution::Explicit { width, height } => (even(width).max(2), even(height).max(2)),
            Resolution::MatchSource => {
                let mut mw = 0u32;
                let mut mh = 0u32;
                for seg in edl
                    .segments
                    .iter()
                    .filter(|s| s.asset.is_some() && s.track_kind == TrackKind::Video)
                {
                    let Some(asset) = project.assets.get(seg.asset.as_deref().unwrap()) else {
                        continue;
                    };
                    if let Some(probe) = asset.probe.as_ref() {
                        let w = probe.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let h = probe.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        // A cropped clip's effective source size is its crop rect.
                        let (w, h) = match seg.crop.as_ref() {
                            Some(c) => (c.w, c.h),
                            None => (w, h),
                        };
                        mw = mw.max(w);
                        mh = mh.max(h);
                    }
                }
                if mw >= 2 && mh >= 2 {
                    (even(mw).max(2), even(mh).max(2))
                } else {
                    (pw, ph) // no probed video source geometry — keep project frame
                }
            }
        }
    }
}
