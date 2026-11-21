//! Which H.264 encoder makes the constant-rate parts.

use crate::MediaFacts;

/// Encoders to try, in order. macOS: x264 `veryfast` first. Measured on
/// a 10-core Mac with real 3840x2160 ScreenCaptureKit segments (one 15.6 s part):
/// still screen 2.0 s (0.9 MB) against VideoToolbox 7.6 s (12.7 MB) and the
/// old default preset 5.5 s; light motion 2.9 s against 7.3 s and 13.1 s.
/// Repeated frames cost x264 almost nothing; VideoToolbox encodes each one in
/// full, so it is only the fallback. Other systems keep their previous preset.
///
/// At 60 fps, use the faster software preset so a completed part is ready
/// promptly while capture continues. The admitted frame grid still owns timing.
///
/// No B-frames anywhere: with B-frames the MP4 muxer writes an edit list whose
/// length is rounded to whole milliseconds, the concat then started each next
/// part up to a millisecond early and the take's average rate read 30.0001
/// instead of 30 (a false "uneven frame timing" note on real 4K takes of
/// an early 0.6.116 build). Screen content loses nothing measurable without them.
pub(crate) fn encoder_choices(
    media: &MediaFacts,
    cadence: Option<record_core::FrameRate>,
) -> Vec<Vec<String>> {
    let software = |extra: &[&str]| {
        let mut args = vec!["-c:v".to_string(), "libx264".to_string()];
        args.extend(["-bf".to_string(), "0".to_string()]);
        args.extend(extra.iter().map(|arg| arg.to_string()));
        args.extend(["-pix_fmt".to_string(), "yuv420p".to_string()]);
        args
    };
    #[cfg(target_os = "macos")]
    {
        let hardware = cut_media::hwencode::hw_codec_args(
            "h264",
            2,
            media.width.unwrap_or(0),
            media.height.unwrap_or(0),
        )
        .map(|(args, _)| args);
        let preset =
            if cadence.is_some_and(|rate| u128::from(rate.num) >= 60 * u128::from(rate.den)) {
                "superfast"
            } else {
                "veryfast"
            };
        std::iter::once(software(&["-preset", preset]))
            .chain(hardware)
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (media, cadence);
        vec![software(&[])]
    }
}
