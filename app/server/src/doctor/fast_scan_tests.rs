#![cfg(unix)]

use crate::doctor::{Card, CardStatus};
use crate::state::AppState;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn run_in_clean_subprocess(exact_test: &str, env_var: &str) -> bool {
    if std::env::var_os(env_var).is_some() {
        return false;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", exact_test, "--nocapture"])
        .env(env_var, "1")
        .env_remove("SHELLX_CUT_NO_HWENC")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "isolated doctor test failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

struct FakeTooling {
    ffmpeg_calls: PathBuf,
    py_calls: PathBuf,
    size_probe_started: PathBuf,
    release_size_probe: PathBuf,
    ffmpeg_path: PathBuf,
}

fn setup_fake_env(root: &Path, auto_ffmpeg: &str) -> FakeTooling {
    let bin = root.join("bin");
    let data = root.join("data");
    let home = root.join("home");
    // The captions install lives where the product looks for it on this system (see
    // cut_perception::appdata_sidecar_dir): the XDG data folder on Linux, Application Support
    // under HOME on macOS.
    let sidecar = if cfg!(target_os = "macos") {
        home.join("Library/Application Support/ShellX Cut/perception")
    } else {
        data.join("shellx-cut/perception")
    };
    let venv_bin = sidecar.join(".venv/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&venv_bin).unwrap();
    fs::create_dir_all(&home).unwrap();

    let ffmpeg = bin.join("ffmpeg");
    let ffprobe = bin.join("ffprobe");
    let python = venv_bin.join("python");
    let ffmpeg_calls = root.join("ffmpeg_calls.log");
    let py_calls = root.join("py_calls.log");
    let size_probe_started = root.join("size_probe_started");
    let release_size_probe = root.join("release_size_probe");

    let ffmpeg_script = format!(
        r#"#!/bin/sh
set -eu
echo "ffmpeg $*" >> "{ffmpeg_calls}"
args=" $* "
case "$args" in
  *" -version "*)
    echo "ffmpeg version 8.0.1-fixture"
    exit 0
    ;;
  *" -filters "*)
    printf " T.. subtitles ...\n T.. vidstabdetect ...\n T.. vidstabtransform ...\n ... zscale ...\n"
    exit 0
    ;;
  *" -encoders "*)
    printf "Encoders:\n V..... libx265\n V..... libsvtav1\n V..... h264_nvenc\n A..... aac\n"
    exit 0
    ;;
  *scale_cuda*)
    exit 0
    ;;
esac
enc=""
size=""
prev=""
for a in "$@"; do
  if [ "$prev" = "-c:v" ]; then enc="$a"; fi
  case "$a" in
    *size=*)
      size="${{a#*size=}}"
      size="${{size%%:*}}"
      ;;
  esac
  prev="$a"
done
case "$enc" in
  libx265|libsvtav1)
    exit 0
    ;;
  h264_nvenc)
    case "$size" in
      256x256)
        exit 0
        ;;
      3840x2160)
        : > "{started}"
        i=0
        while [ ! -f "{release}" ] && [ "$i" -lt 300 ]; do
          sleep 0.05
          i=$((i + 1))
        done
        [ -f "{release}" ]
        exit $?
        ;;
      4096x2160)
        exit 0
        ;;
      7680x4320)
        echo "Cannot allocate" >&2
        exit 1
        ;;
      *)
        exit 1
        ;;
    esac
    ;;
  *)
    exit 1
    ;;
