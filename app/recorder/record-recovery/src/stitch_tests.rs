//! Real-media regressions for capture-clock checkpoint stitching.

use std::process::Command;

use tempfile::tempdir;

use crate::{
    stitch_complete, verify_media, CaptureStart, CheckpointFacts, ManifestOwner, MediaFacts,
};

fn publish_ffmpeg_segment(
    owner: &mut ManifestOwner,
    sequence: u64,
    start_ms: u64,
    end_ms: u64,
    frames: u64,
) -> MediaFacts {
    let staging = owner.begin_segment(sequence, start_ms).unwrap();
    let frames = frames.to_string();
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=32x32:r=25",
            "-frames:v",
            &frames,
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&staging)
        .status()
        .expect("ffmpeg is required for the checkpoint stitch proof")
        .success());
    let media = verify_media("ffmpeg", "ffprobe", &staging).unwrap();
    owner
        .publish(
            sequence,
            &staging,
            CheckpointFacts {
                start_ms,
                end_ms,
                event_offset_ms: start_ms,
                audio_offset_ms: None,
            },
            media.clone(),
        )
        .unwrap();
    media
}

fn assert_capture_clock_duration(source: &std::path::Path, expected_ms: u64) {
    let facts = verify_media("ffmpeg", "ffprobe", source).unwrap();
    assert!(
        facts.duration_ms.abs_diff(expected_ms) <= 120,
        "stitched source must preserve the checkpoint capture clock: {facts:?}"
    );
    assert!(!facts.has_audio, "checkpoints stay video-only");
}

#[test]
fn real_ffmpeg_stitch_pads_sparse_segment_to_measured_capture_span() {
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 600)).unwrap();
    let sparse = publish_ffmpeg_segment(&mut owner, 0, 0, 600, 1);
    let second = publish_ffmpeg_segment(&mut owner, 1, 650, 850, 5);
    assert!(
        sparse.duration_ms < 600,
        "the fixture must model a native segment with missing delivered frames: {sparse:?}"
    );
    let source = stitch_complete(
        dir.path(),
        &owner.manifest().checkpoints,
        "ffmpeg",
        "ffprobe",
        "source.mp4",
    )
    .unwrap();
    assert_capture_clock_duration(&source, 850);
    let facts = verify_media("ffmpeg", "ffprobe", &source).unwrap();
    assert!(
        facts.decoded_video_frames > sparse.decoded_video_frames + second.decoded_video_frames,
        "cloned frame padding must be present instead of collapsing sparse capture time: {facts:?}"
    );
    assert_eq!(
        stitch_complete(
            dir.path(),
            &owner.manifest().checkpoints,
            "ffmpeg",
            "ffprobe",
            "source.mp4",
        )
        .unwrap(),
        source,
        "a completed sparse capture must keep its verified source on retry"
    );
}

