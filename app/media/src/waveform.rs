//! waveform.rs — audio peak extraction for the timeline waveform (workspace contract v1:
//! "media.waveform — peaks JSON from perception").
//!
//! Role: decode an asset's audio to mono PCM at a duration-bounded rate and
//! reduce it to `buckets` abs-max amplitude peaks (0..1) — the data the UI draws
//! on the audio track and agents read to find loud/quiet regions. Deterministic
//! (fixed ffmpeg decode params) and memory-bounded (the decode rate is chosen so
//! total samples ≈ buckets×4 regardless of clip length).
//! Dependencies: ffmpeg subprocess (crate::ffmpeg). Primary caller:
//! server dispatch `media.waveform`.

use crate::ffmpeg::ffmpeg_bin;
use cut_core::{error_codes, CutError};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

/// Per-bucket audio peaks for an asset.
#[derive(Debug, Clone, Serialize)]
pub struct Waveform {
    /// Number of buckets actually produced (== requested, clamped).
    pub bucket_count: usize,
    /// Abs-max amplitude per bucket, normalized 0.0..=1.0, left→right in time.
    pub peaks: Vec<f32>,
    /// Source duration the peaks span (ms).
    pub source_ms: u64,
    /// The mono PCM sample rate actually decoded at (duration-bounded).
    pub sample_rate: u32,
}

/// Extract `buckets` amplitude peaks from `path`'s first audio stream.
/// `duration_ms` is the asset's probed duration (used to bound the decode rate);
/// pass 0 if unknown (a safe default rate is used). Errors when the asset has no
/// audio stream — the caller turns that into an actionable message.
pub fn waveform(path: &Path, duration_ms: u64, buckets: usize) -> Result<Waveform, CutError> {
    let buckets = buckets.clamp(1, 8000);
    // Decode rate: bound total samples to ~2M for long clips (memory) but keep a
    // 2000 Hz FLOOR so short clips don't decode so low that the resampler's
    // anti-alias low-pass attenuates real content (a 200 Hz decode killed a 440
    // Hz tone — the amplitude envelope needs a few kHz). 2000..8000 Hz captures
    // the peak envelope well for a waveform.
    let dur = duration_ms.max(1);
    let max_samples: u64 = 2_000_000;
    let rate = if duration_ms == 0 {
        8000
    } else {
        (max_samples * 1000 / dur).clamp(2000, 8000) as u32
    };
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
            &rate.to_string(),
            "-f",
            "s16le",
            "-",
        ])
        .output()
        .map_err(|e| {
            CutError::new(
                error_codes::FFMPEG,
                "ffmpeg failed to run for waveform",
                e.to_string(),
            )
        })?;
    if !out.status.success() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "waveform extraction failed",
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        )
        .with_suggested_action(
            "the asset may have no audio stream (media.probe reports has_audio)",
        ));
    }
    let pcm = out.stdout;
    let n = pcm.len() / 2; // i16 samples
    if n == 0 {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "waveform extraction decoded no audio samples",
            "ffmpeg exited successfully but produced zero PCM bytes",
        )
        .with_suggested_action(
            "verify the asset has a real audio stream with non-zero decoded duration",
        ));
    }
    let mut peaks = vec![0.0f32; buckets];
    let per = n.div_ceil(buckets); // ceil so every sample lands in a bucket
    for (bi, bucket) in peaks.iter_mut().enumerate() {
        let start = bi * per;
        if start >= n {
            break;
        }
        let end = ((bi + 1) * per).min(n);
        let mut mx: i32 = 0;
        for s in start..end {
            let v = i16::from_le_bytes([pcm[s * 2], pcm[s * 2 + 1]]).unsigned_abs() as i32;
            if v > mx {
                mx = v;
            }
        }
        *bucket = mx as f32 / 32768.0;
    }
    Ok(Waveform {
        bucket_count: buckets,
        peaks,
        source_ms: duration_ms,
        sample_rate: rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ffmpeg_gen(args: &[&str], out: &Path) -> bool {
        let mut full: Vec<String> = vec!["-v".into(), "error".into(), "-y".into()];
        full.extend(args.iter().map(|s| s.to_string()));
        full.push(out.to_string_lossy().into_owned());
        Command::new("ffmpeg")
            .args(&full)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[test]
    fn waveform_peaks_loud_tone_high_silence_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let tone = tmp.path().join("tone.wav");
        if !ffmpeg_gen(
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=2:sample_rate=44100",
                "-ac",
                "1",
            ],
            &tone,
        ) {
            eprintln!("ffmpeg unavailable — skipping waveform test");
            return;
        }
        let wf = waveform(&tone, 2000, 100).unwrap();
        assert_eq!(wf.bucket_count, 100);
        assert_eq!(wf.peaks.len(), 100);
        assert!(
            wf.peaks.iter().all(|&p| (0.0..=1.0).contains(&p)),
            "peaks normalized 0..1"
        );
        // ffmpeg's `sine` emits ~0.125 amplitude (not full-scale); the point is
        // the tone is clearly ABOVE the silence floor and non-trivial.
        let max = wf.peaks.iter().cloned().fold(0.0_f32, f32::max);
        assert!(
            max > 0.05,
            "a steady tone should produce clear peaks, got max {max}"
        );
        let nonzero = wf.peaks.iter().filter(|&&p| p > 0.02).count();
        assert!(
            nonzero > 90,
            "a continuous tone fills (almost) every bucket: {nonzero}/100"
        );

        let sil = tmp.path().join("sil.wav");
        assert!(ffmpeg_gen(
            &["-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono", "-t", "1"],
            &sil
        ));
        let wf2 = waveform(&sil, 1000, 50).unwrap();
        assert!(
            wf2.peaks.iter().all(|&p| p < 0.01),
            "silence → near-zero peaks"
        );
    }

    #[test]
    fn waveform_errors_on_no_audio() {
        let tmp = tempfile::tempdir().unwrap();
        let mp4 = tmp.path().join("noaudio.mp4");
        if !ffmpeg_gen(
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x240:duration=1:rate=10",
                "-pix_fmt",
                "yuv420p",
            ],
            &mp4,
        ) {
            return;
        }
        assert!(
            waveform(&mp4, 1000, 50).is_err(),
            "a video-only asset must error (no audio stream)"
        );
    }

    #[test]
    fn waveform_errors_when_decode_emits_no_pcm() {
        let tmp = tempfile::tempdir().unwrap();
        let wav = tmp.path().join("empty.wav");
        if !ffmpeg_gen(
            &["-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono", "-t", "0"],
            &wav,
        ) {
            eprintln!("ffmpeg unavailable — skipping empty waveform test");
            return;
        }

        let err = waveform(&wav, 0, 50).expect_err("zero decoded samples must error");
        assert_eq!(err.code, error_codes::FFMPEG);
        assert!(
            err.message.contains("no audio samples"),
            "error names empty decode: {err:?}"
        );
    }
}
