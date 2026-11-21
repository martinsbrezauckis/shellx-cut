//! A killed encoder leaves one independently verified checkpoint recoverable.

use super::super::{clock_now_ns, NativeSegment};
use super::{frame, packet_times};
use crate::{checkpoint::Checkpoints, CheckpointConfig};
use record_recovery::{
    owner_state, read_manifest, recover_interrupted, recovery_status, verify_media,
    CaptureRecoveryState, CaptureStart, CheckpointFacts, ManifestOwner, OwnerState, PrivateStaging,
    RecoveryState,
};
use std::{
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const CHILD_ROOT: &str = "SXC_WRITER_RECOVERY_CHILD_ROOT";
const CHILD_TEST: &str =
    "macos_screen_writer::tests::recovery_tests::native_writer_child_stops_during_open_segment";

fn tools() -> (String, String) {
    (
        std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into()),
        std::env::var("SHELLX_RECORD_FFPROBE").unwrap_or_else(|_| "ffprobe".into()),
    )
}

#[test]
fn native_writer_child_stops_during_open_segment() {
    // The child half of the recovery test below: that test starts this one in
    // a second process, hands it a folder and kills it while a segment is
    // open. Started on its own it has no folder and nothing to do.
    let Ok(root) = std::env::var(CHILD_ROOT) else {
        return;
    };
    let root = Path::new(&root);
    ManifestOwner::begin(root, CaptureStart::new("writer-kill-fixture", 1_000)).unwrap();
    let config = CheckpointConfig {
        manifest_dir: root.to_string_lossy().into_owned(),
        interval_ms: 1_000,
    };
    let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
    let (first_sequence, first_reserved) = checkpoints.begin(0).unwrap();
    let first_stage = PrivateStaging::create(root, "writer-first", "segment.mp4").unwrap();
    let first = NativeSegment::new(first_stage.path(), 256, 144, 30).unwrap();
    let base = clock_now_ns();
    let pts = |n: u32| base + i64::from(n) * 1_000_000_000 / 30;
    for n in 0..30 {
        first.append(&frame(pts(n), 30, n)).unwrap();
    }
    first.finish(base + 1_000_000_000).unwrap();
    assert_eq!(first.stats().2, 30);
    record_recovery::publish_new_synced(first_stage.path(), &first_reserved).unwrap();
    checkpoints
        .publish_without_decode(
            first_sequence,
            &first_reserved,
            CheckpointFacts {
                start_ms: 0,
                end_ms: 1_000,
                event_offset_ms: 0,
                audio_offset_ms: None,
            },
        )
        .unwrap();

    // This open reservation deliberately has no published checkpoint. Its
    // writer is still active when the parent kills this process.
    let (_second_sequence, _second_reserved) = checkpoints.begin(1_000).unwrap();
    let second_stage = PrivateStaging::create(root, "writer-second", "segment.mp4").unwrap();
    let second = NativeSegment::new(second_stage.path(), 256, 144, 30).unwrap();
    for n in 30..40 {
        second.append(&frame(pts(n), 30, n)).unwrap();
    }
    std::fs::write(root.join("writer-ready"), b"second writer is active").unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    panic!("parent did not kill the active writer within 60 seconds");
}

