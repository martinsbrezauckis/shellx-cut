//! scopes.rs — video-SCOPES measurement (verb `verify.scopes`).
//!
//! Role: turn one frame into OBJECTIVE signal data — luma levels (min/avg/max),
//! clipping, broadcast-range legality, saturation, and white-balance cast — via
//! ffmpeg `signalstats`, plus (optionally) the classic scope IMAGES (vectorscope /
//! waveform / histogram) for a human or VLM to eyeball. This is the agent-first
//! complement to render.qc: instead of guessing colour from a thumbnail, the agent
//! reads numbers it can reason about ("YMAX 255 → highlights clipped", "VAVG 140 →
//! warm/red cast"). A pure measurement (the verify.* philosophy) — ffmpeg-only, no
//! perception venv.
//!
//! Dependencies: ffmpeg + ffprobe subprocess (crate::ffmpeg). Caller: dispatch
//! `verify.scopes` (which extracts the composed/source frame to a file first).

use crate::ffmpeg::{ffmpeg_bin, run_bounded_command};
use cut_core::{error_codes, CutError};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

/// Signal measurements of one frame (ffmpeg `signalstats`, 8-bit scale 0..255).
#[derive(Debug, Clone, Serialize)]
pub struct Scopes {
    /// Luma minimum / average / maximum (0..255).
    pub y_min: f64,
    pub y_avg: f64,
    pub y_max: f64,
    /// Highlights clipped (Y at/above 254) — detail lost in the brights.
    pub clip_highlights: bool,
    /// Shadows crushed (Y at/below 1) — detail lost in the blacks.
    pub clip_shadows: bool,
    /// Whole frame inside the broadcast-legal luma window [16,235] (Rec.709 limited).
    pub broadcast_legal: bool,
    /// Chroma saturation average / maximum (0..≈181; >118 risks illegal chroma).
    pub sat_avg: f64,
    pub sat_max: f64,
    /// Mean Cb (U) / Cr (V); 128 = neutral. Deviations = a colour cast.
    pub u_avg: f64,
    pub v_avg: f64,
    /// Human label for the white-balance cast (neutral / warm / cool / green / magenta).
    pub white_balance: String,
    /// Mean / median hue (degrees).
    pub hue_avg: f64,
    pub hue_med: f64,
}

fn measure_command(path: &Path) -> Command {
    let mut command = Command::new(ffmpeg_bin());
    command.args(["-v", "error", "-i"]).arg(path).args([
        "-vf",
        "signalstats,metadata=print:file=-",
        "-frames:v",
        "1",
        "-f",
        "null",
        "-",
    ]);
    command
}

/// Measure a decodable image/frame file (PNG/JPEG) with `signalstats`. The path
/// is passed as a normal ffmpeg input argument rather than interpolated into a
/// lavfi `movie=` expression, which keeps Windows drive/extended paths out of
/// filter syntax. Errors if ffmpeg fails or the stats are absent.
pub fn measure(path: &Path) -> Result<Scopes, CutError> {
    let mut command = measure_command(path);
    let out = run_bounded_command(&mut command, "measure video scopes")?;
    if !out.status.success() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "scopes signalstats failed",
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let mut tags = serde_json::Map::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.starts_with("lavfi.signalstats.") {
            tags.insert(
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }
    if tags.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "no signalstats for that frame (not a valid frame?)",
            "ffmpeg returned no signalstats metadata",
        ));
    }
    scopes_from_tags(&serde_json::Value::Object(tags))
}

fn scopes_from_tags(tags: &serde_json::Value) -> Result<Scopes, CutError> {
    let g = |k: &str| -> Result<f64, CutError> {
        let full_key = format!("lavfi.signalstats.{k}");
        tags.get(&full_key)
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite())
            .ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    format!("missing signalstats field {full_key}"),
                    "ffprobe did not return the numeric frame tag needed for scopes",
                )
            })
    };
    let (y_min, y_avg, y_max) = (g("YMIN")?, g("YAVG")?, g("YMAX")?);
    let (u_avg, v_avg) = (g("UAVG")?, g("VAVG")?);
    let (sat_avg, sat_max) = (g("SATAVG")?, g("SATMAX")?);
    let (hue_avg, hue_med) = (g("HUEAVG")?, g("HUEMED")?);
    // White-balance cast from the chroma means (128 = neutral). Cr (V) high = red/warm,
    // low = green; Cb (U) high = blue/cool, low = yellow. Report the dominant axis.
    let du = u_avg - 128.0;
    let dv = v_avg - 128.0;
    let white_balance = if du.abs() < 6.0 && dv.abs() < 6.0 {
        "neutral".to_string()
    } else if dv.abs() >= du.abs() {
        if dv > 0.0 {
            "warm (red/orange cast)"
        } else {
            "green cast"
        }
        .to_string()
    } else if du > 0.0 {
        "cool (blue cast)".to_string()
    } else {
        "yellow cast".to_string()
    };
    Ok(Scopes {
        y_min,
        y_avg,
        y_max,
        clip_highlights: y_max >= 254.0,
        clip_shadows: y_min <= 1.0,
        broadcast_legal: y_min >= 16.0 && y_max <= 235.0,
        sat_avg,
        sat_max,
        u_avg,
        v_avg,
        white_balance,
        hue_avg,
        hue_med,
    })
}

/// A scope-IMAGE kind (vectorscope / waveform / histogram).
#[derive(Debug, Clone, Copy)]
pub enum ScopeKind {
    /// Vectorscope: chroma spread (saturation + hue), skin-tone line at ~123°.
    Vectorscope,
    /// Luma waveform (per-column brightness) + RGB parade legality.
    Waveform,
    /// RGB histogram (tonal distribution per channel).
    Histogram,
}

