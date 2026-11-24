//! hwencode.rs — GPU / hardware video-encoder detection (the ACCELERATED render
//! tier, on top of the universal software tier in render::format_codec_args).
//!
//! WHY A PROBE (not just `ffmpeg -encoders`): an encoder being LISTED does not
//! mean it RUNS — NVENC/QSV/AMF/VideoToolbox all need the matching GPU + a working
//! driver. The only reliable check is to actually encode a frame. So we run a tiny
//! (0.1 s, 256×256) test encode to `-f null` per candidate and keep the ones that
//! succeed. Result is cached (the probe runs several encodes).
//!
//! DROP-IN ONLY: we support encoders that accept the existing filter_complex
//! `[vout]` (system-memory frames they upload internally) — NVENC, QSV, AMF,
//! VideoToolbox. VAAPI is deliberately NOT here: it needs `-vaapi_device` + an
//! in-graph `hwupload`, i.e. surgery on build_graph's filter — a later addition.
//!
//! SAFETY: every backend's arg set is GATED by the probe, so even an arg set we
//! could not test on this machine can never produce a broken render — a failed
//! test encode just disables that encoder and we fall back to software.
//!
//! Dependencies: std::process, crate::ffmpeg (resolved ffmpeg path). Primary
//! caller: render::format_codec_args (HW-aware selection) + the server doctor.

use crate::ffmpeg::{ffmpeg_bin, run_owned_command, OwnedProcessControl};
use crate::toolpath::ToolCache;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Candidate HW encoders per codec, BEST-FIRST. The first that passes the test
/// encode wins. Order: NVIDIA NVENC (fastest + best quality), Intel QSV, AMD AMF,
/// Apple VideoToolbox. (VAAPI excluded — see header.)
const CANDIDATES: &[(&str, &[&str])] = &[
    (
        "h264",
        &["h264_nvenc", "h264_qsv", "h264_amf", "h264_videotoolbox"],
    ),
    (
        "hevc",
        &["hevc_nvenc", "hevc_qsv", "hevc_amf", "hevc_videotoolbox"],
    ),
    (
        "av1",
        &["av1_nvenc", "av1_qsv", "av1_amf", "av1_videotoolbox"],
    ),
];

/// The HW encoder (if any) that WORKS for each codec on this machine.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct HwCaps {
    /// e.g. Some("h264_nvenc"); None when only software h264 is available.
    pub h264: Option<String>,
    pub hevc: Option<String>,
    pub av1: Option<String>,
}

impl HwCaps {
    /// The working HW encoder for a base codec id ("h264"|"hevc"|"av1"), if any.
    pub fn for_codec(&self, codec: &str) -> Option<&str> {
        match codec {
            "h264" | "mp4" => self.h264.as_deref(),
            "hevc" | "h265" => self.hevc.as_deref(),
            "av1" => self.av1.as_deref(),
            _ => None,
        }
    }
    /// True when ANY hardware encoder was detected (the doctor "gpu-encode" tier).
    pub fn any(&self) -> bool {
        self.h264.is_some() || self.hevc.is_some() || self.av1.is_some()
    }
}

static PROBE: ToolCache<&'static (HwCaps, HwDiagnosis)> = ToolCache::new();
const HW_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// The CUDA filter probe's budget. The first run of FFmpeg's CUDA filters for a
/// user (and after a driver update) waits while the driver compiles their
/// kernels for this GPU and stores them in its cache: measured 8.0 s on an RTX
/// PRO 4500 (Blackwell) against 0.2 s once cached. Within 5 s that first probe
/// timed out, the "no CUDA chain" answer stayed for the whole session and an
/// opted-in render went to software. A machine without CUDA fails at once.
const CUDA_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Size probes synthesise real frames, and 8K costs more than 256x256, so the
/// existence probe's budget is too tight to reuse. A timeout here reads as
/// "cannot do this size" and falls back to software, which is the safe answer.
const HW_SIZE_PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Detected HW encoders (probed once per tools generation: installing or
/// choosing another ffmpeg re-probes after the next doctor re-scan; each
/// generation's answer is leaked so callers keep a `'static` borrow).
/// `SHELLX_CUT_NO_HWENC=1` forces the software tier (CI / reproducibility / a
/// machine whose HW encoder is flaky).
pub fn hw_caps() -> &'static HwCaps {
    &cached_probe().0
}

/// Why the resolved ffmpeg has no working hardware encoder (empty when it has
/// one). Comes from the same probe as [`hw_caps`], so asking costs no extra
/// encode.
pub fn hw_diagnosis() -> &'static HwDiagnosis {
    &cached_probe().1
}

fn cached_probe() -> &'static (HwCaps, HwDiagnosis) {
    PROBE.get_or_init(|| {
        let probed = if std::env::var_os("SHELLX_CUT_NO_HWENC").is_some() {
            (
                HwCaps::default(),
                HwDiagnosis {
                    disabled_by_setting: true,
                    failures: Vec::new(),
                },
            )
        } else {
            probe_at(&ffmpeg_bin())
        };
        Box::leak(Box::new(probed))
    })
}

/// Probe every candidate encoder of one ffmpeg binary: the working ones, plus
/// the reason each failing one failed (best-first per codec; the first working
/// candidate of a codec ends that codec's scan). Each codec family (`h264`,
/// `hevc`, `av1`) is independent and probed concurrently.
fn probe_at(ffmpeg: &OsStr) -> (HwCaps, HwDiagnosis) {
    probe_at_with_encoders(ffmpeg, None)
}

fn probe_at_with_encoders(
    ffmpeg: &OsStr,
    compiled_encoders: Option<&std::collections::HashSet<String>>,
) -> (HwCaps, HwDiagnosis) {
    let results: [(&str, Option<String>, Vec<EncoderFailure>); 3] = std::thread::scope(|scope| {
        let handles: Vec<_> = CANDIDATES
            .iter()
            .map(|&(codec, names)| {
                scope.spawn(move || {
                    let mut chosen = None;
                    let mut failures = Vec::new();
                    for &name in names {
                        if compiled_encoders.is_some_and(|set| !set.contains(name)) {
                            failures.push(EncoderFailure {
                                encoder: name.to_string(),
                                kind: EncoderFailureKind::NotInBuild,
                                detail: None,
                            });
                            continue;
                        }
                        match probe_encoder_at_size(ffmpeg, name, 256, 256, HW_SIZE_PROBE_TIMEOUT) {
                            Ok(()) => {
                                chosen = Some(name.to_string());
                                break; // best-first: first working candidate wins
                            }
                            Err(Some(failure)) => failures.push(failure),
                            // ffmpeg itself did not run: nothing to learn about the encoder.
                            Err(None) => {}
                        }
                    }
                    (codec, chosen, failures)
                })
            })
            .collect();
        let mut iter = handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| ("", None, Vec::new())));
        [
            iter.next().unwrap(),
            iter.next().unwrap(),
            iter.next().unwrap(),
        ]
    });
    let mut caps = HwCaps::default();
    let mut diagnosis = HwDiagnosis::default();
    for (codec, chosen, failures) in results {
        match codec {
            "h264" => caps.h264 = chosen,
            "hevc" => caps.hevc = chosen,
            "av1" => caps.av1 = chosen,
            _ => {}
        }
        diagnosis.failures.extend(failures);
    }
    (caps, diagnosis)
}

// === Why hardware encoding is not available ===================================
//
// A failed test encode is not one condition. The same "no hardware encoder"
// outcome has four different causes and each needs different advice, so the
// probe keeps ffmpeg's own error text and sorts it.

/// The cause of one failed test encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EncoderFailureKind {
    /// This ffmpeg build does not contain the encoder at all.
    NotInBuild,
    /// The encoder is in the build but the installed graphics driver is older
    /// than the encoder needs.
    DriverTooOld,
    /// The encoder is in the build but this computer has no graphics device it
    /// can use (no GPU, no driver loaded, or an unsupported GPU).
    NoDevice,
    /// The encoder is in the build and failed to open for another reason.
    CannotOpen,
}

/// One failed test encode: the encoder, the sorted cause, and ffmpeg's own error
/// lines (kept for the technical detail, never rewritten).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EncoderFailure {
    pub encoder: String,
    pub kind: EncoderFailureKind,
    /// ffmpeg's own error lines, trimmed to a few hundred characters.
    pub detail: Option<String>,
}

