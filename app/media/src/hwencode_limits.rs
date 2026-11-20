//! The largest frame each working hardware encoder accepts on this machine.
//!
//! Hardware encoders have their own size ceilings (NVIDIA's H.264 engine stops
//! at 4096 px wide while its HEVC engine encodes 8K). The render already asks
//! the machine before it uses an encoder (`hwencode::encoder_supports_size`);
//! this module walks a short ladder of common delivery sizes with the same
//! cached probe, so the doctor card, the render plan and the fallback message
//! can name the limit instead of saying only "too large".

use crate::hwencode::{encoder_supports_size, hw_caps};

/// Delivery sizes checked in order, smallest first: UHD, DCI 4K, 8K UHD, DCI 8K.
/// Every working encoder already passed a small probe, so HD is not repeated.
pub const SIZE_LADDER: &[(u32, u32)] = &[(3840, 2160), (4096, 2160), (7680, 4320), (8192, 4320)];

/// What one encoder accepted on the ladder.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EncoderSizeLimit {
    pub encoder: String,
    /// Largest ladder size the encoder accepted ("WxH"); None when it refused
    /// even the first one (it then works only below UHD).
    pub largest_accepted: Option<String>,
    /// First ladder size it refused ("WxH"); None when it accepted all of them.
    pub smallest_refused: Option<String>,
}

/// Walk the ladder with `supports` until the first refusal. Sizes above a
/// refused one are not tried: a larger frame of the same shape is never easier.
pub fn limit_with(encoder: &str, supports: impl Fn(&str, u32, u32) -> bool) -> EncoderSizeLimit {
    let mut largest = None;
    let mut refused = None;
    for &(w, h) in SIZE_LADDER {
        if supports(encoder, w, h) {
            largest = Some(format!("{w}x{h}"));
        } else {
            refused = Some(format!("{w}x{h}"));
            break;
        }
    }
    EncoderSizeLimit {
        encoder: encoder.to_string(),
        largest_accepted: largest,
        smallest_refused: refused,
    }
}

/// The ladder result for every working hardware encoder of the ffmpeg renders
/// use. Each probe answer is cached per tools generation, so only the first
/// call pays for the test encodes.
pub fn size_limits() -> Vec<EncoderSizeLimit> {
    let caps = hw_caps();
    [&caps.h264, &caps.hevc, &caps.av1]
        .into_iter()
        .flatten()
        .map(|enc| limit_with(enc, encoder_supports_size))
        .collect()
}

/// A plain phrase for the size limit of an encoder that refused `(w, h)`:
/// "accepts at most 4096x2160" when a smaller ladder size passed, else
/// "cannot encode WxH". Used in the software-fallback sentence.
pub fn limit_phrase(limit: &EncoderSizeLimit, w: u32, h: u32) -> String {
    match &limit.largest_accepted {
        Some(max) => format!("accepts at most {max}"),
        None => format!("cannot encode {w}x{h}"),
    }
}

/// The other hardware codec that CAN encode `(w, h)` on this machine, for the
/// advice "choose HEVC". Only H.264 -> HEVC/AV1 is suggested: those are the
/// formats whose hardware engines reach 8K on current GPUs.
pub fn hardware_alternative(codec: &str, w: u32, h: u32) -> Option<(&'static str, String)> {
    alternative_with(
        codec,
        w,
        h,
        |c| hw_caps().for_codec(c).map(str::to_string),
        encoder_supports_size,
    )
}

pub(crate) fn alternative_with(
    codec: &str,
    w: u32,
    h: u32,
    encoder_for: impl Fn(&str) -> Option<String>,
    supports: impl Fn(&str, u32, u32) -> bool,
) -> Option<(&'static str, String)> {
    let options: &[(&str, &'static str)] = match codec {
        "h264" | "mp4" => &[("hevc", "HEVC"), ("av1", "AV1")],
        "hevc" | "h265" => &[("av1", "AV1")],
        _ => &[],
    };
    options.iter().find_map(|(c, label)| {
        let enc = encoder_for(c)?;
        supports(&enc, w, h).then_some((*label, enc))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ladder stops at the first refusal and names both ends, the shape
    /// the RTX 5080 gives for h264_nvenc (4096 wide at most).
    #[test]
    fn ladder_names_largest_accepted_and_first_refused() {
        let nvenc_h264 = |_: &str, w: u32, _: u32| w <= 4096;
        let lim = limit_with("h264_nvenc", nvenc_h264);
        assert_eq!(lim.largest_accepted.as_deref(), Some("4096x2160"));
        assert_eq!(lim.smallest_refused.as_deref(), Some("7680x4320"));
        assert_eq!(limit_phrase(&lim, 7680, 4320), "accepts at most 4096x2160");

        let all = limit_with("hevc_nvenc", |_, _, _| true);
        assert_eq!(all.largest_accepted.as_deref(), Some("8192x4320"));
        assert_eq!(all.smallest_refused, None);

        let none = limit_with("h264_x", |_, _, _| false);
        assert_eq!(none.largest_accepted, None);
        assert_eq!(limit_phrase(&none, 3840, 2160), "cannot encode 3840x2160");
    }

    /// 8K H.264 that the hardware cannot do: HEVC is suggested only when the
    /// hardware HEVC encoder really accepts the size.
    #[test]
    fn hevc_is_suggested_only_when_it_fits() {
        let enc = |c: &str| match c {
            "hevc" => Some("hevc_nvenc".to_string()),
            "av1" => Some("av1_nvenc".to_string()),
            _ => None,
        };
        let alt = alternative_with("h264", 7680, 4320, enc, |e, _, _| e == "hevc_nvenc");
        assert_eq!(alt, Some(("HEVC", "hevc_nvenc".to_string())));
        let alt = alternative_with("h264", 7680, 4320, enc, |e, _, _| e == "av1_nvenc");
        assert_eq!(alt, Some(("AV1", "av1_nvenc".to_string())));
        assert_eq!(
            alternative_with("h264", 7680, 4320, enc, |_, _, _| false),
            None
        );
        assert_eq!(
            alternative_with("vp9", 7680, 4320, enc, |_, _, _| true),
            None
        );
    }
}
