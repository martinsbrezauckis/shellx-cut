//! Item 139: a segment window planted in a received project's
//! `.cache/segrender` is never used as part of a render.
//!
//! Render a 4 s timeline in 2 s windows, replace every cached window with a
//! valid but different (red) clip of the same format, as a sender could under
//! the predictable cache names, and render again: the second output must equal
//! the first. Before the fix the red windows were reused.

use cut_core::{edl_from_project, Clip, MediaClip, Project, ProjectSettings};
use cut_media::{PathFence, RenderOptions, RenderPreset};
use std::path::{Path, PathBuf};

fn private_dir() -> ::tempfile::TempDir {
    let dir = ::tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    dir
}

fn ffmpeg(args: &[&str], out: &Path) {
    let args: Vec<String> = args
        .iter()
        .map(|s| s.to_string())
        .chain([out.display().to_string()])
        .collect();
    cut_media::ffmpeg::run_ffmpeg(&args).expect("ffmpeg");
}

fn media_clip(id: &str) -> Clip {
    Clip::Media(MediaClip {
        id: id.into(),
        asset: "a1".into(),
        src_in_ms: 0,
        src_out_ms: 4000,
        effects: vec![],
        gain_db: 0.0,
        transform: None,
        crop: None,
        fade: None,
        xfade_in_ms: 0,
        xfade_kind: None,
        speed: 1.0,
        grade: None,
        matte: None,
        mask: None,
        reverse: false,
        freeze: None,
        animation: None,
        keyframes: vec![],
        eq: None,
        mute_ranges: vec![],
        stabilize: None,
        speed_ramp: None,
        input_color_space: None,
        nest: None,
        grade_stack: vec![],
        grade_windows: vec![],
        markers: vec![],
    })
}

fn sha256(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(std::fs::read(path).unwrap()))
}

#[test]
fn planted_segment_windows_are_not_reused() {
    let dir = private_dir();
    let src = dir.path().join("src.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=4",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ],
        &src,
    );
    let mut project = Project::new(
        "t",
        ProjectSettings {
            width: 320,
            height: 240,
            fps: 30.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    project.assets.insert(
        "a1".into(),
        cut_core::Asset {
            path: src.display().to_string(),
            hash: "sha256:test".into(),
            probe: Some(serde_json::to_value(cut_media::probe(&src).unwrap()).unwrap()),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    project.track_mut("v1").unwrap().clips = vec![media_clip("c1")];
    let edl = edl_from_project(&project);
    let fence = PathFence::new(dir.path()).unwrap();
    let preset = RenderPreset::default();
    std::env::set_var("SHELLX_CUT_RENDER_WINDOW_SEC", "2");
    let render = |name: &str| -> PathBuf {
        let out = dir.path().join(name);
        cut_media::render::render_segmented(
            &project,
            &edl,
            &fence,
            &out,
            &preset,
            RenderOptions::default(),
            None,
        )
        .expect("segmented render");
        out
    };
    let first = render("first.mp4");

    // Plant: every cached window becomes a red clip of the same format.
    let cache = dir.path().join(".cache").join("segrender");
    let windows: Vec<PathBuf> = std::fs::read_dir(&cache)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("seg_"))
        .collect();
    assert_eq!(windows.len(), 2, "{windows:?}");
    let red = dir.path().join("red.mp4");
    ffmpeg(
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=red:size=320x240:rate=30:duration=2",
            "-c:v",
            "libx264",
            "-preset",
            "slow",
            "-crf",
            "20",
            "-pix_fmt",
            "yuv420p",
        ],
        &red,
    );
    for w in &windows {
        std::fs::copy(&red, w).unwrap();
    }
    let second = render("second.mp4");
    std::env::remove_var("SHELLX_CUT_RENDER_WINDOW_SEC");
    assert_eq!(
        sha256(&second),
        sha256(&first),
        "a planted window was used as part of the render"
    );
}

cut_core::isolate_test_process!();