/// Sort ffmpeg's stderr from a failed test encode into a cause.
///
/// Matching is on lower-cased phrases ffmpeg and the vendor libraries print.
/// Driver-too-old is tested before no-device because an old NVIDIA driver also
/// prints "minimum required driver" text that a missing driver does not.
pub fn classify_encoder_failure(stderr: &str) -> EncoderFailureKind {
    let low = stderr.to_ascii_lowercase();
    let has = |needle: &str| low.contains(needle);
    if has("unknown encoder") || has("encoder not found") {
        EncoderFailureKind::NotInBuild
    } else if has("required nvenc api version")
        || has("driver is too old")
        || has("driver version is too old")
        || has("driver is older")
        || has("update your driver")
        || has("update the driver")
    {
        EncoderFailureKind::DriverTooOld
    } else if has("cannot load")
        || has("could not dynamically load")
        || has("no capable devices")
        || has("no nvenc capable")
        || has("no encode device")
        || has("unsupported device")
        || has("error initializing an internal mfx session")
        || has("amfrt")
        || has("no device available")
        || has("no such device")
    {
        EncoderFailureKind::NoDevice
    } else {
        EncoderFailureKind::CannotOpen
    }
}

/// The stderr lines that explain a failed test encode, joined and bounded. These
/// are ffmpeg's own words, so a person or an agent can search them.
///
/// ffmpeg ends a failed encode with generic lines ("Error while opening encoder",
/// "Conversion failed!"); the useful ones come earlier, prefixed with the encoder's
/// own log context (`[h264_nvenc @ ...]`). Prefer those, and fall back to the last
/// few lines when the encoder printed nothing under its own name.
fn failure_detail(encoder: &str, stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    let context = format!("[{encoder} @");
    let own: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| l.starts_with(&context))
        .take(3)
        .collect();
    let chosen: &[&str] = if own.is_empty() {
        &lines[lines.len().saturating_sub(3)..]
    } else {
        &own
    };
    let mut joined = chosen.join(" / ");
    if joined.len() > 400 {
        let mut cut = 400;
        while !joined.is_char_boundary(cut) {
            cut -= 1;
        }
        joined.truncate(cut);
    }
    Some(joined)
}

/// What to tell the person, in the order of how much it helps: the cause that
/// the person can fix beats the one they cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HwShortfall {
    /// Hardware encoding is switched off for this installation.
    SwitchedOff,
    /// The encoder exists but the graphics driver is too old for it.
    DriverTooOld,
    /// The encoder exists and failed to open on this computer for another reason.
    CannotOpen,
    /// No usable graphics device.
    NoGpu,
    /// This ffmpeg contains no hardware encoder.
    NoEncoderInBuild,
    /// Nothing recorded (the probe found no failure to explain).
    Unknown,
}

impl HwShortfall {
    /// The cause in one plain sentence (no encoder acronyms).
    pub fn reason(self) -> &'static str {
        match self {
            Self::SwitchedOff => "Hardware encoding is switched off for this installation.",
            Self::DriverTooOld => {
                "The graphics driver is too old for the hardware encoder in this ffmpeg."
            }
            Self::CannotOpen => {
                "The hardware encoder in this ffmpeg could not be started on this computer."
            }
            Self::NoGpu => "No graphics card that can encode video was found.",
            Self::NoEncoderInBuild => "This ffmpeg has no hardware video encoder.",
            Self::Unknown => "The hardware encoder could not be used.",
        }
    }

    /// What the person can do about it.
    pub fn action(self) -> &'static str {
        match self {
            Self::SwitchedOff => "Software encoding always works, only slower.",
            Self::DriverTooOld => "Update the graphics driver, then restart ShellX Cut.",
            Self::CannotOpen => {
                "Update the graphics driver or pick another ffmpeg with 'Change ffmpeg'; \
                 the technical detail has ffmpeg's own message."
            }
            Self::NoGpu => {
                "If this computer has an NVIDIA, Intel or AMD graphics card, install or \
                 update its driver."
            }
            Self::NoEncoderInBuild => {
                "Install an ffmpeg build that includes GPU encoders, then pick it with \
                 'Change ffmpeg'."
            }
            Self::Unknown => "Software encoding always works, only slower.",
        }
    }
}

/// Why no hardware encoder worked on one ffmpeg binary.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct HwDiagnosis {
    /// True when `SHELLX_CUT_NO_HWENC` stopped the probe from running.
    pub disabled_by_setting: bool,
    /// One entry per encoder that was tried and failed.
    pub failures: Vec<EncoderFailure>,
}

impl HwDiagnosis {
    /// The shortfall that explains the missing hardware encoder. `codec` limits
    /// the view to one codec's encoders ("h264", "hevc", "av1"); None looks at
    /// every encoder that was tried.
    pub fn shortfall(&self, codec: Option<&str>) -> HwShortfall {
        if self.disabled_by_setting {
            return HwShortfall::SwitchedOff;
        }
        let of_codec = |f: &&EncoderFailure| match codec {
            Some(c) => f.encoder.starts_with(c),
            None => true,
        };
        let kinds: Vec<EncoderFailureKind> = self
            .failures
            .iter()
            .filter(of_codec)
            .map(|f| f.kind)
            .collect();
        let any = |k: EncoderFailureKind| kinds.contains(&k);
        if any(EncoderFailureKind::DriverTooOld) {
            HwShortfall::DriverTooOld
        } else if any(EncoderFailureKind::CannotOpen) {
            HwShortfall::CannotOpen
        } else if any(EncoderFailureKind::NoDevice) {
            HwShortfall::NoGpu
        } else if any(EncoderFailureKind::NotInBuild) {
            HwShortfall::NoEncoderInBuild
        } else {
            HwShortfall::Unknown
        }
    }

    /// ffmpeg's own error lines for the failure that decided [`Self::shortfall`].
    pub fn technical_detail(&self, codec: Option<&str>) -> Option<String> {
        let wanted = match self.shortfall(codec) {
            HwShortfall::DriverTooOld => EncoderFailureKind::DriverTooOld,
            HwShortfall::CannotOpen => EncoderFailureKind::CannotOpen,
            HwShortfall::NoGpu => EncoderFailureKind::NoDevice,
            _ => return None,
        };
        self.failures
            .iter()
            .filter(|f| codec.is_none_or(|c| f.encoder.starts_with(c)))
            .find(|f| f.kind == wanted)
            .and_then(|f| f.detail.as_ref().map(|d| format!("{}: {d}", f.encoder)))
    }
}

/// Why a render that asked for hardware encoding used software, ready to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftwareFallback {
    /// One or two plain sentences: what happened and what the person can do.
    pub message: String,
    /// ffmpeg's own error line when there is one.
    pub detail: Option<String>,
}

/// Explain a software render that was asked to use hardware.
///
/// `codec` is the base codec ("h264"|"hevc"|"av1") or "" for a format that has no
/// hardware tier; `(w, h)` is the real output size (0 = unknown). A working
/// hardware encoder that declined the frame size is the one case where the
/// encoder exists and the probe found nothing wrong.
pub fn software_fallback(codec: &str, w: u32, h: u32) -> SoftwareFallback {
    if codec.is_empty() {
        return SoftwareFallback {
            message: "This format has no hardware encoder, so this export used software \
                      encoding."
                .into(),
            detail: None,
        };
    }
    if let Some(enc) = hw_caps().for_codec(codec) {
        if w == 0 || h == 0 {
            return SoftwareFallback {
                message: format!(
                    "The hardware encoder ({enc}) cannot encode a picture this large, so this \
                     export used software encoding."
                ),
                detail: None,
            };
        }
        // Item 96: name the encoder's size limit and the hardware codec that can
        // do this size (HEVC reaches 8K where H.264 hardware stops at 4096 px).
        let limit = crate::hwencode_limits::limit_with(enc, encoder_supports_size);
        let alternative = crate::hwencode_limits::hardware_alternative(codec, w, h);
        return SoftwareFallback {
            message: size_fallback_message(
                enc,
                &crate::hwencode_limits::limit_phrase(&limit, w, h),
                w,
                h,
                alternative.as_ref().map(|(label, e)| (*label, e.as_str())),
            ),
            detail: None,
        };
    }
    let diagnosis = hw_diagnosis();
    let shortfall = diagnosis.shortfall(Some(codec));
    SoftwareFallback {
        message: format!(
            "No usable hardware encoder was found, so this export used software encoding. {} {}",
            shortfall.reason(),
            shortfall.action()
        ),
        detail: diagnosis.technical_detail(Some(codec)),
    }
}

/// The plan and receipt sentence for a frame larger than the hardware encoder
/// takes: the limit, the size, and the faster choice when one exists.
pub(crate) fn size_fallback_message(
    encoder: &str,
    limit: &str,
    w: u32,
    h: u32,
    alternative: Option<(&str, &str)>,
) -> String {
    let mut message = format!(
        "The hardware encoder ({encoder}) {limit}; this export is {w}x{h}, so it used \
         software encoding, which is much slower."
    );
    if let Some((label, alt)) = alternative {
        message.push_str(&format!(
            " Choose {label} to encode this size in hardware ({alt})."
        ));
    }
    message
}

