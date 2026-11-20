//! Item 140: names inside a project's cache may be hard links (second names)
//! of files outside the project. An ordinary draft preview must never write
//! through them, and a cached file with a second name is never reused.
//! Uses the real preview writer and the installed FFmpeg; returns early with
//! a note when FFmpeg is not installed.

use cut_core::{edl_from_project, Clip, MediaClip, Project, ProjectSettings};
use cut_media::preview::render_preview_incremental;
use cut_media::RenderPreset;
use std::path::Path;

fn ffmpeg_present() -> bool {
    let ok = std::process::Command::new(cut_media::ffmpeg::ffmpeg_bin())
        .arg("-version")
        .output()
        .is_ok();
    if !ok {
        eprintln!("FFmpeg not installed: preview link check not run");
    }
    ok
}

/// A project folder only its owner can write (Cut's staging rule).
fn private_dir(path: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut std::fs::DirBuilder::new(), 0o700)
        .create(path)
        .unwrap();
    #[cfg(not(unix))]
    std::fs::create_dir(path).unwrap();
}

/// A one-clip timeline of a still image: the draft preview needs no proxy.
fn still_project(image: &Path) -> Project {
    let mut p = Project::new(
        "links",
        ProjectSettings {
            width: 320,
            height: 180,
            fps: 25.0,
            audio_rate: 48_000,
            color: cut_core::ColorConfig::default(),
        },
    );
    p.assets.insert(
        "img".into(),
        cut_core::Asset {
            path: image.display().to_string(),
            hash: "sha256:still".into(),
            probe: Some(serde_json::json!({"kind": "image"})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    p.track_mut("v1").unwrap().clips = vec![Clip::Media(MediaClip {
        id: "c1".into(),
        asset: "img".into(),
        src_in_ms: 0,
        src_out_ms: 1000,
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
    })];
    p
}

#[test]
fn draft_preview_never_writes_through_a_hard_linked_list_name() {
    if !ffmpeg_present() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside-user-file.txt");
    std::fs::write(&outside, "KEEP ORIGINAL").unwrap();
    let project_dir = root.path().join("received.cutproj");
    private_dir(&project_dir);
    private_dir(&project_dir.join("proxies"));
    let cache = project_dir.join("proxies").join("preview-cache");
    private_dir(&cache);
    // The received project carries its preview list name as a second name
    // of the outside file.
    std::fs::hard_link(&outside, cache.join("concat.txt")).unwrap();

    let image = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../branding/shellx-cut-icon.png");
    let project = still_project(&image);
    let edl = edl_from_project(&project);
    let draft = RenderPreset::named("draft").unwrap();
    let result = render_preview_incremental(&project, &edl, &project_dir, &cache, &draft);

    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "KEEP ORIGINAL",
        "the preview wrote through the project's list name into a file outside the project"
    );
    let preview = result.expect("the preview itself still renders");
    assert!(preview.path.exists());
}

#[test]
fn a_cached_preview_segment_with_a_second_name_is_not_reused() {
    if !ffmpeg_present() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("received.cutproj");
    private_dir(&project_dir);
    private_dir(&project_dir.join("proxies"));
    let cache = project_dir.join("proxies").join("preview-cache");
    private_dir(&cache);
    let image = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../branding/shellx-cut-icon.png");
    let project = still_project(&image);
    let edl = edl_from_project(&project);
    let draft = RenderPreset::named("draft").unwrap();
    // A first preview names its cached segment.
    let first = render_preview_incremental(&project, &edl, &project_dir, &cache, &draft).unwrap();
    let segment = cache.join(&first.rendered[0]);
    // Replace the cached segment with a second name of an outside file.
    let outside = root.path().join("outside-private.mp4");
    std::fs::rename(&segment, &outside).unwrap();
    std::fs::hard_link(&outside, &segment).unwrap();

    let error = render_preview_incremental(&project, &edl, &project_dir, &cache, &draft)
        .expect_err("a cached segment with a second name must not be reused");
    assert!(
        error.message.starts_with("unsafe media cache file"),
        "{error:?}"
    );
    assert!(
        error
            .suggested_action
            .as_deref()
            .is_some_and(|a| a.contains("cache")),
        "{error:?}"
    );
}

cut_core::isolate_test_process!();
