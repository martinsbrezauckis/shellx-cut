//! A hardware encoder that fails DURING a render (item 96).
//!
//! The probe in `hwencode` proves an encoder works on an idle machine; it can
//! still fail when the render runs: the graphics card's memory is taken by
//! other programs, the card's limit of simultaneous encode sessions is reached,
//! the driver errors, or this device refuses the frame size or pixel format.
//! Software encoding shares none of those causes, so a render that asked for
//! hardware with `hardware:"auto"` is retried in software and says why.
//!
//! The classifier reads ffmpeg's real stderr. It only reports a cause when an
//! error line comes from the hardware encoder itself (its log context such as
//! `[h264_nvenc @ 0x..]` or `[enc:hevc_qsv @ ..]`) or carries a vendor error
//! that only a GPU path prints; a failure software would share (a missing
//! input, a filter error, a full disk) is left alone.

/// Why a hardware encoder failed at run time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HwRuntimeFailure {
    /// The graphics card had no free video memory for the encoder.
    DeviceOutOfMemory,
    /// The card's limit of encode sessions at the same time was reached.
    SessionLimit,
    /// The driver or the device reported an error or is not available now.
    DriverError,
    /// This device does not accept this frame size or pixel format.
    UnsupportedInput,
}

impl HwRuntimeFailure {
    /// The cause in plain words.
    pub fn reason(self) -> &'static str {
        match self {
            Self::DeviceOutOfMemory => "the graphics card ran out of video memory",
            Self::SessionLimit => {
                "the graphics card is already running as many video encodes as it allows"
            }
            Self::DriverError => "the graphics driver reported an error",
            Self::UnsupportedInput => {
                "the graphics card does not accept this frame size or pixel format"
            }
        }
    }

    /// What the person can do when software is not used instead.
    pub fn action(self) -> &'static str {
        match self {
            Self::DeviceOutOfMemory | Self::SessionLimit => {
                "Close other programs that use the graphics card (games, other video \
                 apps, AI tools), or turn Faster exports off to use software encoding."
            }
            Self::DriverError => {
                "Update or restart the graphics driver, or turn Faster exports off to use \
                 software encoding."
            }
            Self::UnsupportedInput => {
                "Turn Faster exports off to use software encoding, or choose HEVC, whose \
                 hardware encoder accepts larger frames."
            }
        }
    }
}

const HW_SUFFIXES: [&str; 4] = ["_nvenc", "_qsv", "_amf", "_videotoolbox"];

/// Does this stderr line come from a hardware encoder's own log context?
fn from_hw_encoder(line: &str) -> bool {
    HW_SUFFIXES.iter().any(|s| {
        // "[h264_nvenc @ 0x..]" or "[enc:h264_nvenc @ 0x..]" / "[vost#0:0/h264_nvenc @"
        line.contains(&format!("{s} @"))
    })
}

/// Up to `max` stderr lines from any hardware encoder's own log context, in
/// order.
pub fn hardware_lines(stderr: &str, max: usize) -> Vec<&str> {
    stderr
        .lines()
        .map(str::trim)
        .filter(|l| from_hw_encoder(l))
        .take(max)
        .collect()
}

/// Sort a failed render's stderr into a hardware cause, or None when the
/// failure is not the hardware encoder's.
pub fn classify_runtime_failure(stderr: &str) -> Option<HwRuntimeFailure> {
    let mut kind = None;
    for line in stderr.lines() {
        let low = line.to_ascii_lowercase();
        let hw_line = from_hw_encoder(line);
        let found = if low.contains("cuda_error_out_of_memory")
            || low.contains("mfx_err_memory_alloc")
            || low.contains("amf_out_of_memory")
            || (hw_line && (low.contains("out of memory") || low.contains("memory allocation")))
        {
            Some(HwRuntimeFailure::DeviceOutOfMemory)
        } else if hw_line
            && (low.contains("incompatible client key")
                || low.contains("too many concurrent")
                || low.contains("session limit")
                || low.contains("maximum number of sessions"))
        {
            Some(HwRuntimeFailure::SessionLimit)
        } else if hw_line
            && (low.contains("exceeds")
                || low.contains("dimension")
                || low.contains("unsupported pixel format")
                || low.contains("not supported for this")
                || low.contains("invalid param")
                || low.contains("-12902"))
        {
            Some(HwRuntimeFailure::UnsupportedInput)
        } else if low.contains("cuda_error")
            || low.contains("mfx_err_device_failed")
            || low.contains("mfx_err_device_lost")
            || (hw_line
                && (low.contains("no capable devices")
                    || low.contains("cannot create compression session")
                    || low.contains("encoder malfunction")
                    || low.contains("-12915")
                    || low.contains("device failed")
                    || low.contains("driver")
                    || low.contains("failed to initialise")
                    || low.contains("failed to initialize")
                    || low.contains("openencodesessionex failed")
                    || low.contains("initializeencoder failed")
                    || low.contains("error while opening encoder")
                    || low.contains("error encoding")))
        {
            Some(HwRuntimeFailure::DriverError)
        } else {
            None
        };
        // The most specific cause wins: memory and session limits over the
        // generic open failure that ffmpeg prints after them.
        kind = match (kind, found) {
            (None, f) => f,
            (Some(HwRuntimeFailure::DriverError), Some(f)) => Some(f),
            (k, _) => k,
        };
    }
    kind
}

