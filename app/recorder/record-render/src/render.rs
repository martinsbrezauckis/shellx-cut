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
    render_video_audio_with_control_progress(source_path, plan, out_path, audio, control, |_, _| {})
}

/// Cancellable render with a callback for confirmed compositor frame flow.
///
/// The callback runs after the compositor produced a frame and immediately
/// before it is written to the owned ffmpeg encoder pipe. It deliberately
/// exposes only frame counts, never source paths or ffmpeg output.
pub fn render_video_audio_with_control_progress(
    source_path: &str,
    plan: &EditPlan,
    out_path: &str,
    audio: Option<&str>,
    control: &ffmpeg::ProcessControl,
    mut on_frame: impl FnMut(u64, u64),
) -> Result<u64> {
    plan.validate()?;
    let (out_w, out_h) = output_size(plan)?;
    let fps = plan.fps as f64;
    let expected_frames = ((plan.duration_ms as f64 * fps / 1000.0).ceil() as u64).max(1);
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
    }?;

    // Stream the webcam alongside output frames. Keeping its entire raw track
    // would both truncate at the diagnostic pipe cap and grow with take length.
    let mut webcam = match &plan.webcam {
        Some(wc) => {
            let bp = (((wc.size * out_h as f64).round() as u32) & !1).max(2);
            let mut frames = ffmpeg::stream_square_with_control(&wc.source, bp, fps, control)?;
            if frames.at(0)?.is_none() {
                return Err(RecordError::new(
                    error_codes::FFMPEG,
                    "webcam decode",
                    "selected camera has no decoded frames",
                ));
            }
            Some((bp, frames))
        }
        None => None,
    };

    let audio_input = audio.unwrap_or(source_path);
    // Normalize loudness only when an explicit audio track (e.g. mic) is muxed.
    let normalize = audio.is_some();
    let mut rendered_frames = 0_u64;
    let result = ffmpeg::render_pipe_with_control(
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
            let cam = match (&mut webcam, &plan.webcam) {
                (Some((bp, frames)), Some(wc)) => match webcam_frame_index(wc, t_ms, fps) {
                    Some(idx) => {
                        let size = IntSize::from_wh(*bp, *bp).expect("validated camera size");
                        frames
                            .at(idx)?
                            .map(|frame| {
                                Pixmap::from_vec(frame.to_vec(), size).ok_or_else(|| {
                                    RecordError::new(
                                        error_codes::FFMPEG,
                                        "webcam decode",
                                        "invalid RGBA camera frame",
                                    )
                                })
                            })
                            .transpose()?
                    }
                    None => None,
                },
                _ => None,
            };
            let frame = comp.frame_webcam(&src, cam.as_ref(), t_ms).data().to_vec();
            rendered_frames += 1;
            on_frame(rendered_frames, expected_frames);
            Ok(frame)
        },
    );
    if result.is_ok() {
        if let Some((_, frames)) = webcam {
            frames.finish()?;
        }
    }
    result
}