/// Does `encoder` run AT THIS FRAME SIZE on the resolved ffmpeg?
///
/// Size matters and a fixed-size probe cannot answer it. Hardware encoders carry
/// their own dimension ceilings: NVENC's H.264 engine refuses anything wider than
/// 4096 px (`Width 7680 exceeds 4096` → `No capable devices found`) while the
/// same GPU's HEVC engine encodes 8K happily. A 256x256 probe therefore proves
/// only that the encoder EXISTS, and swapping it in for an 8K timeline fails the
/// whole render at encoder-open time — after the job has been accepted, leaving a
/// 0-byte file behind.
///
/// Probing at the real geometry avoids hard-coding a vendor/generation limit
/// table that would be wrong somewhere: the machine answers for itself, and a
/// future driver that lifts a ceiling is picked up with no code change.
fn encoder_works_at_size(ffmpeg: &OsStr, encoder: &str, w: u32, h: u32) -> bool {
    probe_encoder_at_size_with_cancel(ffmpeg, encoder, w, h, HW_SIZE_PROBE_TIMEOUT, None).is_ok()
}

/// Bounded resolution probe with caller cancellation support (used by deferred
/// Doctor enrichment so invalidation/shutdown kills in-flight 4K/8K test encodes).
pub fn encoder_works_at_size_cancelable(
    ffmpeg: &OsStr,
    encoder: &str,
    w: u32,
    h: u32,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> bool {
    // Same memo as `encoder_supports_size`, but keyed on the explicit binary the
    // Doctor scanned, so an explicit re-scan of an unchanged ffmpeg does not
    // repeat the 4K/8K test encodes. An answer from a probe that was cancelled
    // (before or while it ran) says nothing about the encoder and is never kept.
    static SCAN_CACHE: OnceLock<Mutex<HashMap<(u64, std::ffi::OsString, String, u32, u32), bool>>> =
        OnceLock::new();
    let cache = SCAN_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let was_cancelled = |flag: &Option<std::sync::Arc<std::sync::atomic::AtomicBool>>| {
        flag.as_ref()
            .is_some_and(|f| f.load(std::sync::atomic::Ordering::Acquire))
    };
    let key = (
        crate::toolpath::tools_generation(),
        ffmpeg.to_os_string(),
        encoder.to_string(),
        w,
        h,
    );
    if let Ok(map) = cache.lock() {
        if let Some(hit) = map.get(&key) {
            return *hit;
        }
    }
    let ok = probe_encoder_at_size_with_cancel(
        ffmpeg,
        encoder,
        w,
        h,
        HW_SIZE_PROBE_TIMEOUT,
        cancel.clone(),
    )
    .is_ok();
    if !was_cancelled(&cancel) {
        if let Ok(mut map) = cache.lock() {
            map.insert(key, ok);
        }
    }
    ok
}

/// Run one test encode and say why it failed. `Err(None)` means ffmpeg itself
/// did not run (missing binary, timeout), which says nothing about the encoder.
fn probe_encoder_at_size(
    ffmpeg: &OsStr,
    encoder: &str,
    w: u32,
    h: u32,
    timeout: Duration,
) -> Result<(), Option<EncoderFailure>> {
    probe_encoder_at_size_with_cancel(ffmpeg, encoder, w, h, timeout, None)
}

fn probe_encoder_at_size_with_cancel(
    ffmpeg: &OsStr,
    encoder: &str,
    w: u32,
    h: u32,
    timeout: Duration,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(), Option<EncoderFailure>> {
    let size_input = format!("testsrc=duration=0.1:size={w}x{h}:rate=30");
    // The test picture must have the pixel format a render sends. testsrc is
    // RGB, and without `-pix_fmt` ffmpeg hands the encoder a 4:4:4 RGB format
    // it lists; NVIDIA's AV1 engine refuses 4:4:4 ("No capable devices found")
    // although it encodes every 4:2:0 render, so the probe reported no AV1
    // encoder on a GPU that has one.
    let output = command_output_with_cancel(
        ffmpeg,
        &[
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            &size_input,
            "-c:v",
            encoder,
            "-pix_fmt",
            render_pix_fmt(encoder),
            "-f",
            "null",
            "-",
        ],
        timeout,
        cancel,
    )
    .ok_or(None)?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(Some(EncoderFailure {
        encoder: encoder.to_string(),
        kind: classify_encoder_failure(&stderr),
        detail: failure_detail(encoder, &stderr),
    }))
}

/// True when the ffmpeg a colour-managed render would use has the `zscale`
/// filter (libzimg): the one Cut needs to tone-map HDR sources to SDR. The
/// render routes a `zscale` graph to such a build when one exists
/// (`toolpath::ffmpeg_for`); this asks whether that build really has it.
/// One `ffmpeg -filters` per tools generation.
pub fn zscale_available() -> bool {
    static CACHE: OnceLock<Mutex<Option<(u64, bool)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let generation = crate::toolpath::tools_generation();
    if let Ok(guard) = cache.lock() {
        if let Some((g, ok)) = *guard {
            if g == generation {
                return ok;
            }
        }
    }
    let bin = crate::toolpath::ffmpeg_for(&[crate::toolpath::FfmpegFeature::Zscale]);
    let ok = feature_filters_at(&bin).2;
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((generation, ok));
    }
    ok
}

/// True when `encoder` (`libx265` or `libsvtav1`) exists in the resolved ffmpeg
/// AND actually encodes a 1-frame 10-bit (`yuv420p10le`, plus `-profile:v main10`
/// for libx265) null test without failing. Memoized per `(tools_generation, ffmpeg, encoder)`.
pub fn encoder_supports_10bit(encoder: &str) -> bool {
    let bin = crate::toolpath::ffmpeg_for(&[crate::toolpath::FfmpegFeature::Zscale]);
    encoder_supports_10bit_at(&bin, encoder)
}

/// Same 10-bit encoder capability probe against an explicit `ffmpeg` binary path.
pub fn encoder_supports_10bit_at(ffmpeg: &std::ffi::OsStr, encoder: &str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<(u64, std::ffi::OsString, String), bool>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let generation = crate::toolpath::tools_generation();
    let key = (generation, ffmpeg.to_os_string(), encoder.to_string());
    if let Ok(map) = cache.lock() {
        if let Some(hit) = map.get(&key) {
            return *hit;
        }
    }
    let ok = probe_10bit_encoder(ffmpeg, encoder);
    if let Ok(mut map) = cache.lock() {
        map.insert(key, ok);
    }
    ok
}

fn probe_10bit_encoder(ffmpeg: &std::ffi::OsStr, encoder: &str) -> bool {
    let mut args = vec![
        "-hide_banner",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=64x64:d=0.04",
        "-frames:v",
        "1",
        "-c:v",
        encoder,
        "-pix_fmt",
        "yuv420p10le",
    ];
    if encoder == "libx265" {
        args.extend(["-profile:v", "main10"]);
    }
    args.extend(["-f", "null", "-"]);
    command_status_with_timeout(ffmpeg, &args, Duration::from_secs(8))
}

/// The pixel format a render hands this encoder (see `hwencode_args::args_for`):
/// Intel's encoders take NV12, every other backend 8-bit 4:2:0 planar.
fn render_pix_fmt(encoder: &str) -> &'static str {
    if encoder.ends_with("_qsv") {
        "nv12"
    } else {
        "yuv420p"
    }
}

/// Memoised [`encoder_works_at_size`] for the resolved ffmpeg.
///
/// Generating a few 8K frames is not free, and a render can repeat, so each
/// (encoder, width, height) answer is computed once per process. Keyed on the
/// size too — the whole point is that one encoder can pass at 4K and fail at 8K.
pub fn encoder_supports_size(encoder: &str, w: u32, h: u32) -> bool {
    static SIZE_CACHE: OnceLock<Mutex<HashMap<(u64, String, u32, u32), bool>>> = OnceLock::new();
    let cache = SIZE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    // The tools generation is part of the key: another ffmpeg may differ.
    let key = (
        crate::toolpath::tools_generation(),
        encoder.to_string(),
        w,
        h,
    );
    if let Ok(map) = cache.lock() {
        if let Some(hit) = map.get(&key) {
            return *hit;
        }
    }
    let ok = encoder_works_at_size(&ffmpeg_bin(), encoder, w, h);
    if let Ok(mut map) = cache.lock() {
        map.insert(key, ok);
    }
    ok
}

