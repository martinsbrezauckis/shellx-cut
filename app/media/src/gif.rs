//! gif.rs — animated-GIF export from a rendered video clip (cut-media).
//!
//! Role: convert an already-rendered mp4 (a short timeline window) into a
//! high-quality LOOPING GIF. Used by the `export.gif` verb (server) after it
//! renders the chosen range. Primary caller: app/server export_gif.
//!
//! Approach (researched 2026-06): ffmpeg `palettegen` + `paletteuse` in ONE
//! filter graph — the standard dependency-free high-quality path (the tool
//! already requires ffmpeg; gifski would add a binary + larger files, a future
//! quality tier). A naive `-pix_fmt rgb24` GIF quantizes to a generic 256-colour
//! palette with visible banding; a per-clip OPTIMISED palette (`palettegen`) +
//! error-diffusion dithering (`paletteuse`) looks close to the source at a
//! fraction of the size. `fps`/`scale` are the file-size levers (GIFs balloon —
//! 10-15 fps + a scaled width keep them shareable); `-loop 0` = infinite loop.

use cut_core::{error_codes, CutError};
use std::path::Path;

/// Dither mode for `paletteuse` (export.gif `dither`). floyd (default) = error
/// diffusion, smoothest gradients; bayer = ordered, reads as texture (smaller,
/// no shimmer on flat areas); none = hard quantize (smallest, visible banding).
pub fn dither_spec(dither: &str) -> &'static str {
    match dither {
        "bayer" => "bayer:bayer_scale=3",
        "none" => "none",
        _ => "floyd_steinberg", // default + "floyd"
    }
}

/// Build the ffmpeg args (after run_ffmpeg's `-hide_banner -nostdin -y`) that
/// turn `input` into a looping GIF at `out`. `fps` (smoothness/size), `width`
/// (px, height auto-even via `-2`, lanczos for crisp downscale), `dither` (see
/// [`dither_spec`]). `palettegen=stats_mode=diff` optimises the palette for the
/// CHANGED regions across frames (better for talking-head/motion than `full`).
pub fn gif_ffmpeg_args(
    input: &Path,
    out: &Path,
    fps: u32,
    width: u32,
    dither: &str,
) -> Vec<String> {
    let chain = format!(
        "fps={fps},scale={width}:-2:flags=lanczos,split[a][b];\
         [a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither={d}",
        fps = fps,
        width = width,
        d = dither_spec(dither),
    );
    vec![
        "-i".into(),
        input.display().to_string(),
        "-filter_complex".into(),
        chain,
        "-loop".into(),
        "0".into(), // infinite loop (the GIF convention)
        out.display().to_string(),
    ]
}

/// Render `input` (a short rendered mp4) → a looping GIF at `out`. `fps` and
/// `width` are clamped to sane ranges so a bad arg can't produce a multi-GB GIF
/// or a degenerate 0-px frame.
pub fn make_gif(
    input: &Path,
    out: &Path,
    fps: u32,
    width: u32,
    dither: &str,
) -> Result<(), CutError> {
    if !input.exists() {
        return Err(CutError::new(
            error_codes::FFMPEG,
            "gif input not found",
            format!("expected a rendered clip at {}", input.display()),
        ));
    }
    let fps = fps.clamp(2, 50);
    let width = width.clamp(64, 1920);
    crate::ffmpeg::run_ffmpeg(&gif_ffmpeg_args(input, out, fps, width, dither))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn arg_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .map(|s| s.as_str())
    }

    #[test]
    fn dither_spec_maps_modes() {
        assert_eq!(dither_spec("floyd"), "floyd_steinberg");
        assert_eq!(dither_spec(""), "floyd_steinberg"); // default
        assert_eq!(dither_spec("bayer"), "bayer:bayer_scale=3");
        assert_eq!(dither_spec("none"), "none");
    }

    #[test]
    fn gif_args_build_palette_pipeline() {
        let args = gif_ffmpeg_args(
            &PathBuf::from("in.mp4"),
            &PathBuf::from("out.gif"),
            12,
            480,
            "floyd",
        );
        // input + output present, infinite loop, single filter_complex graph.
        assert_eq!(arg_after(&args, "-i"), Some("in.mp4"));
        assert_eq!(arg_after(&args, "-loop"), Some("0"));
        assert_eq!(args.last().map(String::as_str), Some("out.gif"));
        let fc = arg_after(&args, "-filter_complex").unwrap();
        assert!(fc.contains("fps=12"));
        assert!(fc.contains("scale=480:-2:flags=lanczos"));
        assert!(fc.contains("palettegen=stats_mode=diff"));
        assert!(fc.contains("paletteuse=dither=floyd_steinberg"));
        // bayer mode threads through.
        let b = gif_ffmpeg_args(
            &PathBuf::from("in.mp4"),
            &PathBuf::from("o.gif"),
            15,
            360,
            "bayer",
        );
        assert!(arg_after(&b, "-filter_complex")
            .unwrap()
            .contains("dither=bayer:bayer_scale=3"));
    }
}
