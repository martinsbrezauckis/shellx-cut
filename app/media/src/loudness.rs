//! loudness.rs — integrated-loudness MEASUREMENT (verb `verify.loudness`) via a
//! single ffmpeg `loudnorm` analysis pass (EBU R128).
//!
//! Role: report a file's integrated loudness (LUFS), true peak (dBTP), loudness
//! range (LRA), and gating threshold — the MEASURE half of the loudness loop.
//! The NORMALIZE half already exists as render.final's single-pass
//! `normalize_loudness` (render.rs); the two compose into measure → normalize →
//! re-measure. This is a fast `-f null` pass (no re-encode) and needs ONLY ffmpeg
//! (no perception venv), so an agent can check delivery loudness cheaply on any
//! audio-bearing asset or finished render.
//!
//! Dependencies: ffmpeg subprocess (crate::ffmpeg). Primary caller: server
//! dispatch `verify.loudness`.

use crate::ffmpeg::{ffmpeg_bin, run_bounded_command};
use cut_core::{error_codes, CutError};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

/// EBU R128 loudness measurement of one audio-bearing file.
#[derive(Debug, Clone, Serialize)]
pub struct Loudness {
    /// Integrated (program) loudness over the whole file, LUFS — the single
    /// number broadcast/social targets are set against (e.g. -14 social).
    pub integrated_lufs: f64,
    /// True peak (dBTP) — the inter-sample peak; ≤ -1 is the safe-delivery ceiling.
    pub true_peak_dbtp: f64,
    /// Loudness range (LRA, LU) — the macro-dynamic spread between quiet and loud.
    pub lra: f64,
    /// Relative gating threshold (LUFS) loudnorm measured.
    pub threshold_lufs: f64,
}

/// Measure `path`'s integrated loudness via one ffmpeg `loudnorm` analysis pass
/// (`print_format=json`, `-f null` so nothing is re-encoded). loudnorm prints the
/// measurement JSON to stderr; we parse the last `{…}` block. Errors when the file
/// has no MEASURABLE audio (loudnorm reports a non-finite integrated value — true
/// silence or no audio stream), which the caller surfaces as an actionable message.
pub fn measure(path: &Path) -> Result<Loudness, CutError> {
    let mut command = Command::new(ffmpeg_bin());
    command.args([
        // NOT `-v error`: loudnorm prints its JSON at the INFO level, so the
        // default verbosity is required for the measurement to appear. Banner
        // and progress stats are suppressed to keep stderr clean for parsing.
        "-hide_banner",
        "-nostats",
        "-nostdin",
        "-i",
        &path.to_string_lossy(),
        "-vn",
        "-af",
        "loudnorm=print_format=json",
        "-f",
        "null",
        "-",
    ]);
    let out = run_bounded_command(&mut command, "measure loudness")?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "loudness analysis failed",
            stderr.trim().to_string(),
        )
        .with_suggested_action(
            "the file may have no audio stream (media.probe reports has_audio)",
        ));
    }
    let json = extract_last_json(&stderr).ok_or_else(|| {
        CutError::new(
            error_codes::FFMPEG,
            "loudnorm produced no JSON measurement",
            stderr.trim().to_string(),
        )
    })?;
    let v: serde_json::Value = serde_json::from_str(&json).map_err(|e| {
        CutError::new(
            error_codes::FFMPEG,
            "loudnorm JSON did not parse",
            e.to_string(),
        )
    })?;
    loudness_from_value(&v)
}

fn loudness_from_value(v: &serde_json::Value) -> Result<Loudness, CutError> {
    // loudnorm emits the numbers as STRINGS ("-21.80"); silence yields "-inf".
    let field = |k: &str| -> Option<f64> { v.get(k)?.as_str()?.trim().parse::<f64>().ok() };
    let integrated_lufs = field("input_i").filter(|x| x.is_finite()).ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "no measurable integrated loudness (silence or no audio stream)",
            "loudnorm `input_i` was not a finite number",
        )
        .with_suggested_action("measure a file that has an audible audio stream")
    })?;
    Ok(Loudness {
        integrated_lufs,
        true_peak_dbtp: field("input_tp").unwrap_or(f64::NAN),
        lra: field("input_lra").unwrap_or(f64::NAN),
        threshold_lufs: field("input_thresh").unwrap_or(f64::NAN),
    })
}

/// Extract the LAST `{ … }` JSON object from ffmpeg's stderr (loudnorm prints one
/// at the end of its analysis). Brace-matched on the outermost pair.
fn extract_last_json(s: &str) -> Option<String> {
    let end = s.rfind('}')?;
    let start = s[..end].rfind('{')?;
    Some(s[start..=end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Cmd;

    /// Generate a sine tone (known, deterministic) and measure it — proves the
    /// loudnorm pass runs, the JSON parses, and the integrated value is a sane
    /// finite LUFS. Skips if ffmpeg is unavailable.
    #[test]
    fn measures_a_sine_tone() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("tone.wav");
        let st = Cmd::new(ffmpeg_bin())
            .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
            .arg("sine=frequency=440:duration=3")
            .arg(&wav)
            .status();
        match st {
            Ok(s) if s.success() => {}
            _ => {
                eprintln!("ffmpeg unavailable — skipping loudness measure test");
                return;
            }
        }
        let m = measure(&wav).expect("measure a sine tone");
        // A -3 dBFS sine sits around -18..-20 LUFS; just assert a sane finite range.
        assert!(
            m.integrated_lufs > -70.0 && m.integrated_lufs < 0.0,
            "integrated {} LUFS out of sane range",
            m.integrated_lufs
        );
        assert!(m.true_peak_dbtp.is_finite() || m.true_peak_dbtp.is_nan());
        assert!(m.lra >= 0.0, "LRA must be non-negative: {}", m.lra);
    }

    /// extract_last_json pulls the trailing object out of a noisy stderr.
    #[test]
    fn extracts_trailing_json() {
        let s = "frame=  10 fps=0\n[Parsed_loudnorm] noise\n{\n  \"input_i\" : \"-21.80\"\n}\n";
        let j = extract_last_json(s).expect("found json");
        let v: serde_json::Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["input_i"], "-21.80");
        assert!(extract_last_json("no braces here").is_none());
    }

    #[test]
    fn missing_lra_is_nan_not_fake_zero() {
        let v = serde_json::json!({
            "input_i": "-21.80",
            "input_tp": "-2.0",
            "input_thresh": "-31.0"
        });
        let m = loudness_from_value(&v).unwrap();
        assert!(m.lra.is_nan(), "missing input_lra is unknown, not 0.0 LU");
    }
}
