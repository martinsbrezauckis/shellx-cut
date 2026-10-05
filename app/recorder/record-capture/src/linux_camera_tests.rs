//! Linux camera native and finalization regressions.

use super::*;
use crate::camera_finalization::CameraMediaProbe;
use crate::camera_finalization_anchored::open_capture_dir;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::process::Command;
use std::time::Duration;

#[test]
fn actual_unnamed_mp4_is_probed_through_held_parent_descriptor() {
    let root = tempfile::tempdir().unwrap();
    let capture = root.path().join("capture");
    fs::create_dir(&capture).unwrap();
    let owner = open_capture_dir(&capture).unwrap();
    let writer = owner.nameless_writable_stage().unwrap();
    let output = format!("/proc/{}/fd/{}", std::process::id(), writer.as_raw_fd());
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=30",
            "-frames:v",
            "3",
            "-c:v",
            "mpeg4",
            "-f",
            "mp4",
            "-y",
            &output,
        ])
        .status()
        .unwrap();
    assert!(
        status.success(),
        "synthetic movie did not encode into nameless stage"
    );
    let closed = File::open(format!("/proc/self/fd/{}", writer.as_raw_fd())).unwrap();
    drop(writer);
    let facts = LinuxMediaProbe.measure(&closed).unwrap();
    assert_eq!((facts.width, facts.height, facts.frame_count), (64, 48, 3));
    assert!(facts.duration_ms > 0);
    assert_eq!(closed.metadata().unwrap().nlink(), 0);
    owner.ensure_plain_child(c"camera").unwrap();
    let capture_owner = CameraCaptureDirectory::reserve(&capture).unwrap();
    let staged = StagedCameraMedia {
        artifact_id: "camera_01".into(),
        video: "camera/camera.mp4".into(),
        declared: DeclaredCameraMediaFacts {
            width: facts.width,
            height: facts.height,
            fps_num: facts.fps_num,
            fps_den: facts.fps_den,
            frame_count: facts.frame_count,
            duration_ms: facts.duration_ms,
        },
    };
    let seal = finalize_staged_camera_media(
        &mut ClosedStage(Some(closed)),
        &LinuxMediaProbe,
        &capture_owner,
        &staged,
    )
    .unwrap();
    assert_eq!(seal.media().frame_count, 3);
    assert_eq!(seal.media().duration_ms, facts.duration_ms);
    assert!(capture.join("camera/camera.mp4").is_file());
}

#[test]
fn native_interval_projection_is_bounded_to_one_millisecond() {
    let origin = Instant::now();
    let start = origin + Duration::from_nanos(1_234_600_000);
    let end = start + Duration::from_nanos(33_333_333);
    let (projected, duration_ms) = project_interval(start, end, origin).unwrap();
    assert_eq!(duration_ms, 33);
    assert_eq!(
        projected.started_at.duration_since(origin),
        Duration::from_millis(1235)
    );
    assert_eq!(
        projected.ended_at.duration_since(origin),
        Duration::from_millis(1268)
    );
    assert!(projected.ended_at.duration_since(end) < Duration::from_millis(1));
    assert!(project_interval(origin - Duration::from_millis(1), end, origin).is_err());
}

#[test]
fn root_replacement_cannot_create_camera_child_outside_reserved_owner() {
    let root = tempfile::tempdir().unwrap();
    let capture = root.path().join("capture");
    let outside = root.path().join("outside");
    fs::create_dir(&capture).unwrap();
    fs::create_dir(&outside).unwrap();
    let reserved = CameraCaptureDirectory::reserve(&capture).unwrap();
    fs::rename(&capture, root.path().join("old_capture")).unwrap();
    std::os::unix::fs::symlink(&outside, &capture).unwrap();
    assert!(reserved.capture_dir().is_err());
    assert!(!outside.join("camera").exists());
}

#[test]
fn native_bus_reasons_are_conservative_and_do_not_expose_paths() {
    assert_eq!(reason_kind("Permission denied"), NativeReason::Permission);
    assert_eq!(reason_kind("Device or resource busy"), NativeReason::Busy);
    assert_eq!(reason_kind("No such device"), NativeReason::Disconnected);
    assert_eq!(
        reason_kind("codec failure /dev/video15"),
        NativeReason::Other
    );
    assert!(!NativeReason::Other.detail().contains("/dev/"));
}

#[test]
fn native_opened_fd_rejects_wrong_generation_and_non_camera_device() {
    let null = File::open("/dev/null").unwrap();
    let meta = null.metadata().unwrap();
    assert_eq!(
        unsafe { sxc_linux_camera_validate_fd(null.as_raw_fd(), meta.rdev(), meta.ino()) },
        0,
        "a character node without V4L2 capture capability must be refused"
    );
    assert_eq!(
        unsafe { sxc_linux_camera_validate_fd(null.as_raw_fd(), meta.rdev(), meta.ino() + 1) },
        0,
        "a replaced node generation must be refused"
    );
}

#[test]
fn actual_gstreamer_zero_frame_source_refuses_and_retires() {
    let pointer = unsafe { sxc_linux_camera_test_empty() };
    let pointer = NonNull::new(pointer).expect("core GStreamer fakesrc/fakesink must be installed");
    let first = unsafe { sxc_linux_camera_first(pointer.as_ptr(), 200) };
    assert_ne!(first, 0, "zero-buffer source cannot prove a camera frame");
    let stop = unsafe { sxc_linux_camera_stop(pointer.as_ptr(), 200, FIRST_FRAME_MS) };
    assert_ne!(stop, 0, "zero-buffer source cannot become sealed media");
    assert_ne!(unsafe { sxc_linux_camera_retired(pointer.as_ptr()) }, 0);
    unsafe { sxc_linux_camera_free(pointer.as_ptr()) };
}

#[test]
fn native_probe_handles_eos_frame_race_but_rejects_regression_and_stale_stop() {
    assert_eq!(unsafe { sxc_linux_camera_test_timestamp_contract() }, 0);
}
