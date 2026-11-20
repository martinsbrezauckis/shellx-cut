//! The software retry passes the same memory and disk check as the first
//! attempt (item 132).
//!
//! The render's preflight ran once, for the HARDWARE preset. A hardware
//! encoder keeps far less in system memory than libx264 (8K: about 6 GB with
//! NVENC against about 12 GB in software), so a retry that skipped the check
//! could start a software render this computer cannot finish. The retry now
//! runs the preflight with the software preset: a refusal ends the job with an
//! error that names both facts (the hardware failure and what software would
//! need); a short-memory adaptation pins the encoder threads as on a first
//! attempt; warnings are added to the one progress message.

use super::hw_fallback::SoftwareRetry;
use super::preflight::Preflight;
use cut_core::{error_codes, CutError};
use cut_media::hwencode::SoftwareFallback;
use cut_media::render::RenderPreset;

/// The preset to retry with, or the error that ends the job instead.
pub(super) fn checked_retry(
    retry: &SoftwareRetry,
    fallback: &SoftwareFallback,
    check: &Preflight,
) -> Result<RenderPreset, CutError> {
    if let Some(refusal) = check.refusal.as_ref() {
        let code = if refusal.code == error_codes::NOT_ENOUGH_SPACE {
            error_codes::NOT_ENOUGH_SPACE
        } else {
            error_codes::OUT_OF_MEMORY
        };
        let mut error = CutError::new(
            code,
            format!(
                "the hardware encoder ({}) failed, and the software encoder ({}) cannot \
                 make this export here: {}",
                retry.hardware_encoder, retry.encoder, refusal.message
            ),
            format!(
                "hardware: {}; software: {}",
                fallback.message, refusal.cause
            ),
        );
        error.suggested_action = refusal.suggested_action.clone();
        return Err(error);
    }
    let mut preset = retry.preset.clone();
    if let Some(n) = check.adapt_threads {
        if !preset.video_args.iter().any(|a| a == "-threads") {
            preset
                .video_args
                .extend(["-threads".to_string(), n.to_string()]);
        }
    }
    Ok(preset)
}

/// The one progress message before the software pass: the hardware reason
/// plus the thread reduction (item 165) and any memory or disk warning of the
/// software check.
pub(super) fn retry_message(fallback: &SoftwareFallback, check: &Preflight) -> String {
    let mut message = fallback.message.clone();
    if let Some(note) = check.note.as_deref() {
        message.push(' ');
        message.push_str(note);
    }
    for warning in &check.warnings {
        message.push(' ');
        message.push_str(&warning.message);
    }
    message
}

#[cfg(test)]
mod tests {
    use super::super::hw_fallback::software_retry;
    use super::super::preflight::{decide, Machine, Plan};
    use super::*;
    use cut_media::render::resources::MemoryEstimate;
    use std::path::Path;

    fn fallback() -> SoftwareFallback {
        SoftwareFallback {
            message: "The hardware encoder (h264_nvenc) failed during this export: the \
                      graphics card ran out of video memory."
                .into(),
            detail: None,
        }
    }

    fn check(available_mb: u64, total_mb: u64, software_mb: u64) -> (SoftwareRetry, Preflight) {
        let retry = software_retry("h264", "high", "h264_nvenc", None, None, None).unwrap();
        let mut project = cut_core::Project::new("t", cut_core::ProjectSettings::default());
        project.settings.fps = 60.0;
        let edl = cut_core::Edl {
            duration_ms: 12_000,
            ..Default::default()
        };
        let plan = Plan {
            project: &project,
            edl: &edl,
            width: 7680,
            height: 4320,
            video_args: &retry.preset.video_args,
            audio_kbps: 192,
            out: Path::new("/x/exports/render_001.mp4"),
        };
        let machine = Machine {
            total_mb: Some(total_mb),
            available_mb: Some(available_mb),
            swap_mb: 0,
            disk_free_mb: Some(500_000),
            limit_mb: None,
        };
        let pf = decide(&plan, &machine, Path::new("/x"), |args| MemoryEstimate {
            total_mb: if args.iter().any(|a| a == "-threads") {
                software_mb - 2_000
            } else {
                software_mb
            },
            encoder_threads: Some(12),
            ..Default::default()
        });
        (retry, pf)
    }

    /// 8K H.264: NVENC failed; libx264 needs about 12 GB on an 8 GB machine.
    #[test]
    fn refusal_names_the_hardware_failure_and_the_software_need() {
        let (retry, pf) = check(6_000, 8_192, 12_000);
        let err = checked_retry(&retry, &fallback(), &pf).unwrap_err();
        assert_eq!(err.code, error_codes::OUT_OF_MEMORY);
        assert!(
            err.message.contains("hardware encoder (h264_nvenc) failed"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("software encoder (libx264)"),
            "{}",
            err.message
        );
        assert!(err.message.contains("9.8 GB"), "{}", err.message);
        assert!(err.cause.contains("video memory"), "{}", err.cause);
        assert!(err.suggested_action.is_some());
    }

    /// Short but possible: the retry pins 4 encoder threads and says so.
    #[test]
    fn short_memory_adapts_the_retry_and_warns() {
        let (retry, pf) = check(9_000, 64_000, 12_000);
        let preset = checked_retry(&retry, &fallback(), &pf).unwrap();
        assert!(preset
            .video_args
            .ends_with(&["-threads".to_string(), "4".to_string()]));
        let message = retry_message(&fallback(), &pf);
        assert!(
            message.contains("video memory") && message.contains("of memory"),
            "{message}"
        );
    }
}