impl ScopeKind {
    pub fn key(self) -> &'static str {
        match self {
            ScopeKind::Vectorscope => "vectorscope",
            ScopeKind::Waveform => "waveform",
            ScopeKind::Histogram => "histogram",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "vectorscope" => Some(ScopeKind::Vectorscope),
            "waveform" => Some(ScopeKind::Waveform),
            "histogram" => Some(ScopeKind::Histogram),
            _ => None,
        }
    }
    /// The ffmpeg `-vf` filter that renders this scope from the input frame.
    fn filter(self) -> &'static str {
        match self {
            // graticule + green flecks so the skin line + targets are visible.
            ScopeKind::Vectorscope => "vectorscope=mode=color3:graticule=green:flags=name",
            // luma+chroma parade-style waveform, full-range.
            ScopeKind::Waveform => "waveform=mode=column:display=overlay:components=7",
            ScopeKind::Histogram => "histogram=display_mode=parade:levels_mode=linear",
        }
    }
}

/// Render one scope IMAGE for `frame_path` to `out_path` (a PNG). Single ffmpeg
/// pass, one frame. The image is for human/VLM review; the numeric receipt
/// ([`measure`]) is the agent-readable part.
pub fn render_scope(frame_path: &Path, kind: ScopeKind, out_path: &Path) -> Result<(), CutError> {
    let mut command = Command::new(ffmpeg_bin());
    command
        .args(["-y", "-v", "error", "-i"])
        .arg(frame_path)
        .args(["-frames:v", "1", "-vf", kind.filter()])
        .arg(out_path);
    let st = run_bounded_command(&mut command, "render video scope")?.status;
    if !st.success() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            format!("scope render failed ({})", kind.key()),
            "ffmpeg returned non-zero",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Cmd;

    fn gen_frame(dir: &Path, name: &str, src: &str) -> std::path::PathBuf {
        let out = dir.join(name);
        let ok = Cmd::new(ffmpeg_bin())
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                src,
                "-frames:v",
                "1",
            ])
            .arg(&out)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "frame gen failed for {name}");
        out
    }

    /// Measure a mid-gray frame → luma ≈128, neutral white balance, legal levels.
    #[test]
    fn measures_gray_frame() {
        let dir = tempfile::tempdir().unwrap();
        let f = gen_frame(dir.path(), "gray.png", "color=gray:s=160x120");
        let s = measure(&f).expect("measure gray");
        assert!(
            (s.y_avg - 128.0).abs() < 30.0,
            "gray luma ~128, got {}",
            s.y_avg
        );
        assert_eq!(
            s.white_balance, "neutral",
            "gray has no cast (u={}, v={})",
            s.u_avg, s.v_avg
        );
        assert!(s.broadcast_legal, "mid-gray is within [16,235]");
    }

    /// A pure-white frame clips highlights and is NOT broadcast-legal.
    #[test]
    fn detects_highlight_clipping() {
        let dir = tempfile::tempdir().unwrap();
        // The product measures extracted JPEG frames. Keep this fixture full-range
        // too: RGB PNG decoding is normalized to limited-range Y=235 by ffmpeg,
        // which is legal white rather than a clipped Y=255 sample.
        let f = gen_frame(dir.path(), "white.jpg", "color=white:s=160x120");
        let s = measure(&f).expect("measure white");
        assert!(
            s.clip_highlights,
            "pure white clips highlights (ymax={})",
            s.y_max
        );
        assert!(
            !s.broadcast_legal,
            "255 luma exceeds the 235 broadcast ceiling"
        );
    }

    /// A saturated red frame reads a warm (red) cast + high saturation.
    #[test]
    fn detects_warm_cast() {
        let dir = tempfile::tempdir().unwrap();
        let f = gen_frame(dir.path(), "red.png", "color=red:s=160x120");
        let s = measure(&f).expect("measure red");
        assert!(
            s.sat_max > 80.0,
            "red is saturated, got sat_max {}",
            s.sat_max
        );
        assert!(
            s.white_balance.contains("warm") || s.white_balance.contains("red"),
            "red frame → warm/red cast, got '{}'",
            s.white_balance
        );
    }

    /// render_scope produces a non-empty PNG for each kind.
    #[test]
    fn renders_scope_images() {
        let dir = tempfile::tempdir().unwrap();
        let f = gen_frame(dir.path(), "src.png", "testsrc2=s=320x240");
        for kind in [
            ScopeKind::Vectorscope,
            ScopeKind::Waveform,
            ScopeKind::Histogram,
        ] {
            let out = dir.path().join(format!("{}.png", kind.key()));
            render_scope(&f, kind, &out).expect("render scope");
            let sz = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            assert!(sz > 200, "{} scope PNG too small: {sz}B", kind.key());
        }
    }

    #[test]
    fn missing_required_signalstats_tags_are_errors() {
        let tags = serde_json::json!({
            "lavfi.signalstats.YMIN": "16",
            "lavfi.signalstats.YMAX": "235"
        });
        let err = scopes_from_tags(&tags).unwrap_err();
        assert!(
            err.message.contains("missing signalstats"),
            "missing numeric fields should not become NaN-backed booleans: {err:?}"
        );
    }

    #[test]
    fn windows_paths_are_normal_input_arguments_not_filtergraph_text() {
        let path = Path::new(r"\\?\C:\Users\Example\Documents\ShellX Cut Projects\frame.jpg");
        let command = measure_command(path);
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|arg| arg == &path.to_string_lossy()));
        assert!(args
            .iter()
            .any(|arg| arg == "signalstats,metadata=print:file=-"));
        assert!(args.iter().all(|arg| !arg.starts_with("movie=")));
    }
}
