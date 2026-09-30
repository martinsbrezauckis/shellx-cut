use super::*;
use crate::dispatch::dispatch;
use crate::state::AppState;
use base64::Engine;
use cut_core::{Actor, ProjectStore};
use serde_json::json;

const FILE_NAME: &str = "generate_preview_test_0123456789abcdef.png";

fn png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aZuoAAAAASUVORK5CYII=")
        .unwrap()
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("preview.cutproj");
    std::fs::create_dir_all(project.join("frames")).unwrap();
    let source = temp.path().join("source.png");
    std::fs::write(&source, png()).unwrap();
    (temp, project, source)
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
        Err(error) => panic!("create PNG symlink: {error}"),
    }
}

#[test]
fn motion_preview_copy_preserves_deterministic_overwrite_and_same_source() {
    let (_temp, project, source) = fixture();
    let target = project.join("frames").join(FILE_NAME);
    let first = copy_motion_preview(&project, &source, FILE_NAME).unwrap();
    assert_eq!(first, target);
    assert_eq!(std::fs::read(&first).unwrap(), png());
    std::fs::write(&target, b"old ordinary preview").unwrap();
    assert_eq!(
        copy_motion_preview(&project, &source, FILE_NAME).unwrap(),
        first
    );
    assert_eq!(std::fs::read(&target).unwrap(), png());
    assert_eq!(
        copy_motion_preview(&project, &target, FILE_NAME).unwrap(),
        first
    );
    assert_eq!(std::fs::read(&source).unwrap(), png());
}

#[test]
fn motion_preview_copy_refuses_hardlink_even_when_source_is_target() {
    let (temp, project, source) = fixture();
    let outside = temp.path().join("literal-outside-sentinel.txt");
    std::fs::write(&outside, b"outside stays unchanged").unwrap();
    let target = project.join("frames").join(FILE_NAME);
    std::fs::hard_link(&outside, &target).unwrap();
    assert!(copy_motion_preview(&project, &source, FILE_NAME).is_err());
    assert!(copy_motion_preview(&project, &target, FILE_NAME).is_err());
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside stays unchanged");
    assert_eq!(std::fs::read(&source).unwrap(), png());
}

#[cfg(any(unix, windows))]
#[test]
fn motion_preview_copy_refuses_static_and_dangling_symlink_leaves() {
    let (temp, project, source) = fixture();
    let target = project.join("frames").join(FILE_NAME);
    for existing in [true, false] {
        let outside = temp.path().join(format!("literal-outside-{existing}.txt"));
        if existing {
            std::fs::write(&outside, b"outside stays unchanged").unwrap();
        }
        if !symlink(&outside, &target) {
            return;
        }
        assert!(copy_motion_preview(&project, &source, FILE_NAME).is_err());
        assert!(copy_motion_preview(&project, &target, FILE_NAME).is_err());
        assert!(std::fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink());
        if existing {
            assert_eq!(std::fs::read(&outside).unwrap(), b"outside stays unchanged");
        } else {
            assert!(!outside.exists(), "no outside file was created");
        }
        std::fs::remove_file(&target).unwrap();
    }
    assert_eq!(std::fs::read(&source).unwrap(), png());
}

#[test]
fn motion_preview_copy_failure_keeps_existing_png_and_refuses_directory_leaf() {
    let (temp, project, _source) = fixture();
    let target = project.join("frames").join(FILE_NAME);
    std::fs::write(&target, png()).unwrap();
    assert!(copy_motion_preview(&project, &temp.path().join("missing.png"), FILE_NAME).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), png());
    std::fs::remove_file(&target).unwrap();
    std::fs::create_dir(&target).unwrap();
    assert!(project_preview_path(&project, FILE_NAME).is_err());
}

async fn native_fixture() -> (
    tempfile::TempDir,
    AppState,
    PathBuf,
    serde_json::Value,
    String,
) {
    let temp = tempfile::tempdir().unwrap();
    let store = ProjectStore::create(temp.path(), "native", None).unwrap();
    let project = store.dir.clone();
    let state = AppState::new();
    *state.project.write().await = Some(store);
    let args = json!({
        "id": "builtin.lower-third.clean",
        "params": {"name": "Preview boundary"},
        "width": 320,
        "height": 180,
        "frame_ms": 1000,
    });
    let template = crate::generate::registry()
        .get("builtin.lower-third.clean")
        .unwrap();
    let params =
        crate::generate::resolve_params(template, args["params"].as_object().unwrap()).unwrap();
    let duration = crate::generate::resolve_duration_ms(template, &params);
    let lowering = crate::generate::interpolate_args(template, &params, [0, duration]).unwrap();
    let id = crate::generate_handlers::generate_preview_id(&template.id, &lowering, 320, 180, 1000);
    (temp, state, project, args, id)
}

#[tokio::test]
async fn native_title_preview_renders_png_and_overwrites_the_same_served_path() {
    let (_temp, state, project, args, id) = native_fixture().await;
    for _ in 0..2 {
        let response = dispatch(&state, "generate.preview", args.clone(), Actor::system()).await;
        assert!(response.ok, "native preview failed: {:?}", response.error);
        let result = response.result.unwrap();
        assert_eq!(result["preview_id"], id);
        assert_eq!(result["url"], format!("/frames/{id}.png"));
        assert_eq!(
            result["path"],
            json!(project.join("frames").join(format!("{id}.png")))
        );
        assert_eq!(result["warnings"], json!([]));
        assert!(std::fs::read(result["path"].as_str().unwrap())
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n"));
    }
}

#[tokio::test]
async fn native_title_preview_refuses_an_imported_hardlink_before_rendering() {
    let (temp, state, project, args, id) = native_fixture().await;
    let outside = temp.path().join("literal-outside-native-title.txt");
    std::fs::write(&outside, b"outside stays unchanged").unwrap();
    std::fs::create_dir(project.join("frames")).unwrap();
    std::fs::hard_link(&outside, project.join("frames").join(format!("{id}.png"))).unwrap();
    // Ordinary project admission does not inspect final preview leaves.
    *state.project.write().await = Some(ProjectStore::open(&project).unwrap());
    let response = dispatch(&state, "generate.preview", args, Actor::system()).await;
    assert!(
        !response.ok,
        "imported hardlink must refuse native rendering"
    );
    assert!(response.error.unwrap().message.contains("not a plain file"));
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside stays unchanged");
}

#[cfg(any(unix, windows))]
#[tokio::test]
async fn native_title_preview_refuses_an_imported_symlink_before_rendering() {
    let (temp, state, project, args, id) = native_fixture().await;
    let outside = temp.path().join("literal-outside-native-title.txt");
    std::fs::write(&outside, b"outside stays unchanged").unwrap();
    std::fs::create_dir(project.join("frames")).unwrap();
    if !symlink(&outside, &project.join("frames").join(format!("{id}.png"))) {
        return;
    }
    *state.project.write().await = Some(ProjectStore::open(&project).unwrap());
    let response = dispatch(&state, "generate.preview", args, Actor::system()).await;
    assert!(
        !response.ok,
        "imported symlink must refuse native rendering"
    );
    assert!(response.error.unwrap().message.contains("not a plain file"));
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside stays unchanged");
}