/// ffmpeg's own lines from the hardware encoder (up to three, joined), for the
/// technical detail; the last 300 characters when it printed none.
pub fn runtime_detail(encoder: &str, stderr: &str) -> String {
    let context = format!("{encoder} @");
    let own: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| l.contains(&context) && !l.is_empty())
        .take(3)
        .collect();
    if own.is_empty() {
        let n = stderr.chars().count();
        stderr.chars().skip(n.saturating_sub(300)).collect()
    } else {
        own.join(" / ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Item 96: the HQ record's NVENC failure with 0.9 GB of video memory free.
    #[test]
    fn nvenc_out_of_memory_from_the_hq_record() {
        let stderr = "[h264_nvenc @ 0x5d1] OpenEncodeSessionEx failed: out of memory (10): (no details)\n\
                      [h264_nvenc @ 0x5d1] InitializeEncoder failed: out of memory (10)\n\
                      [vost#0:0/h264_nvenc @ 0x5c9] [enc:h264_nvenc @ 0x5d0] Error while opening encoder - maybe incorrect parameters such as bit_rate, rate, width or height.\n\
                      Conversion failed!";
        assert_eq!(
            classify_runtime_failure(stderr),
            Some(HwRuntimeFailure::DeviceOutOfMemory)
        );
        let detail = runtime_detail("h264_nvenc", stderr);
        assert!(
            detail.starts_with("[h264_nvenc @ 0x5d1] OpenEncodeSessionEx failed: out of memory"),
            "{detail}"
        );
        assert!(!detail.contains("Conversion failed"), "{detail}");
    }

    #[test]
    fn vendor_messages_are_sorted() {
        let cases = [
            ("[hevc_nvenc @ 0x1] OpenEncodeSessionEx failed: incompatible client key (21): (no details)", HwRuntimeFailure::SessionLimit),
            ("[h264_nvenc @ 0x1] Width 7680 exceeds 4096\n[h264_nvenc @ 0x1] No capable devices found", HwRuntimeFailure::UnsupportedInput),
            ("[h264_qsv @ 0x1] Error during encoding: memory allocation error (-4) MFX_ERR_MEMORY_ALLOC", HwRuntimeFailure::DeviceOutOfMemory),
            ("[hevc_qsv @ 0x1] Error initializing the encoder: device failed (-17)", HwRuntimeFailure::DriverError),
            ("[h264_amf @ 0x1] CreateComponent(AMFVideoEncoderVCE_AVC) failed with error 13: AMF_OUT_OF_MEMORY", HwRuntimeFailure::DeviceOutOfMemory),
            ("[h264_amf @ 0x1] AMF failed to initialise on the given D3D11 device", HwRuntimeFailure::DriverError),
            ("[h264_videotoolbox @ 0x1] Error: cannot create compression session: -12915", HwRuntimeFailure::DriverError),
            ("[hevc_videotoolbox @ 0x1] Error: cannot create compression session: -12902", HwRuntimeFailure::UnsupportedInput),
        ];
        for (stderr, want) in cases {
            assert_eq!(classify_runtime_failure(stderr), Some(want), "{stderr}");
        }
    }

    /// Failures software would share are not the hardware encoder's.
    #[test]
    fn other_failures_are_not_hardware() {
        for stderr in [
            "media.mp4: No such file or directory",
            "[fc#0 @ 0x1] Error reinitializing filters!\nFailed to inject frame into filter network: Out of memory",
            "[libx264 @ 0x1] malloc of size 1 failed",
            "av_interleaved_write_frame(): No space left on device",
        ] {
            assert_eq!(classify_runtime_failure(stderr), None, "{stderr}");
        }
    }
}
