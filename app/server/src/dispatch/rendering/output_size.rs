//! Refuse a render whose output size cannot be encoded, before any work.
//!
//! The output size comes from the project, from explicit width/height, or
//! from the largest source (`resolution:"match_source"`). All three pass the
//! one frame limit in `cut_core::frame_size`. An encoder with its own floor is
//! named too: SVT-AV1 (`libsvtav1`) refuses frames smaller than 64x64.

use cut_core::{error_codes, CutError};

/// Smallest frame libsvtav1 encodes (verified: 32x32 fails, 64x64 works).
const SVT_AV1_MIN_SIDE: u32 = 64;

/// Check the real output size for `encoder` (the resolved `-c:v`).
pub(super) fn check_output_size(
    width: u32,
    height: u32,
    resolution: cut_media::render::Resolution,
    encoder: &str,
) -> Result<(), CutError> {
    let what = match resolution {
        cut_media::render::Resolution::MatchSource => {
            "render output (resolution:match_source, the largest source on the timeline)"
        }
        cut_media::render::Resolution::Explicit { .. } => "render output (width/height)",
        cut_media::render::Resolution::Project => "render output (the project format)",
    };
    cut_core::frame_size::check_frame_size(width, height, what)?;
    if encoder == "libsvtav1" && (width < SVT_AV1_MIN_SIDE || height < SVT_AV1_MIN_SIDE) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "{what} {width}x{height} is too small for AV1: the AV1 encoder (libsvtav1) needs at least {SVT_AV1_MIN_SIDE}x{SVT_AV1_MIN_SIDE}"
            ),
            "SVT-AV1 refuses frames below 64 px on a side",
        )
        .with_suggested_action("choose a larger size, or another format such as h264"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cut_media::render::Resolution;

    #[test]
    fn match_source_beyond_the_limit_names_where_the_size_came_from() {
        let e = check_output_size(12288, 6480, Resolution::MatchSource, "libx264").unwrap_err();
        assert!(e.message.contains("match_source"), "{}", e.message);
        assert!(
            e.message.contains("12288x6480 is too large"),
            "{}",
            e.message
        );
        assert!(check_output_size(8192, 4096, Resolution::MatchSource, "libx264").is_ok());
    }

    #[test]
    fn av1_names_its_own_floor() {
        let e = check_output_size(
            32,
            32,
            Resolution::Explicit {
                width: 32,
                height: 32,
            },
            "libsvtav1",
        )
        .unwrap_err();
        assert!(e.message.contains("libsvtav1"), "{}", e.message);
        assert!(check_output_size(
            32,
            32,
            Resolution::Explicit {
                width: 32,
                height: 32
            },
            "libx264"
        )
        .is_ok());
    }
}