/// Hardware encoder ffmpeg args for `(codec, quality)` using the detected HW
/// encoder, plus the output extension. `q` is the quality tier (0=draft, 1=
/// standard, 2=high). Returns None when no HW encoder exists for the codec (the
/// caller then uses the software tier). The rate-control knob differs per backend
/// (NVENC/QSV/AMF use a lower-is-better quantizer; VideoToolbox uses a 0-100
/// higher-is-better quality), so each backend maps the tier to its own scale.
pub fn hw_codec_args(
    codec: &str,
    q: usize,
    width: u32,
    height: u32,
) -> Option<(Vec<String>, &'static str)> {
    let enc = hw_caps().for_codec(codec)?.to_string();
    // The encoder EXISTS — but existing is not the same as being able to encode
    // THIS frame size, and getting that wrong fails the render rather than
    // degrading it. Decline the hardware tier when the size probe says no; the
    // caller then keeps its software args, which have no such ceiling.
    let args = crate::hwencode_args::args_for(codec, &enc, q, width, height, encoder_supports_size);
    if args.is_none() && width > 0 && height > 0 {
        tracing::info!(
            encoder = %enc,
            width,
            height,
            "hardware encoder cannot encode this frame size; using the software tier"
        );
    }
    args
}

// === GPU filter chain (the render fast-track prerequisite) ====================
//
// The HW *encoder* probe above proves NVENC can encode system-memory frames. The
// render fast-track goes further: it keeps frames in VRAM end-to-end
// (NVDEC -> scale_cuda/overlay_cuda -> NVENC, no PCIe round-trip per frame), which
// needs the CUDA *filters* to run too — a capability SEPARATE from the encoder
// (a box can have a working NVENC but a filter chain that fails to init). On a
// representative CUDA system, the fast-track was ~1.5x and freed the CPU
// on real, CPU-decode-bound 4K — but it loses on trivial sources and its output is
// not bit-reproducible, so the deterministic software graph stays the DEFAULT and
// the GPU path is opt-in and probe-gated.

static GPU_FILTERS: ToolCache<bool> = ToolCache::new();

/// True when the full CUDA render chain — `hwupload` -> `scale_cuda` ->
/// `overlay_cuda` -> `h264_nvenc` — verified by a runtime probe,
/// cached per tools generation).
///
/// WHY the whole chain, not "is the filter listed": scale_cuda/overlay_cuda need a
/// working CUDA device + driver, and the fast-track also needs NVENC to encode the
/// VRAM frames. A LISTED filter can still fail to init (no device / wrong driver),
/// so — exactly like the encoder probe — the only reliable signal is to RUN it. The
/// probe composites a tiny CUDA overlay and encodes one frame to `-f null`; a clean
/// exit means the GPU render path's filters are usable here.
/// `SHELLX_CUT_NO_HWENC=1` forces it off (the GPU path needs NVENC, so the no-HW
/// reproducibility switch disables the GPU filter path too).
pub fn gpu_filters_available() -> bool {
    GPU_FILTERS.get_or_init(|| {
        if std::env::var_os("SHELLX_CUT_NO_HWENC").is_some() {
            return false;
        }
        gpu_filter_chain_works()
    })
}

/// Run the CUDA filter chain end-to-end on a 0.1 s synthetic clip. Quiet; true on
/// a clean exit. (A synthetic source is fine HERE — we are probing CAPABILITY, not
/// measuring speed; the "synthetic sources lie" gotcha only applies to perf
/// benchmarks, never to a does-it-run probe.)
fn gpu_filter_chain_works() -> bool {
    gpu_filter_chain_works_at(&ffmpeg_bin())
}

/// Like [`gpu_filter_chain_works`] but against an EXPLICIT ffmpeg binary — used by
/// the doctor's candidate scan to learn whether the FULL fast-track (not just
/// nvenc encode) runs on each discovered ffmpeg.
fn gpu_filter_chain_works_at(ffmpeg: &OsStr) -> bool {
    gpu_filter_chain_works_at_cancelable(ffmpeg, None)
}

