//! A readable header does not prove that a still can be decoded.
cut_core::isolate_test_process!();
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn damaged_picture_is_refused_by_source_probe() {
    let path = fixture("original-still.png");
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 777);
    let error = cut_media::probe_source(&path).unwrap_err();
    assert_eq!(error.code, cut_core::error_codes::UNREADABLE_MEDIA);
    assert_eq!(
        error.message,
        "This picture file is damaged and cannot be used: original-still.png"
    );
    assert!(!error.cause.is_empty());
    assert!(error.suggested_action.is_some());
}

#[test]
fn looped_damaged_picture_cannot_outlive_owned_deadline() {
    let mut command = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin());
    command.args(["-nostdin", "-v", "error", "-loop", "1"]);
    cut_media::ffmpeg::push_ffmpeg_literal_input(&mut command, &fixture("original-still.png"))
        .unwrap();
    command.args(["-frames:v", "1", "-f", "null", "-"]);
    let control =
        cut_media::ffmpeg::OwnedProcessControl::bounded(std::time::Duration::from_secs(2), || {
            false
        });
    let started = std::time::Instant::now();
    match cut_media::ffmpeg::run_owned_command(&mut command, &control, "damaged looped still") {
        Err(error) => assert!(error.message.contains("timed out"), "{error:?}"),
        // A tool version which rejects the bad file itself is also safe.
        Ok(output) => assert!(!output.status.success()),
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(120));
    // A subsequent good picture still decodes; the first process was reaped
    // before the owner returned (the platform tree tests verify descendant PIDs).
    assert_eq!(
        cut_media::probe_source(&fixture("valid-still.png"))
            .unwrap()
            .kind,
        "image"
    );
}

#[test]
fn valid_picture_still_probes_and_renders() {
    let path = fixture("valid-still.png");
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 777);
    let probe = cut_media::probe_source(&path).unwrap();
    assert_eq!(probe.kind, "image");
    assert_eq!((probe.width, probe.height), (Some(320), Some(180)));
    let mut command = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin());
    command.args(["-v", "error", "-loop", "1"]);
    cut_media::ffmpeg::push_ffmpeg_literal_input(&mut command, &path).unwrap();
    command.args(["-frames:v", "1", "-f", "hash", "-hash", "sha256", "-"]);
    let output =
        cut_media::ffmpeg::run_bounded_command(&mut command, "valid still comparison").unwrap();
    assert!(output.status.success(), "{:?}", output);
    println!(
        "VALID_STILL_FRAME {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
}