#[test]
fn killed_native_writer_recovers_only_its_durable_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().unwrap();
    let mut child = Command::new(exe)
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env(CHILD_ROOT, dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !dir.path().join("writer-ready").is_file() {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            panic!(
                "native writer child exited {status}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "native writer child did not reach its open segment: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
    let before = read_manifest(dir.path()).unwrap();
    assert_eq!(before.checkpoints.len(), 1);
    assert!(before.has_open_segment());
    let before_status = recovery_status(dir.path());
    assert_eq!(before_status.checkpoints, 1);
    assert!(before_status.has_open_segment);
    assert!(before.receipt.is_none());
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGKILL) }, 0);
    let killed = child.wait_with_output().unwrap();
    assert!(!killed.status.success());
    assert_eq!(owner_state(&before.start), OwnerState::Dead);

    let (ffmpeg, ffprobe) = tools();
    let recovered = recover_interrupted(dir.path(), &ffmpeg, &ffprobe, owner_state(&before.start))
        .unwrap()
        .unwrap();
    assert_eq!(recovered.receipt.state, RecoveryState::Recovered);
    assert_eq!(recovered.receipt.recovered_segments, 1);
    assert_eq!(recovered.receipt.source.as_deref(), Some("recovered.mp4"));
    let after_status = recovery_status(dir.path());
    assert_eq!(after_status.state, CaptureRecoveryState::Recovered);
    assert_eq!(after_status.checkpoints, 1);
    assert!(after_status.has_open_segment);
    let measured = verify_media(&ffmpeg, &ffprobe, &dir.path().join("recovered.mp4")).unwrap();
    assert_eq!(measured.decoded_video_frames, 30);
    assert!(measured.duration_ms.abs_diff(1_000) <= 34, "{measured:?}");
    assert!(
        recover_interrupted(dir.path(), &ffmpeg, &ffprobe, OwnerState::Dead)
            .unwrap()
            .is_none()
    );
}