/// Probe whether `ffmpeg`'s CUDA filter chain (`hwupload` -> `scale_cuda` ->
/// `overlay_cuda` -> `h264_nvenc`) works, supporting cooperative cancellation.
pub fn gpu_filters_work_cancelable(
    ffmpeg: &Path,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> bool {
    gpu_filter_chain_works_at_cancelable(ffmpeg.as_os_str(), cancel)
}

fn gpu_filter_chain_works_at_cancelable(
    ffmpeg: &OsStr,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> bool {
    command_output_with_cancel(
        ffmpeg,
        &[
            "-hide_banner",
            "-v",
            "error",
            // Init a CUDA device for hwupload (no real hwaccel decode needed to
            // prove the filters; NVDEC is exercised with real footage separately).
            "-init_hw_device",
            "cuda=cu:0",
            "-filter_hw_device",
            "cu",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=0.1",
            "-f",
            "lavfi",
            "-i",
            "color=red:size=96x96:rate=30:duration=0.1",
            // Upload both to VRAM, conform the base with scale_cuda, composite the
            // overlay with overlay_cuda — the exact shape the GPU graph will use.
            "-filter_complex",
            "[0:v]format=yuv420p,hwupload,scale_cuda=320:240[base];\
             [1:v]format=yuv420p,hwupload[ov];[base][ov]overlay_cuda=10:10[o]",
            "-map",
            "[o]",
            "-c:v",
            "h264_nvenc",
            "-f",
            "null",
            "-",
        ],
        CUDA_PROBE_TIMEOUT,
        cancel,
    )
    .is_some_and(|output| output.status.success())
}

fn command_status_with_timeout(prog: &OsStr, args: &[&str], timeout: Duration) -> bool {
    command_output_with_timeout(prog, args, timeout).is_some_and(|output| output.status.success())
}

/// Hardware capability probes are foreground work, but still own their whole
/// process tree: a hung driver helper must not outlive the timed probe.
fn command_output_with_timeout(prog: &OsStr, args: &[&str], timeout: Duration) -> Option<Output> {
    command_output_with_cancel(prog, args, timeout, None)
}

fn command_output_with_cancel(
    prog: &OsStr,
    args: &[&str],
    timeout: Duration,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Option<Output> {
    let mut command = Command::new(prog);
    command.args(args);
    let control = OwnedProcessControl::bounded(timeout, move || {
        cancel
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
    });
    run_owned_command(&mut command, &control, "hardware encoder probe").ok()
}

// ── VRAM sizing for the GPU render fast-track ───────────────────────────────────
//
// GPU frames live in VRAM, which — unlike system RAM — has NO cgroup backstop (the
// render_command cgroup governs system RAM only). NVDEC/NVENC fail HARD on a VRAM
// OOM ("OpenEncodeSessionEx failed" / "cudaErrorMemoryAllocation"), so the render
// gate must size the GPU graph against the device's VRAM and fall back to software
// when an estimate exceeds budget. This is the VRAM analogue of the total-system-RAM
// the segmentation budget reads; the GPU graph itself is single-pass (not windowed),
// so the bound is a per-render peak estimate vs this device size.

static CUDA_VRAM: OnceLock<Option<u64>> = OnceLock::new();

/// Read the selected CUDA card's reported generation using the same bounded
/// NVIDIA driver helper as VRAM detection. Missing/old drivers and ambiguous
/// visibility selections are unknown, never grounds for refusing an install.
pub fn cuda_compute_capability() -> Option<(u32, u32)> {
    let visible = match std::env::var("CUDA_VISIBLE_DEVICES") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return None,
    };
    let device = match visible.as_deref().and_then(|s| s.split(',').next()) {
        None | Some("0") => "0",
        Some(s) if s.starts_with("GPU-") || s.starts_with("MIG-") => s,
        _ => return None,
    };
    // Mixed-card ordering can differ from CUDA. Multiple reports remain
    // unknown rather than refusing a different, usable card.
    let mut args = vec!["--query-gpu=compute_cap", "--format=csv,noheader,nounits"];
    if device != "0" {
        args.extend(["--id", device]);
    }
    let out = command_output_with_timeout(OsStr::new("nvidia-smi"), &args, HW_PROBE_TIMEOUT)?;
    if !out.status.success() {
        return None;
    }
    parse_compute_capability(&String::from_utf8_lossy(&out.stdout))
}

fn parse_compute_capability(reported: &str) -> Option<(u32, u32)> {
    let (major, minor) = reported.trim().split_once('.')?;
    if !major.chars().all(|c| c.is_ascii_digit()) || !minor.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Total VRAM (bytes) of the primary CUDA device, read once from `nvidia-smi
/// --query-gpu=memory.total` (MiB). `None` when nvidia-smi is absent or the query
/// fails — the render gate then uses a conservative default budget rather than
/// guessing the device size. Cached for the process lifetime (VRAM does not change).
///
/// First CSV line = the primary device (index 0), which is the one ffmpeg's
/// `cuda=cu:0` device binds — matching [`gpu_filter_chain_works_at`].
pub fn cuda_total_vram_bytes() -> Option<u64> {
    *CUDA_VRAM.get_or_init(|| {
        let out = command_output_with_timeout(
            OsStr::new("nvidia-smi"),
            &["--query-gpu=memory.total", "--format=csv,noheader,nounits"],
            HW_PROBE_TIMEOUT,
        )?;
        if !out.status.success() {
            return None;
        }
        let mib: u64 = String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()?
            .trim()
            .parse()
            .ok()?;
        (mib > 0).then(|| mib * 1024 * 1024)
    })
}

// ── Per-binary capability probe (the doctor's "find any installed ffmpeg") ──────

/// The capabilities of ONE specific ffmpeg binary, learned by running it. The
/// doctor builds one per discovered candidate to report the most capable ffmpeg
/// and to suggest a download when none is hardware-accelerated.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FfmpegCaps {
    /// The probed binary's path (display form).
    pub path: String,
    /// Version token from `ffmpeg -version` (e.g. "6.1.1" / "N-125095-…"); None
    /// when the binary did not run — the honest "is it usable" bit.
    pub version: Option<String>,
    /// Working HW encoders (nvenc/qsv/amf/videotoolbox) on THIS binary.
    pub hw: HwCaps,
    /// True when the FULL CUDA fast-track (hwupload → scale_cuda → overlay_cuda →
    /// h264_nvenc) actually runs on this binary — not just nvenc encode.
    pub cuda_filters: bool,
    /// True when this binary has the libass-backed `subtitles`/`ass` filter, i.e.
    /// it can BURN CAPTIONS. Homebrew dropped libass from plain ffmpeg 8.x, so a
    /// HW-capable-but-libass-less build would silently lose caption burn-in — the
    /// feature-aware selector (toolpath::ffmpeg_for) uses this to pick a caption-
    /// capable build for caption renders even when it is not the fastest.
    #[serde(default)]
    pub libass: bool,
    /// True when this binary has the `vidstabtransform`/`vidstabdetect` filters
    /// (edit.stabilize). Same selection concern as libass.
    #[serde(default)]
    pub vidstab: bool,
    /// True when this binary has the `zscale` filter (libzimg), i.e. it can run a
    /// COLOR-MANAGED render. A project working/output space (or a tagged clip input)
    /// other than rec709 emits a `zscale` colorspace hop; a build without libzimg
    /// fails that render exit-8 with "No such filter: 'zscale'". Homebrew's plain
    /// ffmpeg 8.x dropped libzimg, so a HW-capable build can lack it — the feature-
    /// aware selector (toolpath::ffmpeg_for) uses this to pick a zscale-capable build
    /// for a color-managed render even when it is not the fastest.
    #[serde(default)]
    pub zscale: bool,
    /// Why this binary has no working hardware encoder (empty when it has one,
    /// or when it did not run).
    #[serde(default)]
    pub diagnosis: HwDiagnosis,
}

impl FfmpegCaps {
    /// Short backend label from any detected encoder (nvenc|qsv|amf|videotoolbox).
    pub fn backend(&self) -> Option<String> {
        self.hw
            .h264
            .as_deref()
            .or(self.hw.hevc.as_deref())
            .or(self.hw.av1.as_deref())
            .and_then(|e| e.rsplit('_').next())
            .map(|b| b.to_string())
    }
    /// Acceleration rank used to pick the BEST candidate (higher = better):
    /// 3 = full GPU fast-track (cuda filters + nvenc), 2 = HW encode only,
    /// 1 = runnable software, 0 = not runnable. Lets the doctor rank by
    /// `max_by_key(FfmpegCaps::rank)`.
    pub fn rank(&self) -> u8 {
        if self.cuda_filters {
            3
        } else if self.hw.any() {
            2
        } else if self.version.is_some() {
            1
        } else {
            0
        }
    }
}

/// The version token from `ffmpeg -version` (3rd whitespace field of line 1), or
/// None when the binary cannot be spawned / exits non-zero. Quiet. Cached per
/// selected program and tools generation, just like the per-build filter probe.
pub fn ffmpeg_version_at(ffmpeg: &OsStr) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    type Versions = HashMap<std::ffi::OsString, Option<String>>;
    static SEEN: Mutex<Option<(u64, Versions)>> = Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|p| p.into_inner());
    let generation = crate::toolpath::tools_generation();
    if seen.as_ref().is_none_or(|(g, _)| *g != generation) {
        *seen = Some((generation, HashMap::new()));
    }
    let (_, versions) = seen.as_mut()?;
    versions
        .entry(ffmpeg.to_os_string())
        .or_insert_with(|| read_ffmpeg_version_at(ffmpeg))
        .clone()
}

/// Version outcome from `<ffmpeg> -version` so Doctor's essential card and
/// capability scan reuse one spawn instead of executing `ffmpeg -version` 2-3x.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FfmpegVersionOutcome {
    Ran {
        banner_line: String,
        version: Option<String>,
    },
    NotFound,
    Timeout,
}

/// Complete single-binary probe output shared across Doctor's `ffmpeg` and
/// `gpu-encode` cards (and `ffmpeg_selection`), avoiding duplicate subprocesses.
#[derive(Debug, Clone)]
pub struct FfmpegScanReport {
    pub caps: FfmpegCaps,
    pub version_outcome: FfmpegVersionOutcome,
    /// `(libass, vidstabtransform || vidstabdetect, zscale)` for `ffmpeg` card.
    pub filter_caps: Option<(bool, bool, bool)>,
    /// `(libx265, libvpx-vp9, libsvtav1)` from `ffmpeg -encoders`.
    pub encoder_caps: Option<(bool, bool, bool)>,
    /// `(libx265_10bit, libsvtav1_10bit)`.
    pub enc_10bit: Option<(bool, bool)>,
}

fn read_ffmpeg_version_at(ffmpeg: &OsStr) -> Option<String> {
    match read_ffmpeg_version_outcome(ffmpeg, HW_PROBE_TIMEOUT) {
        FfmpegVersionOutcome::Ran { version, .. } => version,
        _ => None,
    }
}

fn looks_like_missing_error(error: &cut_core::CutError) -> bool {
    if error.code == cut_core::error_codes::TOOL_MISSING {
        return true;
    }
    let cause = error.cause.to_ascii_lowercase();
    cause.contains("no such file") || cause.contains("not found") || cause.contains("os error 2")
}

fn read_ffmpeg_version_outcome(ffmpeg: &OsStr, timeout: Duration) -> FfmpegVersionOutcome {
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-version"]);
    let control = OwnedProcessControl::bounded(timeout, || false);
    match run_owned_command(&mut command, &control, "ffmpeg version probe") {
        Ok(out) => {
            let text = if !out.stdout.is_empty() {
                String::from_utf8_lossy(&out.stdout).into_owned()
            } else {
                String::from_utf8_lossy(&out.stderr).into_owned()
            };
            let banner_line = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string();
            let version = out
                .status
                .success()
                .then(|| {
                    String::from_utf8_lossy(&out.stdout)
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(2))
                        .map(|v| v.to_string())
                })
                .flatten();
            FfmpegVersionOutcome::Ran {
                banner_line,
                version,
            }
        }
        Err(ref err) if looks_like_missing_error(err) => FfmpegVersionOutcome::NotFound,
        Err(_) => FfmpegVersionOutcome::Timeout,
    }
}

/// Which feature-filters this ffmpeg has, learned from `ffmpeg -filters`. A filter
/// is listed by ffmpeg IFF its backing library was compiled in, so this is the
/// ground truth for "can this binary burn captions / stabilize / color-manage" —
/// the exact thing the auto-selector must respect (a HW build that dropped libass
/// would silently lose caption burn-in; one that dropped libzimg would fail a
/// color-managed render). Returns `(libass, vidstab, zscale)`; `(false,false,false)`
/// if the binary does not run. One cheap spawn (no encode).
pub(crate) fn feature_filters_at(ffmpeg: &OsStr) -> (bool, bool, bool) {
    let raw = read_filters_raw(ffmpeg);
    raw.map(|r| (r.libass, r.vidstab_transform, r.zscale))
        .unwrap_or((false, false, false))
}

