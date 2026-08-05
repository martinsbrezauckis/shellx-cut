//! sync.rs — MULTICAM AUDIO SYNC: align N clips of the same event by
//! cross-correlating their audio ENERGY ENVELOPES. Two cameras recording one
//! interview hear the same sound at the same wall-clock time, so the lag that best
//! aligns their audio envelopes IS the time offset between the clips.
//!
//! Pure DSP (envelope + normalized cross-correlation) + one ffmpeg decode per clip:
//!   - decode mono PCM at a FIXED rate (so two clips' envelopes are time-comparable),
//!   - reduce to an RMS envelope at `ENV_HZ` (20 ms resolution — finer than a video
//!     frame),
//!   - normalized cross-correlation over a bounded lag window → the best lag + score.
//! The cross-correlation is mean-subtracted + energy-normalized, so it's robust to
//! per-camera level/gain differences. Caller: dispatch.rs `edit_multicam_sync`.

use crate::ffmpeg::ffmpeg_bin;
use cut_core::{error_codes, CutError};
use std::path::Path;
use std::process::Command;

/// Envelope resolution (Hz). 50 Hz = one sample per 20 ms (sub-frame precision).
pub const ENV_HZ: usize = 50;
const ENV_HZ_I64: i64 = 50;
/// PCM decode rate (Hz) — fixed so all clips' envelopes share a time base.
const DECODE_HZ: u32 = 4000;
const DECODE_HZ_USIZE: usize = 4000;

/// Decode `path`'s first audio stream to a mono RMS ENERGY ENVELOPE at [`ENV_HZ`].
/// Errors when the asset has no audio (sync needs a common sound).
pub fn clip_envelope(path: &Path) -> Result<Vec<f32>, CutError> {
    let out = Command::new(ffmpeg_bin())
        .args([
            "-v",
            "error",
            "-nostdin",
            "-i",
            &path.to_string_lossy(),
            "-vn",
            "-ac",
            "1",
            "-ar",
            &DECODE_HZ.to_string(),
            "-f",
            "s16le",
            "-",
        ])
        .output()
        .map_err(|e| CutError::new(error_codes::FFMPEG, "ffmpeg failed for sync", e.to_string()))?;
    if !out.status.success() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "audio decode for sync failed",
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        )
        .with_suggested_action(
            "the clip needs an audio stream to sync (media.probe → has_audio)",
        ));
    }
    let pcm = out.stdout;
    let n = pcm.len() / 2; // i16 samples
    let win = DECODE_HZ_USIZE / ENV_HZ; // 80 samples per envelope point
    let mut env = Vec::new();
    let mut i = 0usize;
    while i < n {
        let end = (i + win).min(n);
        let mut sumsq = 0.0f64;
        for s in i..end {
            let v = i16::from_le_bytes([pcm[s * 2], pcm[s * 2 + 1]]) as f64 / 32768.0;
            sumsq += v * v;
        }
        let rms = (sumsq / (end - i) as f64).sqrt() as f32;
        env.push(rms);
        i = end;
    }
    Ok(env)
}

fn mean(x: &[f32]) -> f32 {
    if x.is_empty() {
        0.0
    } else {
        x.iter().sum::<f32>() / x.len() as f32
    }
}

