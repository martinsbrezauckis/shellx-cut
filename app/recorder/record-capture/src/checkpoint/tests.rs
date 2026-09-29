use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

use record_recovery::{CaptureStart, ManifestOwner};
use tempfile::tempdir;

use super::{trim_extrapolated_wgc_tail, CheckpointConfig, Checkpoints};
use crate::windows_wgc_timing::{retained_windows_594_frame_tail, WgcTimingRecorder};

#[test]
fn retained_windows_partial_gap_tail_clips_verified_mp4_to_independent_stop() {
    let root = tempdir().unwrap();
    let staging = root.path().join("source.mp4");
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=32x32:r=30"
        ])
        .args([
            "-frames:v",
            "594",
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p"
        ])
        .arg(&staging)
        .status()
        .unwrap()
        .success());
    let original = record_recovery::verify_media("ffmpeg", "ffprobe", &staging).unwrap();
    assert_eq!(original.duration_ms, 19_800);
    assert_eq!(original.decoded_video_frames, 594);

    let origin = Instant::now();
    let timing = retained_windows_594_frame_tail(origin, &root.path().join("._timing.d/s.mp4"));
    let corrected = trim_extrapolated_wgc_tail(
        &staging,
        &record_recovery::CheckpointFacts {
            start_ms: 86_407,
            end_ms: 101_506,
            event_offset_ms: 86_407,
            audio_offset_ms: None,
        },
        original,
        &timing,
        "ffmpeg",
        "ffprobe",
    )
    .unwrap();
    assert!(corrected.duration_ms.abs_diff(15_099) <= 100);
    assert!(corrected.decoded_video_frames >= 103);
    assert!(corrected.decoded_video_frames < 594);
    let readback = record_recovery::verify_media("ffmpeg", "ffprobe", &staging).unwrap();
    assert_eq!(readback.duration_ms, corrected.duration_ms);
    assert_eq!(
        readback.decoded_video_frames,
        corrected.decoded_video_frames
    );
}

#[test]
fn zero_frame_final_wgc_checkpoint_holds_verified_frame_and_stitches_full_span() {
    let root = tempdir().unwrap();
    ManifestOwner::begin(root.path(), CaptureStart::new("zero-final-wgc", 400)).unwrap();
    let config = CheckpointConfig {
        manifest_dir: root.path().to_string_lossy().into_owned(),
        interval_ms: 400,
    };
    let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
    let (sequence, first) = checkpoints.begin_windows_wgc(0).unwrap();
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=32x32:r=30"
        ])
        .args(["-t", "0.4", "-an", "-c:v", "libx264"])
        .arg(&first)
        .status()
        .unwrap()
        .success());
    let facts = record_recovery::CheckpointFacts {
        start_ms: 0,
        end_ms: 400,
        event_offset_ms: 0,
        audio_offset_ms: None,
    };
    checkpoints
        .publish_with_tools(sequence, &first, facts, "ffmpeg", "ffprobe", None)
        .unwrap();

    let (sequence, empty) = checkpoints.begin_windows_wgc(400).unwrap();
    let origin = Instant::now();
    let timing = WgcTimingRecorder::new(origin, 30, &empty);
    timing.control_stopped(origin + Duration::from_millis(800), true);
    let held = checkpoints
        .publish_with_tools(
            sequence,
            &empty,
            record_recovery::CheckpointFacts {
                start_ms: 400,
                end_ms: 800,
                event_offset_ms: 400,
                audio_offset_ms: None,
            },
            "ffmpeg",
            "ffprobe",
            Some(&timing),
        )
        .unwrap();
    assert_eq!(held.held_last_frame_ms, Some(400));
    assert!(held.media.as_ref().unwrap().duration_ms.abs_diff(400) <= 100);
    assert_ne!(held.file, checkpoints.owner.manifest().checkpoints[0].file);
    assert!(root.path().join(&held.file).is_file());
    let (source, media) = checkpoints
        .stitch("ffmpeg", "ffprobe", "source.mp4")
        .unwrap();
    assert!(source.is_file());
    assert!(media.duration_ms.abs_diff(800) <= 100);
    let manifest = record_recovery::read_manifest(root.path()).unwrap();
    assert!(!manifest.has_open_segment());
    assert_eq!(manifest.checkpoints[1].held_last_frame_ms, Some(400));
    assert_eq!(
        record_recovery::recovery_status(root.path()).held_last_frame_ms,
        Some(400)
    );
}

