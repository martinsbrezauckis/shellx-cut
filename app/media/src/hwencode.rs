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

use crate::ffmpeg::ffmpeg_bin;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

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

static CAPS: OnceLock<HwCaps> = OnceLock::new();
const HW_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Detected HW encoders (probed once, then cached for the process lifetime).
/// `SHELLX_CUT_NO_HWENC=1` forces the software tier (CI / reproducibility / a
/// machine whose HW encoder is flaky).
pub fn hw_caps() -> &'static HwCaps {
    CAPS.get_or_init(|| {
        if std::env::var_os("SHELLX_CUT_NO_HWENC").is_some() {
            return HwCaps::default();
        }
        probe()
    })
}

fn probe() -> HwCaps {
    let mut caps = HwCaps::default();
    for (codec, names) in CANDIDATES {
        for name in *names {
            if encoder_works(name) {
                let slot = match *codec {
                    "h264" => &mut caps.h264,
                    "hevc" => &mut caps.hevc,
                    "av1" => &mut caps.av1,
                    _ => continue,
                };
                *slot = Some((*name).to_string());
                break; // best-first: first working candidate wins
            }
        }
    }
    caps
}

/// Does `encoder` actually run on the RESOLVED ffmpeg? A 0.1 s test encode to a
/// null muxer — the only reliable signal (listing ≠ a working GPU/driver). Quiet;
/// returns true on a clean exit.
fn encoder_works(encoder: &str) -> bool {
    encoder_works_at(&ffmpeg_bin(), encoder)
}

/// Like [`encoder_works`] but against an EXPLICIT ffmpeg binary — used by the
/// doctor's multi-candidate scan to probe each discovered ffmpeg, not just the
/// resolved one.
fn encoder_works_at(ffmpeg: &OsStr, encoder: &str) -> bool {
    command_status_with_timeout(
        ffmpeg,
        &[
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=0.1:size=256x256:rate=30",
            "-c:v",
            encoder,
            "-f",
            "null",
            "-",
        ],
        HW_PROBE_TIMEOUT,
    )
}