struct RawFilters {
    libass: bool,
    vidstab_transform: bool,
    vidstab_any: bool,
    zscale: bool,
    has_filters_header: bool,
    has_cuda_filters: bool,
}

fn read_filters_raw(ffmpeg: &OsStr) -> Option<RawFilters> {
    let o = command_output_with_timeout(ffmpeg, &["-hide_banner", "-filters"], HW_PROBE_TIMEOUT)?;
    if !o.status.success() {
        return None;
    }
    let t = String::from_utf8_lossy(&o.stdout);
    let libass = filter_listing_has(&t, "subtitles") || filter_listing_has(&t, "ass");
    let vidstab_transform = filter_listing_has(&t, "vidstabtransform");
    let vidstab_any = vidstab_transform || filter_listing_has(&t, "vidstabdetect");
    let zscale = filter_listing_has(&t, "zscale");
    let has_filters_header = t.contains("Filters:");
    let has_cuda_filters = filter_listing_has(&t, "hwupload")
        && filter_listing_has(&t, "scale_cuda")
        && filter_listing_has(&t, "overlay_cuda");
    Some(RawFilters {
        libass,
        vidstab_transform,
        vidstab_any,
        zscale,
        has_filters_header,
        has_cuda_filters,
    })
}

struct RawEncoders {
    x265: bool,
    vp9: bool,
    svtav1: bool,
    compiled_set: Option<std::collections::HashSet<String>>,
}

fn read_encoders_raw(ffmpeg: &OsStr) -> Option<RawEncoders> {
    let out =
        command_output_with_timeout(ffmpeg, &["-hide_banner", "-encoders"], HW_PROBE_TIMEOUT)?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut names = std::collections::HashSet::new();
    for line in text.lines() {
        let mut cols = line.split_whitespace();
        let Some(flags) = cols.next() else {
            continue;
        };
        let Some(name) = cols.next() else {
            continue;
        };
        if flags.len() >= 2 && flags.chars().all(|c| c == '.' || c.is_ascii_alphabetic()) {
            names.insert(name.to_string());
        }
    }
    let x265 = names.contains("libx265");
    let vp9 = names.contains("libvpx-vp9");
    let svtav1 = names.contains("libsvtav1");
    let compiled_set =
        (out.status.success() && text.contains("Encoders:") && !names.is_empty()).then_some(names);
    Some(RawEncoders {
        x265,
        vp9,
        svtav1,
        compiled_set,
    })
}

/// Whether an `ffmpeg -filters` listing names `target` in its filter column.
/// The one parser for that listing: the doctor's capability card and the
/// per-candidate scan both use it, so they cannot disagree.
pub fn filter_listing_has(listing: &str, target: &str) -> bool {
    listing.lines().any(|line| {
        let mut cols = line.split_whitespace();
        let Some(flags) = cols.next() else {
            return false;
        };
        let Some(name) = cols.next() else {
            return false;
        };
        // FFmpeg up to 8.x prints three flag characters ("T.."); 9.x prints
        // two (".."). Accept both, or every 9.x build reads as lacking libass,
        // vid.stab and zimg and the per-feature build selection never fires.
        flags.len() >= 2
            && flags
                .chars()
                .all(|c| c == '.' || c == '|' || c.is_ascii_alphabetic())
            && name == target
    })
}

/// Probe ONE ffmpeg binary's capabilities (version + HW encoders + CUDA filters +
/// libass/vidstab feature filters) by running it. The expensive part (test encodes)
/// is skipped when the binary is not runnable or when `SHELLX_CUT_NO_HWENC=1` forces
/// the software tier — so it stays honest with [`hw_caps`]. Unlike `hw_caps`/
/// `gpu_filters_available` this is NOT cached: the doctor scan controls when it runs.
pub fn probe_ffmpeg_caps(ffmpeg: &Path) -> FfmpegCaps {
    probe_ffmpeg_internal(ffmpeg, false, true).caps
}

/// Complete per-binary probe for the Doctor scan: collects version outcome,
/// filter caps, software encoder capabilities, 10-bit checks, HW encoders, and
/// CUDA filter chain in two parallel waves instead of repeating commands per card.
pub fn probe_ffmpeg_detailed(ffmpeg: &Path, probe_cuda: bool) -> FfmpegScanReport {
    probe_ffmpeg_internal(ffmpeg, true, probe_cuda)
}

