//! Imported output leaves must never grant authority over another file.
use cut_core::{Clip, GapClip, Project, ProjectSettings};
use cut_media::{PathFence, RenderOptions, RenderPreset};
use std::path::Path;

fn gap_project() -> Project {
    let mut project = Project::new(
        "output boundary",
        ProjectSettings {
            width: 64,
            height: 48,
            fps: 10.0,
            ..ProjectSettings::default()
        },
    );
    project.tracks[0].clips.push(Clip::Gap(GapClip::new(400)));
    project
}

fn range(project: &Project, fence: &PathFence, output: &Path) -> Result<(), cut_core::CutError> {
    cut_media::render::render_range(
        project,
        &cut_core::edl_from_project(project),
        fence,
        output,
        &RenderPreset::named("draft").unwrap(),
        [0, 400],
        RenderOptions::default(),
        None,
    )
    .map(|_| ())
}

#[test]
fn range_refuses_predictable_gif_intermediate_hardlink_before_encode() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("preferences.bin");
    std::fs::write(&sentinel, b"keep").unwrap();
    let intermediate = root.path().join("gif_0_400.src.mp4");
    std::fs::hard_link(&sentinel, &intermediate).unwrap();
    let fence = PathFence::new(root.path()).unwrap();
    assert_eq!(
        range(&gap_project(), &fence, &intermediate)
            .unwrap_err()
            .code,
        cut_core::error_codes::INVALID_ARGS
    );
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep");
    assert_eq!(std::fs::read(&intermediate).unwrap(), b"keep");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn gif_final_hardlink_is_refused_before_input_or_encoder() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = outside.path().join("preferences.bin");
    std::fs::write(&sentinel, b"keep").unwrap();
    let output = root.path().join("final.gif");
    std::fs::hard_link(&sentinel, &output).unwrap();
    assert_eq!(
        cut_media::gif::make_gif(&root.path().join("missing.mp4"), &output, 10, 64, "none")
            .unwrap_err()
            .code,
        cut_core::error_codes::INVALID_ARGS
    );
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn range_and_gif_refuse_symlink_and_dangling_output_leaves() {
    for dangling in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let sentinel = outside.path().join("preferences.bin");
        if !dangling {
            std::fs::write(&sentinel, b"keep").unwrap();
        }
        let intermediate = root.path().join("gif_0_400.src.mp4");
        let output = root.path().join("final.gif");
        std::os::unix::fs::symlink(&sentinel, &intermediate).unwrap();
        std::os::unix::fs::symlink(&sentinel, &output).unwrap();
        let fence = PathFence::new(root.path()).unwrap();
        assert_eq!(
            range(&gap_project(), &fence, &intermediate)
                .unwrap_err()
                .code,
            cut_core::error_codes::INVALID_ARGS
        );
        assert_eq!(
            cut_media::gif::make_gif(&intermediate, &output, 10, 64, "none")
                .unwrap_err()
                .code,
            cut_core::error_codes::INVALID_ARGS
        );
        if dangling {
            assert!(!sentinel.exists());
        } else {
            assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep");
        }
    }
}

#[test]
fn real_ffmpeg_range_and_gif_allow_plain_overwrite_in_shared_output_folder() {
    let project_root = tempfile::tempdir().unwrap();
    let output_root = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(output_root.path(), std::fs::Permissions::from_mode(0o770))
            .unwrap();
    }
    let fence = PathFence::new(project_root.path())
        .unwrap()
        .with_extra_root(output_root.path())
        .unwrap();
    let intermediate = output_root.path().join("gif_0_400.src.mp4");
    let output = output_root.path().join("final.gif");
    std::fs::write(&intermediate, b"ordinary previous intermediate").unwrap();
    range(&gap_project(), &fence, &intermediate).unwrap();
    let facts = cut_media::probe(&intermediate).unwrap();
    assert_eq!((facts.width, facts.height), (Some(64), Some(48)));
    assert!(facts
        .duration_ms
        .is_some_and(|duration| (300..=500).contains(&duration)));
    std::fs::write(&output, b"ordinary previous GIF").unwrap();
    cut_media::gif::make_gif(&intermediate, &output, 10, 64, "none").unwrap();
    assert!(std::fs::read(&output).unwrap().starts_with(b"GIF8"));
    range(&gap_project(), &fence, &intermediate).unwrap();
    cut_media::gif::make_gif(&intermediate, &output, 10, 64, "none").unwrap();
    assert!(std::fs::read(&output).unwrap().starts_with(b"GIF8"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(output_root.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o770
        );
    }
}
