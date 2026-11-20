//! Analysis files under a project's receipts folder are read and written only
//! as ordinary files: a link or shared file placed there by a received project
//! never redirects an analysis write outside the project.

use super::*;

const SENTINEL: &[u8] = b"outside-project-sentinel\n";

/// Points the sidecar at a stand-in `instruments.py` that answers every
/// request with a small report (one transcript word) and runs no model.
struct StandInSidecar {
    _dir: tempfile::TempDir,
    previous: Vec<(&'static str, Option<std::ffi::OsString>)>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl StandInSidecar {
    fn install() -> Self {
        let guard = super::lock_native_runtime_env();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("instruments.py"),
            r#"import json, os, sys
request = json.load(sys.stdin)
# Like the real tool, fail for missing media (other tests rely on that).
if not os.path.exists(str(request["media_path"])):
    sys.exit("media not found")
print(json.dumps({
    "schema": "shellx-cut/perception/1",
    "asset_hash": request["asset_hash"],
    "source_path": str(request["media_path"]),
    "instruments_run": request["instruments"],
    "words": {"asset": request["asset_id"], "model": "stand-in", "language": "en",
              "words": [{"idx": 0, "word": "standin", "start_ms": 0, "end_ms": 100}]},
}))
"#,
        )
        .unwrap();
        let keys = [
            ENV_SIDECAR_DIR,
            ENV_PYTHON,
            cut_native_runtime_context::CONTEXT_ENV,
        ];
        let previous = keys
            .iter()
            .map(|key| (*key, std::env::var_os(key)))
            .collect();
        std::env::set_var(ENV_SIDECAR_DIR, dir.path());
        std::env::set_var(ENV_PYTHON, if cfg!(windows) { "python" } else { "python3" });
        std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
        Self {
            _dir: dir,
            previous,
            _guard: guard,
        }
    }
}

impl Drop for StandInSidecar {
    fn drop(&mut self) {
        for (key, value) in self.previous.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn other_report(hash: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema": PERCEPTION_SCHEMA,
        "asset_hash": hash,
        "source_path": "/elsewhere/other.mp4",
        "instruments_run": ["words"],
    }))
    .unwrap()
}

fn media(dir: &Path) -> PathBuf {
    let media = dir.join("clip.mp4");
    std::fs::write(&media, b"stand-in media").unwrap();
    media
}

/// The report write is guarded by the earlier report read: a linked report
/// stops the run before the sidecar result is written anywhere.
#[cfg(unix)]
#[test]
fn linked_report_stops_analysis_before_any_write() {
    let _sidecar = StandInSidecar::install();
    let temp = tempfile::tempdir().unwrap();
    let receipts = temp.path().join("receipts");
    std::fs::create_dir(&receipts).unwrap();
    let outside = temp.path().join("outside.json");
    std::fs::write(&outside, SENTINEL).unwrap();
    std::os::unix::fs::symlink(&outside, receipts.join("a1.perception.json")).unwrap();

    let result = run_instruments(
        &media(temp.path()),
        &receipts,
        "a1",
        "sha256:new",
        InstrumentSet::WordsOnly,
        None,
    );

    assert!(result.is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), SENTINEL);
}

#[test]
fn shared_report_is_never_written_through() {
    let _sidecar = StandInSidecar::install();
    let temp = tempfile::tempdir().unwrap();
    let receipts = temp.path().join("receipts");
    std::fs::create_dir(&receipts).unwrap();
    let outside = temp.path().join("other-project-report.json");
    let original = other_report("sha256:other-video");
    std::fs::write(&outside, &original).unwrap();
    std::fs::hard_link(&outside, receipts.join("a1.perception.json")).unwrap();

    let _ = run_instruments(
        &media(temp.path()),
        &receipts,
        "a1",
        "sha256:new",
        InstrumentSet::WordsOnly,
        None,
    );

    assert_eq!(std::fs::read(&outside).unwrap(), original);
}

#[test]
fn shared_report_is_not_loaded() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("other-project-report.json");
    std::fs::write(&outside, other_report("sha256:other-video")).unwrap();
    std::fs::hard_link(&outside, temp.path().join("a1.perception.json")).unwrap();

    assert!(load_report(temp.path(), "a1").is_err());
}

fn assert_transcript_replaced_name(words: &Path, outside: &Path) {
    assert_eq!(std::fs::read(outside).unwrap(), SENTINEL);
    let metadata = std::fs::symlink_metadata(words).unwrap();
    assert!(metadata.file_type().is_file() && !metadata.file_type().is_symlink());
    let saved: Transcript = serde_json::from_str(&std::fs::read_to_string(words).unwrap()).unwrap();
    assert_eq!(saved.words[0].word, "standin");
}

