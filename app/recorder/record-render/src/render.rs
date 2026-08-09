//! render.rs — high-level render entry points (decode → compose → encode).
//!
//! `render_video` streams a source MP4 through the compositor into a polished
//! MP4; `render_frame_png` bakes a single composed frame to PNG (fast visual /
//! golden check without a full encode). Both delegate pixels to `compose_frame`
//! and I/O to `ffmpeg`.

use record_core::{error_codes, EditPlan, RecordError, Result};
use tiny_skia::{IntSize, Pixmap};

use crate::{compose_frame, ffmpeg, output_size};

/// Render the full polished video: decode `source_path`, compose every frame per
/// `plan`, encode to `out_path` (MP4). Returns the frame count written.
pub fn render_video(source_path: &str, plan: &EditPlan, out_path: &str) -> Result<u64> {
    render_video_audio(source_path, plan, out_path, None)
}

/// Like `render_video`, but muxes `audio` (e.g. a recorded mic WAV) into the
/// output; falls back to the source's own audio track when `audio` is None.
pub fn render_video_audio(
    source_path: &str,
    plan: &EditPlan,
    out_path: &str,
    audio: Option<&str>,
) -> Result<u64> {
    render_video_audio_with_control(
        source_path,
        plan,
        out_path,
        audio,
        &ffmpeg::ProcessControl::bounded(std::time::Duration::from_secs(30 * 60), || false),
    )
}

/// Cancellable/bounded form used by the server's tracked recording jobs.
pub fn render_video_audio_with_control(
    source_path: &str,
    plan: &EditPlan,
    out_path: &str,
    audio: Option<&str>,
    control: &ffmpeg::ProcessControl,
) -> Result<u64> {
    plan.validate()?;
    let (out_w, out_h) = output_size(plan);
    let fps = plan.fps as f64;
    let p = ffmpeg::probe_with_control(source_path, control)?;
    let size = IntSize::from_wh(p.w, p.h).ok_or_else(|| {
        RecordError::new(
            error_codes::FFMPEG,
            "bad source size",
            format!("{}x{}", p.w, p.h),
        )
    })?;
    // Build the compositor ONCE (caches background + shadow + rounded mask). For a
    // BlurScreen background, grab a representative source frame to blur into the backdrop.
    let comp = if matches!(plan.background, record_core::Background::BlurScreen { .. }) {
        let t = (plan.duration_ms / 4).min(plan.duration_ms.saturating_sub(1));
        match ffmpeg::grab_frame_with_control(source_path, t, control)
            .ok()
            .and_then(|(fw, fh, bytes)| {
                IntSize::from_wh(fw, fh).and_then(|s| Pixmap::from_vec(bytes, s))
            }) {
            Some(f) => crate::Compositor::with_bg(plan, Some(&f)),
            None => crate::Compositor::new(plan),
        }
    } else {
        crate::Compositor::new(plan)
    };

    // Pre-decode the webcam (if any) into bubble-sized frames, indexed by time.
    let webcam = match &plan.webcam {
        Some(wc) => {
            let bp = (((wc.size * out_h as f64).round() as u32) & !1).max(2);
            let frames = ffmpeg::decode_square_with_control(&wc.source, bp, fps, control)?;
            Some((bp, frames))
        }
        None => None,
    };

    let audio_input = audio.unwrap_or(source_path);
    // Normalize loudness only when an explicit audio track (e.g. mic) is muxed.
    let normalize = audio.is_some();
    ffmpeg::render_pipe_with_control(
        source_path,
        out_path,
        out_w,
        out_h,
        fps,
        audio_input,
        normalize,
        control,
        |buf, t_ms| {
            let src = Pixmap::from_vec(buf.to_vec(), size).expect("source pixmap from frame bytes");
            let cam = webcam.as_ref().and_then(|(bp, frames)| {
                if frames.is_empty() {
                    return None;
                }
                let idx = ((t_ms as f64 * fps / 1000.0) as usize).min(frames.len() - 1);
                let s = IntSize::from_wh(*bp, *bp)?;
                Pixmap::from_vec(frames[idx].clone(), s)
            });
            comp.frame_webcam(&src, cam.as_ref(), t_ms).data().to_vec()
        },
    )
}

/// Render a SINGLE composed frame to a PNG (no encode — fast visual/golden check).
pub fn render_frame_png(
    source_path: &str,
    plan: &EditPlan,
    t_ms: u64,
    png_path: &str,
) -> Result<()> {
    plan.validate()?;
    let (w, h, bytes) = ffmpeg::grab_frame(source_path, t_ms)?;
    let size = IntSize::from_wh(w, h).ok_or_else(|| {
        RecordError::new(error_codes::FFMPEG, "bad frame size", format!("{w}x{h}"))
    })?;
    let src = Pixmap::from_vec(bytes, size).ok_or_else(|| {
        RecordError::new(error_codes::IO, "decode frame", "Pixmap::from_vec failed")
    })?;
    // For BlurScreen, blur this frame into the backdrop (consistent with render_video).
    let out = if matches!(plan.background, record_core::Background::BlurScreen { .. }) {
        crate::Compositor::with_bg(plan, Some(&src)).frame(&src, t_ms)
    } else {
        compose_frame(&src, plan, t_ms)
    };
    out.save_png(png_path)
        .map_err(|e| RecordError::new(error_codes::IO, "save png", e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use record_core::{fixtures, Ease, EditPlan, ZoomKey};

    fn ffmpeg_present() -> bool {
        std::process::Command::new(
            std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()),
        )
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    }

    /// End-to-end: synthetic source → polished MP4. Skips if ffmpeg is absent so
    /// the suite stays green on ffmpeg-less machines (e.g. a fresh mac).
    #[test]
    fn full_pipeline_produces_a_real_video() {
        if !ffmpeg_present() {
            eprintln!("skip full_pipeline: ffmpeg not on PATH");
            return;
        }
        let dir = std::env::temp_dir().join("shellx_record_e2e");
        std::fs::create_dir_all(&dir).unwrap();
        // small + fast: 1080p fixture → 480x270.
        let events = fixtures::generate("click-walkthrough")
            .unwrap()
            .scaled(0.25);
        let src = dir.join("src.mp4");
        let n = crate::generate_source(&events, src.to_str().unwrap(), 15.0).unwrap();
        assert!(n > 0, "source frames");

        let mut plan = EditPlan::empty(events.screen_w, events.screen_h, events.duration_ms, 15.0);
        plan.zoom.keys.push(ZoomKey {
            t_ms: 0,
            scale: 1.5,
            cx: 0.5,
            cy: 0.5,
            ease: Ease::EaseInOut,
        });

        let out = dir.join("out.mp4");
        let frames =
            crate::render_video(src.to_str().unwrap(), &plan, out.to_str().unwrap()).unwrap();
        assert!(frames > 0, "rendered frames");
        let len = std::fs::metadata(&out).unwrap().len();
        assert!(
            len > 1000,
            "output mp4 should be non-trivial, got {len} bytes"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
