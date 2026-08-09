//! poster.rs — single-frame poster / waveform thumbnails for arbitrary media
//! files (the global asset Library panel).
//!
//! Role: the Library lists cross-project media by file PATH (not a project asset
//! id), so the project-scoped `/filmstrip` + `/api/frame` routes — which resolve
//! through the OPEN project's asset registry — can't thumbnail them. This module
//! renders ONE representative still per source into a caller-keyed cache:
//!   * video → a representative FRAME (ffmpeg `thumbnail` filter),
//!   * image → a scaled-down still,
//!   * audio → a static WAVEFORM strip (`showwavespic`; a frame can't exist).
//! So a library card shows real content instead of a flat kind glyph.
//!
//! Idempotent — an existing cache file is returned untouched. Callers key the
//! output FILENAME by (source path + mtime), so an unchanged source is a cache
//! hit and a changed/replaced source re-renders. Bounded cost: the video recipe
//! only reads the opening frame batch (`-frames:v 1` stops it).
//!
//! Callers: server `serve_library_poster` (GET /api/library-poster). Deps: ffmpeg
//! (via [`crate::ffmpeg::run_ffmpeg`] — the single ffmpeg spawn point).

use cut_core::CutError;
use std::path::{Path, PathBuf};

/// Which poster recipe to render for a source — selected from the library item's
/// probed `kind` by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PosterKind {
    /// A representative video FRAME. `thumbnail` analyses the opening batch and
    /// emits the most-representative frame — robust on short clips and black
    /// intros, where a fixed `-ss` seek would miss or land on black.
    Video,
    /// A scaled-down still of an IMAGE source (linked images have no blob to serve
    /// directly, so they need a rendered thumbnail too).
    Image,
    /// A static WAVEFORM strip (`showwavespic`) — audio has no frame to grab.
    Audio,
}

/// Default poster image height (px). Video/image posters are this tall (width
/// auto, kept even for chroma subsampling); the audio waveform uses it as the
/// strip height. 2× a typical ~72px card thumb for crispness on HiDPI displays.
pub const POSTER_HEIGHT: u32 = 144;

/// Render a poster for `src` into `out` and return `out`. `out` is the FULL target
/// path including extension — `.jpg` for video/image, `.png` for the waveform — so
/// ffmpeg infers the muxer from it and the caller owns the cache key. Idempotent:
/// an existing `out` is returned without re-running ffmpeg. `height` is clamped to
/// a sane band. ffmpeg failure (unreadable / non-media source) propagates as a
/// `CutError` so the HTTP layer can map it to a 404 + glyph fallback.
pub fn make_poster(
    src: &Path,
    out: &Path,
    kind: PosterKind,
    height: u32,
) -> Result<PathBuf, CutError> {
    if out.exists() {
        if crate::image_cache::existing_image_cache_is_complete(out) {
            return Ok(out.to_path_buf());
        }
        let _ = std::fs::remove_file(out);
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let h = height.clamp(48, 480);
    let src_s = src.display().to_string();
    let out_s = out.display().to_string();
    let args: Vec<String> = match kind {
        PosterKind::Video => vec![
            "-i".into(),
            src_s,
            "-vf".into(),
            format!("thumbnail,scale=-2:{h}"),
            "-frames:v".into(),
            "1".into(),
            "-q:v".into(),
            "4".into(), // mid JPEG quality — small file, fine for a card thumb
            out_s,
        ],
        PosterKind::Image => vec![
            "-i".into(),
            src_s,
            "-vf".into(),
            format!("scale=-2:{h}"),
            "-frames:v".into(),
            "1".into(),
            "-q:v".into(),
            "4".into(),
            out_s,
        ],
        PosterKind::Audio => {
            // Width ≈ 16:9 of the height so the strip fills the card thumb area.
            // Neutral slate fill — intentionally restrained and easy to tune later.
            let w = (h * 16 / 9).max(64);
            vec![
                "-i".into(),
                src_s,
                "-filter_complex".into(),
                format!("showwavespic=s={w}x{h}:colors=0x8a93a6"),
                "-frames:v".into(),
                "1".into(),
                out_s,
            ]
        }
    };
    crate::ffmpeg::run_ffmpeg_atomic_output(&args, out)?;
    Ok(out.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_output_is_a_cache_hit() {
        // An already-rendered poster is returned as-is WITHOUT spawning ffmpeg —
        // proven here by pointing `src` at a path that does not exist: if ffmpeg
        // ran it would error, but the present `out` short-circuits before that.
        let dir = std::env::temp_dir().join(format!("cut_poster_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("hit.jpg");
        std::fs::write(&out, [0xff, 0xd8, 0xff, 0xd9]).unwrap();
        let got = make_poster(
            Path::new("/no/such/source.mp4"),
            &out,
            PosterKind::Video,
            144,
        );
        assert_eq!(got.unwrap(), out);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn incomplete_existing_poster_is_not_a_cache_hit() {
        let dir =
            std::env::temp_dir().join(format!("cut_poster_partial_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("partial.jpg");
        std::fs::write(&out, [0xff, 0xd8]).unwrap();
        let got = make_poster(
            Path::new("/no/such/source.mp4"),
            &out,
            PosterKind::Video,
            144,
        );
        assert!(
            got.is_err(),
            "stale partial poster should be rejected and rerender attempted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
