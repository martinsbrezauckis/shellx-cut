use super::*;
use crate::dispatch::dispatch;
use crate::state::AppState;
use cut_core::{Actor, ProjectStore};
use serde_json::json;

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("scopes.cutproj");
    std::fs::create_dir_all(project.join("exports/scopes")).unwrap();
    let outside = temp.path().join("literal-outside-scope-sentinel.txt");
    std::fs::write(&outside, b"outside stays unchanged").unwrap();
    (temp, project, outside)
}

#[test]
fn scope_images_admit_missing_and_ordinary_overwrite_with_strict_kind_names() {
    let (_temp, project, _outside) = fixture();
    let kinds = [
        ScopeKind::Vectorscope,
        ScopeKind::Waveform,
        ScopeKind::Histogram,
    ];
    let first = image_paths(&project, 1250, true, &kinds).unwrap();
    assert_eq!(first.len(), 3);
    for (kind, path) in &first {
        assert_eq!(
            *path,
            project.join(format!("exports/scopes/scope_1250ms_{}.png", kind.key()))
        );
        std::fs::write(path, b"ordinary prior PNG").unwrap();
    }
    let second = image_paths(&project, 1250, true, &kinds).unwrap();
    for ((kind, path), (prior_kind, prior_path)) in second.iter().zip(&first) {
        assert_eq!(kind.key(), prior_kind.key());
        assert_eq!(path, prior_path);
        assert_eq!(std::fs::read(path).unwrap(), b"ordinary prior PNG");
    }
}

#[test]
fn scope_images_refuse_hardlink_but_measurement_only_ignores_image_leaves() {
    let (_temp, project, outside) = fixture();
    let target = project.join("exports/scopes/scope_1250ms_waveform.png");
    std::fs::hard_link(&outside, &target).unwrap();
    assert!(image_paths(&project, 1250, true, &[ScopeKind::Waveform]).is_err());
    assert!(image_paths(&project, 1250, false, &[ScopeKind::Waveform])
        .unwrap()
        .is_empty());
    assert!(image_paths(&project, 1250, true, &[]).unwrap().is_empty());
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside stays unchanged");
    assert!(!project.join("exports/scopes/frame_1250ms.jpg").exists());
}

#[cfg(unix)]
fn symlink(source: &Path, target: &Path) -> bool {
    std::os::unix::fs::symlink(source, target).unwrap();
    true
}

#[cfg(windows)]
fn symlink(source: &Path, target: &Path) -> bool {
    match std::os::windows::fs::symlink_file(source, target) {
        Ok(()) => true,
        Err(error) if error.raw_os_error() == Some(1314) => {
            eprintln!("symlink fixture unavailable without Windows link privilege");
            false
        }
        Err(error) => panic!("create scope PNG symlink: {error}"),
    }
}

#[cfg(any(unix, windows))]
#[test]
fn scope_images_refuse_static_and_dangling_symlink_leaves() {
    let (temp, project, outside) = fixture();
    let target = project.join("exports/scopes/scope_1250ms_histogram.png");
    if !symlink(&outside, &target) {
        return;
    }
    assert!(image_paths(&project, 1250, true, &[ScopeKind::Histogram]).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside stays unchanged");
    std::fs::remove_file(&target).unwrap();
    let missing = temp.path().join("literal-missing-outside.txt");
    assert!(symlink(&missing, &target));
    assert!(image_paths(&project, 1250, true, &[ScopeKind::Histogram]).is_err());
    assert!(!missing.exists());
}

#[test]
fn scope_images_check_only_selected_kinds_and_refuse_directory_leaf() {
    let (_temp, project, outside) = fixture();
    let target = project.join("exports/scopes/scope_1250ms_waveform.png");
    std::fs::hard_link(&outside, &target).unwrap();
    assert!(image_paths(&project, 1250, true, &[ScopeKind::Histogram]).is_ok());
    std::fs::create_dir(project.join("exports/scopes/scope_1250ms_histogram.png")).unwrap();
    assert!(image_paths(&project, 1250, true, &[ScopeKind::Histogram]).is_err());
}

#[tokio::test]
async fn verify_scopes_refuses_image_hardlink_before_frame_materialization() {
    let temp = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(temp.path(), "dispatch-scopes", None).unwrap();
    let project = store.dir.clone();
    std::fs::create_dir_all(project.join("exports/scopes")).unwrap();
    let outside = temp.path().join("literal-outside-dispatch.txt");
    std::fs::write(&outside, b"outside stays unchanged").unwrap();
    std::fs::hard_link(
        &outside,
        project.join("exports/scopes/scope_1250ms_waveform.png"),
    )
    .unwrap();
    let state = AppState::new();
    *state.project.write().await = Some(ProjectStore::open(&project).unwrap());
    let response = dispatch(
        &state,
        "verify.scopes",
        json!({"at_ms": 1250, "scope_images": true, "kinds": ["waveform"]}),
        Actor::system(),
    )
    .await;
    assert!(!response.ok, "imported image hardlink must refuse scopes");
    assert!(response.error.unwrap().message.contains("not a plain file"));
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside stays unchanged");
    assert!(!project.join("exports/scopes/frame_1250ms.jpg").exists());
}

#[tokio::test]
async fn verify_scopes_refuses_unknown_kind_before_output_creation() {
    let temp = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(temp.path(), "unknown-kind", None).unwrap();
    let project = store.dir.clone();
    let state = AppState::new();
    *state.project.write().await = Some(store);
    let response = dispatch(
        &state,
        "verify.scopes",
        json!({"scope_images": true, "kinds": ["../outside"]}),
        Actor::system(),
    )
    .await;
    assert!(!response.ok);
    assert_eq!(
        response.error.unwrap().code,
        cut_core::error_codes::INVALID_ARGS
    );
    assert!(!project.join("exports").exists());
}