#[test]
fn accepted_wgc_frame_never_uses_held_tail_fallback() {
    let root = tempdir().unwrap();
    ManifestOwner::begin(root.path(), CaptureStart::new("accepted-final-wgc", 400)).unwrap();
    let config = CheckpointConfig {
        manifest_dir: root.path().to_string_lossy().into_owned(),
        interval_ms: 400,
    };
    let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
    let (_, first) = checkpoints.begin_windows_wgc(0).unwrap();
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=32x32:r=30"
        ])
        .args(["-t", "0.4", "-an", "-c:v", "libx264"])
        .arg(&first)
        .status()
        .unwrap()
        .success());
    checkpoints
        .publish_with_tools(
            0,
            &first,
            record_recovery::CheckpointFacts {
                start_ms: 0,
                end_ms: 400,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
            "ffmpeg",
            "ffprobe",
            None,
        )
        .unwrap();
    let (sequence, invalid) = checkpoints.begin_windows_wgc(400).unwrap();
    let origin = Instant::now();
    let timing = WgcTimingRecorder::new(origin, 30, &invalid);
    timing.accepted_frame(1, origin + Duration::from_millis(450));
    let result = checkpoints.publish_with_tools(
        sequence,
        &invalid,
        record_recovery::CheckpointFacts {
            start_ms: 400,
            end_ms: 800,
            event_offset_ms: 400,
            audio_offset_ms: None,
        },
        "ffmpeg",
        "ffprobe",
        Some(&timing),
    );
    assert!(result.is_err());
    assert_eq!(checkpoints.owner.manifest().checkpoints.len(), 1);

    let failed_stop = WgcTimingRecorder::new(origin, 30, &invalid);
    failed_stop.control_stopped(origin + Duration::from_millis(800), false);
    let result = checkpoints.publish_with_tools(
        sequence,
        &invalid,
        record_recovery::CheckpointFacts {
            start_ms: 400,
            end_ms: 800,
            event_offset_ms: 400,
            audio_offset_ms: None,
        },
        "ffmpeg",
        "ffprobe",
        Some(&failed_stop),
    );
    assert!(result.is_err());
    assert!(!invalid.exists());
    assert_eq!(checkpoints.owner.manifest().checkpoints.len(), 1);
}

#[test]
fn proven_wgc_tail_clip_preserves_frames_and_stop_span() {
    let root = tempdir().unwrap();
    let staging = root.path().join("source.mp4");
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=32x32:r=60"
        ])
        .args(["-t", "7.75", "-c:v", "libx264"])
        .arg(&staging)
        .status()
        .unwrap()
        .success());
    let original = record_recovery::verify_media("ffmpeg", "ffprobe", &staging).unwrap();
    assert_eq!(original.duration_ms, 7_750);
    assert_eq!(original.decoded_video_frames, 465);
    let origin = Instant::now();
    let timing = WgcTimingRecorder::new(origin, 60, &root.path().join("._timing.d/s.mp4"));
    let first = 5_978_568_688_251i64;
    timing.accepted_frame(first, origin + Duration::from_millis(477));
    timing.accepted_frame(first + 11_000_000, origin + Duration::from_millis(1_577));
    timing.accepted_frame(first + 44_334_192, origin + Duration::from_millis(4_910));
    timing.control_stopped(origin + Duration::from_millis(7_337), true);
    let corrected = trim_extrapolated_wgc_tail(
        &staging,
        &record_recovery::CheckpointFacts {
            start_ms: 0,
            end_ms: 7_338,
            event_offset_ms: 0,
            audio_offset_ms: None,
        },
        original,
        &timing,
        "ffmpeg",
        "ffprobe",
    )
    .unwrap();
    assert!(corrected.duration_ms.abs_diff(7_338) <= 100);
    assert!(corrected.decoded_video_frames >= timing.accepted_frames());
    assert!(corrected.decoded_video_frames < 465);
    assert_eq!(
        record_recovery::verify_media("ffmpeg", "ffprobe", &staging)
            .unwrap()
            .duration_ms,
        corrected.duration_ms
    );
}

