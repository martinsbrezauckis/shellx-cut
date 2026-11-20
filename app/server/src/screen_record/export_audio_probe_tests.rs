#[cfg(unix)]
#[test]
fn prepared_audio_literal_probe_obeys_cancel() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stall%03d.wav");
    std::fs::write(&path, b"not media").unwrap();
    let fake = dir.path().join("slow-ffprobe");
    std::fs::write(
        &fake,
        "#!/bin/sh\ntouch \"$CUT_EXPORT_AUDIO_PROBE_DIR/started\"\nexec sleep 120\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "screen_record::export_audio::probe_tests::prepared_audio_probe_child",
            "--nocapture",
        ])
        .env("CUT_EXPORT_AUDIO_PROBE_DIR", dir.path())
        .env("SHELLX_CUT_FFPROBE", &fake)
        .spawn()
        .unwrap();
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "prepared audio probe child failed");
            assert!(
                dir.path().join("started").is_file(),
                "stand-in probe never started"
            );
            break;
        }
        if started.elapsed() > Duration::from_secs(2) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("prepared audio probe outlived cancellation");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
#[test]
fn prepared_audio_probe_child() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    let Some(dir) = std::env::var_os("CUT_EXPORT_AUDIO_PROBE_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let stopped = Arc::new(AtomicBool::new(false));
    let on_cancel = stopped.clone();
    let control =
        record_render::ffmpeg::ProcessControl::bounded(Duration::from_secs(10), move || {
            on_cancel.load(Ordering::Relaxed)
        });
    let marker = dir.join("started");
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        while !marker.is_file() && started.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(10));
        }
        stopped.store(true, Ordering::Relaxed);
    });
    let inputs = super::CaptureExportAudio {
        mic: None,
        system: Some(dir.join("stall%03d.wav")),
        system_offset_ms: 25,
    };
    let error = inputs
        .prepare(&dir, &control)
        .err()
        .expect("probe must stop");
    assert_eq!(error.code, "render_cancelled", "{error}");
}
