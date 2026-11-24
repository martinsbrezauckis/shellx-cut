//! Real process/owned-runner boundary; the stand-in runs no media models.
use cut_perception::{analysis_attempts::load_attempts, sidecar::*};
use std::{path::Path, sync::Mutex, time::Duration};
static ENV: Mutex<()> = Mutex::new(());
struct Restore(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl Drop for Restore {
    fn drop(&mut self) {
        for (k, v) in &self.0 {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}
fn setup(dir: &Path) -> Restore {
    let keys = [
        ENV_PYTHON,
        ENV_SIDECAR_DIR,
        cut_native_runtime_context::CONTEXT_ENV,
    ];
    let restore = Restore(keys.iter().map(|k| (*k, std::env::var_os(k))).collect());
    std::env::set_var(ENV_PYTHON, if cfg!(windows) { "python" } else { "python3" });
    std::env::set_var(ENV_SIDECAR_DIR, dir);
    std::env::remove_var(cut_native_runtime_context::CONTEXT_ENV);
    std::fs::write(dir.join("instruments.py"), r#"import json, sys
r=json.load(sys.stdin)
e={'instrument':'words','provider':'perception','state':'running','at_unix_ms':1234,'tool_version':sys.version.split()[0]}
print('CUT_ATTEMPT '+json.dumps(e),file=sys.stderr,flush=True)
print('[instruments] PROGRESS 0.5 started',file=sys.stderr,flush=True)
if r['asset_id']=='cancel':
 import time; time.sleep(180)
if r['asset_id']=='bad':
 print(json.dumps({'error':{'code':'sidecar','message':'actual failure','cause':'kept engine reason'}}));sys.exit(1)
e['state']='succeeded'
print('CUT_ATTEMPT '+json.dumps(e),file=sys.stderr,flush=True)
print(json.dumps({'schema':'shellx-cut/perception/1','asset_hash':r['asset_hash'],'source_path':r['media_path'],'instruments_run':['words']}))
"#).unwrap();
    restore
}
#[test]
fn fresh_run_records_real_events_and_cache_hit_does_not_create_a_new_attempt() {
    let _lock = ENV.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _restore = setup(dir.path());
    let path = dir.path().join("input.wav");
    std::fs::write(&path, b"stand-in").unwrap();
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let progress_seen = Arc::new(AtomicBool::new(false));
    let seen = progress_seen.clone();
    let report = run_instruments_progress(
        &path,
        dir.path(),
        "a1",
        "hash",
        InstrumentSet::WordsOnly,
        None,
        Some(Arc::new(move |_: f32, _: &str| {
            seen.store(true, Ordering::Relaxed);
        })),
    )
    .unwrap();
    assert!(progress_seen.load(Ordering::Relaxed));
    let attempt = report.analysis_attempt.unwrap();
    assert!(attempt.instruments[0].ran);
    assert!(attempt.instruments[0].tool_version.is_some());
    let cached = run_instruments(
        &path,
        dir.path(),
        "a1",
        "hash",
        InstrumentSet::WordsOnly,
        None,
    )
    .unwrap();
    assert_eq!(
        cached.analysis_attempt.unwrap().attempt_id,
        attempt.attempt_id
    );
    assert_eq!(load_attempts(dir.path(), "a1", "hash").unwrap().len(), 1);
}
#[test]
fn real_failure_is_kept_and_ephemeral_failure_never_writes_the_source_cache() {
    let _lock = ENV.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _restore = setup(dir.path());
    let control =
        cut_media::ffmpeg::OwnedProcessControl::bounded(Duration::from_secs(180), || false);
    let error = run_instruments_owned_ephemeral(
        Path::new("input.wav"),
        dir.path(),
        "bad",
        "hash",
        InstrumentSet::WordsOnly,
        None,
        &control,
    )
    .unwrap_err();
    assert!(error.cause.contains("kept engine reason"));
    let attempts = load_attempts(dir.path(), "bad", "hash").unwrap();
    assert_eq!(attempts.last().unwrap().error.as_ref(), Some(&error));
    assert!(attempts.last().unwrap().instruments[0].ran);
    assert!(!dir.path().join("bad.analysis-attempts.json").exists());
    assert!(!dir.path().join("bad.perception.json").exists());
}
#[test]
fn failure_before_process_start_records_asked_without_claiming_it_ran() {
    let _lock = ENV.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _restore = setup(dir.path());
    std::env::set_var(ENV_PYTHON, dir.path().join("missing-python"));
    let error = run_instruments(
        Path::new("input.wav"),
        dir.path(),
        "a1",
        "hash",
        InstrumentSet::WordsOnly,
        None,
    )
    .unwrap_err();
    let attempts = load_attempts(dir.path(), "a1", "hash").unwrap();
    assert_eq!(attempts.last().unwrap().error.as_ref(), Some(&error));
    assert!(!attempts.last().unwrap().instruments[0].ran);
}
#[test]
fn real_python_battery_and_owned_runner_keep_the_sound_fallback_provenance() {
    let _lock = ENV.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _restore = setup(dir.path());
    let py_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("py");
    let wrapper = format!(
        r#"import sys, types
sys.path.insert(0,{})
import instruments
silero=types.ModuleType('silero_vad')
def fail(): raise RuntimeError('model file is damaged')
silero.load_silero_vad=fail
silero.get_speech_timestamps=lambda *args: []
sys.modules.update({{'silero_vad':silero,'torch':types.ModuleType('torch'),'soundfile':types.ModuleType('soundfile')}})
raise SystemExit(instruments.main())
"#,
        serde_json::to_string(&py_dir.to_string_lossy()).unwrap()
    );
    std::fs::write(dir.path().join("instruments.py"), wrapper).unwrap();
    let path = dir.path().join("input.wav");
    let mut wav = b"RIFF".to_vec();
    wav.extend(64036u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(32000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(64000u32.to_le_bytes());
    wav.resize(64044, 0);
    std::fs::write(&path, wav).unwrap();
    let control =
        cut_media::ffmpeg::OwnedProcessControl::bounded(Duration::from_secs(180), || false);
    let report = run_instruments_owned_ephemeral(
        &path,
        dir.path(),
        "sound",
        "hash",
        InstrumentSet::RenderChecks,
        None,
        &control,
    )
    .unwrap();
    assert!(report
        .silences
        .iter()
        .any(|s| s.source.as_deref() == Some("ffmpeg")));
    let attempt = report.analysis_attempt.unwrap();
    assert!(attempt.error.is_none());
    use cut_perception::analysis_attempts::ExecutionState;
    assert!(attempt
        .instruments
        .iter()
        .any(|i| i.provider == "ffmpeg.silencedetect"
            && i.state == ExecutionState::Succeeded
            && i.tool_version.is_some()));
    assert!(attempt.instruments.iter().any(|i| i.provider == "silero"
        && i.state == ExecutionState::Failed
        && i.error.as_ref().unwrap()["cause"] == "model file is damaged"));
    assert!(attempt.instruments.iter().any(|i| i.instrument == "silence"
        && i.provider == "perception"
        && i.state == ExecutionState::Succeeded));
    assert!(load_attempts(dir.path(), "sound", "hash").unwrap()[0]
        .finished_at_unix_ms
        .is_some());
    assert!(!dir.path().join("sound.perception.json").exists());
}

#[test]
fn cancellation_keeps_its_existing_error_and_the_started_attempt() {
    let _lock = ENV.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _restore = setup(dir.path());
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let control =
        cut_media::ffmpeg::OwnedProcessControl::bounded(Duration::from_secs(180), move || {
            flag.load(Ordering::Relaxed)
        });
    let error = run_instruments_owned_progress(
        Path::new("input.wav"),
        dir.path(),
        "cancel",
        "hash",
        InstrumentSet::WordsOnly,
        None,
        Some(Arc::new(move |_: f32, _: &str| {
            cancel.store(true, Ordering::Relaxed);
        })),
        &control,
    )
    .unwrap_err();
    assert_eq!(error.code, cut_core::error_codes::RENDER_CANCELLED);
    let attempts = load_attempts(dir.path(), "cancel", "hash").unwrap();
    assert_eq!(attempts[0].error.as_ref(), Some(&error));
    assert!(attempts[0].instruments[0].ran);
}

#[test]
fn a_legacy_cache_hit_does_not_create_history_or_reanalyse_the_file() {
    let _lock = ENV.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _restore = setup(dir.path());
    let receipt = dir.path().join("legacy.perception.json");
    std::fs::write(&receipt,serde_json::to_vec(&serde_json::json!({"schema":"shellx-cut/perception/1", "asset_hash":"hash", "source_path":"old.wav", "instruments_run":["words"]})).unwrap()).unwrap();
    cut_media::cache_trust::remember(&receipt);
    let report = run_instruments(
        Path::new("old.wav"),
        dir.path(),
        "legacy",
        "hash",
        InstrumentSet::WordsOnly,
        None,
    )
    .unwrap();
    assert!(report.analysis_attempt.is_none());
    assert!(load_attempts(dir.path(), "legacy", "hash")
        .unwrap()
        .is_empty());
}

#[test]
fn python_sound_and_stream_probe_regressions_pass_without_optional_models() {
    let output = std::process::Command::new(if cfg!(windows) { "python" } else { "python3" })
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("py/attempt_evidence_test.py"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn every_packager_carries_the_required_analysis_helpers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let conf: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("app/desktop/src-tauri/tauri.conf.json")).unwrap(),
    )
    .unwrap();
    for name in ["attempt_evidence.py", "silence_enhancement.py"] {
        for platform in ["linux", "macos", "windows"] {
            let script =
                std::fs::read_to_string(root.join(format!("scripts/build-{platform}.sh"))).unwrap();
            assert!(script.contains(&format!("app/perception/py/{name}")));
        }
        assert_eq!(
            conf["bundle"]["resources"][format!("../../perception/py/{name}")],
            format!("perception/{name}")
        );
    }
}

#[test]
fn legacy_report_does_not_gain_invented_execution_history() {
    let report:cut_perception::PerceptionReport=serde_json::from_value(serde_json::json!({
        "schema":"shellx-cut/perception/1", "asset_hash":"hash", "source_path":"old.wav", "instruments_run":["words"]
    })).unwrap();
    assert!(report.analysis_attempt.is_none());
}

#[test]
fn subject_and_sheet_attempts_record_without_overwriting_source_cache() {
    let _lock = ENV.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let _restore = setup(dir.path());
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let existing = serde_json::json!({"schema": "shellx-cut/perception/1", "asset_hash": "src_hash", "source_path": "a1.mp4", "instruments_run": ["words"]});
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_vec(&existing).unwrap(),
    )
    .unwrap();
    let path = dir.path().join("edit.mp4");
    std::fs::write(&path, b"stand-in").unwrap();
    std::fs::write(dir.path().join("instruments.py"), r#"import json, sys
r = json.load(sys.stdin)
inst = "contact_sheet" if r.get("contact_sheet") is not None else ("qc_sheet" if r.get("qc_sheet") is not None else "subject")
e = {"instrument": inst, "provider": "perception", "state": "running", "at_unix_ms": 1234, "tool_version": sys.version.split()[0]}
print("CUT_ATTEMPT " + json.dumps(e), file=sys.stderr, flush=True)
if r["asset_hash"] == "bad":
    print(json.dumps({"error": {"code": "sidecar", "message": "cv failed", "cause": "torchvision missing"}})); sys.exit(1)
e["state"] = "succeeded"
print("CUT_ATTEMPT " + json.dumps(e), file=sys.stderr, flush=True)
if inst == "contact_sheet":
    print(json.dumps({"contact_sheet": str(r["contact_sheet"]) + "/contact_sheet.jpg", "scenes": []}))
elif inst == "qc_sheet":
    print(json.dumps({"qc_sheet": str(r["qc_sheet"]) + "/qc_sheet.jpg", "scenes": [], "review_count": 0}))
else:
    print(json.dumps({"schema": "shellx-cut/perception/1", "asset_hash": r["asset_hash"], "source_path": r["media_path"], "instruments_run": ["subject"], "subject_track": {"preset": "talking_head", "fps": 30.0, "frame_width": 64, "frame_height": 64, "device": "cpu", "face_aware": False, "speaker_aware": False, "frames": []}}))
"#).unwrap();
    let control =
        cut_media::ffmpeg::OwnedProcessControl::bounded(Duration::from_secs(30), || false);
    let _track = run_subject_recorded(
        &path,
        &receipts,
        "reframe_001_subject",
        "reframe_hash",
        "talking_head",
        None,
        &control,
    )
    .unwrap();
    let attempts = load_attempts(&receipts, "reframe_001_subject", "reframe_hash").unwrap();
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0]
        .instruments
        .iter()
        .any(|i| i.instrument == "subject"
            && i.state == cut_perception::analysis_attempts::ExecutionState::Succeeded));
    let _sheet = build_contact_sheet_recorded(
        &path,
        &dir.path().join("cs"),
        "talking_head",
        &receipts,
        "direct_001_contact",
        "direct_hash",
        &control,
    )
    .unwrap();
    assert!(
        !load_attempts(&receipts, "direct_001_contact", "direct_hash")
            .unwrap()
            .is_empty()
    );
    let err = build_qc_sheet_recorded(
        &path,
        &dir.path().join("qc"),
        "talking_head",
        &receipts,
        "reframe_001_qc",
        "bad",
        &control,
    )
    .unwrap_err();
    assert!(err.cause.contains("torchvision missing"));
    let qc_attempts = load_attempts(&receipts, "reframe_001_qc", "bad").unwrap();
    let qc_err = qc_attempts[0].error.as_ref().unwrap();
    assert!(
        qc_err.cause.contains("cv failed") && qc_err.cause.contains("torchvision missing"),
        "{qc_err:?}"
    );
    // Source cache must remain untouched.
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(receipts.join("a1.perception.json")).unwrap()
        )
        .unwrap(),
        existing
    );
    assert!(load_attempts(&receipts, "a1", "src_hash")
        .unwrap()
        .is_empty());
}
