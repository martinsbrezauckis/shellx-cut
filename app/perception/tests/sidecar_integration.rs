//! sidecar_integration.rs — REAL python sidecar over generated test footage.
//!
//! Role: proves the Rust↔python wire contract end-to-end: spawn venv python,
//! full instrument battery on testdata/talking_head.mp4, validate the typed
//! report, then prove the asset-hash cache short-circuits the second call.
//!
//! `#[ignore]` by default because it needs local state a bare CI checkout
//! lacks: app/perception/py/.venv installed AND scripts/make-test-assets.sh
//! run. Execute explicitly:
//!     cargo test -p cut-perception --test sidecar_integration -- --ignored

use cut_perception::{run_instruments, transcribe, InstrumentSet};
use std::path::PathBuf;
use std::time::Instant;

/// Repo root = CARGO_MANIFEST_DIR (app/perception) /../..
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

#[test]
#[ignore = "needs venv + make-test-assets.sh output (see module docs)"]
fn full_battery_on_test_footage_then_cache_hit() {
    let media = repo_root().join("testdata/talking_head.mp4");
    assert!(
        media.exists(),
        "run scripts/make-test-assets.sh first — {} missing",
        media.display()
    );
    let receipts = tempfile::tempdir().unwrap();

    // ---- 1. real full run -------------------------------------------------
    let report = run_instruments(
        &media,
        receipts.path(),
        "a1",
        "sha256:itest", // sidecar echoes the request hash; any value works
        InstrumentSet::Full,
        None,
    )
    .expect("full instrument run must succeed");

    let words = report.words.as_ref().expect("words instrument ran");
    assert!(
        words.words.len() > 50,
        "60s spoken clip → >50 words, got {}",
        words.words.len()
    );
    assert!(
        words
            .model
            .starts_with("parakeet-tdt/nemo-parakeet-tdt-0.6b-v3@onnx"),
        "default Parakeet model provenance recorded: {}",
        words.model
    );
    // The test clip has 5 inserted 2-4s silences (make-test-assets.sh truth).
    assert!(
        report.silences.len() >= 5,
        "expected >=5 silences, got {}",
        report.silences.len()
    );
    // One hard testsrc2→testsrc cut at the midpoint.
    assert!(!report.scenes.is_empty(), "scene cut not detected");
    assert!(report.loudness.is_some(), "loudness missing");
    assert!(report.beats.is_some(), "beats missing");
    // testsrc sources are in constant motion — no black/frozen frames.
    assert!(report.black_spans.is_empty() && report.frozen_spans.is_empty());
    // Receipt persisted where the server will look for it.
    assert!(receipts.path().join("a1.perception.json").exists());

    // ---- 2. cache hit: same hash+set returns WITHOUT re-running python ----
    let t0 = Instant::now();
    let cached = run_instruments(
        &media,
        receipts.path(),
        "a1",
        "sha256:itest",
        InstrumentSet::Full,
        None,
    )
    .expect("cache read must succeed");
    assert_eq!(
        cached, report,
        "cache must return the persisted report verbatim"
    );
    assert!(
        t0.elapsed().as_millis() < 500,
        "cache hit took {:?} — python was re-run",
        t0.elapsed()
    );

    // ---- 3. transcribe() writes the words.json sidecar file ----------------
    let transcript = transcribe(&media, receipts.path(), "a1", "sha256:itest", None)
        .expect("transcribe must reuse the cached report");
    assert_eq!(transcript.words.len(), words.words.len());
    assert!(receipts.path().join("a1.words.json").exists());
}

/// audio-only media guard: a plain WAV (audio-only — narration/music-bed bread-and-butter)
/// must survive the audio battery. Pre-fix the chain requested "scenes" and
/// PySceneDetect raised VideoOpenFailure, failing the whole import chain at
/// 80%. AudioFull must complete with silence + loudness + beats facts and an
/// instruments_run that honestly lacks "scenes".
#[test]
#[ignore = "needs venv (see module docs); ffmpeg generates the WAV on the fly"]
fn audio_only_wav_runs_audio_battery() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("bed.wav");
    // 5s sine bed at -24dB — real audio, zero video streams.
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=5",
            "-af",
            "volume=-24dB",
            "-ac",
            "1",
            "-ar",
            "48000",
        ])
        .arg(&wav)
        .status()
        .expect("ffmpeg present")
        .success();
    assert!(ok, "wav generation failed");
    let receipts = tempfile::tempdir().unwrap();
    let report = run_instruments(
        &wav,
        receipts.path(),
        "a1",
        "sha256:wavtest",
        InstrumentSet::AudioFull,
        None,
    )
    .expect("audio battery must succeed on a plain WAV");
    assert!(
        !report.instruments_run.iter().any(|i| i == "scenes"),
        "video instruments must not run on audio: {:?}",
        report.instruments_run
    );
    assert!(
        report.words.is_some(),
        "words instrument ran (content may be empty)"
    );
    assert!(report.loudness.is_some(), "loudness facts present");
    assert!(report.beats.is_some(), "beat grid present");
    // A continuous tone has no speech — silero inverts it to silence spans
    // (a sine IS silence to a VAD); the instrument must have RUN either way.
    assert!(report.instruments_run.iter().any(|i| i == "silence"));
    assert!(receipts.path().join("a1.perception.json").exists());
}

/// Final-render checks must work with the base-only sidecar install. `python -S`
/// deliberately hides site-packages, proving missing Silero/PySceneDetect fall
/// back to bundled FFmpeg rather than aborting the receipt.
#[test]
#[ignore = "needs system python + ffmpeg; generates a short render on the fly"]
fn render_checks_fall_back_without_optional_python_packages() {
    let root = repo_root();
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("render.mp4");
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=10:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=2",
            "-shortest",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
        ])
        .arg(&media)
        .status()
        .expect("ffmpeg present")
        .success();
    assert!(ok, "test render generation failed");

    let output = std::process::Command::new("python3")
        .arg("-S")
        .arg(root.join("app/perception/py/instruments.py"))
        .arg(&media)
        .args(["--instruments", "silence,scenes,loudness"])
        .output()
        .expect("system python present");
    assert!(
        output.status.success(),
        "base-only render checks failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["instruments_run"],
        serde_json::json!(["silence", "scenes", "loudness"])
    );
    assert!(report["content_bbox"].is_object());
    assert!(report["loudness"].is_object());
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(log.contains("using ffmpeg silencedetect"), "{log}");
    assert!(log.contains("using ffmpeg"), "{log}");
}
