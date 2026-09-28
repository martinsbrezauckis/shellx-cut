use super::camera::read_raw_frame;
use super::{decode_square, grab_frame, rgba_frame_bytes};

#[test]
fn rgba_geometry_rejects_zero_and_overflow_before_allocation() {
    assert_eq!(rgba_frame_bytes(2, 3, "test").unwrap(), 24);
    assert!(rgba_frame_bytes(0, 3, "test").is_err());
    assert!(rgba_frame_bytes(u32::MAX, u32::MAX, "test").is_err());
}

#[test]
fn webcam_decode_rejects_invalid_geometry_and_fps_before_spawning() {
    assert!(decode_square("unused", 0, 30.0).is_err());
    assert!(decode_square("unused", 8193, 30.0).is_err());
    assert!(decode_square("unused", 64, f64::NAN).is_err());
    assert!(decode_square("unused", 64, 241.0).is_err());
}

#[test]
fn raw_frames_above_diagnostic_cap_are_complete_and_partial_frames_fail() {
    let full = vec![0x73; 800 * 400 * 4];
    let mut input = std::io::Cursor::new(full.clone());
    assert_eq!(read_raw_frame(&mut input, full.len()).unwrap(), Some(full));
    assert!(read_raw_frame(&mut input, 4).unwrap().is_none());
    let mut partial = std::io::Cursor::new(vec![7; 3]);
    assert!(read_raw_frame(&mut partial, 4).is_err());
}

#[test]
fn grab_frame_accepts_raw_video_larger_than_diagnostic_cap() {
    let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    if !std::process::Command::new(&ffmpeg)
        .arg("-version")
        .output()
        .is_ok_and(|out| out.status.success())
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("wide.mp4");
    let encoded = std::process::Command::new(&ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=800x400:r=1:d=1",
            "-frames:v",
            "1",
            "-c:v",
            "libx264",
        ])
        .arg(&src)
        .output()
        .unwrap();
    assert!(encoded.status.success());
    let (w, h, pixels) = grab_frame(src.to_str().unwrap(), 0).unwrap();
    assert_eq!((w, h, pixels.len()), (800, 400, 800 * 400 * 4));
    assert!(pixels[0] > 150);
}

#[test]
fn camera_stream_accepts_a_clean_short_tail_but_rejects_decode_failure() {
    let ffmpeg = std::env::var("SHELLX_RECORD_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    if !std::process::Command::new(&ffmpeg)
        .arg("-version")
        .output()
        .is_ok_and(|out| out.status.success())
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("short.mp4");
    let encoded = std::process::Command::new(&ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=64x64:r=10:d=0.3",
            "-c:v",
            "libx264",
        ])
        .arg(&src)
        .output()
        .unwrap();
    assert!(encoded.status.success());
    let mut camera = super::camera::stream_square_with_control(
        src.to_str().unwrap(),
        32,
        10.0,
        &super::default_control(),
    )
    .unwrap();
    assert_eq!(camera.at(0).unwrap().unwrap().len(), 32 * 32 * 4);
    assert!(camera.at(10).unwrap().is_none());
    assert!(camera.at(11).unwrap().is_none());
    camera.finish().unwrap();

    let invalid = dir.path().join("invalid.mov");
    std::fs::write(&invalid, b"not a camera movie").unwrap();
    let mut failed = super::camera::stream_square_with_control(
        invalid.to_str().unwrap(),
        32,
        10.0,
        &super::default_control(),
    )
    .unwrap();
    assert!(
        failed.at(0).is_err(),
        "nonzero ffmpeg exit must not be a clean tail"
    );

    let longer = dir.path().join("long-camera.mp4");
    let encoded = std::process::Command::new(&ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=64x64:r=10:d=5",
            "-c:v",
            "libx264",
        ])
        .arg(&longer)
        .output()
        .unwrap();
    assert!(encoded.status.success());
    let mut unused_tail = super::camera::stream_square_with_control(
        longer.to_str().unwrap(),
        32,
        10.0,
        &super::default_control(),
    )
    .unwrap();
    assert!(unused_tail.at(0).unwrap().is_some());
    unused_tail.finish().unwrap();
}