#[test]
fn earlier_max_gap_clip_verifies_real_mp4_at_observed_stop() {
    let root = tempdir().unwrap();
    let staging = root.path().join("source.mp4");
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=32x32:r=60",
        ])
        .args(["-t", "7.933", "-c:v", "libx264"])
        .arg(&staging)
        .status()
        .unwrap()
        .success());
    let original = record_recovery::verify_media("ffmpeg", "ffprobe", &staging).unwrap();
    assert_eq!(original.duration_ms, 7_933);
    assert_eq!(original.decoded_video_frames, 476);

    let origin = Instant::now();
    let timing = WgcTimingRecorder::new(origin, 60, &root.path().join("._timing.d/s.mp4"));
    let first = 6_146_209_642_756i64;
    timing.accepted_frame(first, origin + Duration::from_millis(480));
    timing.accepted_frame(first + 34_000_076, origin + Duration::from_millis(3_880));
    timing.accepted_frame(first + 45_500_670, origin + Duration::from_millis(5_030));
    timing.control_stopped(origin + Duration::from_millis(7_497), true);
    let corrected = trim_extrapolated_wgc_tail(
        &staging,
        &record_recovery::CheckpointFacts {
            start_ms: 0,
            end_ms: 7_499,
            event_offset_ms: 0,
            audio_offset_ms: None,
        },
        original,
        &timing,
        "ffmpeg",
        "ffprobe",
    )
    .unwrap();
    assert!(corrected.duration_ms.abs_diff(7_499) <= 100);
    assert!(corrected.decoded_video_frames >= timing.accepted_frames());
    assert!(corrected.decoded_video_frames < 476);
    assert_eq!(
        record_recovery::verify_media("ffmpeg", "ffprobe", &staging)
            .unwrap()
            .duration_ms,
        corrected.duration_ms
    );
}

#[test]
fn checkpoint_rejects_a_planted_native_output_link_before_media_verification() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    drop(ManifestOwner::begin(root.path(), CaptureStart::new("capture", 100)).unwrap());
    let config = CheckpointConfig {
        manifest_dir: root.path().display().to_string(),
        interval_ms: 100,
    };
    let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
    let (sequence, staging) = checkpoints.begin(0).unwrap();
    let outside = tempdir().unwrap();
    let target = outside.path().join("outside.mp4");
    fs::write(&target, b"outside remains untouched").unwrap();
    symlink(&target, &staging).unwrap();

    assert!(checkpoints
        .publish(
            sequence,
            &staging,
            record_recovery::CheckpointFacts {
                start_ms: 0,
                end_ms: 100,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
        )
        .is_err());
    assert_eq!(fs::read(target).unwrap(), b"outside remains untouched");
}

#[test]
fn checkpoint_strips_native_encoder_audio_before_publication() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().unwrap();
    drop(ManifestOwner::begin(root.path(), CaptureStart::new("capture", 100)).unwrap());
    let config = CheckpointConfig {
        manifest_dir: root.path().display().to_string(),
        interval_ms: 100,
    };
    let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
    let (sequence, staging) = checkpoints.begin_windows_wgc(0).unwrap();
    fs::write(&staging, b"native-with-audio").unwrap();

    let ffmpeg = root.path().join("fake-ffmpeg");
    let ffprobe = root.path().join("fake-ffprobe");
    fs::write(
        &ffmpeg,
        "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\n[ \"$last\" = - ] && exit 0\nprintf video-only > \"$last\"\n",
    )
    .unwrap();
    fs::write(
        &ffprobe,
        "#!/bin/sh\nlast=\nfor arg in \"$@\"; do last=\"$arg\"; done\nif grep -q video-only \"$last\"; then audio=; else audio=',{\"codec_type\":\"audio\",\"nb_read_frames\":\"1\"}'; fi\nprintf '{\"format\":{\"duration\":\"0.100\"},\"streams\":[{\"codec_type\":\"video\",\"nb_read_frames\":\"3\"}%s]}' \"$audio\"\n",
    )
    .unwrap();
    for tool in [&ffmpeg, &ffprobe] {
        fs::set_permissions(tool, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let checkpoint = checkpoints
        .publish_with_tools(
            sequence,
            &staging,
            record_recovery::CheckpointFacts {
                start_ms: 1,
                end_ms: 101,
                event_offset_ms: 1,
                audio_offset_ms: None,
            },
            ffmpeg.to_str().unwrap(),
            ffprobe.to_str().unwrap(),
            None,
        )
        .unwrap();

    assert_eq!(checkpoint.sequence, 0);
    assert_eq!(checkpoint.file, "checkpoints/segment-000000.mp4");
    let checkpoint = &checkpoints.owner.manifest().checkpoints[0];
    assert!(!checkpoint.media.as_ref().unwrap().has_audio);
    assert_eq!(checkpoint.media.as_ref().unwrap().decoded_video_frames, 3);
    assert_eq!(
        fs::read(root.path().join("checkpoints/segment-000000.mp4")).unwrap(),
        b"video-only"
    );
}
