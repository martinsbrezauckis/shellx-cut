//! Admitted cadence must survive sparse native packets and restart recovery.

use std::{path::Path, process::Command};

use record_core::FrameRate;
use tempfile::tempdir;

use crate::{
    read_manifest, recover_interrupted, stitch_complete, verify_media, CaptureStart,
    CheckpointFacts, ManifestOwner, MediaFacts, OwnerState,
};

fn sparse_capture(root: &Path, fps: Option<u64>) -> ManifestOwner {
    sparse_capture_with_rate(root, fps.map(|fps| FrameRate::new(fps, 1).unwrap()))
}

fn sparse_capture_with_rate(root: &Path, rate: Option<FrameRate>) -> ManifestOwner {
    let mut start = CaptureStart::new("sparse", 15_000);
    start.output_cadence = rate;
    let mut owner = ManifestOwner::begin(root, start).unwrap();
    let staging = owner.begin_segment(0, 0).unwrap();
    // Seven native frames cover 283 ms, on a nominal 60 Hz timestamp grid.
    // This reproduces WGC's sparse checkpoint rather than a CFR 24 fps input.
    assert!(Command::new("ffmpeg")
        .args([
            "-v", "error", "-y", "-f", "lavfi", "-i",
            "color=c=black:s=32x32:r=60", "-frames:v", "7", "-vf",
            "setpts=if(eq(N\\,0)\\,0\\,if(eq(N\\,1)\\,1\\,if(eq(N\\,2)\\,3\\,if(eq(N\\,3)\\,7\\,if(eq(N\\,4)\\,9\\,if(eq(N\\,5)\\,12\\,16))))))",
            "-fps_mode", "vfr", "-c:v", "libx264", "-bf", "0", "-pix_fmt", "yuv420p",
        ])
        .arg(&staging)
        .status().unwrap().success());
    let media = verify_media("ffmpeg", "ffprobe", &staging).unwrap();
    assert_eq!(media.decoded_video_frames, 7);
    assert_eq!(media.r_frame_rate, Some(FrameRate::new(60, 1).unwrap()));
    assert!((280..=285).contains(&media.duration_ms), "{media:?}");
    owner
        .publish(
            0,
            &staging,
            CheckpointFacts {
                start_ms: 0,
                end_ms: 937,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
            media,
        )
        .unwrap();
    owner
}

fn assert_output(root: &Path, path: &Path, fps: u64) -> MediaFacts {
    let media = verify_media("ffmpeg", "ffprobe", path).unwrap();
    let rate = Some(FrameRate::new(fps, 1).unwrap());
    assert_eq!(media.avg_frame_rate, rate);
    assert_eq!(media.r_frame_rate, rate);
    assert!(media.duration_ms.abs_diff(937) <= 1_000_u64.div_ceil(fps));
    assert_eq!(
        read_manifest(root).unwrap().checkpoints[0].facts.end_ms,
        937
    );
    media
}

#[test]
fn real_ffmpeg_sparse_nominal_60_preserves_admitted_24_on_stop_and_recovery() {
    for interrupted in [false, true] {
        let dir = tempdir().unwrap();
        let owner = sparse_capture(dir.path(), Some(24));
        let segments = owner.manifest.checkpoints.clone();
        drop(owner);
        assert_eq!(
            read_manifest(dir.path()).unwrap().start.output_cadence,
            Some(FrameRate::new(24, 1).unwrap())
        );
        let path = if interrupted {
            let result = recover_interrupted(dir.path(), "ffmpeg", "ffprobe", OwnerState::Dead)
                .unwrap()
                .unwrap();
            dir.path().join(result.receipt.source.unwrap())
        } else {
            stitch_complete(dir.path(), &segments, "ffmpeg", "ffprobe", "source.mp4").unwrap()
        };
        assert!((22..=23).contains(&assert_output(dir.path(), &path, 24).decoded_video_frames));
        assert_eq!(
            stitch_complete(
                dir.path(),
                &segments,
                "ffmpeg",
                "ffprobe",
                path.file_name().unwrap().to_str().unwrap()
            )
            .unwrap(),
            path
        );
    }
}

#[test]
fn real_ffmpeg_admitted_cadence_below_30_and_legacy_absence() {
    for (admitted, expected) in [(Some(12), 12), (Some(1), 1), (None, 60)] {
        let dir = tempdir().unwrap();
        let owner = sparse_capture(dir.path(), admitted);
        let path = stitch_complete(
            dir.path(),
            &owner.manifest.checkpoints,
            "ffmpeg",
            "ffprobe",
            "source.mp4",
        )
        .unwrap();
        assert_output(dir.path(), &path, expected);
        if admitted.is_none() {
            assert!(
                !std::fs::read_to_string(dir.path().join(crate::MANIFEST_FILE))
                    .unwrap()
                    .contains("output_cadence")
            );
        }
    }
}

#[test]
fn real_ffmpeg_existing_60_output_is_refused_for_admitted_24() {
    let dir = tempdir().unwrap();
    let owner = sparse_capture(dir.path(), Some(24));
    let output = dir.path().join("source.mp4");
    assert!(Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=32x32:r=60",
            "-frames:v",
            "56",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&output)
        .status()
        .unwrap()
        .success());
    let original = std::fs::read(&output).unwrap();
    let error = stitch_complete(
        dir.path(),
        &owner.manifest.checkpoints,
        "ffmpeg",
        "ffprobe",
        "source.mp4",
    )
    .unwrap_err();
    assert!(error.to_string().contains("existing stitched output"));
    assert_eq!(
        std::fs::read(output).unwrap(),
        original,
        "no-replace publication preserves evidence"
    );
}