esac
"#,
        ffmpeg_calls = ffmpeg_calls.display(),
        started = size_probe_started.display(),
        release = release_size_probe.display(),
    );
    fs::write(&ffmpeg, ffmpeg_script).unwrap();
    fs::set_permissions(&ffmpeg, fs::Permissions::from_mode(0o755)).unwrap();

    fs::write(
        &ffprobe,
        "#!/bin/sh\necho 'ffprobe version 8.0.1-fixture'\n",
    )
    .unwrap();
    fs::set_permissions(&ffprobe, fs::Permissions::from_mode(0o755)).unwrap();

    let python_script = format!(
        r#"#!/bin/sh
set -eu
echo "python $*" >> "{py_calls}"
case " $* " in
  *" --version "*)
    echo "Python 3.12.15"
    exit 0
    ;;
  *ladder_judge.py*)
    echo '{{"rungs":[]}}'
    exit 0
    ;;
  *__cut_import_probe*)
    echo '{{"version":"Python 3.12.15","onnx_asr":true,"onnxruntime":true,"numpy":true,"soundfile":true,"PIL":true}}'
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
"#,
        py_calls = py_calls.display(),
    );
    fs::write(&python, python_script).unwrap();
    fs::set_permissions(&python, fs::Permissions::from_mode(0o755)).unwrap();

    for sleep_candidate in ["/bin/sleep", "/usr/bin/sleep"] {
        let target = Path::new(sleep_candidate);
        if target.exists() {
            let _ = std::os::unix::fs::symlink(target, bin.join("sleep"));
            break;
        }
    }

    std::env::set_var("PATH", &bin);
    std::env::set_var("XDG_DATA_HOME", &data);
    std::env::set_var("HOME", &home);
    std::env::set_var("SHELLX_CUT_HOME", &home);
    std::env::set_var("SHELLX_CUT_FFMPEG", &ffmpeg);
    std::env::set_var("SHELLX_CUT_FFPROBE", &ffprobe);
    std::env::set_var("SHELLX_CUT_FFMPEG_AUTO", auto_ffmpeg);
    std::env::set_var("SHELLX_CUT_ADAPTER_PYTHON", &python);
    std::env::remove_var("SHELLX_CUT_NO_HWENC");
    std::env::remove_var("MATANYONE_PY");
    std::env::remove_var("MATANYONE_RUNNER");
    std::env::remove_var("MATANYONE_MODEL");

    FakeTooling {
        ffmpeg_calls,
        py_calls,
        size_probe_started,
        release_size_probe,
        ffmpeg_path: ffmpeg,
    }
}

fn find_card<'a>(cards: &'a [Card], id: &str) -> &'a Card {
    cards
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("missing card {id}"))
}

