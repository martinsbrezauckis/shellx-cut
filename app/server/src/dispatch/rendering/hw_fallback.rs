//! Software retry for a hardware encoder that fails during a render (item 96).
//!
//! The dispatcher picks the hardware encoder before the job starts, from a
//! probe on an idle machine. When the render then fails for a reason software
//! does not share (video memory, encode-session limit, driver, a size or pixel
//! format this device refuses) and the request was `hardware:"auto"`, the
//! render runs again with the software encoder of the same format and quality,
//! and the job result and receipt say why in `encode.fallback_reason`.
//!
//! The whole render is retried, not only the failed window: a segmented render
//! joins its windows by stream copy, which needs every window from the same
//! encoder with the same parameters.

use cut_core::{CutError, EncodeReport};
use cut_media::hwencode::SoftwareFallback;
use cut_media::hwencode_runtime::classify_runtime_failure;
use cut_media::render::RenderPreset;

/// The software preset a failed hardware render retries with.
pub(super) struct SoftwareRetry {
    pub preset: RenderPreset,
    pub encoder: String,
    pub hardware_encoder: String,
}

/// Build the retry for a render that will use `hw_encoder`. `rate` is the
/// video bitrate target (kbps, constant?) when the request set one; `label`
/// the receipt suffix for it; `threads` the encoder threads the preflight
/// pinned.
pub(super) fn software_retry(
    format: &str,
    quality: &str,
    hw_encoder: &str,
    rate: Option<(u32, bool)>,
    label: Option<&str>,
    threads: Option<u32>,
) -> Option<SoftwareRetry> {
    let (mut video_args, audio_args, _) = cut_media::render::format_codec_args(format, quality)?;
    let encoder = video_args
        .windows(2)
        .find(|w| w[0] == "-c:v")
        .map(|w| w[1].clone())?;
    if let Some((kbps, cbr)) = rate {
        video_args = cut_media::render::apply_bitrate(video_args, kbps, cbr, &encoder);
    }
    if let Some(n) = threads {
        video_args.extend(["-threads".to_string(), n.to_string()]);
    }
    let base = if matches!(format, "h264" | "mp4") && encoder == "libx264" {
        quality.to_string()
    } else {
        format!("{quality}/{encoder}")
    };
    Some(SoftwareRetry {
        preset: RenderPreset {
            name: match label {
                Some(b) => format!("{base} @{b}"),
                None => base,
            },
            video_args,
            audio_args,
        },
        encoder,
        hardware_encoder: hw_encoder.to_string(),
    })
}

/// Why the hardware encoder failed, when the failure is its own: the reason
/// the job result and receipt carry after the software retry.
pub(super) fn runtime_fallback(
    retry: &SoftwareRetry,
    error: &CutError,
) -> Option<SoftwareFallback> {
    let kind = classify_runtime_failure(&error.cause)?;
    Some(SoftwareFallback {
        message: format!(
            "The hardware encoder ({}) failed during this export: {}. The export was made \
             again with software encoding, which is slower. {}",
            retry.hardware_encoder,
            kind.reason(),
            kind.action()
        ),
        detail: Some(cut_media::hwencode_runtime::runtime_detail(
            &retry.hardware_encoder,
            &error.cause,
        )),
    })
}

/// A software retry that also failed: keep its error and say that hardware
/// was tried first, so the person does not retry the same thing.
pub(super) fn both_failed(mut software: CutError, fallback: &SoftwareFallback) -> CutError {
    software.message = format!(
        "{} (software encoding was tried after the hardware encoder failed)",
        software.message
    );
    software.cause = format!("{}; hardware: {}", software.cause, fallback.message);
    software
}

/// The encode record after a runtime fallback (hardware asked, software ran).
pub(super) fn fallback_report(retry: &SoftwareRetry, fallback: &SoftwareFallback) -> EncodeReport {
    super::encode_report::encode_report(&retry.encoder, true, Some(fallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_keeps_format_quality_and_rate() {
        let r = software_retry(
            "hevc",
            "high",
            "hevc_nvenc",
            Some((12_000, true)),
            Some("12000k cbr"),
            None,
        )
        .unwrap();
        assert_eq!(r.encoder, "libx265");
        assert_eq!(r.preset.name, "high/libx265 @12000k cbr");
        assert!(
            r.preset.video_args.iter().any(|a| a.contains("12000")),
            "{:?}",
            r.preset.video_args
        );
        let h = software_retry("h264", "standard", "h264_nvenc", None, None, Some(4)).unwrap();
        assert_eq!(h.preset.name, "standard");
        assert!(h
            .preset
            .video_args
            .ends_with(&["-threads".to_string(), "4".to_string()]));
    }

    /// Item 96: the HQ record's NVENC out-of-memory becomes a software retry
    /// whose reason names video memory; a filter error does not.
    #[test]
    fn nvenc_out_of_memory_falls_back_and_says_why() {
        let r = software_retry("h264", "high", "h264_nvenc", None, None, None).unwrap();
        let e = CutError::new(
            cut_core::error_codes::FFMPEG,
            "ffmpeg exited with exit status: 244",
            "[h264_nvenc @ 0x1] InitializeEncoder failed: out of memory (10)\nConversion failed!",
        );
        let fb = runtime_fallback(&r, &e).expect("hardware failure");
        assert!(
            fb.message.contains("h264_nvenc") && fb.message.contains("video memory"),
            "{}",
            fb.message
        );
        let report = fallback_report(&r, &fb);
        assert_eq!(report.encoder, "libx264");
        assert!(report.hardware_requested && !report.hardware_used);
        assert!(report
            .fallback_reason
            .unwrap()
            .contains("ran out of video memory"));

        let filter = CutError::new(
            cut_core::error_codes::FFMPEG,
            "x",
            "[fc#0 @ 0x1] Error reinitializing filters!",
        );
        assert!(runtime_fallback(&r, &filter).is_none());
        let both = both_failed(filter, &fb);
        assert!(both.message.contains("software encoding was tried"));
    }
}