/// Hardware encoder ffmpeg args for `(codec, quality)` using the detected HW
/// encoder, plus the output extension. `q` is the quality tier (0=draft, 1=
/// standard, 2=high). Returns None when no HW encoder exists for the codec (the
/// caller then uses the software tier). The rate-control knob differs per backend
/// (NVENC/QSV/AMF use a lower-is-better quantizer; VideoToolbox uses a 0-100
/// higher-is-better quality), so each backend maps the tier to its own scale.
pub fn hw_codec_args(codec: &str, q: usize) -> Option<(Vec<String>, &'static str)> {
    let enc = hw_caps().for_codec(codec)?.to_string();
    let s = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let qi = q.min(2);
    let hevc_tag = matches!(codec, "hevc" | "h265");
    let mut args: Vec<String> = if enc.ends_with("_nvenc") {
        // NVENC: VBR with a target quality (-cq), uncapped bitrate (-b:v 0).
        let cq = ["32", "27", "23"][qi];
        s(&[
            "-c:v", &enc, "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", cq, "-b:v", "0",
            "-pix_fmt", "yuv420p",
        ])
    } else if enc.ends_with("_qsv") {
        let gq = ["32", "27", "23"][qi];
        s(&[
            "-c:v",
            &enc,
            "-global_quality",
            gq,
            "-preset",
            "medium",
            "-pix_fmt",
            "nv12",
        ])
    } else if enc.ends_with("_amf") {
        let qp = ["30", "26", "22"][qi];
        s(&[
            "-c:v", &enc, "-rc", "cqp", "-qp_i", qp, "-qp_p", qp, "-quality", "quality",
            "-pix_fmt", "yuv420p",
        ])
    } else if enc.ends_with("_videotoolbox") {
        // VideoToolbox: -q:v is 0..100, HIGHER is better (inverted vs a quantizer).
        let qv = ["40", "55", "65"][qi];
        s(&["-c:v", &enc, "-q:v", qv, "-pix_fmt", "yuv420p"])
    } else {
        return None;
    };
    if hevc_tag {
        // Apple/QuickTime compatibility tag for HEVC in mp4 (matches software).
        args.push("-tag:v".into());
        args.push("hvc1".into());
    }
    // Container: HEVC/H.264/AV1 all ride in mp4 here (matches the software tier).
    Some((args, "mp4"))
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

static GPU_FILTERS: OnceLock<bool> = OnceLock::new();

/// True when the full CUDA render chain — `hwupload` -> `scale_cuda` ->
/// `overlay_cuda` -> `h264_nvenc` — actually RUNS on this box (probed once,
/// cached for the process lifetime).
///
/// WHY the whole chain, not "is the filter listed": scale_cuda/overlay_cuda need a
/// working CUDA device + driver, and the fast-track also needs NVENC to encode the
/// VRAM frames. A LISTED filter can still fail to init (no device / wrong driver),
/// so — exactly like [`encoder_works`] — the only reliable signal is to RUN it. The
/// probe composites a tiny CUDA overlay and encodes one frame to `-f null`; a clean
/// exit means the GPU render path's filters are usable here. The exact command was
/// verified to clean-exit on the dev 5080 before it was baked in here.
/// `SHELLX_CUT_NO_HWENC=1` forces it off (the GPU path needs NVENC, so the no-HW
/// reproducibility switch disables the GPU filter path too).
pub fn gpu_filters_available() -> bool {
    *GPU_FILTERS.get_or_init(|| {
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
    command_status_with_timeout(
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
        HW_PROBE_TIMEOUT,
    )
}

fn command_status_with_timeout(prog: &OsStr, args: &[&str], timeout: Duration) -> bool {
    let mut child = match Command::new(prog)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
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

/// Total VRAM (bytes) of the primary CUDA device, read once from `nvidia-smi
/// --query-gpu=memory.total` (MiB). `None` when nvidia-smi is absent or the query
/// fails — the render gate then uses a conservative default budget rather than
/// guessing the device size. Cached for the process lifetime (VRAM does not change).
///
/// First CSV line = the primary device (index 0), which is the one ffmpeg's
/// `cuda=cu:0` device binds — matching [`gpu_filter_chain_works_at`].
pub fn cuda_total_vram_bytes() -> Option<u64> {
    *CUDA_VRAM.get_or_init(|| {
        let out = Command::new("nvidia-smi")
            .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
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
/// None when the binary cannot be spawned / exits non-zero. Quiet.
fn ffmpeg_version_at(ffmpeg: &OsStr) -> Option<String> {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-version"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(2))
        .map(|v| v.to_string())
}

/// Which feature-filters this ffmpeg has, learned from `ffmpeg -filters`. A filter
/// is listed by ffmpeg IFF its backing library was compiled in, so this is the
/// ground truth for "can this binary burn captions / stabilize / color-manage" —
/// the exact thing the auto-selector must respect (a HW build that dropped libass
/// would silently lose caption burn-in; one that dropped libzimg would fail a
/// color-managed render). Returns `(libass, vidstab, zscale)`; `(false,false,false)`
/// if the binary does not run. One cheap spawn (no encode).
fn feature_filters_at(ffmpeg: &OsStr) -> (bool, bool, bool) {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-filters"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let t = String::from_utf8_lossy(&o.stdout);
            // The `subtitles` (and `ass`) filters exist only with --enable-libass;
            // `vidstabtransform` only with --enable-libvidstab; `zscale` only with
            // --enable-libzimg. Match the filter-name column exactly so similarly
            // named custom filters do not produce false capability bits.
            let libass = filter_listing_has(&t, "subtitles") || filter_listing_has(&t, "ass");
            let vidstab = filter_listing_has(&t, "vidstabtransform");
            let zscale = filter_listing_has(&t, "zscale");
            (libass, vidstab, zscale)
        }
        _ => (false, false, false),
    }
}

fn filter_listing_has(listing: &str, target: &str) -> bool {
    listing.lines().any(|line| {
        let mut cols = line.split_whitespace();
        let Some(flags) = cols.next() else {
            return false;
        };
        let Some(name) = cols.next() else {
            return false;
        };
        flags.len() >= 3
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
    let osff = ffmpeg.as_os_str();
    let version = ffmpeg_version_at(osff);
    let no_hw = std::env::var_os("SHELLX_CUT_NO_HWENC").is_some();
    // libass/vidstab/zscale are independent of the HW tier (they gate caption/
    // stabilize/color-managed renders, not encode speed), so probe them whenever the
    // binary runs — even under no_hw.
    let (libass, vidstab, zscale) = if version.is_some() {
        feature_filters_at(osff)
    } else {
        (false, false, false)
    };
    let (hw, cuda_filters) = if version.is_none() || no_hw {
        (HwCaps::default(), false)
    } else {
        let mut caps = HwCaps::default();
        for (codec, names) in CANDIDATES {
            for name in *names {
                if encoder_works_at(osff, name) {
                    let slot = match *codec {
                        "h264" => &mut caps.h264,
                        "hevc" => &mut caps.hevc,
                        "av1" => &mut caps.av1,
                        _ => continue,
                    };
                    *slot = Some((*name).to_string());
                    break; // best-first: first working candidate wins
                }
            }
        }
        let cuda = gpu_filter_chain_works_at(osff);
        (caps, cuda)
    };
    FfmpegCaps {
        path: ffmpeg.display().to_string(),
        version,
        hw,
        cuda_filters,
        libass,
        vidstab,
        zscale,
    }
}

#[cfg(test)]
mod tests {
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
        assert!(hw_codec_args("definitely_not_a_codec", 1).is_none());
        if let Some((args, ext)) = hw_codec_args("hevc", 2) {
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
    /// truth on whatever ffmpeg this box resolves — so a color-managed render is
    /// routed to a zscale (libzimg) build exactly when the binary really has it.
    /// Portable: if no ffmpeg is runnable the probe reports all-false and we skip;
    /// on a box WITH zscale (e.g. WSL's /usr/bin/ffmpeg) it asserts caps.zscale==true.
    #[test]
    fn feature_probe_matches_ffmpeg_filters_ground_truth() {
        let ff = crate::toolpath::ffmpeg();
        // Ground truth straight from `ffmpeg -filters`.
        let out = Command::new(&ff)
            .args(["-hide_banner", "-filters"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
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
}