#[tokio::test]
async fn doctor_cached_answers_fast_before_size_limits_and_deduplicates_probes() {
    if run_in_clean_subprocess(
        "doctor::fast_scan_tests::doctor_cached_answers_fast_before_size_limits_and_deduplicates_probes",
        "SHELLX_CUT_DOCTOR_FAST_SCAN_CHILD_1",
    ) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let tools = setup_fake_env(tmp.path(), "1");
    let state = AppState::new();
    let mut events = state.events.subscribe();

    let started_wait = std::time::Instant::now();
    // First doctor call must NOT block on multi-second size_limits 4K/8K encodes.
    let first = match tokio::time::timeout(Duration::from_secs(6), state.doctor_cached()).await {
        Ok(rep) => rep,
        Err(e) => {
            let ff_log = fs::read_to_string(&tools.ffmpeg_calls).unwrap_or_default();
            let py_log = fs::read_to_string(&tools.py_calls).unwrap_or_default();
            panic!(
                "first doctor_cached must return without waiting for size_limits ({:?}, elapsed={:?}, size_started={}):\nFFMPEG:\n{ff_log}\nPY:\n{py_log}",
                e,
                started_wait.elapsed(),
                tools.size_probe_started.exists()
            );
        }
    };

    assert!(first.essential_ok);
    let perception = find_card(&first.cards, "perception");
    assert_eq!(perception.status, CardStatus::Ok);
    assert_eq!(perception.details["stt_ready"], true);

    let matte = find_card(&first.cards, "matte");
    assert_eq!(matte.details["onnxruntime"], true);
    assert_eq!(matte.details["numpy"], true);
    assert_eq!(matte.details["pillow"], true);

    let gpu = find_card(&first.cards, "gpu-encode");
    assert_eq!(gpu.status, CardStatus::Ok);
    assert_eq!(gpu.details["hardware_available"], true);
    assert_eq!(gpu.details["h264"], "h264_nvenc");
    assert_eq!(gpu.details["size_limits"], serde_json::json!([]));
    assert_eq!(gpu.details["size_limits_pending"], true);
    assert_eq!(gpu.details["cuda_filters_pending"], true);
    assert_eq!(gpu.details["full_fasttrack"], false);

    // Every single ffmpeg/python check ran at most once per capability in Phase 1.
    let ff_log = fs::read_to_string(&tools.ffmpeg_calls).unwrap();
    let count_matching = |needle: &str| ff_log.lines().filter(|l| l.contains(needle)).count();
    assert_eq!(
        count_matching("-version"),
        1,
        "ffmpeg -version log:\n{ff_log}"
    );
    assert_eq!(
        count_matching("-filters"),
        1,
        "ffmpeg -filters log:\n{ff_log}"
    );
    assert_eq!(
        count_matching("-encoders"),
        1,
        "ffmpeg -encoders log:\n{ff_log}"
    );
    assert_eq!(
        count_matching("size=256x256"),
        1,
        "256x256 probe log:\n{ff_log}"
    );
    let py_log = fs::read_to_string(&tools.py_calls).unwrap_or_default();
    let import_calls = py_log.lines().filter(|l| l.contains(" -c ")).count();
    assert!(
        import_calls <= 2,
        "expected <= 2 python import spawns (shared or 1 per card), got {import_calls}:\n{py_log}"
    );

    // Release size_limits and verify background enrichment pushes doctor_updated.
    fs::write(&tools.release_size_probe, b"go").unwrap();
    let updated = tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if let crate::events::Event::DoctorUpdated { report } = events.recv().await.unwrap() {
                let g = find_card(&report.cards, "gpu-encode");
                if g.details["size_limits_pending"] == false {
                    break report;
                }
            }
        }
    })
    .await
    .expect("doctor_updated must arrive once background size_limits finishes");

    let updated_gpu = find_card(&updated.cards, "gpu-encode");
    assert_eq!(updated_gpu.details["full_fasttrack"], true);
    assert_eq!(updated_gpu.details["cuda_filters_pending"], false);
    assert_eq!(
        updated_gpu.details["size_limits"][0]["encoder"],
        "h264_nvenc"
    );
    assert_eq!(
        updated_gpu.details["size_limits"][0]["largest_accepted"],
        "4096x2160"
    );
    assert_eq!(
        updated_gpu.details["size_limits"][0]["smallest_refused"],
        "7680x4320"
    );

    // Cached read after background fill returns the enriched report without re-probing.
    let before_cached_len = fs::read_to_string(&tools.ffmpeg_calls).unwrap().len();
    let second = state.doctor_cached().await;
    assert_eq!(
        find_card(&second.cards, "gpu-encode").details["size_limits"],
        updated_gpu.details["size_limits"]
    );
    assert_eq!(
        fs::read_to_string(&tools.ffmpeg_calls).unwrap().len(),
        before_cached_len
    );
}

#[tokio::test]
async fn doctor_cache_invalidate_cancels_stale_background_size_limits() {
    if run_in_clean_subprocess(
        "doctor::fast_scan_tests::doctor_cache_invalidate_cancels_stale_background_size_limits",
        "SHELLX_CUT_DOCTOR_FAST_SCAN_CHILD_2",
    ) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let tools = setup_fake_env(tmp.path(), "0");
    let state = AppState::new();

    let first = tokio::time::timeout(Duration::from_secs(6), state.doctor_cached())
        .await
        .expect("first doctor_cached must return immediately while size_limits is pending");
    assert_eq!(
        find_card(&first.cards, "gpu-encode").details["size_limits_pending"],
        true
    );

    // Wait briefly so background task has actually started size_limits if spawned.
    let wait_start = tokio::time::Instant::now();
    while !tools.size_probe_started.exists() && wait_start.elapsed() < Duration::from_secs(1) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Invalidate (just like a tool_missing refusal) and remove ffmpeg BEFORE releasing the old probe.
    fs::remove_file(&tools.ffmpeg_path).unwrap();
    let mut events = state.events.subscribe();
    let missing_err = cut_core::CutError::new(
        cut_core::error_codes::TOOL_MISSING,
        "ffmpeg is not installed",
        "install ffmpeg",
    );
    assert!(state.doctor_cache.invalidate_on_missing(&missing_err));
    fs::write(&tools.release_size_probe, b"go").unwrap();

    let next = state.doctor_cached().await;
    assert!(!next.essential_ok);
    assert_eq!(find_card(&next.cards, "ffmpeg").status, CardStatus::Missing);

    // Ensure no stale DoctorUpdated from the old generation overwrites the invalidated state.
    tokio::time::sleep(Duration::from_millis(250)).await;
    while let Ok(ev) = events.try_recv() {
        if let crate::events::Event::DoctorUpdated { report } = ev {
            assert!(
                !report.essential_ok,
                "stale background enrichment leaked old ok report after invalidate"
            );
        }
    }
    assert!(!state.doctor_cached().await.essential_ok);
}

