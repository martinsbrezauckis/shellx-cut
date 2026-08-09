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
