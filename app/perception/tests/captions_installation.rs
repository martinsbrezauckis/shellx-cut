use cut_perception::captions_installation::{
    probe_selected_python, remember_captions_installation, InstallationState,
};
#[cfg(unix)]
use std::path::Path;
use std::time::Duration;

#[test]
fn absent_configuration_and_a_disappeared_configured_interpreter_differ() {
    let absent = probe_selected_python(None, false, Duration::from_secs(1));
    assert_eq!(absent.state, InstallationState::NotInstalled);
    assert!(absent.error.is_none());
    let dir = tempfile::tempdir().unwrap();
    let expected = dir.path().join("missing-python");
    let damaged = probe_selected_python(Some(&expected), false, Duration::from_secs(1));
    assert_eq!(damaged.state, InstallationState::Damaged);
    assert!(damaged.error.is_some());
    assert!(damaged.installation_id.is_some());
    assert!(!cut_perception::is_captions_not_installed(
        damaged.error.as_ref().unwrap()
    ));
}

#[test]
fn an_install_record_is_independent_and_setup_repairs_invalid_metadata() {
    let dir = tempfile::tempdir().unwrap();
    remember_captions_installation(dir.path(), "3.12.9").unwrap();
    assert!(!dir.path().join(".venv").exists());
    let marker = dir.path().join("captions-installed.json");
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
    assert_eq!(record["schema"], "shellx-cut/captions-installation/1");
    // Successful setup refreshes metadata without changing the installed tools.
    remember_captions_installation(dir.path(), "3.12.10").unwrap();
    std::fs::write(&marker, b"not json").unwrap();
    remember_captions_installation(dir.path(), "3.12.10").unwrap();
    let repaired: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&marker).unwrap()).unwrap();
    assert_eq!(repaired["python_version"], "3.12.10");
}

#[cfg(unix)]
#[test]
fn installation_record_replaces_a_link_without_writing_its_target() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("not-the-record");
    std::fs::write(&target, b"keep this").unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join("captions-installed.json")).unwrap();
    remember_captions_installation(dir.path(), "3.12.9").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"keep this");
}

#[cfg(unix)]
fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn probe_keeps_a_failed_exit_reason_instead_of_calling_it_missing() {
    let dir = tempfile::tempdir().unwrap();
    let python = dir.path().join("python");
    script(&python, "echo 'runtime loader failed' >&2; exit 7");
    let result = probe_selected_python(Some(&python), false, Duration::from_secs(1));
    assert_eq!(result.state, InstallationState::Damaged);
    let error = result.error.unwrap();
    assert!(
        error.cause.contains("runtime loader failed"),
        "{}",
        error.cause
    );
    assert!(error.cause.contains('7'), "{}", error.cause);
    assert!(error.suggested_action.is_some());
    assert!(result.version.is_none());
}

#[cfg(unix)]
#[test]
fn runnable_python_is_ready_but_does_not_claim_analysis_ran() {
    let dir = tempfile::tempdir().unwrap();
    let python = dir.path().join("python");
    script(&python, "echo 'Python 3.12.9'");
    let result = probe_selected_python(Some(&python), false, Duration::from_secs(1));
    assert_eq!(result.state, InstallationState::Ready);
    assert_eq!(result.version.as_deref(), Some("Python 3.12.9"));
    assert!(result.error.is_none());
}

#[cfg(unix)]
#[test]
fn an_empty_banner_and_a_hung_probe_are_check_errors_not_absence() {
    let dir = tempfile::tempdir().unwrap();
    let python = dir.path().join("python");
    script(&python, "exit 0");
    let empty = probe_selected_python(Some(&python), false, Duration::from_secs(1));
    assert_eq!(empty.state, InstallationState::Damaged);
    script(&python, "sleep 10");
    let hung = probe_selected_python(Some(&python), false, Duration::from_millis(100));
    assert_eq!(hung.state, InstallationState::Damaged);
    assert!(hung.error.is_some());
}
