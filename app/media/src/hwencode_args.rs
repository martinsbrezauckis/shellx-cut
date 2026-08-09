//! Hardware encoder argument selection, isolated from live ffmpeg probing.
//!
//! The live probe in `hwencode` answers whether an encoder exists on this
//! machine. This module combines that answer with the target geometry and
//! produces its backend-specific arguments. Keeping the decision pure makes a
//! size-limited encoder fallback testable without an NVIDIA host.

/// Return hardware-video arguments for a probe-verified encoder, or `None` to
/// keep the caller's software arguments. A nonzero geometry must pass the
/// supplied capability check: an encoder that can make the tiny existence probe
/// may still reject the requested frame size.
pub(crate) fn args_for(
    codec: &str,
    encoder: &str,
    quality: usize,
    width: u32,
    height: u32,
    supports_size: impl FnOnce(&str, u32, u32) -> bool,
) -> Option<(Vec<String>, &'static str)> {
    let codec = match codec {
        "h264" | "mp4" => "h264",
        "hevc" | "h265" => "hevc",
        "av1" => "av1",
        _ => return None,
    };
    if !encoder.starts_with(&format!("{codec}_")) {
        return None;
    }
    if width > 0 && height > 0 && !supports_size(encoder, width, height) {
        return None;
    }

    let s = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let qi = quality.min(2);
    let mut args: Vec<String> = if encoder.ends_with("_nvenc") {
        let cq = ["32", "27", "23"][qi];
        s(&[
            "-c:v", encoder, "-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", cq, "-b:v", "0",
            "-pix_fmt", "yuv420p",
        ])
    } else if encoder.ends_with("_qsv") {
        let gq = ["32", "27", "23"][qi];
        s(&[
            "-c:v",
            encoder,
            "-global_quality",
            gq,
            "-preset",
            "medium",
            "-pix_fmt",
            "nv12",
        ])
    } else if encoder.ends_with("_amf") {
        let qp = ["30", "26", "22"][qi];
        s(&[
            "-c:v", encoder, "-rc", "cqp", "-qp_i", qp, "-qp_p", qp, "-quality", "quality",
            "-pix_fmt", "yuv420p",
        ])
    } else if encoder.ends_with("_videotoolbox") {
        let qv = ["40", "55", "65"][qi];
        s(&["-c:v", encoder, "-q:v", qv, "-pix_fmt", "yuv420p"])
    } else {
        return None;
    };
    if codec == "hevc" {
        args.extend(["-tag:v".into(), "hvc1".into()]);
    }
    Some((args, "mp4"))
}

#[cfg(test)]
mod tests {
    use super::args_for;

    #[test]
    fn h264_nvenc_at_8k_declines_hardware_and_keeps_software_fallback_available() {
        let result = args_for(
            "h264",
            "h264_nvenc",
            1,
            7680,
            4320,
            |encoder, width, height| {
                assert_eq!((encoder, width, height), ("h264_nvenc", 7680, 4320));
                false // Reproduces the real H.264 NVENC 8K capability rejection.
            },
        );
        assert_eq!(
            result, None,
            "caller must retain libx264 instead of failing late"
        );
    }

    #[test]
    fn hevc_can_use_the_same_nvidia_host_when_its_8k_probe_succeeds() {
        let (args, ext) = args_for("hevc", "hevc_nvenc", 2, 7680, 4320, |_, _, _| true)
            .expect("a probe-confirmed HEVC encoder should be selected");
        assert_eq!(ext, "mp4");
        assert!(args.windows(2).any(|pair| pair == ["-c:v", "hevc_nvenc"]));
        assert!(args.windows(2).any(|pair| pair == ["-tag:v", "hvc1"]));
    }

    #[test]
    fn av1_uses_a_compatible_hardware_encoder_without_hevc_container_tag() {
        let (args, ext) = args_for("av1", "av1_qsv", 0, 3840, 2160, |_, _, _| true)
            .expect("a probe-confirmed AV1 encoder should be selected");
        assert_eq!(ext, "mp4");
        assert!(args.windows(2).any(|pair| pair == ["-c:v", "av1_qsv"]));
        assert!(!args.windows(2).any(|pair| pair == ["-tag:v", "hvc1"]));
    }

    #[test]
    fn software_only_or_mismatched_formats_never_select_hardware_arguments() {
        for codec in ["vp9", "prores", "h264"] {
            let encoder = if codec == "h264" {
                "hevc_nvenc"
            } else {
                "h264_nvenc"
            };
            assert!(args_for(codec, encoder, 1, 1920, 1080, |_, _, _| true).is_none());
        }
    }
}
