//! The largest picture Cut makes, in one place.
//!
//! Every frame size that reaches an encoder passes this check: the project
//! format (`project.create` settings, `project.format`, the first-import
//! adoption) and a render's own output size (`render.final` width/height and
//! `resolution:"match_source"`). The limit is the one every software encoder
//! Cut uses can write and common players read: at most 8192 px on either side
//! and at most 8192x4320 pixels in total (DCI 8K, the largest 8K camera frame;
//! H.264 level 6.2, its highest level, tops out at 139,264 macroblocks, which
//! 8192x4320 fits). Hardware encoders can have lower limits; those are probed
//! per render and fall back to software with the reason on the receipt.

use crate::error::{codes, CutError};

mod fit;
pub use fit::{fit_frame, fit_project_formats, FormatFit};

/// Smallest frame side Cut makes.
pub const MIN_FRAME_SIDE: u32 = 16;
/// Largest frame side Cut makes (either orientation).
pub const MAX_FRAME_SIDE: u32 = 8192;
/// Largest frame area: 8192x4320 (DCI 8K).
pub const MAX_FRAME_PIXELS: u64 = 8192 * 4320;

/// The limit in words, for messages and schema text.
pub const FRAME_LIMIT_TEXT: &str =
    "16 to 8192 px on each side and at most 8192x4320 pixels in total";

/// Check a frame size. `what` names the thing being sized ("project format",
/// "render output") so the person knows which setting to change.
pub fn check_frame_size(width: u32, height: u32, what: &str) -> Result<(), CutError> {
    let side_ok = (MIN_FRAME_SIDE..=MAX_FRAME_SIDE).contains(&width)
        && (MIN_FRAME_SIDE..=MAX_FRAME_SIDE).contains(&height);
    let area_ok = u64::from(width) * u64::from(height) <= MAX_FRAME_PIXELS;
    if side_ok && area_ok {
        return Ok(());
    }
    let why = if width < MIN_FRAME_SIDE || height < MIN_FRAME_SIDE {
        "too small"
    } else {
        "too large"
    };
    Err(CutError::new(
        codes::INVALID_ARGS,
        format!("{what} {width}x{height} is {why}: Cut makes frames of {FRAME_LIMIT_TEXT}"),
        "8192x4320 is the largest frame H.264 (level 6.2) allows and the largest that \
         every encoder Cut uses can write",
    )
    .with_suggested_action(
        "choose a size within the limit, for example 7680x4320 (8K UHD), 8192x4320 (DCI 8K) or 3840x2160",
    ))
}

/// Check a PROJECT format: the frame limit, and even sides, because every
/// export is 4:2:0 video whose encoders refuse odd widths and heights (an odd
/// project format failed every render at the pad/encoder stage).
pub fn check_project_frame_size(width: u32, height: u32) -> Result<(), CutError> {
    check_frame_size(width, height, "project format")?;
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Err(CutError::new(
            codes::INVALID_ARGS,
            format!("project format {width}x{height} has an odd side: width and height must be even numbers"),
            "exports are 4:2:0 video, which needs even frame sides",
        )
        .with_suggested_action(format!(
            "use {}x{}",
            width - width % 2,
            height - height % 2
        )));
    }
    Ok(())
}

/// The even size just below `width`x`height` (an odd side loses one pixel).
pub fn even_size(width: u32, height: u32) -> (u32, u32) {
    (width - width % 2, height - height % 2)
}

/// The refusal for a video export at an odd project size. Nothing is rounded
/// quietly: the person is told first, and the fix is one `project.format` call
/// (in `suggested_action`, exactly as an agent sends it).
pub fn odd_project_size_error(width: u32, height: u32) -> CutError {
    let (ew, eh) = even_size(width, height);
    CutError::new(
        codes::ODD_PROJECT_SIZE,
        format!(
            "This project is {width}x{height}, but video exports need an even width and height. \
             Change the project to {ew}x{eh}"
        ),
        "video is exported as 4:2:0, where width and height must be even; Cut does not change \
         the size during an export",
    )
    .with_suggested_action(format!("project.format {{\"width\":{ew},\"height\":{eh}}}"))
}

/// Refuse a video export at an odd project size (see [`odd_project_size_error`]).
pub fn require_even_project(width: u32, height: u32) -> Result<(), CutError> {
    if width.is_multiple_of(2) && height.is_multiple_of(2) {
        Ok(())
    } else {
        Err(odd_project_size_error(width, height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_k_frames_including_the_nasa_8192x4096_fit() {
        for (w, h) in [
            (7680, 4320),
            (8192, 4320),
            (8192, 4096),
            (4320, 7680),
            (16, 16),
        ] {
            assert!(check_frame_size(w, h, "frame").is_ok(), "{w}x{h}");
        }
    }

    #[test]
    fn an_odd_project_export_is_refused_with_the_exact_call() {
        let e = require_even_project(641, 361).unwrap_err();
        assert_eq!(e.code, codes::ODD_PROJECT_SIZE);
        assert_eq!(
            e.message,
            "This project is 641x361, but video exports need an even width and height. Change \
             the project to 640x360"
        );
        assert_eq!(
            e.suggested_action.as_deref(),
            Some(r#"project.format {"width":640,"height":360}"#)
        );
        assert!(require_even_project(640, 360).is_ok());
        assert_eq!(even_size(641, 360), (640, 360));
    }

    #[test]
    fn a_project_format_must_have_even_sides() {
        assert!(check_project_frame_size(8192, 4096).is_ok());
        let e = check_project_frame_size(641, 361).unwrap_err();
        assert!(e.message.contains("odd side"), "{}", e.message);
        assert_eq!(e.suggested_action.as_deref(), Some("use 640x360"));
    }

    #[test]
    fn oversize_and_undersize_frames_name_the_limit() {
        for (w, h) in [(8194, 4320), (8192, 8192), (12288, 6480), (8, 720)] {
            let e = check_frame_size(w, h, "render output").unwrap_err();
            assert_eq!(e.code, codes::INVALID_ARGS);
            assert!(
                e.message
                    .starts_with(&format!("render output {w}x{h} is too")),
                "{}",
                e.message
            );
            assert!(e.message.contains("8192x4320"), "{}", e.message);
            assert!(e.suggested_action.is_some());
        }
    }
}