#[test]
fn real_ffmpeg_stitch_normalizes_sparse_vfr_and_mixed_checkpoint_rates() {
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 15_000)).unwrap();
    let encoded = dir.path().join("vfr-encoded.mp4");
    let staging = owner.begin_segment(0, 7).unwrap();
    // ScreenCaptureKit can write sixteen variable-spaced frames in the first
    // 1.4 seconds, then no new pixels while its capture clock reaches 15 s.
    // The final packet's 350 ms duration matters: padding from its PTS alone
    // would finish early even though the verified container runs longer.
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=32x32:r=60",
            "-frames:v",
            "16",
            "-vf",
            "setpts=if(eq(N\\,1)\\,2\\,if(lt(N\\,14)\\,N*3\\,if(eq(N\\,14)\\,43\\,64)))",
            "-fps_mode",
            "vfr",
            "-c:v",
            "libx264",
            "-bf",
            "0",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&encoded)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(&encoded)
        .args([
            "-c",
            "copy",
            "-bsf:v",
            "setts=duration=if(eq(N\\,15)\\,5376\\,DURATION)",
        ])
        .arg(&staging)
        .status()
        .unwrap()
        .success());
    let vfr = verify_media("ffmpeg", "ffprobe", &staging).unwrap();
    assert_eq!(vfr.decoded_video_frames, 16);
    assert_eq!(vfr.duration_ms, 1_417);
    assert_eq!(vfr.r_frame_rate.unwrap().num, 60);
    owner
        .publish(
            0,
            &staging,
            CheckpointFacts {
                start_ms: 7,
                end_ms: 15_027,
                event_offset_ms: 7,
                audio_offset_ms: None,
            },
            vfr,
        )
        .unwrap();
    publish_ffmpeg_segment(&mut owner, 1, 15_531, 16_531, 25);
    let stitched = stitch_complete(
        dir.path(),
        &owner.manifest().checkpoints,
        "ffmpeg",
        "ffprobe",
        "source.mp4",
    )
    .unwrap();
    assert_capture_clock_duration(&stitched, 16_531);
    let media = verify_media("ffmpeg", "ffprobe", &stitched).unwrap();
    assert_eq!(media.r_frame_rate.unwrap().num, 60);
    assert!(
        media.decoded_video_frames >= 985,
        "missing cloned capture time: {media:?}"
    );
}

#[test]
fn real_ffmpeg_stitch_accepts_bounded_encoder_drain_but_rejects_longer_overrun() {
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 7_033)).unwrap();
    // PC2's native 25 fps Stop produced 179 frames (7_160 ms) against a
    // 7_033 ms capture-clock span. The checked media must not be discarded.
    let media = publish_ffmpeg_segment(&mut owner, 0, 0, 7_033, 179);
    assert_eq!(media.duration_ms, 7_160);
    let source = stitch_complete(
        dir.path(),
        &owner.manifest().checkpoints,
        "ffmpeg",
        "ffprobe",
        "source.mp4",
    )
    .unwrap();
    assert_capture_clock_duration(&source, 7_160);

    let mut inconsistent = owner.manifest().checkpoints.clone();
    inconsistent[0].facts.end_ms = 6_933; // 227 ms short: beyond encoder drain.
    let error = stitch_complete(
        dir.path(),
        &inconsistent,
        "ffmpeg",
        "ffprobe",
        "inconsistent.mp4",
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("checkpoint media exceeds its observed capture span"));
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires native Linux GStreamer"]
fn gstreamer_sparse_checkpoint_span_stitches_on_capture_clock() {
    let dir = tempdir().unwrap();
    let mut owner = ManifestOwner::begin(dir.path(), CaptureStart::new("cap", 600)).unwrap();
    let staging = owner.begin_segment(0, 0).unwrap();
    let location = format!("location={}", staging.display());
    assert!(Command::new("gst-launch-1.0")
        .args([
            "-e",
            "videotestsrc",
            "num-buffers=1",
            "pattern=black",
            "!",
            "video/x-raw,framerate=25/1,width=32,height=32",
            "!",
            "videoconvert",
            "!",
            "x264enc",
            "speed-preset=ultrafast",
            "tune=zerolatency",
            "!",
            "mp4mux",
            "!",
            "filesink",
            &location,
        ])
        .status()
        .expect("native GStreamer is required for this rig")
        .success());
    let media = verify_media("ffmpeg", "ffprobe", &staging).unwrap();
    assert!(
        media.duration_ms < 600,
        "the real GStreamer fixture must stay sparse: {media:?}"
    );
    owner
        .publish(
            0,
            &staging,
            CheckpointFacts {
                start_ms: 0,
                end_ms: 600,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
            media,
        )
        .unwrap();
    let source = stitch_complete(
        dir.path(),
        &owner.manifest().checkpoints,
        "ffmpeg",
        "ffprobe",
        "source.mp4",
    )
    .unwrap();
    assert_capture_clock_duration(&source, 600);
}