fn seam_picture_hashes(path: &Path, ffmpeg: &str) -> Vec<String> {
    let output = Command::new(ffmpeg)
        .args(["-v", "error", "-ss", "14.8"])
        .arg("-i")
        .arg(path)
        .args([
            "-t",
            "0.4",
            "-an",
            "-vf",
            "scale=32:32",
            "-f",
            "framemd5",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "seam decode: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| line.rsplit(',').next().unwrap().trim().to_owned())
        .collect()
}

#[test]
fn native_writer_checkpoints_stitch_without_a_held_picture_at_30_and_60_fps() {
    let (ffmpeg, ffprobe) = tools();
    for fps in [30_u32, 60] {
        let dir = tempfile::tempdir().unwrap();
        let mut start = CaptureStart::new(format!("writer-stitch-{fps}"), 15_000);
        start.output_cadence = Some(record_core::FrameRate::new(u64::from(fps), 1).unwrap());
        ManifestOwner::begin(dir.path(), start).unwrap();
        let config = CheckpointConfig {
            manifest_dir: dir.path().to_string_lossy().into_owned(),
            interval_ms: 15_000,
        };
        let mut checkpoints = Checkpoints::open(Some(&config)).unwrap().unwrap();
        let base = clock_now_ns();
        for (sequence, first_frame, end_frame) in
            [(0_u64, 0_u32, 15 * fps), (1_u64, 15 * fps, 16 * fps)]
        {
            let start_ms = u64::from(first_frame) * 1_000 / u64::from(fps);
            let end_ms = u64::from(end_frame) * 1_000 / u64::from(fps);
            let (reserved_sequence, staging) = checkpoints.begin(start_ms).unwrap();
            assert_eq!(reserved_sequence, sequence);
            let writer = NativeSegment::new(&staging, 256, 144, fps).unwrap();
            for number in first_frame..end_frame {
                let pts = base + i64::from(number) * 1_000_000_000 / i64::from(fps);
                writer.append(&frame(pts, fps, number)).unwrap();
            }
            let end_pts = base + i64::from(end_frame) * 1_000_000_000 / i64::from(fps);
            writer.finish(end_pts).unwrap();
            assert_eq!(writer.stats().2, u64::from(end_frame - first_frame));
            checkpoints
                .publish_without_decode(
                    sequence,
                    &staging,
                    CheckpointFacts {
                        start_ms,
                        end_ms,
                        event_offset_ms: start_ms,
                        audio_offset_ms: None,
                    },
                )
                .unwrap();
        }
        let (source, media, notes) = checkpoints.stitch(&ffmpeg, &ffprobe, "source.mp4").unwrap();
        assert!(notes.is_empty(), "{fps} fps: {notes:?}");
        assert_eq!(media.decoded_video_frames, u64::from(16 * fps));
        assert!(
            media.duration_ms.abs_diff(16_000) <= 1_000 / u64::from(fps) + 1,
            "{fps} fps: {media:?}"
        );
        assert_eq!(
            media.avg_frame_rate,
            Some(record_core::FrameRate::new(u64::from(fps), 1).unwrap())
        );
        let times = packet_times(&source);
        assert_eq!(times.len(), (16 * fps) as usize);
        assert!(times.windows(2).all(|pair| pair[1] > pair[0]));
        let maximum_gap = times
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .fold(0.0_f64, f64::max);
        assert!(
            maximum_gap <= 2.0 / f64::from(fps) + 0.000_001,
            "{fps} fps: maximum stitched packet gap {maximum_gap:.6}s"
        );
        let hashes = seam_picture_hashes(&source, &ffmpeg);
        assert!(
            hashes.len() >= (fps / 4) as usize,
            "{fps} fps: too few seam pictures"
        );
        let longest_identical_run = hashes
            .windows(2)
            .fold((1_usize, 1_usize), |(current, longest), pair| {
                let current = if pair[0] == pair[1] { current + 1 } else { 1 };
                (current, longest.max(current))
            })
            .1;
        assert!(
            longest_identical_run <= 2,
            "{fps} fps: {longest_identical_run} identical decoded seam frames"
        );
        println!(
            "{fps} fps: {} stitched frames, {maximum_gap:.6}s maximum timestamp gap, {} seam pictures, no hold notes",
            media.decoded_video_frames,
            hashes.len(),
        );
    }
}
#[test]
fn a_sparse_native_segment_holds_a_short_empty_tail_at_the_recording_cadence() {
    for fps in [30_u32, 60] {
        let dir = tempfile::tempdir().unwrap();
        let mut start = CaptureStart::new(format!("sparse-native-{fps}"), 15_000);
        start.output_cadence = Some(record_core::FrameRate::new(u64::from(fps), 1).unwrap());
        ManifestOwner::begin(dir.path(), start).unwrap();
        let config = crate::CheckpointConfig {
            manifest_dir: dir.path().to_string_lossy().into_owned(),
            interval_ms: 15_000,
        };
        let mut checkpoints = crate::checkpoint::Checkpoints::open(Some(&config))
            .unwrap()
            .unwrap();
        let (sequence, stage) = checkpoints.begin(0).unwrap();
        let writer = NativeSegment::new(&stage, 256, 144, fps).unwrap();
        let base = clock_now_ns();
        for (index, offset) in [0, 16_666_667, 1_016_666_667, 8_983_333_333]
            .into_iter()
            .enumerate()
        {
            let mut error = [0; super::ERROR_CAPACITY];
            let raw = unsafe {
                super::sxc_screen_writer_fixture_frame(
                    256,
                    144,
                    base + offset,
                    7_966_666_667,
                    index as u32,
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            assert!(!raw.is_null(), "{}", super::message(&error));
            let sample = unsafe { super::CMSampleBuffer::from_ptr(raw) };
            writer.append(&sample).unwrap();
        }
        writer.finish(base + 10_112_000_000).unwrap();
        let facts = |start_ms, end_ms| record_recovery::CheckpointFacts {
            start_ms,
            end_ms,
            event_offset_ms: start_ms,
            audio_offset_ms: None,
        };
        checkpoints
            .publish_without_decode(sequence, &stage, facts(0, 10_112))
            .unwrap();
        let (sequence, empty) = checkpoints.begin(10_112).unwrap();
        let held = checkpoints
            .publish_held_frame(sequence, &empty, facts(10_112, 10_267))
            .unwrap();
        let media = held.media.unwrap();
        assert_eq!(media.decoded_video_frames, 1);
        assert!(media.duration_ms <= 1_000 / u64::from(fps) + 1, "{media:?}");
        assert_eq!(held.held_last_frame_ms, Some(155));
        assert!(record_recovery::read_manifest(dir.path()).is_ok());
        let (ffmpeg, ffprobe) = tools();
        let (_, stitched, notes) = checkpoints.stitch(&ffmpeg, &ffprobe, "source.mp4").unwrap();
        assert!(notes.is_empty(), "{fps} fps: {notes:?}");
        assert!(stitched.duration_ms.abs_diff(10_267) <= 34, "{stitched:?}");
        println!("{fps} fps sparse native tail: one {}ms held frame covers155ms; manifest and stitched take valid", media.duration_ms);
    }
}
