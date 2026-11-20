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

#[test]
fn normalized_audio_is_bounded_by_the_video_length_not_by_shortest() {
    let args = normalize_audio_args(2_000, 30.0);
    assert_eq!(args[0], "-af");
    assert!(
        args[1].ends_with("apad=whole_dur=2.000,atrim=duration=2.000"),
        "{args:?}"
    );
    assert!(!args.iter().any(|arg| arg == "-shortest"), "{args:?}");
    // A length that is not a whole number of frames rounds up to the next one.
    let odd = normalize_audio_args(1_010, 30.0);
    assert!(odd[1].contains("whole_dur=1.034"), "{odd:?}");
}

/// Run `encode_frames_with_control` against a fake ffmpeg script (a shell script,
/// or a batch file on Windows), passed by path so no process-wide setting changes.
fn encode_with_fake_ffmpeg(
    unix_body: &str,
    windows_body: &str,
    control: ProcessControl,
) -> (Result<u64>, std::time::Duration) {
    let dir = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let fake = {
        let p = dir.path().join("fake-ffmpeg.cmd");
        std::fs::write(&p, windows_body.replace('\n', "\r\n")).unwrap();
        let _ = unix_body;
        p
    };
    #[cfg(not(windows))]
    let fake = {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.path().join("fake-ffmpeg.sh");
        std::fs::write(&p, unix_body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = windows_body;
        p
    };
    // 512x512 RGBA frames are 1 MiB each: more than a pipe buffer, so the write
    // cannot finish before the child stops reading or is gone.
    let out = dir.path().join("out.mp4");
    let started = std::time::Instant::now();
    let result = super::encode::encode_frames_with_control(
        &fake.display().to_string(),
        &out.display().to_string(),
        512,
        512,
        30.0,
        8,
        control,
        |_| vec![0u8; 512 * 512 * 4],
    );
    let elapsed = started.elapsed();
    (result, elapsed)
}

/// ffmpeg that stops at once, without reading a frame, must be reported by its
/// own exit status and stderr, not as a broken pipe on the write.
#[test]
fn ffmpeg_that_dies_while_frames_are_written_reports_its_own_error() {
    let (result, _) = encode_with_fake_ffmpeg(
        "#!/bin/sh\necho 'fake-ffmpeg: unsupported codec' >&2\nexit 7\n",
        "@echo fake-ffmpeg: unsupported codec 1>&2\n@exit /b 7\n",
        ProcessControl::bounded(std::time::Duration::from_secs(60), || false),
    );
    let text = format!("{:?}", result.expect_err("a dead encoder is an error"));
    assert!(text.contains("ffmpeg encode failed"), "{text}");
    assert!(text.contains("unsupported codec"), "{text}");
    assert!(!text.contains("write encode frame"), "{text}");
}

/// ffmpeg that stays alive but never reads blocks the frame write; the call must
/// still end at its deadline with the timeout error, not wait forever.
#[test]
fn ffmpeg_that_stops_reading_ends_at_the_deadline_with_a_timeout() {
    let (result, elapsed) = encode_with_fake_ffmpeg(
        "#!/bin/sh\nsleep 120\n",
        "@ping -n 120 127.0.0.1 >nul\n",
        ProcessControl::bounded(std::time::Duration::from_secs(2), || false),
    );
    let text = format!("{:?}", result.expect_err("a stalled encoder must time out"));
    assert!(text.contains("timed out"), "{text}");
    assert!(elapsed < std::time::Duration::from_secs(30), "{elapsed:?}");
}

/// The same bound answers a cancellation request.
#[test]
fn ffmpeg_that_stops_reading_ends_on_cancellation() {
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let (result, elapsed) = encode_with_fake_ffmpeg(
        "#!/bin/sh\nsleep 120\n",
        "@ping -n 120 127.0.0.1 >nul\n",
        ProcessControl::bounded(std::time::Duration::from_secs(600), move || {
            cancel.load(std::sync::atomic::Ordering::SeqCst)
        }),
    );
    canceller.join().unwrap();
    let text = format!("{:?}", result.expect_err("a cancelled encode is an error"));
    assert!(
        text.contains("render_cancelled") || text.contains("cancelled"),
        "{text}"
    );
    assert!(elapsed < std::time::Duration::from_secs(30), "{elapsed:?}");
}

/// Parallel composition writes exactly the frames, in the order, a serial
/// render writes: the two outputs decode to the same pictures.
#[test]
fn parallel_and_serial_renders_write_the_same_frames() {
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
    let made = Command::new(&ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc=size=64x64:rate=10:duration=2")
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(made.status.success());
    // Each output frame carries its time in its pixels, so a frame written
    // out of order or twice changes the picture sequence.
    let paint = |frame: Vec<u8>, time_ms: u64| {
        let mut out = frame;
        let shade = (time_ms / 100 * 12) as u8;
        for pixel in out.chunks_mut(4).take(64 * 16) {
            pixel.copy_from_slice(&[shade, 255 - shade, shade / 2, 255]);
        }
        Ok(out)
    };
    let render = |workers: usize, name: &str| {
        let out = dir.path().join(name);
        let count = render_pipe_parallel_with_control(
            source.to_str().unwrap(),
            out.to_str().unwrap(),
            64,
            64,
            10.0,
            source.to_str().unwrap(),
            false,
            &default_control(),
            workers,
            X264_EXPORT_PRESET,
            |frame, time_ms| Ok((frame, time_ms)),
            |(frame, time_ms), _| paint(frame, time_ms),
        )
        .unwrap();
        let hashes = Command::new(ffmpeg_bin())
            .args(["-v", "error", "-i"])
            .arg(&out)
            .args(["-f", "framemd5", "-"])
            .output()
            .unwrap();
        (count, String::from_utf8_lossy(&hashes.stdout).into_owned())
    };
    let (serial_count, serial) = render(1, "serial.mp4");
    let (parallel_count, parallel) = render(4, "parallel.mp4");
    assert_eq!(serial_count, 20);
    assert_eq!(parallel_count, serial_count);
    assert_eq!(parallel, serial);
}