#[cfg(unix)]
#[test]
fn linked_transcript_file_is_replaced_not_written_through() {
    let _sidecar = StandInSidecar::install();
    let temp = tempfile::tempdir().unwrap();
    let receipts = temp.path().join("receipts");
    std::fs::create_dir(&receipts).unwrap();
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, SENTINEL).unwrap();
    let words = receipts.join("a1.words.json");
    std::os::unix::fs::symlink(&outside, &words).unwrap();

    transcribe(&media(temp.path()), &receipts, "a1", "sha256:new", None).unwrap();

    assert_transcript_replaced_name(&words, &outside);
}

#[test]
fn shared_transcript_file_is_replaced_not_written_through() {
    let _sidecar = StandInSidecar::install();
    let temp = tempfile::tempdir().unwrap();
    let receipts = temp.path().join("receipts");
    std::fs::create_dir(&receipts).unwrap();
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, SENTINEL).unwrap();
    let words = receipts.join("a1.words.json");
    std::fs::hard_link(&outside, &words).unwrap();

    transcribe(&media(temp.path()), &receipts, "a1", "sha256:new", None).unwrap();

    assert_transcript_replaced_name(&words, &outside);
}

#[cfg(windows)]
#[test]
#[ignore = "requires Windows symbolic-link privilege or Developer Mode"]
fn windows_symlink_transcript_file_is_replaced_not_written_through() {
    let _sidecar = StandInSidecar::install();
    let temp = tempfile::tempdir().unwrap();
    let receipts = temp.path().join("receipts");
    std::fs::create_dir(&receipts).unwrap();
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, SENTINEL).unwrap();
    let words = receipts.join("a1.words.json");
    std::os::windows::fs::symlink_file(&outside, &words).unwrap();

    transcribe(&media(temp.path()), &receipts, "a1", "sha256:new", None).unwrap();

    assert_transcript_replaced_name(&words, &outside);
}

fn job_control() -> cut_media::ffmpeg::OwnedProcessControl {
    cut_media::ffmpeg::OwnedProcessControl::bounded(std::time::Duration::from_secs(60), || false)
}

/// The job-owned transcription used by media.transcribe and import.
#[cfg(unix)]
#[test]
fn job_transcription_replaces_a_linked_transcript_file() {
    let _sidecar = StandInSidecar::install();
    let temp = tempfile::tempdir().unwrap();
    let receipts = temp.path().join("receipts");
    std::fs::create_dir(&receipts).unwrap();
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, SENTINEL).unwrap();
    let words = receipts.join("a1.words.json");
    std::os::unix::fs::symlink(&outside, &words).unwrap();

    transcribe_owned_progress(
        &media(temp.path()),
        &receipts,
        "a1",
        "sha256:new",
        None,
        None,
        &job_control(),
    )
    .unwrap();

    assert_transcript_replaced_name(&words, &outside);
}

#[test]
fn job_transcription_replaces_a_shared_transcript_file() {
    let _sidecar = StandInSidecar::install();
    let temp = tempfile::tempdir().unwrap();
    let receipts = temp.path().join("receipts");
    std::fs::create_dir(&receipts).unwrap();
    let outside = temp.path().join("outside.txt");
    std::fs::write(&outside, SENTINEL).unwrap();
    let words = receipts.join("a1.words.json");
    std::fs::hard_link(&outside, &words).unwrap();

    transcribe_owned_progress(
        &media(temp.path()),
        &receipts,
        "a1",
        "sha256:new",
        None,
        None,
        &job_control(),
    )
    .unwrap();

    assert_transcript_replaced_name(&words, &outside);
}

/// render.final's output analysis (`<render_id>.output.perception.json`) goes
/// through the same job-owned run.
#[test]
fn render_output_analysis_never_writes_through_a_shared_report() {
    let _sidecar = StandInSidecar::install();
    let temp = tempfile::tempdir().unwrap();
    let receipts = temp.path().join("receipts");
    std::fs::create_dir(&receipts).unwrap();
    let outside = temp.path().join("other-project-report.json");
    let original = other_report("sha256:other-video");
    std::fs::write(&outside, &original).unwrap();
    std::fs::hard_link(&outside, receipts.join("render_001.output.perception.json")).unwrap();

    let _ = run_instruments_owned(
        &media(temp.path()),
        &receipts,
        "render_001.output",
        "sha256:new",
        InstrumentSet::RenderChecks,
        None,
        &job_control(),
    );

    assert_eq!(std::fs::read(&outside).unwrap(), original);
}