fn probe_ffmpeg_internal(ffmpeg: &Path, include_10bit: bool, probe_cuda: bool) -> FfmpegScanReport {
    let osff = ffmpeg.as_os_str();
    let version_outcome = read_ffmpeg_version_outcome(osff, Duration::from_secs(8));
    let runnable = matches!(version_outcome, FfmpegVersionOutcome::Ran { .. });
    let version = match &version_outcome {
        FfmpegVersionOutcome::Ran { version, .. } => version.clone(),
        _ => None,
    };
    if !runnable {
        return FfmpegScanReport {
            caps: FfmpegCaps {
                path: ffmpeg.display().to_string(),
                version,
                hw: HwCaps::default(),
                cuda_filters: false,
                libass: false,
                vidstab: false,
                zscale: false,
                diagnosis: HwDiagnosis::default(),
            },
            version_outcome,
            filter_caps: None,
            encoder_caps: None,
            enc_10bit: None,
        };
    }

    let (raw_filters, raw_encoders) = std::thread::scope(|scope| {
        let filters = scope.spawn(|| read_filters_raw(osff));
        let encoders = scope.spawn(|| read_encoders_raw(osff));
        (
            filters.join().unwrap_or(None),
            encoders.join().unwrap_or(None),
        )
    });

    let no_hw = std::env::var_os("SHELLX_CUT_NO_HWENC").is_some();
    let (libass, vidstab, zscale) = raw_filters
        .as_ref()
        .map(|f| (f.libass, f.vidstab_transform, f.zscale))
        .unwrap_or((false, false, false));
    let filter_caps = raw_filters
        .as_ref()
        .map(|f| (f.libass, f.vidstab_any, f.zscale));
    let encoder_caps = raw_encoders.as_ref().map(|e| (e.x265, e.vp9, e.svtav1));
    let compiled_set = raw_encoders.as_ref().and_then(|e| e.compiled_set.as_ref());

    let can_probe_hw = version.is_some();
    let need_cuda_probe = probe_cuda
        && can_probe_hw
        && !no_hw
        && compiled_set.is_none_or(|s| s.contains("h264_nvenc"))
        && raw_filters
            .as_ref()
            .is_none_or(|f| !f.has_filters_header || f.has_cuda_filters);

    let (x265_10, svtav1_10, (hw, diagnosis), cuda_filters) = std::thread::scope(|scope| {
        let j_x265 = (include_10bit && raw_encoders.as_ref().is_some_and(|e| e.x265))
            .then(|| scope.spawn(|| encoder_supports_10bit_at(osff, "libx265")));
        let j_svt = (include_10bit && raw_encoders.as_ref().is_some_and(|e| e.svtav1))
            .then(|| scope.spawn(|| encoder_supports_10bit_at(osff, "libsvtav1")));
        let j_hw = (can_probe_hw && !no_hw)
            .then(|| scope.spawn(|| probe_at_with_encoders(osff, compiled_set)));
        let j_cuda = need_cuda_probe.then(|| scope.spawn(|| gpu_filter_chain_works_at(osff)));

        let x265_10 = j_x265.map(|h| h.join().unwrap_or(false)).unwrap_or(false);
        let svt_10 = j_svt.map(|h| h.join().unwrap_or(false)).unwrap_or(false);
        let hw_res = if !can_probe_hw {
            (HwCaps::default(), HwDiagnosis::default())
        } else if no_hw {
            (
                HwCaps::default(),
                HwDiagnosis {
                    disabled_by_setting: true,
                    failures: Vec::new(),
                },
            )
        } else {
            j_hw.map(|h| h.join().unwrap_or_default())
                .unwrap_or_default()
        };
        let cuda = j_cuda.map(|h| h.join().unwrap_or(false)).unwrap_or(false);
        (x265_10, svt_10, hw_res, cuda)
    });

    let enc_10bit = encoder_caps.map(|_| (x265_10, svtav1_10));
    FfmpegScanReport {
        caps: FfmpegCaps {
            path: ffmpeg.display().to_string(),
            version,
            hw,
            cuda_filters,
            libass,
            vidstab,
            zscale,
            diagnosis,
        },
        version_outcome,
        filter_caps,
        encoder_caps,
        enc_10bit,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reported_cuda_generation_fixture_is_not_a_card_name_table() {
        assert_eq!(super::parse_compute_capability("7.0\n"), Some((7, 0)));
        assert_eq!(super::parse_compute_capability("12.0\n"), Some((12, 0)));
        for unknown in ["", "N/A", "Tesla V100", "7", "7.0\n12.0", "seven.zero"] {
            assert_eq!(super::parse_compute_capability(unknown), None);
        }
    }
    use super::*;

    /// FfmpegCaps::rank ladders correctly (not-runnable < software < hw < fast-track)
    /// and backend() extracts the vendor — the keys the doctor ranks candidates by.
    /// Also: probing a non-existent binary is a clean "not runnable", never a panic.
    #[test]
    fn ffmpeg_caps_rank_and_backend() {
        // A path that cannot run → version None, rank 0, no hardware (portable:
        // holds on every machine incl. CI, no ffmpeg required).
        let dead = probe_ffmpeg_caps(Path::new("/nonexistent/shellx-no-such-ffmpeg"));
        assert_eq!(dead.version, None);
        assert_eq!(dead.rank(), 0);
        assert!(!dead.hw.any());
        assert_eq!(dead.backend(), None);

        let mk = |hw: HwCaps, cuda: bool, runnable: bool| FfmpegCaps {
            path: "x".into(),
            version: runnable.then(|| "6.1".to_string()),
            hw,
            cuda_filters: cuda,
            libass: false,
            vidstab: false,
            zscale: false,
            diagnosis: HwDiagnosis::default(),
        };
        assert_eq!(
            mk(HwCaps::default(), false, false).rank(),
            0,
            "not runnable"
        );
        assert_eq!(
            mk(HwCaps::default(), false, true).rank(),
            1,
            "runnable software"
        );
        let nvenc = HwCaps {
            h264: Some("h264_nvenc".into()),
            ..Default::default()
        };
        let hw = mk(nvenc.clone(), false, true);
        assert_eq!(hw.rank(), 2, "hw encode");
        assert_eq!(hw.backend().as_deref(), Some("nvenc"));
        assert_eq!(mk(nvenc, true, true).rank(), 3, "full fast-track wins");
    }

    /// The probe never panics + caps are self-consistent (codec lookups match the
    /// stored encoder). On a software-only box every slot is None.
    #[test]
    fn caps_are_consistent() {
        let caps = hw_caps();
        for codec in ["h264", "hevc", "av1"] {
            if let Some(enc) = caps.for_codec(codec) {
                assert!(enc.contains(codec), "encoder {enc} should be for {codec}");
            }
        }
        // `any()` agrees with the slots.
        assert_eq!(
            caps.any(),
            caps.h264.is_some() || caps.hevc.is_some() || caps.av1.is_some()
        );
    }

    /// The GPU-filter probe never panics and is cached/stable within a process
    /// (the result is hardware-dependent — true only on a box whose CUDA filter
    /// chain + NVENC actually run). This guards the probe wiring, not the GPU.
    #[test]
    fn gpu_filters_probe_is_stable() {
        let a = gpu_filters_available();
        let b = gpu_filters_available();
        assert_eq!(
            a, b,
            "probe must be cached + deterministic within a process"
        );
    }

    /// hw_codec_args is None for an unknown codec, and (when a HW encoder exists)
    /// carries the right rate-control flag for its backend.
    #[test]
    fn hw_args_shape() {
        // 0x0 means "size unknown, skip the size gate" — this test is about arg
        // SHAPE, and a real size probe here would make it depend on the host GPU.
        assert!(hw_codec_args("definitely_not_a_codec", 1, 0, 0).is_none());
        if let Some((args, ext)) = hw_codec_args("hevc", 2, 0, 0) {
            assert_eq!(ext, "mp4");
            let enc = args[1].clone();
            assert!(enc.contains("hevc"));
            // hevc in mp4 gets the hvc1 tag regardless of backend.
            assert!(args.windows(2).any(|w| w[0] == "-tag:v" && w[1] == "hvc1"));
        }
    }

    #[test]
    fn command_status_with_timeout_kills_wedged_probe() {
        use std::io::Write;
        use std::time::{Duration, Instant};

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join(if cfg!(windows) {
            "sleepy-probe.cmd"
        } else {
            "sleepy-probe.sh"
        });
        if cfg!(windows) {
            std::fs::write(
                &script,
                "@echo off\r\nping 127.0.0.1 -n 3 > nul\r\nexit /b 0\r\n",
            )
            .unwrap();
        } else {
            let mut f = std::fs::File::create(&script).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            writeln!(f, "sleep 2").unwrap();
            writeln!(f, "exit 0").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&script).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&script, perms).unwrap();
            }
        }

        let start = Instant::now();
        let ok = command_status_with_timeout(
            script.as_os_str(),
            &["ignored"],
            Duration::from_millis(150),
        );
        assert!(!ok, "timed-out probe must not report success");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "timeout should kill the wedged probe promptly"
        );
    }

    /// The libass/vidstab/zscale feature probe AGREES with `ffmpeg -filters` ground
    /// truth on the resolved ffmpeg — so a color-managed render is
    /// routed to a zscale (libzimg) build exactly when the binary really has it.
    /// Portable: if no ffmpeg is runnable the probe reports all-false and we skip;
    /// when the resolved binary has zscale it asserts caps.zscale==true.
    #[test]
    fn feature_probe_matches_ffmpeg_filters_ground_truth() {
        let ff = crate::toolpath::ffmpeg();
        // Ground truth straight from `ffmpeg -filters`.
        let out = Command::new(&ff)
            .args(["-hide_banner", "-filters"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output();
        let Ok(o) = out else { return }; // no ffmpeg here — nothing to assert
        if !o.status.success() {
            return;
        }
        let listing = String::from_utf8_lossy(&o.stdout);
        let truth_zscale = filter_listing_has(&listing, "zscale");
        let truth_subtitles = filter_listing_has(&listing, "subtitles");

        let caps = probe_ffmpeg_caps(Path::new(&ff));
        assert!(
            caps.version.is_some(),
            "resolved ffmpeg ran for -filters but not -version"
        );
        assert_eq!(
            caps.zscale, truth_zscale,
            "probe zscale bit must match `ffmpeg -filters` ground truth"
        );
        // libass is detected via `subtitles`/` ass `; subtitles is the reliable signal.
        if truth_subtitles {
            assert!(
                caps.libass,
                "subtitles filter present ⇒ libass must be detected"
            );
        }
    }

    #[test]
    fn feature_filter_parser_matches_exact_filter_names_only() {
        let listing = "\
Filters:\n\
 ... ass               V->V       Render ASS subtitles.\n\
 ... notzscale         V->V       Deliberately similar name.\n\
 ... vidstabtransform2 V->V       Deliberately similar name.\n";

        assert!(filter_listing_has(listing, "ass"));
        assert!(!filter_listing_has(listing, "zscale"));
        assert!(!filter_listing_has(listing, "vidstabtransform"));
    }

    /// ffmpeg's real messages sort into the four causes. The first is the exact
    /// text from a machine whose ffmpeg has NVENC but whose driver is too old.
    #[test]
    fn failure_text_is_sorted_into_causes() {
        let old_driver = "[h264_nvenc @ 000001] Driver does not support the required nvenc API \
             version. Required: 13.1 Found: 13.0\n[h264_nvenc @ 000001] The minimum required \
             Nvidia driver for nvenc is 610.00 or newer\n";
        assert_eq!(
            classify_encoder_failure(old_driver),
            EncoderFailureKind::DriverTooOld
        );
        assert_eq!(
            classify_encoder_failure("Unknown encoder 'h264_nvenc'\n"),
            EncoderFailureKind::NotInBuild
        );
        assert_eq!(
            classify_encoder_failure("[h264_nvenc @ 1] Cannot load libcuda.so.1\n"),
            EncoderFailureKind::NoDevice
        );
        assert_eq!(
            classify_encoder_failure("[h264_nvenc @ 1] No capable devices found\n"),
            EncoderFailureKind::NoDevice
        );
        assert_eq!(
            classify_encoder_failure("something nobody has seen before\n"),
            EncoderFailureKind::CannotOpen
        );
    }

    /// The advice follows the cause: a driver that is too old is never answered
    /// with "install another ffmpeg", and a build without encoders is.
    #[test]
    fn shortfall_picks_the_cause_the_person_can_fix() {
        let failure = |encoder: &str, kind| EncoderFailure {
            encoder: encoder.into(),
            kind,
            detail: Some(format!("{encoder} said no")),
        };
        let driver = HwDiagnosis {
            disabled_by_setting: false,
            failures: vec![
                failure("h264_nvenc", EncoderFailureKind::DriverTooOld),
                failure("h264_qsv", EncoderFailureKind::NoDevice),
                failure("h264_amf", EncoderFailureKind::NotInBuild),
            ],
        };
        assert_eq!(driver.shortfall(None), HwShortfall::DriverTooOld);
        assert!(HwShortfall::DriverTooOld
            .action()
            .contains("Update the graphics driver"));
        assert!(!HwShortfall::DriverTooOld.action().contains("ffmpeg build"));
        assert_eq!(
            driver.technical_detail(Some("h264")).as_deref(),
            Some("h264_nvenc: h264_nvenc said no")
        );
        // hevc was never tried on this machine in this fixture: no cause recorded.
        assert_eq!(driver.shortfall(Some("hevc")), HwShortfall::Unknown);

        let none_in_build = HwDiagnosis {
            disabled_by_setting: false,
            failures: vec![
                failure("h264_nvenc", EncoderFailureKind::NotInBuild),
                failure("h264_qsv", EncoderFailureKind::NotInBuild),
            ],
        };
        assert_eq!(none_in_build.shortfall(None), HwShortfall::NoEncoderInBuild);
        assert!(HwShortfall::NoEncoderInBuild
            .action()
            .contains("ffmpeg build that includes GPU encoders"));

        let no_gpu = HwDiagnosis {
            disabled_by_setting: false,
            failures: vec![failure("h264_nvenc", EncoderFailureKind::NoDevice)],
        };
        assert_eq!(no_gpu.shortfall(None), HwShortfall::NoGpu);

        let off = HwDiagnosis {
            disabled_by_setting: true,
            failures: vec![],
        };
        assert_eq!(off.shortfall(None), HwShortfall::SwitchedOff);
    }

    /// Writes an executable stand-in for ffmpeg that fails every encode with the
    /// given stderr, so the probe wiring is checked without a GPU.
    #[cfg(unix)]
    fn failing_ffmpeg(dir: &Path, stderr: &str) -> std::path::PathBuf {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let script = dir.join("fake-ffmpeg.sh");
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "cat >&2 <<'EOF'\n{stderr}\nEOF").unwrap();
        writeln!(f, "exit 1").unwrap();
        drop(f);
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        script
    }

    /// End to end through the real probe loop: an ffmpeg that lists the encoder
    /// but reports an old driver yields "no hardware", the driver cause and
    /// ffmpeg's own line in the technical detail.
    #[cfg(unix)]
    #[test]
    fn probe_reports_old_driver_with_ffmpeg_message() {
        let dir = tempfile::tempdir().unwrap();
        // ffmpeg's real tail: the encoder's own lines first, then generic ones.
        let ff = failing_ffmpeg(
            dir.path(),
            "[h264_nvenc @ 0x1] Driver does not support the required nvenc API version. \
             Required: 13.1 Found: 13.0\n[h264_nvenc @ 0x1] The minimum required Nvidia driver \
             for nvenc is 610.00 or newer\n[vost#0:0/h264_nvenc @ 0x2] Error while opening \
             encoder - maybe incorrect parameters such as bit rate, frame rate, width or \
             height.\n[vf#0:0 @ 0x3] Error sending frames to consumers: Function not \
             implemented\nConversion failed!",
        );
        let (caps, diagnosis) = probe_at(ff.as_os_str());
        assert!(!caps.any());
        assert_eq!(diagnosis.shortfall(Some("h264")), HwShortfall::DriverTooOld);
        let detail = diagnosis.technical_detail(Some("h264")).unwrap();
        assert!(detail.contains("Required: 13.1 Found: 13.0"), "{detail}");
        assert!(detail.contains("610.00"), "{detail}");
        assert!(
            !detail.contains("Conversion failed"),
            "the generic closing lines must not push out the cause: {detail}"
        );
    }

    /// Item 97: an RTX 5080 encodes AV1 from 4:2:0 but refuses the 4:4:4 RGB
    /// that ffmpeg picks for a bare testsrc, so the probe must feed the pixel
    /// format a render sends. The stand-in behaves like that GPU.
    #[cfg(unix)]
    #[test]
    fn av1_probe_feeds_the_render_pixel_format() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("rgb-refusing-ffmpeg.sh");
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(
            f,
            "case \" $* \" in *\" -pix_fmt yuv420p \"*) exit 0;; esac"
        )
        .unwrap();
        writeln!(f, "echo '[av1_nvenc @ 0x1] No capable devices found' >&2").unwrap();
        writeln!(f, "exit 1").unwrap();
        drop(f);
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();
        let (caps, _) = probe_at(script.as_os_str());
        assert_eq!(caps.av1.as_deref(), Some("av1_nvenc"));
    }

    /// Item 96: 8K H.264 on an NVIDIA GPU says the encoder's limit and points
    /// at HEVC, which the same GPU encodes at 8K.
    #[test]
    fn size_fallback_names_the_limit_and_hevc() {
        let m = size_fallback_message(
            "h264_nvenc",
            "accepts at most 4096x2160",
            7680,
            4320,
            Some(("HEVC", "hevc_nvenc")),
        );
        assert!(m.contains("(h264_nvenc) accepts at most 4096x2160"), "{m}");
        assert!(m.contains("7680x4320"), "{m}");
        assert!(
            m.contains("Choose HEVC to encode this size in hardware (hevc_nvenc)"),
            "{m}"
        );
        let none = size_fallback_message(
            "hevc_videotoolbox",
            "cannot encode 8192x8192",
            8192,
            8192,
            None,
        );
        assert!(!none.contains("Choose"), "{none}");
    }

    /// An ffmpeg without the encoders is the one case where "install an ffmpeg
    /// with GPU encoders" is the right advice.
    #[cfg(unix)]
    #[test]
    fn probe_reports_missing_encoders_in_the_build() {
        let dir = tempfile::tempdir().unwrap();
        let ff = failing_ffmpeg(dir.path(), "Unknown encoder 'h264_nvenc'");
        let (_, diagnosis) = probe_at(ff.as_os_str());
        assert_eq!(
            diagnosis.shortfall(Some("h264")),
            HwShortfall::NoEncoderInBuild
        );
    }

    /// FFmpeg 9.x shortened the flags column to two characters.
    #[test]
    fn feature_filter_parser_reads_the_two_flag_listing_of_ffmpeg_9() {
        let listing = "\
Filters:\n\
  T.. = Timeline support\n\
  A = Audio input/output\n\
  ------\n\
 .. ass               V->V       Render ASS subtitles onto input video using the libass library.\n\
 .. vidstabtransform  V->V       Transform the frames, pass 2 of 2 for stabilization.\n\
 .S zscale            V->V       Apply resizing, colorspace and bit depth conversion.\n";

        assert!(filter_listing_has(listing, "ass"));
        assert!(filter_listing_has(listing, "vidstabtransform"));
        assert!(filter_listing_has(listing, "zscale"));
        assert!(!filter_listing_has(listing, "Audio"));
    }

    /// A stand-in ffmpeg that answers like FFmpeg 9: `-version`, and a
    /// `-filters` listing with the two-character flags column. Everything else
    /// (the hardware test encodes) fails, as on a machine without encoders.
    #[cfg(unix)]
    const FAKE_FFMPEG_9: &str = "#!/bin/sh
