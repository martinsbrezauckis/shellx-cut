//! A still-screen segment that never received a picture is closed by holding
//! the previous segment's last frame; publication without a decode keeps the
//! native file's container facts.

use std::process::Command;

use record_recovery::{CaptureStart, CheckpointFacts, ManifestOwner};
use tempfile::tempdir;

use super::{CheckpointConfig, Checkpoints};

fn facts(start_ms: u64, end_ms: u64) -> CheckpointFacts {
    CheckpointFacts {
        start_ms,
        end_ms,
        event_offset_ms: start_ms,
        audio_offset_ms: None,
    }
}

fn open(root: &std::path::Path) -> Checkpoints {
    let mut start = CaptureStart::new("held-frame", 15_000);
    start.output_cadence = record_core::FrameRate::new(30, 1).ok();
    ManifestOwner::begin(root, start).unwrap();
    let config = CheckpointConfig {
        manifest_dir: root.to_string_lossy().into_owned(),
        interval_ms: 15_000,
    };
    Checkpoints::open(Some(&config)).unwrap().unwrap()
}

/// Red frames, then one blue frame last.
fn red_then_blue(path: &std::path::Path) {
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=64x36:r=30:d=0.3"
        ])
        .args(["-f", "lavfi", "-i", "color=c=blue:s=64x36:r=30:d=0.034"])
        .args([
            "-filter_complex",
            "[0:v][1:v]concat=n=2:v=1[v]",
            "-map",
            "[v]"
        ])
        .args(["-an", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(path)
        .status()
        .unwrap()
        .success());
}

fn rgb_at(path: &std::path::Path, at_s: f64) -> [u8; 3] {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &format!("{at_s:.3}"), "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "crop=1:1:32:18,format=rgb24",
            "-f",
            "rawvideo",
            "-",
        ])
        .output()
        .unwrap();
    [out.stdout[0], out.stdout[1], out.stdout[2]]
}

#[test]
fn an_empty_final_segment_holds_the_last_picture_over_its_whole_span() {
    let root = tempdir().unwrap();
    let mut checkpoints = open(root.path());
    let (sequence, first) = checkpoints.begin(0).unwrap();
    red_then_blue(&first);
    checkpoints
        .publish_without_decode(sequence, &first, facts(0, 15_000))
        .unwrap();
    // The still screen: the second output never got a picture; ScreenCaptureKit
    // left no file at all.
    let (sequence, empty) = checkpoints.begin(15_600).unwrap();
    assert!(!empty.exists());
    let held = checkpoints
        .publish_held_frame(sequence, &empty, facts(15_000, 62_000))
        .unwrap();
    assert_eq!(held.media.as_ref().unwrap().decoded_video_frames, 1);
    assert_eq!(held.held_last_frame_ms, Some(47_000));
    let (source, media, notes) = checkpoints
        .stitch("ffmpeg", "ffprobe", "source.mp4")
        .unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert!(media.duration_ms.abs_diff(62_000) <= 34, "{media:?}");
    // The picture held to the end is the last one before the still stretch.
    let [r, _, b] = rgb_at(&source, 61.5);
    assert!(
        b > r,
        "the held frame is the blue last frame, got r={r} b={b}"
    );
    assert!(!record_recovery::read_manifest(root.path())
        .unwrap()
        .has_open_segment());
}

#[test]
fn a_held_frame_needs_an_earlier_checkpoint() {
    let root = tempdir().unwrap();
    let mut checkpoints = open(root.path());
    let (sequence, empty) = checkpoints.begin(0).unwrap();
    assert!(checkpoints
        .publish_held_frame(sequence, &empty, facts(0, 1_000))
        .is_err());
}

#[test]
fn publication_without_decode_keeps_container_facts_and_refuses_embedded_sound() {
    let root = tempdir().unwrap();
    let mut checkpoints = open(root.path());
    let (sequence, first) = checkpoints.begin(0).unwrap();
    red_then_blue(&first);
    let published = checkpoints
        .publish_without_decode(sequence, &first, facts(0, 400))
        .unwrap();
    let media = published.media.unwrap();
    // The packet count equals what a full decode counts for this file.
    let decoded =
        record_recovery::verify_media("ffmpeg", "ffprobe", &root.path().join(&published.file))
            .unwrap();
    assert_eq!(media.decoded_video_frames, decoded.decoded_video_frames);
    assert_eq!(media.duration_ms, decoded.duration_ms);
    assert!(!media.has_audio);

    let (sequence, second) = checkpoints.begin(500).unwrap();
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=64x36:r=30:d=0.3"
        ])
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=d=0.3",
            "-c:v",
            "libx264",
            "-c:a",
            "aac"
        ])
        .arg(&second)
        .status()
        .unwrap()
        .success());
    assert!(checkpoints
        .publish_without_decode(sequence, &second, facts(500, 900))
        .is_err());
}
