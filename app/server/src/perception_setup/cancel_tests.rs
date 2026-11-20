//! A cancelled Install captions stops its helper at once and leaves no
//! half-installed environment behind.

use super::*;
use std::time::{Duration, Instant};

fn sleeper() -> (PathBuf, tempfile::TempDir) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // A stand-in for a long uv/pip run: a parent that keeps a child alive.
    let script = dir.path().join("fake-uv.sh");
    std::fs::write(&script, "#!/bin/sh\nsleep 120 &\nwait\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (script, dir)
}

#[test]
fn cancel_stops_a_running_helper_promptly_and_reads_as_cancelled() {
    let (script, _dir) = sleeper();
    let cancellation = JobCancellation::test_active();
    let canceller = cancellation.clone();
    let worker = std::thread::spawn(move || {
        let _scope = CancellationScope::enter(&cancellation);
        let progress = |_: f32, _: &str| {};
        let started = Instant::now();
        let result = run_streaming(&script, &[], "uv pip install fixture", &progress, 0.5, 0.6);
        (result, started.elapsed())
    });
    std::thread::sleep(Duration::from_millis(300));
    canceller.request_cancel();
    let (result, elapsed) = worker.join().unwrap();
    let error = result.expect_err("a cancelled helper is an error");
    assert_eq!(error.code, "job_cancelled", "{error}");
    assert!(
        elapsed < Duration::from_secs(10),
        "the helper tree must stop at once, took {elapsed:?}"
    );
}

#[test]
fn an_uncancelled_setup_still_runs_its_helpers() {
    let cancellation = JobCancellation::test_active();
    let _scope = CancellationScope::enter(&cancellation);
    let out = run_capture(Path::new("/bin/sh"), &["-c", "echo ready"]).unwrap();
    assert_eq!(out.trim(), "ready");
    assert!(check_cancelled().is_ok());
}

#[test]
fn an_unfinished_venv_is_removed_and_a_finished_one_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let venv = dir.path().join(".venv");
    std::fs::create_dir_all(venv.join("bin")).unwrap();
    std::fs::write(venv.join("bin").join("python"), "").unwrap();
    drop(PartialVenv {
        dir: &venv,
        armed: true,
    });
    assert!(
        !venv.exists(),
        "a cancelled install must not leave a venv the resolver would trust"
    );

    std::fs::create_dir_all(&venv).unwrap();
    drop(PartialVenv {
        dir: &venv,
        armed: false,
    });
    assert!(venv.exists());
}

#[test]
fn the_uv_download_step_tells_a_waiting_cancel_why() {
    assert!(UV_DOWNLOAD_STEP.contains("about 20 MB"));
    assert!(UV_DOWNLOAD_STEP.contains("a cancel stops right after it"));
}
