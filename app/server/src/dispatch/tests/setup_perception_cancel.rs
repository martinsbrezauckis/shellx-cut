//! Cancelling Install captions while a helper runs: the whole setup ends at
//! once as cancelled, the helper's process tree is gone, and no half-installed
//! venv is left for the resolver to trust. A fake `uv` stands in for the real
//! one so the test needs no network.

// The stand-in installer is a shell script, so the whole file is Unix-only.
#![cfg(unix)]

use super::lock_agent_cli_env;
use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

struct Env(Vec<(&'static str, Option<OsString>)>);

impl Env {
    fn set(vars: &[(&'static str, &Path)]) -> Self {
        let saved = vars
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        for (key, value) in vars {
            std::env::set_var(key, value);
        }
        Self(saved)
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for (key, value) in self.0.drain(..) {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

/// Which install is cancelled.
#[derive(Clone, Copy)]
enum Install {
    Captions,
    PremiumMatte,
}

fn cancel_mid_install(install: Install) {
    use std::os::unix::fs::PermissionsExt;
    let _guard = lock_agent_cli_env();
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let payload = root.path().join("payload");
    let bin = root.path().join("bin");
    for dir in [&home, &payload, &bin] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(payload.join("instruments.py"), "").unwrap();
    std::fs::write(payload.join("requirements.txt"), "").unwrap();

    // A Python that only answers --version, as uv's managed one would.
    let python = bin.join("fake-python");
    std::fs::write(&python, "#!/bin/sh\necho 'Python 3.12.7'\n").unwrap();
    // A uv whose `venv` step creates the interpreter and then takes "forever"
    // (a stand-in for a long pip install) with a child of its own.
    let uv = bin.join("uv");
    let created = root.path().join("venv-created");
    let sleeper_pid = root.path().join("sleeper.pid");
    std::fs::write(
        &uv,
        format!(
            "#!/bin/sh\n\
             case \"$1\" in\n\
               --version) echo 'uv 0.0.0-fake' ;;\n\
               python) echo '{python}' ;;\n\
               venv) mkdir -p \"$2/bin\"; cp '{python}' \"$2/bin/python\"; touch '{created}'; sleep 120 & echo $! > '{pid}'; wait ;;\n\
             esac\n",
            python = python.display(),
            created = created.display(),
            pid = sleeper_pid.display(),
        ),
    )
    .unwrap();
    for file in [&python, &uv] {
        std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let _env = Env::set(&[
        ("HOME", &home),
        ("XDG_DATA_HOME", &home),
        (crate::perception_setup::ENV_UV, &uv),
        (cut_perception::sidecar::ENV_SIDECAR_DIR, &payload),
    ]);
    let venv = match install {
        Install::Captions => cut_perception::appdata_sidecar_dir().unwrap().join(".venv"),
        Install::PremiumMatte => crate::matte::matanyone_venv_dir().unwrap(),
    };

    let cancellation = crate::jobs::JobCancellation::test_active();
    let canceller = cancellation.clone();
    let worker = std::thread::spawn(move || {
        let started = Instant::now();
        let progress = |_: f32, _: &str| {};
        let failed_to_install = match install {
            Install::Captions => {
                crate::perception_setup::setup_perception(false, &progress, &cancellation)
                    .map(|_| ())
            }
            Install::PremiumMatte => {
                crate::perception_setup::setup_matanyone(&progress, &cancellation).map(|_| ())
            }
        };
        (failed_to_install, started.elapsed())
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while !created.exists() {
        assert!(
            Instant::now() < deadline,
            "the fake venv step never started"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        venv.join("bin").join("python").exists(),
        "the half-built venv exists mid-install"
    );
    let cancelled_at = Instant::now();
    canceller.request_cancel();
    let (result, _) = worker.join().unwrap();
    let error = result.expect_err("a cancelled install is not a success");
    assert_eq!(error.code, "job_cancelled", "{error:?}");
    assert!(
        cancelled_at.elapsed() < Duration::from_secs(10),
        "cancel must take effect promptly, took {:?}",
        cancelled_at.elapsed()
    );
    assert!(
        !venv.exists(),
        "a cancelled install must not leave a venv that reads as installed"
    );
    let pid: u32 = std::fs::read_to_string(&sleeper_pid)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let gone_by = Instant::now() + Duration::from_secs(5);
    while Path::new(&format!("/proc/{pid}")).exists() {
        let state = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        if state.contains(") Z") {
            break; // a reaped-pending zombie holds no work
        }
        assert!(
            Instant::now() < gone_by,
            "the helper's child {pid} survived the cancel"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn cancelling_install_captions_mid_install_stops_at_once_and_leaves_no_venv() {
    cancel_mid_install(Install::Captions);
}

#[test]
fn cancelling_premium_background_removal_mid_install_stops_at_once_and_leaves_no_venv() {
    cancel_mid_install(Install::PremiumMatte);
}
