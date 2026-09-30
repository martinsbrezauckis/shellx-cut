use super::*;
use record_core::{Anchor, EditPlan, WebcamOverlay, WebcamShape};

pub(crate) fn camera_plan(path: &Path) -> EditPlan {
    let mut plan = EditPlan::empty(160, 90, 1000, 25.0);
    plan.webcam = Some(WebcamOverlay {
        source: path.to_string_lossy().into_owned(),
        shape: WebcamShape::Circle,
        anchor: Anchor::BottomRight,
        margin: 0.02,
        size: 0.2,
        camera_clock: None,
        timeline: Vec::new(),
    });
    plan
}

#[test]
fn embedded_camera_requires_project_or_exact_registered_source() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let local = dir.path().join("camera.mp4");
    let external = outside.path().join("camera.mp4");
    std::fs::write(&local, b"captured camera").unwrap();
    std::fs::write(&external, b"registered camera").unwrap();
    let mut project = Project::new("camera", Default::default());
    let mut plan = camera_plan(Path::new("camera.mp4"));
    admit(dir.path(), &project, &mut plan).unwrap();
    assert_eq!(webcam_path(&plan).unwrap(), local.canonicalize().unwrap());
    let mut plan = camera_plan(&external);
    assert!(admit(dir.path(), &project, &mut plan).is_err());
    project.assets.insert(
        "camera".into(),
        cut_core::Asset {
            path: external.to_string_lossy().into_owned(),
            hash: "sha256:synthetic".into(),
            probe: None,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    admit(dir.path(), &project, &mut plan).unwrap();
    let unrelated = outside.path().join("other.mp4");
    std::fs::write(&unrelated, b"unregistered").unwrap();
    assert!(admit(dir.path(), &project, &mut camera_plan(&unrelated)).is_err());
}

#[cfg(unix)]
#[test]
fn imported_camera_link_cannot_escape_project_or_reach_renderer() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let external = outside.path().join("outside.mp4");
    std::fs::write(&external, b"outside camera").unwrap();
    std::os::unix::fs::symlink(&external, dir.path().join("camera.mp4")).unwrap();
    let project = Project::new("camera", Default::default());
    assert!(admit(
        dir.path(),
        &project,
        &mut camera_plan(Path::new("camera.mp4"))
    )
    .is_err());
    assert_eq!(std::fs::read(&external).unwrap(), b"outside camera");
}