#[test]
fn invalid_output_cadence_is_rejected_by_writer_and_restart_reader() {
    for fps in [0, 241] {
        let dir = tempdir().unwrap();
        let mut start = CaptureStart::new("invalid", 100);
        start.output_cadence = Some(FrameRate { num: fps, den: 1 });
        assert!(ManifestOwner::begin(dir.path(), start.clone()).is_err());
        let line = serde_json::to_string(&crate::journal::Entry::Start(start)).unwrap();
        std::fs::write(dir.path().join(crate::MANIFEST_FILE), format!("{line}\n")).unwrap();
        assert!(read_manifest(dir.path()).is_err());
    }
}

#[test]
fn real_ffmpeg_fractional_requests_keep_admitted_backend_and_normalized_rationals() {
    for request in [29.97, 59.94] {
        let cadence = record_core::CaptureCadence::from_server_fps(request).unwrap();
        assert_eq!(
            cadence.requested,
            FrameRate::from_server_decimal(request).unwrap()
        );
        let dir = tempdir().unwrap();
        let owner = sparse_capture_with_rate(dir.path(), Some(cadence.backend_requested));
        let path = stitch_complete(
            dir.path(),
            &owner.manifest.checkpoints,
            "ffmpeg",
            "ffprobe",
            "source.mp4",
        )
        .unwrap();
        let media = assert_output(dir.path(), &path, cadence.backend_requested.num);
        assert_eq!(media.avg_frame_rate, Some(cadence.backend_requested));
        assert_eq!(media.r_frame_rate, Some(cadence.backend_requested));
    }

    // Exercise an exact fractional manifest rate as well as current native
    // whole-FPS admission. Equivalent unreduced JSON must normalize before
    // comparison with FFprobe's reduced container-rate evidence.
    let dir = tempdir().unwrap();
    let owner = sparse_capture_with_rate(
        dir.path(),
        Some(FrameRate {
            num: 60_000,
            den: 2_002,
        }),
    );
    let expected = FrameRate::new(30_000, 1_001).unwrap();
    assert_eq!(
        read_manifest(dir.path()).unwrap().start.output_cadence,
        Some(expected)
    );
    let path = stitch_complete(
        dir.path(),
        &owner.manifest.checkpoints,
        "ffmpeg",
        "ffprobe",
        "source.mp4",
    )
    .unwrap();
    let media = verify_media("ffmpeg", "ffprobe", &path).unwrap();
    assert_eq!(media.avg_frame_rate, Some(expected));
    assert_eq!(media.r_frame_rate, Some(expected));
    assert!(media.decoded_video_frames > 0);
    assert!(media.duration_ms.abs_diff(937) <= 34);
    assert_eq!(FrameRate::from_ffprobe("60000/2002"), Some(expected));
    assert_eq!(
        stitch_complete(
            dir.path(),
            &owner.manifest.checkpoints,
            "ffmpeg",
            "ffprobe",
            "source.mp4"
        )
        .unwrap(),
        path
    );
}