for a in \"$@\"; do
  case \"$a\" in
    -version) echo 'ffmpeg version 9.0.2 Copyright (c) 2000-2026 the FFmpeg developers'; exit 0;;
    -filters) printf 'Filters:\\n  T.. = Timeline support\\n  ------\\n .. ass               V->V       Render ASS subtitles.\\n .. subtitles         V->V       Render text subtitles.\\n .. vidstabdetect     V->V       Extract transformations.\\n .. vidstabtransform  V->V       Transform the frames.\\n .S zscale            V->V       Apply resizing.\\n'; exit 0;;
  esac
done
exit 1
";

    /// The per-candidate scan (doctor gpu card, automatic and per-feature
    /// selection) reads the FFmpeg 9 listing: an earlier parser read every 9.x
    /// build as lacking libass, vid.stab and zimg.
    #[cfg(unix)]
    #[test]
    fn candidate_scan_reads_the_ffmpeg_9_filter_listing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("ffmpeg");
        std::fs::write(&fake, FAKE_FFMPEG_9).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A freshly written script can briefly be busy while another test
        // thread forks; retry the spawn a few times.
        let mut caps = probe_ffmpeg_caps(&fake);
        for _ in 0..5 {
            if caps.version.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            caps = probe_ffmpeg_caps(&fake);
        }
        assert_eq!(caps.version.as_deref(), Some("9.0.2"));
        assert!(caps.libass && caps.vidstab && caps.zscale, "{caps:?}");
    }
}