/// Resolve a camera-frame index without fabricating a frame for an unrecorded
/// interval. A successfully ended camera track leaves the overlay absent.
/// CameraArtifact@1 supplies an optional shared-clock range; legacy plans
/// retain a zero origin.
pub(crate) fn webcam_frame_index(
    webcam: &record_core::WebcamOverlay,
    t_ms: u64,
    fps: f64,
) -> Option<usize> {
    if !fps.is_finite() || fps <= 0.0 {
        return None;
    }
    let camera_ms = if let Some(clock) = webcam.camera_clock {
        if t_ms < clock.first_frame_offset_ms || t_ms >= clock.end_frame_offset_ms {
            return None;
        }
        t_ms - clock.first_frame_offset_ms
    } else {
        t_ms
    };
    Some((camera_ms as f64 * fps / 1000.0).floor() as usize)
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
        crate::Compositor::with_bg(plan, Some(&src))?.frame(&src, t_ms)
    } else {
        compose_frame(&src, plan, t_ms)?
    };
    out.save_png(png_path)
        .map_err(|e| RecordError::new(error_codes::IO, "save png", e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn unsafe_output_is_rejected_before_composition_or_media_access() {
        let source = tiny_skia::Pixmap::new(2, 2).unwrap();
        for (sw, sh, aspect) in [
            (1920, 1080, Some((10_000, 1))),
            (1920, 1080, Some((u32::MAX, 1))),
            (1920, 1080, Some((0, 1))),
            (1, 1080, None),
            (8193, 1080, None),
            (1920, 1, Some((1, 1))),
        ] {
            let mut plan = record_core::EditPlan::empty(sw, sh, 1000, 30.0);
            if let Some((w, h)) = aspect {
                plan.reframe = record_core::Reframe::Aspect { w, h };
            }
            assert!(crate::Compositor::new(&plan).is_err());
            assert!(crate::Compositor::with_bg(&plan, Some(&source)).is_err());
            assert_eq!(
                crate::compose_frame(&source, &plan, 0).unwrap_err().code,
                record_core::error_codes::INVALID_ARGS
            );
            assert_eq!(
                super::render_video("missing-source", &plan, "missing-output")
                    .unwrap_err()
                    .code,
                record_core::error_codes::INVALID_ARGS
            );
            assert_eq!(
                super::render_frame_png("missing-source", &plan, 0, "missing-output")
                    .unwrap_err()
                    .code,
                record_core::error_codes::INVALID_ARGS
            );
        }
    }

    use record_core::{
        fixtures, Anchor, CameraClockRange, Ease, EditPlan, WebcamOverlay, WebcamShape, ZoomKey,
    };

    #[test]
    fn camera_clock_never_fills_before_first_or_after_last_frame() {
        let webcam = WebcamOverlay {
            source: "camera.mp4".into(),
            shape: WebcamShape::Circle,
            anchor: Anchor::BottomRight,
            margin: 0.04,
            size: 0.22,
            camera_clock: Some(CameraClockRange {
                first_frame_offset_ms: 500,
                end_frame_offset_ms: 1_500,
            }),
            timeline: vec![],
        };
        assert_eq!(super::webcam_frame_index(&webcam, 499, 30.0), None);
        assert_eq!(super::webcam_frame_index(&webcam, 500, 30.0), Some(0));
        assert_eq!(super::webcam_frame_index(&webcam, 1_000, 30.0), Some(15));
        assert_eq!(super::webcam_frame_index(&webcam, 1_499, 30.0), Some(29));
        assert_eq!(super::webcam_frame_index(&webcam, 1_500, 30.0), None);
    }

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

    #[test]
    fn installed_mac_camera_clock_and_geometry_survive_full_ffmpeg_pipeline() {
        if !ffmpeg_present() {
            eprintln!("skip camera pipeline: ffmpeg not on PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let src = dir.path().join("screen.mp4");
        let cam = dir.path().join("camera.mov");
        let out = dir.path().join("render.mp4");
        for (path, input) in [
            (&src, "color=c=0x202830:s=1040x504:r=24:d=5.2"),
            (&cam, "color=c=red:s=1920x1080:r=24:d=4.0"),
        ] {
            let encoded = std::process::Command::new(&ffmpeg)
                .args([
                    "-v", "error", "-y", "-f", "lavfi", "-i", input, "-c:v", "libx264", "-pix_fmt",
                    "yuv420p",
                ])
                .arg(path)
                .output()
                .unwrap();
            assert!(
                encoded.status.success(),
                "{}",
                String::from_utf8_lossy(&encoded.stderr)
            );
        }
        let mut plan = EditPlan::empty(1040, 504, 5_200, 24.0);
        plan.webcam = Some(WebcamOverlay {
            source: cam.to_string_lossy().into_owned(),
            shape: WebcamShape::Circle,
            anchor: Anchor::BottomRight,
            margin: 0.04,
            size: 0.22,
            camera_clock: Some(CameraClockRange {
                first_frame_offset_ms: 1_122,
                end_frame_offset_ms: 5_074,
            }),
            timeline: vec![],
        });
        let frames =
            super::render_video(src.to_str().unwrap(), &plan, out.to_str().unwrap()).unwrap();
        assert!(frames >= 95);
        let sampled = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&out)
            .args([
                "-vf",
                "select=eq(n\\,94)",
                "-vsync",
                "0",
                "-frames:v",
                "1",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            sampled.status.success(),
            "{}",
            String::from_utf8_lossy(&sampled.stderr)
        );
        assert_eq!(sampled.stdout.len(), 1040 * 504 * 3);
        let center = (428 * 1040 + 964) * 3;
        assert!(
            sampled.stdout[center] > 150 && sampled.stdout[center + 1] < 80,
            "full render omitted the admitted camera center: {:?}",
            &sampled.stdout[center..center + 3]
        );
    }

    #[test]
    fn legacy_short_camera_track_ends_overlay_without_aborting_screen_render() {
        if !ffmpeg_present() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let src = dir.path().join("screen.mp4");
        let cam = dir.path().join("short-camera.mp4");
        let out = dir.path().join("render.mp4");
        for (path, input) in [
            (&src, "color=c=0x202830:s=128x72:r=10:d=1.5"),
            (&cam, "color=c=red:s=64x64:r=10:d=0.3"),
        ] {
            let encoded = std::process::Command::new(&ffmpeg)
                .args([
                    "-v", "error", "-y", "-f", "lavfi", "-i", input, "-c:v", "libx264",
                ])
                .arg(path)
                .output()
                .unwrap();
            assert!(
                encoded.status.success(),
                "{}",
                String::from_utf8_lossy(&encoded.stderr)
            );
        }
        let mut plan = EditPlan::empty(128, 72, 1_500, 10.0);
        plan.webcam = Some(WebcamOverlay {
            source: cam.to_string_lossy().into_owned(),
            shape: WebcamShape::Circle,
            anchor: Anchor::BottomRight,
            margin: 0.04,
            size: 0.35,
            camera_clock: None,
            timeline: vec![],
        });
        let frames =
            super::render_video(src.to_str().unwrap(), &plan, out.to_str().unwrap()).unwrap();
        assert!(
            frames >= 14,
            "screen frames must continue after camera EOF: {frames}"
        );
        assert!(std::fs::metadata(&out).unwrap().len() > 1000);
    }

    #[test]
    fn selected_camera_is_validated_even_when_its_clock_misses_the_screen() {
        if !ffmpeg_present() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        let src = dir.path().join("screen.mp4");
        let bad_cam = dir.path().join("invalid.mov");
        let out = dir.path().join("render.mp4");
        let encoded = std::process::Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:r=10:d=0.3",
                "-c:v",
                "libx264",
            ])
            .arg(&src)
            .output()
            .unwrap();
        assert!(encoded.status.success());
        std::fs::write(&bad_cam, b"invalid camera movie").unwrap();
        let mut plan = EditPlan::empty(64, 64, 300, 10.0);
        plan.webcam = Some(WebcamOverlay {
            source: bad_cam.to_string_lossy().into_owned(),
            shape: WebcamShape::Circle,
            anchor: Anchor::BottomRight,
            margin: 0.04,
            size: 0.25,
            camera_clock: Some(CameraClockRange {
                first_frame_offset_ms: 240,
                end_frame_offset_ms: 290,
            }),
            timeline: vec![],
        });
        let error =
            super::render_video(src.to_str().unwrap(), &plan, out.to_str().unwrap()).unwrap_err();
        assert_eq!(error.code, super::error_codes::FFMPEG);
        assert!(error.message.contains("webcam decode"), "{error}");
    }
}