#[tokio::test]
async fn doctor_cached_defers_matte_premium_cuda_without_claiming_ready() {
    if run_in_clean_subprocess(
        "doctor::fast_scan_tests::doctor_cached_defers_matte_premium_cuda_without_claiming_ready",
        "SHELLX_CUT_DOCTOR_FAST_SCAN_CHILD_3",
    ) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let tools = setup_fake_env(tmp.path(), "0");
    // Unblock size_limits immediately in this test so we isolate matte_premium's torch CUDA gate.
    fs::write(&tools.release_size_probe, b"go").unwrap();

    let mat_dir = tmp.path().join("matanyone");
    fs::create_dir_all(&mat_dir).unwrap();
    let mat_py = mat_dir.join("python");
    let mat_runner = mat_dir.join("runner.py");
    let mat_model = mat_dir.join("matanyone2.pth");
    let cuda_started = tmp.path().join("cuda_probe_started");
    let release_cuda = tmp.path().join("release_cuda_probe");
    fs::write(&mat_runner, b"pass\n").unwrap();
    fs::write(&mat_model, b"weights").unwrap();
    let mat_script = format!(
        r#"#!/bin/sh
set -eu
case " $* " in
  *torch*)
    : > "{started}"
    i=0
    while [ ! -f "{release}" ] && [ "$i" -lt 300 ]; do
      sleep 0.05
      i=$((i + 1))
    done
    if [ -f "{release}" ]; then
      echo "cuda True"
      exit 0
    fi
    exit 1
    ;;
  *)
    exit 0
    ;;
esac
"#,
        started = cuda_started.display(),
        release = release_cuda.display(),
    );
    fs::write(&mat_py, mat_script).unwrap();
    fs::set_permissions(&mat_py, fs::Permissions::from_mode(0o755)).unwrap();
    std::env::set_var("MATANYONE_PY", &mat_py);
    std::env::set_var("MATANYONE_RUNNER", &mat_runner);
    std::env::set_var("MATANYONE_MODEL", &mat_model);

    let state = AppState::new();
    let mut events = state.events.subscribe();

    let first = tokio::time::timeout(Duration::from_secs(6), state.doctor_cached())
        .await
        .expect("first doctor_cached must not block on torch.cuda.is_available()");
    let premium = find_card(&first.cards, "matte_premium");
    assert_eq!(
        premium.status,
        CardStatus::Unknown,
        "installed MatAnyone2 must stay Unknown until CUDA is actually checked"
    );
    assert_eq!(premium.details["installed"], true);
    assert_eq!(premium.details["checking"], true);
    assert_eq!(premium.details["cuda_available"], serde_json::Value::Null);

    fs::write(&release_cuda, b"go").unwrap();
    let updated = tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            if let crate::events::Event::DoctorUpdated { report } = events.recv().await.unwrap() {
                let p = find_card(&report.cards, "matte_premium");
                if p.status == CardStatus::Ok {
                    break report;
                }
            }
        }
    })
    .await
    .expect("doctor_updated must resolve matte_premium once torch CUDA check completes");

    let resolved = find_card(&updated.cards, "matte_premium");
    assert_eq!(resolved.status, CardStatus::Ok);
    assert_eq!(resolved.details["cuda_available"], true);
    assert!(resolved.details.get("checking").is_none());
}