/// The lag (in envelope samples) of `b` relative to `a` that best aligns them, with
/// the peak normalized-correlation score (−1..1). A POSITIVE lag means `b`'s sound
/// arrives LATER than `a`'s (b's content is delayed by `lag` samples) — so to sync,
/// place b that much EARLIER. Mean-subtracted + energy-normalized → robust to gain.
/// Returns `None` when no candidate has enough overlapping, non-silent signal.
pub fn best_lag(a: &[f32], b: &[f32], max_lag: usize) -> Option<(i64, f32)> {
    let ma = mean(a);
    let mb = mean(b);
    let a: Vec<f32> = a.iter().map(|x| x - ma).collect();
    let b: Vec<f32> = b.iter().map(|x| x - mb).collect();
    let max_lag = bounded_lag_limit(max_lag, a.len(), b.len());
    // REQUIRE a substantial overlap: at large lags only a tiny tail overlaps, and a
    // near-silent tail can correlate ~1.0 by chance (a spurious peak). Demand ≥50%
    // of the shorter envelope (and ≥1 s) so the score reflects the real signal.
    let min_overlap = ((a.len().min(b.len()) / 2).max(ENV_HZ)).max(1);
    let mut best: Option<(i64, f32)> = None;
    for lag in -max_lag..=max_lag {
        let mut dot = 0.0f32;
        let mut na = 0.0f32;
        let mut nb = 0.0f32;
        let mut overlap = 0usize;
        // Sum over i where both a[i] and b[i+lag] are valid.
        for (i, &av) in a.iter().enumerate() {
            let Some(j) = i64::try_from(i).ok().and_then(|idx| idx.checked_add(lag)) else {
                continue;
            };
            let Ok(j) = usize::try_from(j) else {
                continue;
            };
            if j >= b.len() {
                continue;
            }
            let bv = b[j];
            dot += av * bv;
            na += av * av;
            nb += bv * bv;
            overlap += 1;
        }
        if overlap >= min_overlap && na > 1e-9 && nb > 1e-9 {
            let score = dot / (na.sqrt() * nb.sqrt());
            if best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((lag, score));
            }
        }
    }
    best
}

/// Envelope-sample lag → milliseconds.
pub fn lag_to_ms(lag: i64) -> i64 {
    lag * 1000 / ENV_HZ_I64
}

fn bounded_lag_limit(max_lag: usize, a_len: usize, b_len: usize) -> i64 {
    let bounded = max_lag.min(a_len.saturating_add(b_len));
    i64::try_from(bounded).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A known delay is recovered: b = a delayed by K envelope samples → best_lag ≈ K.
    #[test]
    fn best_lag_recovers_a_known_delay() {
        // A distinctive envelope (a few energy bursts).
        let base: Vec<f32> = (0..400)
            .map(|i| {
                let t = i as f32;
                // three gaussian-ish bursts at 50, 180, 300.
                (-((t - 50.0).powi(2)) / 80.0).exp()
                    + (-((t - 180.0).powi(2)) / 80.0).exp()
                    + (-((t - 300.0).powi(2)) / 80.0).exp()
            })
            .collect();
        let k = 37usize; // delay b by 37 samples = 740 ms at 50 Hz
        let a = &base[k..]; // a starts 37 samples in
        let b = &base[..base.len() - k]; // b is the same content, 37 samples earlier
                                         // a[i] = base[i+k]; b[i] = base[i]; so a[i] = b[i+k] → best lag = +k.
        let (lag, score) = best_lag(a, b, 200).expect("known delay should correlate");
        assert_eq!(lag, k as i64, "recovered the delay (score {score:.3})");
        assert!(
            score > 0.95,
            "a clean shift correlates strongly: {score:.3}"
        );
        assert_eq!(lag_to_ms(lag), 740);
    }

    /// Zero offset for identical signals; the score is ~1.
    #[test]
    fn identical_signals_align_at_zero() {
        let a: Vec<f32> = (0..200).map(|i| ((i as f32) * 0.3).sin().abs()).collect();
        let (lag, score) = best_lag(&a, &a, 50).expect("identical signal should correlate");
        assert_eq!(lag, 0);
        assert!(score > 0.99);
    }

    #[test]
    fn best_lag_returns_none_when_no_candidate_has_valid_overlap() {
        let a: Vec<f32> = (0..20).map(|i| i as f32).collect();
        let b: Vec<f32> = (0..20).map(|i| (20 - i) as f32).collect();

        assert!(best_lag(&a, &b, 100).is_none());
    }

    #[test]
    fn bounded_lag_limit_clamps_before_i64_conversion() {
        assert_eq!(bounded_lag_limit(usize::MAX, 2, 3), 5);
    }
}
