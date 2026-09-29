use super::*;

#[test]
fn compose_error_survives_owned_pipe_cleanup() {
    let ffmpeg = ffmpeg_bin();
    if !Command::new(&ffmpeg)
        .arg("-version")
        .output()
        .is_ok_and(|out| out.status.success())
    {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("screen.mp4");
    let output = dir.path().join("out.mp4");
    let made = Command::new(&ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:r=10:d=0.5",
            "-c:v",
            "libx264",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(made.status.success());
    let error = render_pipe_with_control(
        source.to_str().unwrap(),
        output.to_str().unwrap(),
        64,
        64,
        10.0,
        source.to_str().unwrap(),
        false,
        &default_control(),
        |_, _| {
            Err(record_core::RecordError::new(
                "camera_sentinel",
                "camera failed",
                "original",
            ))
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "camera_sentinel");
    assert_eq!(error.cause, "original");
}
