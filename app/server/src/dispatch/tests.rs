use super::media::{
    load_capture_manifest, manifest_event_to_marker, resolve_capture_manifest, utc_to_media_ms,
};
use super::*;
use crate::state::AppState;

static AGENT_CLI_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_agent_cli_env() -> std::sync::MutexGuard<'static, ()> {
    AGENT_CLI_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn test_actor() -> Actor {
    Actor {
        kind: cut_core::ActorKind::Agent,
        name: "test".into(),
        via: "test".into(),
        request: None,
    }
}

mod caption_track_resolution;
mod detach_audio;
mod generation_cli;
mod nest_media_io;
mod output_contract;
mod recipe_runner;
mod render_verify;
mod request_idempotency;
mod schema_contract;
mod screen_record;
mod screen_record_containment;
mod screen_record_export_regression;
mod screen_record_link_consumers;
mod screen_record_sparse_export_regression;
mod sequence_index;
mod smart_bins;
mod system_tools;
mod ui_command_confirmation;

#[test]
fn effect_strips_flattened_track_key_from_detail() {
    let e = effect(Some("v1"), json!({"track":"evil","clip":"c1"}));
    assert_eq!(e.track.as_deref(), Some("v1"));
    assert!(
        !e.detail.contains_key("track"),
        "detail.track would collide with OpEffect.track under serde(flatten): {:?}",
        e.detail
    );
    let encoded = serde_json::to_value(&e).unwrap();
    assert_eq!(encoded["track"], "v1");
    assert_eq!(encoded["clip"], "c1");
}

#[test]
fn title_free_placement_rejects_off_canvas_anchor() {
    let args = TitleArgs {
        text: "Hi".into(),
        range_ms: [0, 1000],
        preset: None,
        font_px: None,
        color: None,
        bg: None,
        x: Some(1.25),
        y: Some(0.50),
        align: None,
        animation: None,
        template: None,
        accent: None,
        emphasis: None,
        rationale: None,
    };

    let err = build_title_spec(&args, 1920, 1080, 30.0).unwrap_err();

    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.message.contains("x"),
        "error should name the invalid anchor: {err:?}"
    );
}

#[test]
fn shape_build_rejects_off_canvas_normalized_geometry() {
    let args = ShapeArgs {
        shape: "rect".into(),
        range_ms: [0, 1000],
        x: Some(0.80),
        y: Some(0.10),
        w: Some(0.40),
        h: Some(0.20),
        x2: None,
        y2: None,
        fill: Some("#FFFFFF".into()),
        stroke: None,
        stroke_px: None,
        opacity: None,
        radius_px: None,
        head_px: None,
        text: None,
        color: None,
        font_px: None,
        animation: None,
        rationale: None,
    };

    let err = build_shape_spec(&args, 1920, 1080, 30.0).unwrap_err();

    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.message.contains("x + w"),
        "error should name the off-canvas box edge: {err:?}"
    );
}

#[test]
fn export_richness_warnings_are_target_specific() {
    let mut project = cut_core::Project::new("warn", cut_core::ProjectSettings::default());
    let mut base = cut_core::edit::make_media_clip("base", "a1", 0, 4000);
    base.speed = 2.0;
    base.gain_db = -3.0;
    project.tracks[0].clips.push(cut_core::Clip::Media(base));
    project.tracks.push(cut_core::Track {
        id: "v2".into(),
        kind: cut_core::TrackKind::Video,
        clips: vec![cut_core::Clip::Media(cut_core::edit::make_media_clip(
            "overlay", "a2", 0, 1000,
        ))],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    project.tracks.push(cut_core::Track {
        id: "cap1".into(),
        kind: cut_core::TrackKind::Caption,
        clips: vec![cut_core::Clip::Caption(cut_core::CaptionClip {
            id: "s1".into(),
            text: "hello".into(),
            style_ref: None,
            range_ms: [0, 1000],
        })],
        gain_db: 0.0,
        gain_windows: vec![],
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    });
    project.adjustments.push(cut_core::Adjustment {
        id: "adj1".into(),
        range_ms: [0, 1000],
        grade: Some(cut_core::ClipGrade {
            contrast: 1.2,
            brightness: 0.0,
            saturation: 1.0,
            gamma: 1.0,
            temperature_k: None,
            lut: None,
        }),
        effects: vec![],
    });

    let dropped = |target| -> Vec<String> {
        export_richness_warnings(&project, target)[0].detail["dropped"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };

    let edl = dropped(ExportWarningTarget::Edl);
    assert!(edl.contains(&"speed changes".to_string()));
    assert!(edl.contains(&"overlay video tracks".to_string()));
    assert!(edl.contains(&"adjustment layers".to_string()));
    assert!(edl.contains(&"captions".to_string()));

    let premiere = dropped(ExportWarningTarget::Xml(cut_export::XmlFormat::Premiere));
    assert!(premiere.contains(&"speed changes".to_string()));
    assert!(premiere.contains(&"adjustment layers".to_string()));
    assert!(!premiere.contains(&"overlay video tracks".to_string()));

    let fcpxml = dropped(ExportWarningTarget::Xml(cut_export::XmlFormat::Fcpxml));
    assert!(fcpxml.contains(&"overlay video tracks".to_string()));
    assert!(fcpxml.contains(&"clip gain".to_string()));

    let mlt = dropped(ExportWarningTarget::Xml(cut_export::XmlFormat::Mlt));
    assert!(
        !mlt.contains(&"clip gain".to_string()),
        "MLT exports clip gain and must not report it as dropped"
    );

    let otio = dropped(ExportWarningTarget::Otio);
    assert!(!otio.contains(&"overlay video tracks".to_string()));
    assert!(otio.contains(&"clip gain".to_string()));
}

#[test]
fn bundle_brand_check_uses_platform_geometry_for_aspect() {
    let mut project = cut_core::Project::new("bundle", cut_core::ProjectSettings::default());
    project.settings.width = 1920;
    project.settings.height = 1080;
    let spec = cut_core::BrandKit {
        aspect: Some("9:16".into()),
        fonts: None,
        colors: None,
        position: None,
        min_size: None,
        max_size: None,
    }
    .normalized()
    .unwrap();
    let result = super::brand::check_bundle_brand(
        &project,
        &spec,
        &[("9:16".into(), (1080, 1920)), ("16:9".into(), (1920, 1080))],
        "stored",
    );

    assert_eq!(result["pass"], false);
    assert_eq!(result["platforms"][0]["check"]["pass"], true);
    assert_eq!(result["platforms"][1]["check"]["pass"], false);
    assert_eq!(
        result["platforms"][1]["check"]["violations"]["aspect"]["geometry"],
        "1920x1080"
    );
    assert_eq!(result["source"], "stored");
}

#[test]
fn heavy_background_jobs_use_bounded_slots() {
    let media_src = include_str!("media.rs");
    let rendering_src = include_str!("rendering.rs");
    let dub_src = include_str!("../dub.rs");
    let diarize_src = include_str!("../diarize.rs");

    fn assert_limited(src: &str, marker: &str, expected: &str) {
        let Some(pos) = src.find(marker) else {
            assert!(src.contains(marker), "missing heavy job marker: {marker}");
            return;
        };
        let end = (pos + 6500).min(src.len());
        let window = &src[pos..end];
        let compact_window: String = window.chars().filter(|c| !c.is_whitespace()).collect();
        let compact_expected: String = expected.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            compact_window.contains(&compact_expected),
            "{marker} must spawn through `{expected}` so tests/UI cannot fan out heavy workers"
        );
    }

    assert_limited(
        dub_src,
        "pub(crate) async fn audio_dub",
        "crate::dispatch::ANALYSIS_MAX_RUNNING",
    );
    assert_limited(
        diarize_src,
        "state.jobs.create(\"diarize\")",
        "crate::dispatch::ANALYSIS_MAX_RUNNING",
    );

    for (source, marker, expected) in [
        (
            media_src,
            "state.jobs.create(\"transcribe\")",
            "spawn_limited(&job_id, \"analysis\", ANALYSIS_MAX_RUNNING",
        ),
        (
            media_src,
            "state.jobs.create(\"perception\")",
            "spawn_limited(&job_id, \"analysis\", ANALYSIS_MAX_RUNNING",
        ),
        (
            rendering_src,
            "state.jobs.create(\"render\")",
            "spawn_limited(&job_id, \"render\", RENDER_MAX_RUNNING",
        ),
        (
            rendering_src,
            "state.jobs.create(\"reframe-direct\")",
            "spawn_limited(&job_id, \"render\", RENDER_MAX_RUNNING",
        ),
        (
            rendering_src,
            "state.jobs.create(\"reframe-qc\")",
            "spawn_limited(&job_id, \"analysis\", ANALYSIS_MAX_RUNNING",
        ),
        (
            rendering_src,
            "state.jobs.create(\"reframe\")",
            "spawn_limited(&job_id, \"render\", RENDER_MAX_RUNNING",
        ),
        (
            rendering_src,
            "state.jobs.create(\"bundle\")",
            "spawn_limited(&job_id, \"render\", RENDER_MAX_RUNNING",
        ),
        (
            rendering_src,
            "state.jobs.create(\"render_queue\")",
            "spawn_limited(&queue_id, \"render_queue\", RENDER_QUEUE_MAX_RUNNING",
        ),
    ] {
        assert_limited(source, marker, expected);
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(path, perms).unwrap();
}

#[cfg(windows)]
fn make_executable(_path: &Path) {}

#[test]
fn translation_auto_keeps_cli_runtime_failures_honest() {
    let _guard = lock_agent_cli_env();
    let old_path = std::env::var_os("PATH");
    let old_runner_py = std::env::var_os("TRANSLATE_RUNNER_PY");
    let old_runner_script = std::env::var_os("TRANSLATE_RUNNER_SCRIPT");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();

    #[cfg(windows)]
    let claude = bin.join("claude.cmd");
    #[cfg(not(windows))]
    let claude = bin.join("claude");
    #[cfg(windows)]
    std::fs::write(
        &claude,
        "@echo off\r\nmore >nul\r\necho {\"type\":\"result\",\"is_error\":true,\"result\":\"weekly limit\"}\r\n",
    )
    .unwrap();
    #[cfg(not(windows))]
        std::fs::write(
            &claude,
            "#!/usr/bin/env sh\ncat >/dev/null\nprintf '%s\\n' '{\"type\":\"result\",\"is_error\":true,\"result\":\"weekly limit\"}'\n",
        )
        .unwrap();
    make_executable(&claude);
    #[cfg(windows)]
    let codex = bin.join("codex.cmd");
    #[cfg(not(windows))]
    let codex = bin.join("codex");
    #[cfg(windows)]
        std::fs::write(
            &codex,
            "@echo off\r\nmore >nul\r\necho {\"type\":\"result\",\"is_error\":true,\"result\":\"codex unavailable\"}\r\n",
        )
        .unwrap();
    #[cfg(not(windows))]
        std::fs::write(
            &codex,
            "#!/usr/bin/env sh\ncat >/dev/null\nprintf '%s\\n' '{\"type\":\"result\",\"is_error\":true,\"result\":\"codex unavailable\"}'\n",
        )
        .unwrap();
    make_executable(&codex);
    #[cfg(windows)]
    let grok = bin.join("grok.cmd");
    #[cfg(not(windows))]
    let grok = bin.join("grok");
    #[cfg(windows)]
        std::fs::write(
            &grok,
            "@echo off\r\nmore >nul\r\necho {\"type\":\"result\",\"is_error\":true,\"result\":\"grok unavailable\"}\r\n",
        )
        .unwrap();
    #[cfg(not(windows))]
        std::fs::write(
            &grok,
            "#!/usr/bin/env sh\ncat >/dev/null\nprintf '%s\\n' '{\"type\":\"result\",\"is_error\":true,\"result\":\"grok unavailable\"}'\n",
        )
        .unwrap();
    make_executable(&grok);

    let local_marker = dir.path().join("local-ran");
    #[cfg(windows)]
    let fake_py = bin.join("fake-python.cmd");
    #[cfg(not(windows))]
    let fake_py = bin.join("fake-python");
    #[cfg(windows)]
        std::fs::write(
            &fake_py,
            format!(
                "@echo off\r\ntype nul > \"{}\"\r\necho {{\"translations\":[\"LOCAL\"],\"model\":\"fake-local\",\"backend\":\"opus-mt\"}}\r\n",
                local_marker.display()
            ),
        )
        .unwrap();
    #[cfg(not(windows))]
        std::fs::write(
            &fake_py,
            format!(
                "#!/usr/bin/env sh\ntouch '{}'\nprintf '%s\\n' '{{\"translations\":[\"LOCAL\"],\"model\":\"fake-local\",\"backend\":\"opus-mt\"}}'\n",
                local_marker.display()
            ),
        )
        .unwrap();
    make_executable(&fake_py);
    let fake_script = bin.join("translate_runner.py");
    std::fs::write(&fake_script, "# fake local runner marker\n").unwrap();

    let new_path = match old_path.clone() {
        Some(prev) => {
            let mut paths = vec![bin.clone()];
            paths.extend(std::env::split_paths(&prev));
            std::env::join_paths(paths).unwrap()
        }
        None => std::env::join_paths([bin.clone()]).unwrap(),
    };
    std::env::set_var("PATH", new_path);
    std::env::set_var("TRANSLATE_RUNNER_PY", &fake_py);
    std::env::set_var("TRANSLATE_RUNNER_SCRIPT", &fake_script);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(crate::translate::run_translation(
        None,
        Some("en"),
        "es",
        &["hello".to_string()],
        None,
        Some(10_000),
    ));

    match old_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }
    match old_runner_py {
        Some(path) => std::env::set_var("TRANSLATE_RUNNER_PY", path),
        None => std::env::remove_var("TRANSLATE_RUNNER_PY"),
    }
    match old_runner_script {
        Some(path) => std::env::set_var("TRANSLATE_RUNNER_SCRIPT", path),
        None => std::env::remove_var("TRANSLATE_RUNNER_SCRIPT"),
    }

    assert!(
        result.is_err(),
        "auto mode must not fall back after a CLI runtime failure"
    );
    let err = match result {
        Err(error) => error,
        Ok(_) => return,
    };
    assert!(
        err.message.contains("weekly limit"),
        "CLI failure should be surfaced directly: {err:?}"
    );
    assert!(
        !local_marker.exists(),
        "local translator ran even though a CLI agent existed and failed"
    );
}

#[test]
fn translation_auto_tries_next_cli_agent_before_local_fallback() {
    let _guard = lock_agent_cli_env();
    let old_path = std::env::var_os("PATH");
    let old_runner_py = std::env::var_os("TRANSLATE_RUNNER_PY");
    let old_runner_script = std::env::var_os("TRANSLATE_RUNNER_SCRIPT");
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();

    #[cfg(windows)]
    let claude = bin.join("claude.cmd");
    #[cfg(not(windows))]
    let claude = bin.join("claude");
    #[cfg(windows)]
    std::fs::write(
        &claude,
        "@echo off\r\nmore >nul\r\necho {\"type\":\"result\",\"is_error\":true,\"result\":\"weekly limit\"}\r\n",
    )
    .unwrap();
    #[cfg(not(windows))]
        std::fs::write(
            &claude,
            "#!/usr/bin/env sh\ncat >/dev/null\nprintf '%s\\n' '{\"type\":\"result\",\"is_error\":true,\"result\":\"weekly limit\"}'\n",
        )
        .unwrap();
    make_executable(&claude);

    #[cfg(windows)]
    let codex = bin.join("codex.cmd");
    #[cfg(not(windows))]
    let codex = bin.join("codex");
    #[cfg(windows)]
        std::fs::write(
            &codex,
            "@echo off\r\nmore >nul\r\necho {\"type\":\"session.created\",\"session_id\":\"t\"}\r\necho {\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[{\\\"i\\\":1,\\\"text\\\":\\\"Hola\\\"}]\"}}\r\n",
        )
        .unwrap();
    #[cfg(not(windows))]
        std::fs::write(
            &codex,
            "#!/usr/bin/env sh\ncat >/dev/null\nprintf '%s\\n' '{\"type\":\"session.created\",\"session_id\":\"t\"}'\nprintf '%s\\n' '{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"[{\\\"i\\\":1,\\\"text\\\":\\\"Hola\\\"}]\"}}'\n",
        )
        .unwrap();
    make_executable(&codex);

    let local_marker = dir.path().join("local-ran");
    #[cfg(windows)]
    let fake_py = bin.join("fake-python.cmd");
    #[cfg(not(windows))]
    let fake_py = bin.join("fake-python");
    #[cfg(windows)]
        std::fs::write(
            &fake_py,
            format!(
                "@echo off\r\ntype nul > \"{}\"\r\necho {{\"translations\":[\"LOCAL\"],\"model\":\"fake-local\",\"backend\":\"opus-mt\"}}\r\n",
                local_marker.display()
            ),
        )
        .unwrap();
    #[cfg(not(windows))]
        std::fs::write(
            &fake_py,
            format!(
                "#!/usr/bin/env sh\ntouch '{}'\nprintf '%s\\n' '{{\"translations\":[\"LOCAL\"],\"model\":\"fake-local\",\"backend\":\"opus-mt\"}}'\n",
                local_marker.display()
            ),
        )
        .unwrap();
    make_executable(&fake_py);
    let fake_script = bin.join("translate_runner.py");
    std::fs::write(&fake_script, "# fake local runner marker\n").unwrap();

    let new_path = match old_path.clone() {
        Some(prev) => {
            let mut paths = vec![bin.clone()];
            paths.extend(std::env::split_paths(&prev));
            std::env::join_paths(paths).unwrap()
        }
        None => std::env::join_paths([bin.clone()]).unwrap(),
    };
    std::env::set_var("PATH", new_path);
    std::env::set_var("TRANSLATE_RUNNER_PY", &fake_py);
    std::env::set_var("TRANSLATE_RUNNER_SCRIPT", &fake_script);

    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(crate::translate::run_translation(
        None,
        Some("en"),
        "es",
        &["hello".to_string()],
        None,
        Some(10_000),
    ));

    match old_path {
        Some(path) => std::env::set_var("PATH", path),
        None => std::env::remove_var("PATH"),
    }
    match old_runner_py {
        Some(path) => std::env::set_var("TRANSLATE_RUNNER_PY", path),
        None => std::env::remove_var("TRANSLATE_RUNNER_PY"),
    }
    match old_runner_script {
        Some(path) => std::env::set_var("TRANSLATE_RUNNER_SCRIPT", path),
        None => std::env::remove_var("TRANSLATE_RUNNER_SCRIPT"),
    }

    let outcome = result.expect("auto mode should try codex after claude quota failure");
    assert_eq!(outcome.backend, "cli");
    assert_eq!(outcome.agent.as_deref(), Some("codex"));
    assert_eq!(outcome.translations, vec!["Hola".to_string()]);
    assert!(
        !local_marker.exists(),
        "local translator ran even though a second CLI agent succeeded"
    );
}

#[test]
fn adapter_python_resolution_never_uses_path_python_on_macos() {
    assert_eq!(
            adapter_python_for_platform(
                None,
                None,
                Some(PathBuf::from("/usr/bin/python3")),
                true,
            ),
            None,
            "clean macOS must not spawn bare/path python because it opens the Command Line Tools prompt"
        );
}

#[test]
fn adapter_python_resolution_prefers_explicit_and_managed_runtimes() {
    assert_eq!(
        adapter_python_for_platform(
            Some(PathBuf::from("/opt/shellx/python")),
            Some(PathBuf::from("/managed/python")),
            Some(PathBuf::from("/usr/bin/python3")),
            true,
        ),
        Some(PathBuf::from("/opt/shellx/python")),
        "explicit adapter Python wins"
    );
    assert_eq!(
        adapter_python_for_platform(
            None,
            Some(PathBuf::from("/managed/python")),
            Some(PathBuf::from("/usr/bin/python3")),
            true,
        ),
        Some(PathBuf::from("/managed/python")),
        "managed perception runtime is safe on macOS"
    );
}

#[test]
fn adapter_python_resolution_keeps_path_fallback_off_macos() {
    assert_eq!(
        adapter_python_for_platform(None, None, Some(PathBuf::from("/usr/bin/python3")), false),
        Some(PathBuf::from("/usr/bin/python3")),
        "Windows/Linux keep the existing PATH-python fallback"
    );
}

/// title.update: editing a placed title's text RE-RENDERS the overlay and
/// SWAPS the clip's asset IN PLACE (clip id/track kept), and the whole edit
/// REPLAYS deterministically (rebuild_from_log == live). The project and render
/// output are temp-contained, so this runs in the normal ffmpeg-backed suite.
#[tokio::test]
async fn title_update_swaps_asset_and_replays() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"r3","dir": dir.path().join("r3.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    // Place a title, then find the title clip + its original asset.
    let r = dispatch(
        &state,
        "title.add",
        json!({"text":"ORIG TITLE","range_ms":[0u64,3000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "title.add: {:?}", r.error);
    let title_clip = {
        let s = dispatch(&state, "project.state", json!({}), test_actor())
            .await
            .result
            .unwrap();
        let tt = s["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"].as_str().unwrap_or("").starts_with("title"))
            .expect("a title track exists");
        tt["clips"][0].clone()
    };
    let clip_id = title_clip["id"].as_str().unwrap().to_string();
    let asset_before = title_clip["asset"].as_str().unwrap().to_string();
    assert_eq!(
        title_clip["title_text"].as_str(),
        Some("ORIG TITLE"),
        "project.state should annotate the title clip with its text"
    );

    // Edit the text in place.
    let r = dispatch(
        &state,
        "title.update",
        json!({"clip": clip_id, "text":"NEW TITLE"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "title.update: {:?}", r.error);
    let asset_after = r.result.unwrap()["asset_id"].as_str().unwrap().to_string();
    assert_ne!(asset_before, asset_after, "asset should have been swapped");

    // The clip kept its id + track, points at the NEW asset, shows the NEW text.
    let live = {
        let guard = state.project.read().await;
        guard.as_ref().unwrap().project.clone()
    };
    let (tid, idx) = live.find_clip(&clip_id).expect("clip still present");
    assert!(tid.starts_with("title"), "clip stayed on its title track");
    match &live.track(tid).unwrap().clips[idx] {
        cut_core::Clip::Media(mc) => {
            assert_eq!(mc.asset, asset_after, "clip now references the new asset");
            assert_ne!(mc.asset, asset_before);
        }
        _ => unreachable!("title clip is not a media clip"),
    }

    // REPLAY DETERMINISM: rebuilding from the op-log reproduces live state
    // byte-for-byte (the new .mov persists; media.import re-registers it and the
    // lowered title.update re-runs edit.set_asset).
    let ops = {
        let guard = state.project.read().await;
        guard.as_ref().unwrap().log.read_all().unwrap()
    };
    let rebuilt = cut_core::rebuild_from_log(&ops).expect("replay");
    assert_eq!(
        rebuilt, live,
        "replayed state != live state after title.update"
    );

    // A non-title clip is rejected. Import a stub onto the base video track and
    // try to title.update it.
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a2","track":"v1","at_ms":0,"src_range_ms":[0u64,2000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "insert: {:?}", r.error);
    let media_clip = {
        let s = dispatch(&state, "project.state", json!({}), test_actor())
            .await
            .result
            .unwrap();
        let v1 = s["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == "v1")
            .unwrap();
        v1["clips"][0]["id"].as_str().unwrap().to_string()
    };
    let r = dispatch(
        &state,
        "title.update",
        json!({"clip": media_clip, "text":"nope"}),
        test_actor(),
    )
    .await;
    assert!(!r.ok, "title.update on a non-title clip must error");
    assert_eq!(r.error.as_ref().unwrap().code, error_codes::INVALID_ARGS);
}

/// shape.update: editing a placed shape's label/color RE-RENDERS the
/// overlay and SWAPS the clip's asset IN PLACE (clip id/track kept), the
/// project.state annotations track the edit, the whole edit REPLAYS
/// deterministically (rebuild_from_log == live), and a non-shape clip is
/// rejected. The project and render output are temp-contained, so this runs in
/// the normal ffmpeg-backed suite.
#[tokio::test]
async fn shape_update_swaps_asset_and_replays() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"r4","dir": dir.path().join("r4.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    // Place a labeled rect shape, then find the shape clip + its original asset.
    let r = dispatch(
        &state,
        "edit.add_shape",
        json!({"shape":"rect","fill":"#FF0000","text":"ORIG","range_ms":[0u64,3000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "edit.add_shape: {:?}", r.error);
    let shape_clip = {
        let s = dispatch(&state, "project.state", json!({}), test_actor())
            .await
            .result
            .unwrap();
        let tt = s["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"].as_str().unwrap_or("").starts_with("title"))
            .expect("a shape/title track exists");
        tt["clips"][0].clone()
    };
    let clip_id = shape_clip["id"].as_str().unwrap().to_string();
    let asset_before = shape_clip["asset"].as_str().unwrap().to_string();
    assert_eq!(
        shape_clip["shape_kind"].as_str(),
        Some("rect"),
        "project.state should annotate the shape clip with its kind"
    );
    assert_eq!(
        shape_clip["shape_label"].as_str(),
        Some("ORIG"),
        "project.state should annotate the shape clip with its label"
    );
    assert_eq!(
        shape_clip["shape_color"].as_str(),
        Some("#FF0000"),
        "project.state should annotate the shape clip with its color"
    );
    // A shape clip is NOT a title clip — it must NOT carry the title_text marker
    // (the two editors are routed by these mutually-exclusive markers).
    assert!(
        shape_clip["title_text"].is_null(),
        "a shape clip must not be annotated as a title"
    );

    // Edit the label + color in place.
    let r = dispatch(
        &state,
        "shape.update",
        json!({"clip": clip_id, "label":"NEW", "fill":"#00FF00"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "shape.update: {:?}", r.error);
    let res = r.result.unwrap();
    let asset_after = res["asset_id"].as_str().unwrap().to_string();
    assert_ne!(asset_before, asset_after, "asset should have been swapped");
    assert_eq!(res["label"].as_str(), Some("NEW"));

    // The clip kept its id + track, points at the NEW asset.
    let live = {
        let guard = state.project.read().await;
        guard.as_ref().unwrap().project.clone()
    };
    let (tid, idx) = live.find_clip(&clip_id).expect("clip still present");
    assert!(tid.starts_with("title"), "clip stayed on its shape track");
    match &live.track(tid).unwrap().clips[idx] {
        cut_core::Clip::Media(mc) => {
            assert_eq!(mc.asset, asset_after, "clip now references the new asset");
            assert_ne!(mc.asset, asset_before);
        }
        _ => unreachable!("shape clip is not a media clip"),
    }

    // project.state now reflects the edited label + color.
    let edited = {
        let s = dispatch(&state, "project.state", json!({}), test_actor())
            .await
            .result
            .unwrap();
        s["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"].as_str().unwrap_or("").starts_with("title"))
            .unwrap()["clips"][0]
            .clone()
    };
    assert_eq!(edited["shape_label"].as_str(), Some("NEW"));
    assert_eq!(edited["shape_color"].as_str(), Some("#00FF00"));

    // REPLAY DETERMINISM: rebuilding from the op-log reproduces live state
    // (the new .mov persists; media.import re-registers it and the lowered
    // shape.update re-runs edit.set_asset).
    let ops = {
        let guard = state.project.read().await;
        guard.as_ref().unwrap().log.read_all().unwrap()
    };
    let rebuilt = cut_core::rebuild_from_log(&ops).expect("replay");
    assert_eq!(
        rebuilt, live,
        "replayed state != live state after shape.update"
    );

    // A non-shape clip is rejected. Import a stub onto the base video track and
    // try to shape.update it.
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a2","track":"v1","at_ms":0,"src_range_ms":[0u64,2000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "insert: {:?}", r.error);
    let media_clip = {
        let s = dispatch(&state, "project.state", json!({}), test_actor())
            .await
            .result
            .unwrap();
        let v1 = s["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == "v1")
            .unwrap();
        v1["clips"][0]["id"].as_str().unwrap().to_string()
    };
    let r = dispatch(
        &state,
        "shape.update",
        json!({"clip": media_clip, "label":"nope"}),
        test_actor(),
    )
    .await;
    assert!(!r.ok, "shape.update on a non-shape clip must error");
    assert_eq!(r.error.as_ref().unwrap().code, error_codes::INVALID_ARGS);
}

/// audio.cleanup_voice (orchestrator): on a project with one audio clip, the
/// macro applies eq:voice + the [denoise, gate, compressor] chain (in order)
/// under ONE auto-checkpoint, returns a composite receipt, and the whole pass
/// reverts in a single step. Proves the agent-first "one-step-revertible voice
/// chain" contract with NO new core/replay arm (the sub-ops carry the state).
#[tokio::test]
async fn cleanup_voice_applies_chain_and_reverts_as_one_unit() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // Seed an audio clip — a stub file is fine: cleanup_voice / edit.eq /
    // edit.effect validate by TRACK kind (audio), not by probing the stream.
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0,"src_range_ms":[0u64,4000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "insert: {:?}", r.error);

    // No scope → every audio clip; default strength = medium.
    let r = dispatch(&state, "audio.cleanup_voice", json!({}), test_actor()).await;
    assert!(r.ok, "cleanup_voice: {:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["status"], json!("cleaned"));
    assert_eq!(res["strength"], json!("medium"));
    assert_eq!(
        res["clips"].as_array().unwrap().len(),
        1,
        "the one audio clip"
    );
    assert_eq!(res["loudness_hint"]["recommended_lufs"], json!(-16));
    let checkpoint = res["checkpoint"].as_str().unwrap().to_string();
    assert!(!checkpoint.is_empty(), "auto-checkpoint recorded");

    // The audio clip now carries eq (voice preset) + the 3-effect chain, in order.
    {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let m = store
            .project
            .tracks
            .iter()
            .find(|t| t.kind == cut_core::TrackKind::Audio)
            .unwrap()
            .clips
            .iter()
            .find_map(|c| {
                if let cut_core::Clip::Media(m) = c {
                    Some(m)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(m.eq.is_some(), "eq:voice applied");
        let kinds: Vec<&str> = m.effects.iter().map(|e| e.kind()).collect();
        assert_eq!(
            kinds,
            vec!["denoise", "gate", "compressor"],
            "the cleanup chain, in render order"
        );
    }

    // One-step revert: project.revert{to:checkpoint} undoes the WHOLE pass.
    let r = dispatch(
        &state,
        "project.revert",
        json!({"to": checkpoint}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "revert: {:?}", r.error);
    {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let m = store
            .project
            .tracks
            .iter()
            .find(|t| t.kind == cut_core::TrackKind::Audio)
            .unwrap()
            .clips
            .iter()
            .find_map(|c| {
                if let cut_core::Clip::Media(m) = c {
                    Some(m)
                } else {
                    None
                }
            })
            .unwrap();
        assert!(m.eq.is_none(), "eq cleared by the one-step revert");
        assert!(
            m.effects.is_empty(),
            "effects cleared by the one-step revert"
        );
    }

    // An unknown strength is a structured error (fails fast, before any apply).
    let r = dispatch(
        &state,
        "audio.cleanup_voice",
        json!({"strength":"nuclear"}),
        test_actor(),
    )
    .await;
    assert!(!r.ok, "unknown strength rejected");
}

/// export.audio{track} per-track STEM (v2b): the track filter validates BEFORE
/// any render — a non-existent track and a VIDEO track are both rejected with a
/// clear error (the success path, where stems sum to the full mix bit-for-bit,
/// is proven live with a real ffmpeg render). Guards the project-view filter.
#[tokio::test]
async fn export_audio_track_stem_validates_target() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // Seed a clip so the timeline isn't empty (else the empty-timeline guard
    // fires first and we wouldn't reach the track validation).
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0,"src_range_ms":[0u64,4000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "insert: {:?}", r.error);

    // A non-existent track id is rejected (before any render).
    let r = dispatch(
        &state,
        "export.audio",
        json!({"track":"nope"}),
        test_actor(),
    )
    .await;
    assert!(!r.ok, "unknown track rejected");
    assert_eq!(r.error.as_ref().unwrap().code, error_codes::NOT_FOUND);

    // A VIDEO track (v1) is rejected — stems are audio-track only.
    let r = dispatch(&state, "export.audio", json!({"track":"v1"}), test_actor()).await;
    assert!(!r.ok, "video track rejected as a stem target");
    assert_eq!(r.error.as_ref().unwrap().code, error_codes::NOT_FOUND);
}

/// edit.slide preset resolution → position keyframes: off-screen = -scale
/// (left/top) or 1.0 (right/bottom), rest = the transform x/y; 'in' at the
/// start, 'out' at the end; slide span clamps to the clip length.
#[test]
fn resolve_slide_builds_position_keyframes() {
    let t = cut_core::ClipTransform {
        x: 0.6,
        y: 0.2,
        scale: 0.3,
        opacity: 1.0,
    };
    // slide IN from the left → pos_x from off-screen (-scale) to rest (x).
    let (param, pts) = resolve_slide(&t, 2000, "left", "in", 500).unwrap();
    assert_eq!(param, "pos_x");
    assert_eq!(pts[0]["t_ms"].as_u64(), Some(0));
    assert_eq!(pts[0]["value"].as_f64(), Some(-0.3));
    assert_eq!(pts[1]["t_ms"].as_u64(), Some(500));
    assert_eq!(pts[1]["value"].as_f64(), Some(0.6));
    // slide OUT to the right → pos_x rest → 1.0 over the LAST slide_ms.
    let (param, pts) = resolve_slide(&t, 2000, "right", "out", 500).unwrap();
    assert_eq!(param, "pos_x");
    assert_eq!(pts[0]["t_ms"].as_u64(), Some(1500));
    assert_eq!(pts[0]["value"].as_f64(), Some(0.6));
    assert_eq!(pts[1]["value"].as_f64(), Some(1.0));
    // top/bottom animate pos_y; slide span clamps to the clip length.
    assert_eq!(
        resolve_slide(&t, 2000, "top", "in", 500).unwrap().0,
        "pos_y"
    );
    let (_, pts) = resolve_slide(&t, 800, "left", "in", 5000).unwrap();
    assert_eq!(pts[1]["t_ms"].as_u64(), Some(800));
    // unknown edge / mode → error.
    assert!(resolve_slide(&t, 2000, "diagonal", "in", 500).is_err());
    assert!(resolve_slide(&t, 2000, "left", "sideways", 500).is_err());
}

/// reflow_cues: SPLIT over-length cues at word boundaries, EXTEND too-fast
/// cues into the following gap; a too-fast cue with no gap stays flagged.
#[test]
fn reflow_splits_and_extends() {
    let cue = |id: &str, text: &str, a: u64, b: u64| cut_core::CaptionClip {
        id: id.into(),
        text: text.into(),
        style_ref: None,
        range_ms: [a, b],
    };
    let opts = ReflowOpts {
        max_cps: 17.0,
        max_chars: 84,
        max_duration_ms: 7000,
        min_gap_ms: 80,
    };

    // (1) Over-length cue (12 eight-char words = 107 chars > 84) → split,
    // sub-cues each ≤ 84 chars, contiguous within the original span. Span is
    // 7000ms so 107 chars = 15.3 cps is NOT too-fast (isolates split from the
    // extend pass).
    let long = "aaaaaaaa ".repeat(12).trim_end().to_string();
    let (out, stats) = reflow_cues(&[cue("c1", &long, 0, 7000)], opts);
    assert!(out.len() >= 2, "long cue split: {stats}");
    assert_eq!(stats["split"], 1);
    for c in &out {
        assert!(
            c.text.chars().count() <= 84,
            "chunk within max_chars: {:?}",
            c.text
        );
    }
    assert_eq!(out.first().unwrap().range_ms[0], 0);
    assert_eq!(out.last().unwrap().range_ms[1], 7000, "span preserved");

    // (2) Too-fast cue (25 chars over 800ms = 31 cps) with a big gap after →
    // extend its end to hit ~17 cps; not still_too_fast.
    let fast = cue("c1", "the quick brown fox jumps", 0, 800);
    let nextc = cue("c2", "later", 5000, 5500);
    let (out, stats) = reflow_cues(&[fast.clone(), nextc.clone()], opts);
    assert_eq!(stats["extended"], 1, "extended into gap: {stats}");
    assert_eq!(stats["still_too_fast"], 0, "{stats}");
    let new_end = out
        .iter()
        .find(|c| c.text.starts_with("the quick"))
        .unwrap()
        .range_ms[1];
    assert!(
        new_end > 800 && new_end <= 5500 - 80,
        "end extended within the gap: {new_end}"
    );

    // (3) Same too-fast cue but the next cue is immediately after (no usable
    // gap) → cannot reach the target, stays flagged still_too_fast.
    let crowd = cue("c2", "later", 900, 1400);
    let (_out, stats) = reflow_cues(&[fast, crowd], opts);
    assert_eq!(stats["still_too_fast"], 1, "no gap to fix CPS: {stats}");

    // (4) Already-compliant cues pass through unchanged in count.
    let ok = cue("c1", "Hello there", 0, 2000);
    let (out, stats) = reflow_cues(&[ok], opts);
    assert_eq!(out.len(), 1);
    assert_eq!(stats["split"], 0);
    assert_eq!(stats["extended"], 0);

    // (5) A very short cue that would split into sub-frame slivers is kept
    // intact instead of producing zero/near-zero-duration caption clips.
    let too_short = cue("c1", "aa bb cc dd ee ff gg", 0, 30);
    let tight_opts = ReflowOpts {
        max_cps: 1000.0,
        max_chars: 5,
        max_duration_ms: 7000,
        min_gap_ms: 80,
    };
    let (out, stats) = reflow_cues(&[too_short], tight_opts);
    assert_eq!(out.len(), 1, "short split should be refused: {stats}");
    assert_eq!(stats["split_refused_short"], 1);
    assert_eq!(out[0].range_ms, [0, 30]);
}

/// dims_from_aspect maps social aspect strings to 1080-baseline even dims,
/// and rejects malformed ratios (render.final reframe).
#[test]
fn dims_from_aspect_maps_social_ratios() {
    assert_eq!(dims_from_aspect("9:16").unwrap(), (1080, 1920));
    assert_eq!(dims_from_aspect("16:9").unwrap(), (1920, 1080));
    assert_eq!(dims_from_aspect("1:1").unwrap(), (1080, 1080));
    assert_eq!(dims_from_aspect("4:5").unwrap(), (1080, 1350));
    assert_eq!(dims_from_aspect("5:4").unwrap(), (1350, 1080));
    // All dims are even (yuv420 chroma).
    for (w, h) in ["9:16", "16:9", "4:5", "2:3", "3:4"].map(|s| dims_from_aspect(s).unwrap()) {
        assert_eq!(w % 2, 0, "{w} even");
        assert_eq!(h % 2, 0, "{h} even");
    }
    for bad in ["", "16", "16:0", "0:9", "a:b", "9:16:1", "200:1"] {
        assert!(dims_from_aspect(bad).is_err(), "should reject '{bad}'");
    }
}

#[test]
fn degraded_output_checks_are_unmeasured_not_content_failures() {
    let mut checks = vec![
        cut_core::CheckResult {
            name: cut_core::check_names::UNIFORM_BORDER.into(),
            pass: false,
            details: json!({"error": "no content bbox"}),
            evidence: json!({"content_bbox": null}),
        },
        cut_core::CheckResult {
            name: cut_core::check_names::DURATION_MATCHES_EDL.into(),
            pass: true,
            details: json!({"tolerance_ms": 34}),
            evidence: json!({"delta_ms": 0}),
        },
    ];
    let error = CutError::new(
        error_codes::SIDECAR,
        "perception sidecar failed",
        "ModuleNotFoundError: No module named 'scenedetect'",
    )
    .with_suggested_action("repair the optional perception extras");

    mark_output_checks_unmeasured(&mut checks, &error);

    assert_eq!(checks[0].details["status"], "unmeasured");
    assert_eq!(checks[0].details["measured"], false);
    assert_eq!(
        checks[0].details["runtime_error"]["cause"],
        "ModuleNotFoundError: No module named 'scenedetect'"
    );
    assert!(!checks[0].pass);
    assert!(checks[1].pass, "structural checks remain measured");
    assert!(
        cut_core::fix_action(&checks[0]).is_none(),
        "unmeasured checks cannot feed autopilot repairs"
    );
}

/// Unknown verbs are rejected with an actionable error, not a panic.
#[tokio::test]
async fn unknown_verb_rejected() {
    let state = AppState::new();
    let r = dispatch(&state, "nope.nothing", json!({}), test_actor()).await;
    assert!(!r.ok);
    assert_eq!(r.error.unwrap().code, "not_found");
}

/// Every registry verb has a dispatch arm (the anti-drift tripwire).
#[tokio::test]
async fn every_verb_has_an_arm() {
    let state = AppState::new();
    let names: Vec<String> = state
        .registry
        .verbs
        .iter()
        .map(|v| v.name.clone())
        .collect();
    for name in names {
        // This tripwire checks dispatch coverage, not the OS capture backend.
        // Valid empty args would launch a real portal capture on headless Linux.
        let args = if name == "debug.screenshot" {
            json!({"monitor":"structural-test"})
        } else {
            json!({})
        };
        let r = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            dispatch(&state, &name, args, test_actor()),
        )
        .await
        .unwrap_or_else(|_| panic!("verb {name} did not return within 30 seconds"));
        if let Some(e) = &r.error {
            assert!(
                !e.message.contains("no dispatch arm"),
                "verb {name} missing from dispatch match"
            );
        }
    }
}

/// project.create → state → ops → close round-trip works.
#[tokio::test]
async fn project_lifecycle_live() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let mut events = state.events.subscribe();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"demo","dir": dir.path().join("demo.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "create failed: {:?}", r.error);
    assert!(matches!(
        events.try_recv(),
        Ok(crate::events::Event::ProjectChanged {
            open: true,
            name: Some(name),
        }) if name == "demo"
    ));
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"demo2","dir": dir.path().join("other.cutproj")}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    let r = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(r.ok);
    assert_eq!(r.result.unwrap()["schema"], "shellx-cut/1");
    let r = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert!(r.ok);
    let r = dispatch(&state, "project.close", json!({}), test_actor()).await;
    assert!(r.ok);
    assert!(matches!(
        events.try_recv(),
        Ok(crate::events::Event::ProjectChanged {
            open: false,
            name: Some(name),
        }) if name == "demo"
    ));
    let r = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert_eq!(r.error.unwrap().code, "no_project");
}

#[tokio::test]
async fn project_close_restores_the_open_project_while_jobs_are_still_draining() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let old_path = dir.path().join("close-drain.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"close-drain","dir":old_path}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "create failed: {:?}", created.error);

    let job = state.jobs.create("enrich");
    let worker_started = std::sync::Arc::new(tokio::sync::Notify::new());
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let started = worker_started.clone();
    state.jobs.spawn(&job.job_id, async move {
        let _ = run_blocking("test.project_close_drain", move || {
            started.notify_one();
            release_rx.recv().expect("test releases blocking worker");
            Ok(())
        })
        .await;
    });
    worker_started.notified().await;

    let close = dispatch(&state, "project.close", json!({}), test_actor()).await;
    assert_eq!(
        close.error.as_ref().map(|error| error.code.as_str()),
        Some("job_cancel_pending")
    );
    let still_open = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(still_open.ok, "close timeout must restore the project");
    assert_eq!(still_open.result.unwrap()["name"], "close-drain");

    release_tx.send(()).unwrap();
    tokio::task::yield_now().await;
    let retried = dispatch(&state, "project.close", json!({}), test_actor()).await;
    assert!(retried.ok, "close retry failed: {:?}", retried.error);
}

#[tokio::test]
async fn project_create_rolls_back_directory_and_index_when_job_drain_times_out() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let old_path = dir.path().join("old-project.cutproj");
    let next_path = dir.path().join("new-project.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"old-project","dir":old_path}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "create failed: {:?}", created.error);

    let job = state.jobs.create("enrich");
    let worker_started = std::sync::Arc::new(tokio::sync::Notify::new());
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let started = worker_started.clone();
    state.jobs.spawn(&job.job_id, async move {
        let _ = run_blocking("test.project_create_drain", move || {
            started.notify_one();
            release_rx.recv().expect("test releases blocking worker");
            Ok(())
        })
        .await;
    });
    worker_started.notified().await;

    let replacement = dispatch(
        &state,
        "project.create",
        json!({"name":"new-project","dir":next_path}),
        test_actor(),
    )
    .await;
    assert_eq!(
        replacement.error.as_ref().map(|error| error.code.as_str()),
        Some("job_cancel_pending")
    );
    let still_open = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(still_open.ok, "failed replacement must restore old project");
    assert_eq!(still_open.result.unwrap()["name"], "old-project");
    assert!(
        !next_path.exists(),
        "failed replacement must remove the unactivated project"
    );
    assert!(
        crate::projects_index::path_for(next_path.to_string_lossy().as_ref()).is_none(),
        "failed replacement must not leave a recent-project ghost"
    );

    release_tx.send(()).unwrap();
    tokio::task::yield_now().await;
    let closed = dispatch(&state, "project.close", json!({}), test_actor()).await;
    assert!(closed.ok, "cleanup close failed: {:?}", closed.error);
}

#[tokio::test]
async fn project_revert_if_tip_refuses_to_remove_newer_work() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"guarded-revert","dir":dir.path().join("guarded-revert.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "create failed: {:?}", created.error);

    let baseline = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms":100,"label":"baseline"}),
        test_actor(),
    )
    .await
    .op_ids
    .unwrap()[0]
        .clone();
    let reviewed_tip = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms":200,"label":"reviewed turn"}),
        test_actor(),
    )
    .await
    .op_ids
    .unwrap()[0]
        .clone();
    let newer_tip = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms":300,"label":"newer human work"}),
        test_actor(),
    )
    .await
    .op_ids
    .unwrap()[0]
        .clone();

    let stale = dispatch(
        &state,
        "project.revert",
        json!({"to":baseline,"if_tip":reviewed_tip}),
        test_actor(),
    )
    .await;
    assert_eq!(stale.error.unwrap().code, error_codes::CONFLICT);
    let unchanged = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(unchanged["markers"].as_array().unwrap().len(), 3);

    let current = dispatch(
        &state,
        "project.revert",
        json!({"to":baseline,"if_tip":newer_tip}),
        test_actor(),
    )
    .await;
    assert!(current.ok, "guarded revert failed: {:?}", current.error);
    let reverted = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(reverted["markers"].as_array().unwrap().len(), 1);
    assert_eq!(reverted["markers"][0]["label"], "baseline");
}

#[tokio::test]
async fn project_sequences_are_independent_and_reopen_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sequences.cutproj");
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"sequences","dir":path}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "create failed: {:?}", created.error);
    assert!(
        dispatch(
            &state,
            "edit.add_marker",
            json!({"at_ms":100,"label":"main"}),
            test_actor(),
        )
        .await
        .ok
    );

    let created_sequence = dispatch(
        &state,
        "project.sequence_create",
        json!({"name":"Review","from":"empty"}),
        test_actor(),
    )
    .await;
    assert!(
        created_sequence.ok,
        "sequence create: {:?}",
        created_sequence.error
    );
    assert_eq!(created_sequence.result.unwrap()["active_sequence"], "seq2");
    assert!(
        dispatch(
            &state,
            "edit.add_marker",
            json!({"at_ms":200,"label":"review"}),
            test_actor(),
        )
        .await
        .ok
    );
    let listed = dispatch(&state, "project.sequence_list", json!({}), test_actor()).await;
    assert!(listed.ok);
    assert_eq!(
        listed.result.unwrap()["sequences"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let switched = dispatch(
        &state,
        "project.sequence_switch",
        json!({"id":"seq1"}),
        test_actor(),
    )
    .await;
    assert!(switched.ok, "switch failed: {:?}", switched.error);
    let main_state = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert_eq!(
        main_state.result.as_ref().unwrap()["markers"][0]["label"],
        "main"
    );

    assert!(
        dispatch(&state, "project.close", json!({}), test_actor())
            .await
            .ok
    );
    let reopened = dispatch(&state, "project.open", json!({"path":path}), test_actor()).await;
    assert!(reopened.ok, "reopen failed: {:?}", reopened.error);
    let review = dispatch(
        &state,
        "project.sequence_switch",
        json!({"id":"seq2"}),
        test_actor(),
    )
    .await;
    assert!(review.ok, "review switch failed: {:?}", review.error);
    let review_state = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert_eq!(
        review_state.result.as_ref().unwrap()["markers"][0]["label"],
        "review"
    );
}

#[tokio::test]
async fn project_create_installs_only_the_named_bundled_starter() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("starter.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"starter","dir":project_dir,"starter":"first-edit"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "starter create failed: {:?}", r.error);
    let result = r.result.expect("starter create result");
    let path = PathBuf::from(result["starter_asset_path"].as_str().expect("starter path"));
    assert!(path.starts_with(project_dir.canonicalize().unwrap()));
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("first-edit-sample.mp4")
    );
    let bytes = std::fs::read(&path).unwrap();
    assert!(
        bytes.len() > 10_000,
        "bundled sample should contain real media"
    );
    assert_eq!(&bytes[4..8], b"ftyp", "bundled sample should be an MP4");
    let report = cut_perception::load_report(&project_dir.join("receipts"), "a1")
        .unwrap()
        .expect("starter perception receipt");
    assert_eq!(report.asset_hash, cut_core::hash_file(&path).unwrap());
    assert_eq!(report.words.as_ref().unwrap().words.len(), 29);
    assert_eq!(report.silences.len(), 2);
    for instrument in ["words", "silence", "scenes", "beats", "loudness"] {
        assert!(
            report.instruments_run.iter().any(|item| item == instrument),
            "starter receipt must cover {instrument}"
        );
    }

    let state = AppState::new();
    let invalid_dir = dir.path().join("invalid.cutproj");
    let invalid = dispatch(
        &state,
        "project.create",
        json!({"name":"invalid","dir":invalid_dir,"starter":"unknown"}),
        test_actor(),
    )
    .await;
    assert_eq!(invalid.error.unwrap().code, error_codes::INVALID_ARGS);
    assert!(
        !invalid_dir.exists(),
        "schema rejection must precede project creation"
    );
}

#[tokio::test]
async fn project_create_rejects_path_like_names_before_joining_parent() {
    let _guard = lock_agent_cli_env();
    let old_home = std::env::var_os("HOME");
    let old_userprofile = std::env::var_os("USERPROFILE");
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    std::env::set_var("USERPROFILE", &home);

    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"../escape"}),
        test_actor(),
    )
    .await;

    match old_home {
        Some(path) => std::env::set_var("HOME", path),
        None => std::env::remove_var("HOME"),
    }
    match old_userprofile {
        Some(path) => std::env::set_var("USERPROFILE", path),
        None => std::env::remove_var("USERPROFILE"),
    }

    assert!(!r.ok, "path-like project name must be rejected");
    assert_eq!(r.error.as_ref().unwrap().code, error_codes::INVALID_ARGS);
    assert!(
        !home.join("escape.cutproj").exists(),
        "project name traversal must not create outside the managed projects dir"
    );
}

#[tokio::test]
async fn project_create_rejects_unknown_nested_settings_keys() {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"demo","settings":{"width":1280,"height":720,"fps":30,"bogus":true}}),
        test_actor(),
    )
    .await;
    let e = r
        .error
        .expect("schema guard must reject unknown nested settings");
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(
        e.message.contains("settings.bogus"),
        "nested unknown argument should be named: {e:?}"
    );
}

#[tokio::test]
async fn project_forget_rejects_both_id_and_path() {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.forget",
        json!({"id":"p1","path":"/tmp/p1.cutproj"}),
        test_actor(),
    )
    .await;
    let e = r
        .error
        .expect("project.forget must require exactly one selector");
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(e.message.contains("exactly one"), "{e:?}");
}

#[tokio::test]
async fn project_forget_rejects_missing_combined_with_id() {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.forget",
        json!({"id":"p1","missing":true}),
        test_actor(),
    )
    .await;
    let e = r
        .error
        .expect("missing:true must be mutually exclusive with id/path");
    assert_eq!(e.code, error_codes::INVALID_ARGS);
}

#[tokio::test]
async fn project_forget_missing_sweeps_dead_entries_and_keeps_live_ones() {
    // Two registered projects: one stays on disk, the other's directory is
    // removed out-of-band (the crashed-test/cleanup-script leak this mode
    // exists for). missing:true must drop ONLY the dead entry — the check
    // is a per-entry stat at call time, never the persisted missing flag.
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let alive_dir = dir.path().join("fm_alive.cutproj");
    let dead_dir = dir.path().join("fm_dead.cutproj");
    for (name, d) in [("fm_alive", &alive_dir), ("fm_dead", &dead_dir)] {
        let r = dispatch(
            &state,
            "project.create",
            json!({"name": name, "dir": d}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "{:?}", r.error);
    }
    // close so the dead dir can be removed and neither is held open
    let r = dispatch(&state, "project.close", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    std::fs::remove_dir_all(&dead_dir).unwrap();

    let r = dispatch(
        &state,
        "project.forget",
        json!({"missing": true}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert!(
        res["removed"].as_u64().unwrap_or(0) >= 1,
        "at least the dead entry must be swept: {res}"
    );

    let r = dispatch(&state, "project.list", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let names: Vec<String> = r.result.unwrap()["projects"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p["name"].as_str().map(String::from))
        .collect();
    assert!(
        names.iter().any(|n| n == "fm_alive"),
        "the live project must SURVIVE the sweep: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "fm_dead"),
        "the dead entry must be gone: {names:?}"
    );
}

#[tokio::test]
async fn project_delete_rejects_both_id_and_path() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("victim.cutproj");
    std::fs::create_dir_all(&project_dir).unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.delete",
        json!({"id":"p1","path":project_dir}),
        test_actor(),
    )
    .await;
    let e = r
        .error
        .expect("project.delete must require exactly one selector");
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(e.message.contains("exactly one"), "{e:?}");
    assert!(
        project_dir.exists(),
        "invalid args must not delete the path"
    );
}

#[tokio::test]
async fn library_add_rejects_both_path_and_asset() {
    let _guard = lock_agent_cli_env();
    let old_home = std::env::var_os("HOME");
    let old_userprofile = std::env::var_os("USERPROFILE");
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    std::env::set_var("USERPROFILE", &home);
    let media = dir.path().join("tone.wav");
    let ffmpeg = std::env::var("SHELLX_CUT_FFMPEG")
        .or_else(|_| std::env::var("FFMPEG_BIN"))
        .unwrap_or_else(|_| "ffmpeg".to_string());
    let ff = std::process::Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.2:sample_rate=44100",
            "-ac",
            "1",
        ])
        .arg(&media)
        .status();
    if !ff.map(|s| s.success()).unwrap_or(false) {
        match old_home {
            Some(path) => std::env::set_var("HOME", path),
            None => std::env::remove_var("HOME"),
        }
        match old_userprofile {
            Some(path) => std::env::set_var("USERPROFILE", path),
            None => std::env::remove_var("USERPROFILE"),
        }
        eprintln!("ffmpeg unavailable — skipping library.add selector test");
        return;
    }

    let state = AppState::new();
    let r = dispatch(
        &state,
        "library.add",
        json!({"path":media,"asset":"a1"}),
        test_actor(),
    )
    .await;

    match old_home {
        Some(path) => std::env::set_var("HOME", path),
        None => std::env::remove_var("HOME"),
    }
    match old_userprofile {
        Some(path) => std::env::set_var("USERPROFILE", path),
        None => std::env::remove_var("USERPROFILE"),
    }

    let e = r
        .error
        .expect("library.add must require exactly one source");
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(e.message.contains("exactly one"), "{e:?}");
}

#[tokio::test]
async fn library_add_rejects_unknown_source_enum() {
    let _guard = lock_agent_cli_env();
    let old_home = std::env::var_os("HOME");
    let old_userprofile = std::env::var_os("USERPROFILE");
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    std::env::set_var("USERPROFILE", &home);
    let media = dir.path().join("tone.wav");
    let ffmpeg = std::env::var("SHELLX_CUT_FFMPEG")
        .or_else(|_| std::env::var("FFMPEG_BIN"))
        .unwrap_or_else(|_| "ffmpeg".to_string());
    let ff = std::process::Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.2:sample_rate=44100",
            "-ac",
            "1",
        ])
        .arg(&media)
        .status();
    if !ff.map(|s| s.success()).unwrap_or(false) {
        match old_home {
            Some(path) => std::env::set_var("HOME", path),
            None => std::env::remove_var("HOME"),
        }
        match old_userprofile {
            Some(path) => std::env::set_var("USERPROFILE", path),
            None => std::env::remove_var("USERPROFILE"),
        }
        eprintln!("ffmpeg unavailable — skipping library.add source enum test");
        return;
    }

    let state = AppState::new();
    let r = dispatch(
        &state,
        "library.add",
        json!({"path":media,"source":"plugin"}),
        test_actor(),
    )
    .await;

    match old_home {
        Some(path) => std::env::set_var("HOME", path),
        None => std::env::remove_var("HOME"),
    }
    match old_userprofile {
        Some(path) => std::env::set_var("USERPROFILE", path),
        None => std::env::remove_var("USERPROFILE"),
    }

    let e = r.error.expect("library.add must validate source");
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(e.message.contains("source"), "{e:?}");
}

/// media.remove refuses while clips
/// still reference the asset (SAFE default), then drops the record while
/// KEEPING the source file, and is REPLAY-SAFE — a removed asset never
/// resurrects when the log (which still holds media.import) is rebuilt.
#[tokio::test]
async fn media_remove_safe_and_replay_safe() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await; // asset a1, used by 2 clips
    let source = dir.path().join("clip.mp4");
    assert!(source.exists(), "fixture wrote the source media file");

    // 1) Refuses while a1 is referenced (the SAFE default) — names the count.
    let r = dispatch(&state, "media.remove", json!({"asset":"a1"}), test_actor()).await;
    let e = r.error.expect("must refuse while in use");
    assert_eq!(e.code, "conflict");
    assert!(
        e.message.contains('2'),
        "names the clip count: {}",
        e.message
    );
    // Unknown asset id → NOT_FOUND.
    let r = dispatch(&state, "media.remove", json!({"asset":"zzz"}), test_actor()).await;
    assert_eq!(r.error.unwrap().code, "not_found");

    // 2) Clear the clips (the timeline delete is the undoable step), then the
    //    asset is unused and media.remove succeeds.
    let r = dispatch(
        &state,
        "edit.ripple_delete",
        json!({"range_ms":[0, 100000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "ripple_delete: {:?}", r.error);
    let r = dispatch(
        &state,
        "media.remove",
        json!({"asset":"a1","rationale":"test: delete file"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "remove should succeed once unused: {:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["removed"], json!(true));
    assert_eq!(res["asset"], json!("a1"));
    // The SOURCE file is KEPT on disk — media.remove NEVER deletes source media.
    assert!(source.exists(), "source media must NOT be deleted");
    assert!(
        res["source_kept"].as_str().unwrap().ends_with("clip.mp4"),
        "source_kept points at the kept source: {:?}",
        res["source_kept"]
    );

    // 3) Gone from project state; a second remove is NOT_FOUND (idempotent-ish).
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert!(
        !project.assets.contains_key("a1"),
        "asset dropped from the project"
    );
    let r = dispatch(&state, "media.remove", json!({"asset":"a1"}), test_actor()).await;
    assert_eq!(r.error.unwrap().code, "not_found");

    // 4) REPLAY-SAFE: the log holds media.import AND media.remove — rebuilding
    //    from it must net to ABSENT (the whole point of recording the op).
    let ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let log: Vec<cut_core::OpRecord> =
        serde_json::from_value(ops.result.unwrap()["ops"].clone()).unwrap();
    let rebuilt = cut_core::rebuild_from_log(&log).expect("replay");
    assert!(
        !rebuilt.assets.contains_key("a1"),
        "replay must not resurrect a removed asset"
    );
}

/// A verb whose core dependency is still todo!() returns a STRUCTURED
/// unimplemented error — the server survives (build rule from the brief).
#[tokio::test]
async fn todo_dependency_is_structured_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok);
    let r = dispatch(
        &state,
        "edit.split",
        json!({"track":"v1","at_ms":100}),
        test_actor(),
    )
    .await;
    // Either core has landed (ok) or we get the structured error — never a panic.
    if !r.ok {
        let e = r.error.unwrap();
        assert!(
            e.code == "unimplemented" || e.code == "invalid_args" || e.code == "not_found",
            "unexpected error shape: {e:?}"
        );
    }
    // Dispatcher still alive afterwards.
    let r = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(r.ok);
}

/// media.import appends an op (the append-only operation-log contract), registers the asset, and
/// returns a chain job id — even when the chain later fails on stubs.
#[tokio::test]
async fn media_import_is_an_op() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let r = dispatch(
        &state,
        "media.import",
        json!({"path": media, "rationale":"test import"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let result = r.result.unwrap();
    assert_eq!(result["asset_id"], "a1"); // verbs.json: {asset_id, job_id}
    assert!(result["job_id"].as_str().unwrap().starts_with("job_"));
    assert_eq!(r.op_ids.unwrap().len(), 1);
    // Op is in the log with the rationale preserved (the rationale-preservation contract). Core makes
    // project.create itself op_000001 (the append-only operation-log contract — create/import/checkpoint
    // are ops, store.rs), so the log holds [project.create, media.import].
    let ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let ops = ops.result.unwrap()["ops"].as_array().unwrap().clone();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0]["verb"], "project.create");
    assert_eq!(ops[1]["verb"], "media.import");
    assert_eq!(ops[1]["rationale"], "test import");
    // Asset visible in state.
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(s.result.unwrap()["assets"]["a1"].is_object());
}

#[tokio::test]
async fn media_index_status_reports_persisted_visual_search_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("indexed.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"indexed","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let import = dispatch(
        &state,
        "media.import",
        json!({"path": media, "rationale":"index status import"}),
        test_actor(),
    )
    .await;
    assert!(import.ok, "{:?}", import.error);

    crate::vissearch::save_index(
        &project_dir,
        &crate::vissearch::EmbeddingIndex {
            schema: "shellx-cut/vissearch/1".to_string(),
            model: "unit-test-model".to_string(),
            dim: 2,
            asset: "a1".to_string(),
            frames: vec![
                crate::vissearch::FrameEmbedding {
                    ms: 0,
                    v: vec![1.0, 0.0],
                },
                crate::vissearch::FrameEmbedding {
                    ms: 1000,
                    v: vec![0.0, 1.0],
                },
            ],
        },
    )
    .expect("seed visual-search index");

    let r = dispatch(&state, "media.index_status", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let result = r.result.unwrap();
    assert_eq!(result["count"], 1);
    assert_eq!(result["assets"][0]["asset"], "a1");
    assert_eq!(result["assets"][0]["indexed_frames"], 2);
    assert_eq!(result["assets"][0]["dim"], 2);
    assert_eq!(result["assets"][0]["model"], "unit-test-model");
}

#[tokio::test]
async fn media_waveform_requires_probe_duration_before_decode() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let media = dir.path().join("clip.wav");
    std::fs::write(&media, b"not-really-audio").unwrap();
    let r = dispatch(
        &state,
        "media.import",
        json!({"path": media, "proxy": false}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let r = dispatch(
        &state,
        "media.waveform",
        json!({"asset":"a1","buckets":50}),
        test_actor(),
    )
    .await;
    let e = r.error.expect("waveform must wait for probe readiness");
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(
        e.message.contains("not probed"),
        "error should name probe readiness: {e:?}"
    );
}

/// Still-image server regression: a real PNG imports cleanly (chain finishes
/// after probe, skipped steps NAMED, no auto-place), placement demands an
/// explicit duration_ms, audio tracks refuse stills, and the recorded op
/// is self-contained (src_range_ms resolved from duration_ms).
#[tokio::test]
async fn still_image_import_and_insert() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // Real PNG via lavfi (one red frame).
    let png = dir.path().join("intro.png");
    let ffmpeg = std::env::var("SHELLX_CUT_FFMPEG")
        .or_else(|_| std::env::var("FFMPEG_BIN"))
        .unwrap_or_else(|_| "ffmpeg".to_string());
    let ff = std::process::Command::new(ffmpeg)
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x240",
            "-frames:v",
            "1",
        ])
        .arg(&png)
        .status()
        .expect("ffmpeg present");
    assert!(ff.success());
    let r = dispatch(&state, "media.import", json!({"path": png}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let job_id = r.result.unwrap()["job_id"].as_str().unwrap().to_string();
    // Poll the chain job — for an image it must FINISH (not fail) fast.
    let rec = {
        let mut rec = None;
        for _ in 0..100 {
            let cur = state.jobs.get(&job_id).expect("job exists");
            match cur.state {
                crate::jobs::JobState::Done => {
                    rec = Some(cur);
                    break;
                }
                crate::jobs::JobState::Failed => {
                    unreachable!("image import chain failed: {:?}", cur.error)
                }
                _ => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
            }
        }
        rec.expect("image chain done within 10s")
    };
    let result = rec.result.expect("job result");
    assert_eq!(result["kind"], "image");
    assert_eq!(
        result["skipped_steps"],
        json!(["proxy", "transcribe", "perception"])
    );
    // No auto-place: a still can't become the timeline (no duration).
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert!(
        project.track("v1").unwrap().clips.is_empty(),
        "stills are never auto-placed"
    );
    assert_eq!(
        project.assets["a1"].probe.as_ref().unwrap()["kind"],
        "image"
    );
    // Insert without duration → actionable error naming duration_ms.
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        test_actor(),
    )
    .await;
    let e = r.error.expect("still without duration must error");
    assert_eq!(e.code, "invalid_args");
    assert!(
        e.suggested_action
            .as_deref()
            .unwrap_or("")
            .contains("duration_ms"),
        "suggested_action names duration_ms: {:?}",
        e.suggested_action
    );
    // Audio track refuses stills.
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0,"duration_ms":3000}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    // duration_ms + src_range_ms together is ambiguous → refused.
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"duration_ms":3000,"src_range_ms":[0,3000]}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    // Proper insert: 3s intro card on v1.
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"duration_ms":3000,"rationale":"intro card"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.as_ref().unwrap()["clip_id"], "c1");
    // The op is self-contained: src_range_ms recorded, duration_ms consumed.
    let op = &r.result.unwrap()["op"];
    assert_eq!(op["args"]["src_range_ms"], json!([0, 3000]));
    assert!(
        op["args"].get("duration_ms").is_none(),
        "convenience arg consumed at dispatch"
    );
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(project.duration_ms(), 3000);
}

/// duration_ms is image-only: timed media must use src_range_ms.
#[tokio::test]
async fn duration_ms_rejected_for_timed_media() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await; // asset a1, no probe.kind
                                                     // Mark a1 as probed video so the kind check is exercised.
    update_asset(&state, "a1", |a| {
        a.probe = Some(json!({"kind":"video","duration_ms":10_000}))
    })
    .await
    .unwrap();
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"duration_ms":2000}),
        test_actor(),
    )
    .await;
    let e = r.error.expect("duration_ms on video must error");
    assert_eq!(e.code, "invalid_args");
    assert!(
        e.cause.contains("video"),
        "cause names the probed kind: {}",
        e.cause
    );
}

#[tokio::test]
async fn update_asset_rejects_missing_asset_id() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let err = update_asset(&state, "missing", |a| {
        a.transcript = Some("receipts/missing.words.json".to_string())
    })
    .await
    .expect_err("missing asset write-back must fail");
    assert_eq!(err.code, error_codes::NOT_FOUND);
    assert!(err.message.contains("missing"));
}

#[tokio::test]
async fn nested_oneof_additional_properties_false_is_runtime_enforced() {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "audio.add_music",
        json!({"asset": "a1", "duck": {"bad_key": true}}),
        test_actor(),
    )
    .await;
    assert!(
        !r.ok,
        "unknown nested duck key must fail before handler defaults it"
    );
    let err = r.error.unwrap();
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(err.message.contains("duck.bad_key"), "{err:?}");
}

#[tokio::test]
async fn add_music_rejects_missing_explicit_duck_against_track() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;

    let r = dispatch(
        &state,
        "audio.add_music",
        json!({
            "asset": "a1",
            "src_range_ms": [0, 1000],
            "duck": {"against_track": "missing_speech"}
        }),
        test_actor(),
    )
    .await;

    assert!(!r.ok, "missing duck.against_track must not become a no-op");
    let err = r.error.unwrap();
    assert_eq!(err.code, error_codes::NOT_FOUND);
    assert!(
        err.message.contains("against_track"),
        "error should name the bad duck source: {err:?}"
    );
}

/// the capture-manifest contract pure mapping: clock-map interpolation + clamping, event→marker
/// time resolution order, payload preservation, schema/sidecar handling.
#[test]
fn capture_manifest_pure_mapping() {
    // utc_to_media_ms: linear inside, clamped outside (paused recording:
    // 2 s wall = 1 s media between the anchors).
    let t0 = chrono::DateTime::parse_from_rfc3339("2026-06-11T00:00:00.000Z")
        .unwrap()
        .timestamp_millis();
    let anchors = vec![(t0, 0u64), (t0 + 2000, 1000u64)];
    assert_eq!(utc_to_media_ms(&anchors, t0 + 1000), Some(500));
    assert_eq!(
        utc_to_media_ms(&anchors, t0 - 500),
        Some(0),
        "clamps before the span"
    );
    assert_eq!(
        utc_to_media_ms(&anchors, t0 + 9000),
        Some(1000),
        "clamps after the span"
    );
    assert_eq!(utc_to_media_ms(&[], t0), None, "no anchors, no mapping");
    // Event time resolution: at_ms > range_ms[0] > utc; payload survives.
    let ev = json!({"at_ms": 250, "type": "scene_switch", "confidence": "clock_mapped",
                        "data": {"from": "Code", "to": "Browser"}});
    let m = manifest_event_to_marker(&ev, &anchors).unwrap();
    assert_eq!(m["at_ms"], 250);
    assert_eq!(m["label"], "capture:scene_switch");
    let note: Value = serde_json::from_str(m["note"].as_str().unwrap()).unwrap();
    assert_eq!(
        note["confidence"], "clock_mapped",
        "confidence tag preserved in the note"
    );
    assert_eq!(note["data"]["to"], "Browser", "full payload preserved");
    let ev = json!({"range_ms": [400, 700], "type": "mute_span"});
    assert_eq!(
        manifest_event_to_marker(&ev, &anchors).unwrap()["at_ms"],
        400
    );
    let ev = json!({"utc": "2026-06-11T00:00:01.000Z", "type": "note"});
    assert_eq!(
        manifest_event_to_marker(&ev, &anchors).unwrap()["at_ms"],
        500
    );
    let ev = json!({"type": "mystery"});
    assert!(
        manifest_event_to_marker(&ev, &anchors).is_err(),
        "no usable time → skipped"
    );
    // Loader: schema tag is enforced.
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.capture.json");
    std::fs::write(&bad, r#"{"schema":"something/else","events":[]}"#).unwrap();
    assert_eq!(
        load_capture_manifest(&bad).unwrap_err().code,
        "invalid_args"
    );
    let oversized = dir.path().join("oversized.capture.json");
    std::fs::write(&oversized, b"12345").unwrap();
    assert_eq!(
        super::media::load_capture_manifest_with_limit(&oversized, 4)
            .unwrap_err()
            .code,
        "invalid_args"
    );
    // Discovery: malformed SIDECAR degrades to a warning, explicit is hard.
    let media = dir.path().join("take.mp4");
    std::fs::write(&media, b"x").unwrap();
    std::fs::write(dir.path().join("take.capture.json"), b"{not json").unwrap();
    let (m, warns) = resolve_capture_manifest(&media, None).unwrap();
    assert!(m.is_none());
    assert_eq!(warns.len(), 1);
    assert_eq!(warns[0].code, "capture_manifest_sidecar_unusable");
    assert!(resolve_capture_manifest(&media, Some(bad.to_str().unwrap())).is_err());
    let outside_dir = tempfile::tempdir().unwrap();
    let outside = outside_dir.path().join("take.capture.json");
    std::fs::write(
        &outside,
        r#"{"schema":"shellx-cut/capture-manifest/1","events":[]}"#,
    )
    .unwrap();
    let outside_err = resolve_capture_manifest(&media, Some(outside.to_str().unwrap()))
        .expect_err("explicit capture_manifest must stay beside imported media");
    assert_eq!(outside_err.code, "invalid_args");
}

/// the capture-manifest contract end-to-end through the import chain: a sidecar manifest is
/// auto-discovered (warned in-band), its events land as capture:<type>
/// marker OPS right after auto-place — payload + confidence in the note,
/// utc events mapped through the clock map. A later still-image import
/// with a manifest creates nothing and says so.
#[tokio::test]
async fn capture_manifest_ingests_markers() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // 1s video-only take (no audio → the chain dies fast at transcribe,
    // AFTER auto-place + marker ingest — the parts under test here).
    let media = dir.path().join("take.mp4");
    let ffmpeg = std::env::var("SHELLX_CUT_FFMPEG")
        .or_else(|_| std::env::var("FFMPEG_BIN"))
        .unwrap_or_else(|_| "ffmpeg".to_string());
    let ff = std::process::Command::new(ffmpeg)
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=1",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&media)
        .status()
        .expect("ffmpeg present");
    assert!(ff.success());
    std::fs::write(
        dir.path().join("take.capture.json"),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/capture-manifest/1",
            "recording": {"clock_map": [
                {"utc": "2026-06-11T00:00:00.000Z", "media_ms": 0},
                {"utc": "2026-06-11T00:00:02.000Z", "media_ms": 1000},
            ]},
            "events": [
                {"at_ms": 250, "type": "scene_switch", "confidence": "clock_mapped",
                 "source": "obs_ws:CurrentProgramSceneChanged",
                 "data": {"from": "Code", "to": "Browser"}},
                {"range_ms": [400, 700], "type": "mute_span", "confidence": "clock_mapped"},
                {"utc": "2026-06-11T00:00:01.000Z", "type": "note", "data": {"label": "take 2"}},
                {"type": "mystery_no_time"},
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    // Auto-discovery path: no capture_manifest arg.
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let warns = r.warnings.as_ref().expect("auto-discovery warns in-band");
    assert!(
        warns
            .iter()
            .any(|w| w.code == "capture_manifest_auto_discovered"),
        "{warns:?}"
    );
    let result = r.result.unwrap();
    assert_eq!(result["capture_manifest"]["events"], 4);
    let video_job = result["job_id"].as_str().unwrap().to_string();
    // Markers appear right after auto-place — poll state, not the job.
    let mut markers = Vec::new();
    for _ in 0..300 {
        let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
        let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
        if project.markers.len() >= 3 {
            markers = project.markers;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(markers.len(), 3, "3 of 4 events have usable times");
    assert_eq!(markers[0].at_ms, 250);
    assert_eq!(markers[0].label, "capture:scene_switch");
    let note: Value = serde_json::from_str(markers[0].note.as_deref().unwrap()).unwrap();
    assert_eq!(note["confidence"], "clock_mapped");
    assert_eq!(note["source"], "obs_ws:CurrentProgramSceneChanged");
    assert_eq!(
        (markers[1].at_ms, markers[1].label.as_str()),
        (400, "capture:mute_span")
    );
    let note: Value = serde_json::from_str(markers[1].note.as_deref().unwrap()).unwrap();
    assert_eq!(
        note["range_ms"],
        json!([400, 700]),
        "span survives in the payload"
    );
    assert_eq!(
        (markers[2].at_ms, markers[2].label.as_str()),
        (500, "capture:note")
    );
    // Markers are real OPS with the system actor (review-rail visible).
    let ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let ops = ops.result.unwrap()["ops"].as_array().unwrap().clone();
    let marker_ops: Vec<_> = ops
        .iter()
        .filter(|o| o["verb"] == "edit.add_marker")
        .collect();
    assert_eq!(marker_ops.len(), 3);
    assert_eq!(marker_ops[0]["actor"]["kind"], "system");
    assert!(marker_ops[0]["rationale"]
        .as_str()
        .unwrap()
        .contains("capture-manifest ingest"));
    // Still image + manifest: parsed but no markers (stills never place);
    // the IMAGE job finishes (not fails) and says so.
    let png = dir.path().join("card.png");
    let ff = std::process::Command::new("ffmpeg")
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x240",
            "-frames:v",
            "1",
        ])
        .arg(&png)
        .status()
        .expect("ffmpeg present");
    assert!(ff.success());
    let manifest2 = dir.path().join("card-manifest.json");
    std::fs::write(
        &manifest2,
        r#"{"schema":"shellx-cut/capture-manifest/1","events":[{"at_ms":1,"type":"x"}]}"#,
    )
    .unwrap();
    let r = dispatch(
        &state,
        "media.import",
        json!({"path": png, "capture_manifest": manifest2}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let img_job = r.result.unwrap()["job_id"].as_str().unwrap().to_string();
    let mut img_result = Value::Null;
    for _ in 0..100 {
        match state.jobs.get(&img_job).expect("job exists").state {
            crate::jobs::JobState::Done => {
                img_result = state.jobs.get(&img_job).unwrap().result.unwrap();
                break;
            }
            crate::jobs::JobState::Failed => unreachable!("image chain must finish"),
            _ => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    assert_eq!(img_result["capture_manifest"]["markers_created"], 0);
    assert!(img_result["capture_manifest"]["note"]
        .as_str()
        .unwrap()
        .contains("never auto-placed"));
    // Let the video chain reach its terminal state before the runtime
    // drops (it fails fast at transcribe — no audio stream — which is
    // EXPECTED and after the marker ingest under test).
    for _ in 0..600 {
        match state.jobs.get(&video_job).expect("job exists").state {
            crate::jobs::JobState::Done | crate::jobs::JobState::Failed => break,
            _ => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
}

/// the ripple-sync contract dispatch contract: the ripple DEFAULT is resolved from the
/// target track (base video/audio → ripple, overlay/extra → float) and
/// recorded EXPLICITLY on the op so logged ops stay self-contained.
#[tokio::test]
async fn insert_ripple_default_base_vs_overlay() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    // Base placement (both tracks, same source — the auto-place shape).
    for track in ["v1", "a1t"] {
        let r = dispatch(
            &state,
            "edit.insert",
            json!({"asset":"a1","track":track,"at_ms":0,"src_range_ms":[0,8000],"ripple":false}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "{:?}", r.error);
    }
    let r = dispatch(
        &state,
        "edit.add_track",
        json!({"kind":"video"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error); // v2 = overlay track
                                    // Base-track insert: default resolves ripple=true, recorded on the op,
                                    // and the sibling audio ripples (the A/V-offset regression).
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,2500]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["ripple"], json!(true), "base-track default is ripple");
    assert_eq!(
        res["op"]["args"]["ripple"],
        json!(true),
        "resolved value recorded on the op"
    );
    assert_eq!(res["rippled_tracks"], json!(["a1t"]));
    // Overlay-track insert: default resolves ripple=false (overlays float).
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v2","at_ms":1000,"src_range_ms":[0,1000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["ripple"], json!(false), "overlay default floats");
    assert_eq!(res["rippled_tracks"], json!([]));
    // AV sync invariant held end-to-end: v1 and a1t ends are equal.
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(
        project.track("v1").unwrap().duration_ms(),
        project.track("a1t").unwrap().duration_ms(),
        "video and audio track ends stay equal after a base insert"
    );
}

/// A/V placement guard: inserting an audio-bearing clip onto a VIDEO track warns
/// in-band (audio_not_mixed) instead of silently dropping its sound — the
/// renderer mixes Audio tracks only. Inserting the same clip onto an AUDIO
/// track does NOT warn.
#[tokio::test]
async fn insert_audio_bearing_on_video_track_warns() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    // Inject a probe marking the asset audio-bearing (the real import chain
    // sets this from ffprobe; set directly to keep the test hermetic).
    update_asset(&state, "a1", |a| {
        a.probe = Some(json!({
            "kind":"video","duration_ms":8000,"width":1920,"height":1080,"has_audio":true
        }));
    })
    .await
    .unwrap();
    // Insert onto the base VIDEO track → audio would be dropped → warn.
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,8000],"ripple":false}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let warns = r.warnings.unwrap_or_default();
    assert!(
        warns.iter().any(|w| w.code == "audio_not_mixed"),
        "video-track audio-bearing insert must warn: {warns:?}"
    );
    // Insert onto the AUDIO track → audio is mixed → no audio_not_mixed warning.
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0,"src_range_ms":[0,8000],"ripple":false}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert!(
        r.warnings
            .unwrap_or_default()
            .iter()
            .all(|w| w.code != "audio_not_mixed"),
        "audio-track insert must not warn"
    );
}

/// Schema-validation regression: the central gate rejects an unknown arg for
/// every verb in the registry (the additionalProperties:false contract is enforced,
/// not silently dropped) — and proves REST/MCP can't accept what the schema
/// forbids. The deprecated include_inverse compatibility option remains accepted
/// only where the public mutator schema retained it.
#[tokio::test]
async fn unknown_args_rejected_for_every_verb() {
    let state = AppState::new();
    let reg = crate::registry::VerbRegistry::load();
    for v in &reg.verbs {
        // additionalProperties:false holds for all 54 (asserted elsewhere);
        // the gate runs before any project/handler logic, so no setup needed.
        let r = dispatch(&state, &v.name, json!({"__bogus_arg__": 1}), test_actor()).await;
        assert!(!r.ok, "verb '{}' accepted an unknown arg", v.name);
        let e = r.error.unwrap();
        assert_eq!(e.code, "invalid_args", "verb '{}' wrong error code", v.name);
        assert!(
            e.message.contains("__bogus_arg__"),
            "verb '{}' message should name the bad arg: {}",
            v.name,
            e.message
        );
    }
    // Read-only/non-op verbs must not silently accept an ignored include_inverse.
    let r = dispatch(
        &state,
        "project.state",
        json!({"include_inverse": true}),
        test_actor(),
    )
    .await;
    assert!(!r.ok, "project.state should reject ignored include_inverse");
    let e = r.error.unwrap();
    assert_eq!(e.code, "invalid_args");
    assert!(e.message.contains("include_inverse"));

    // A real mutator still accepts the boolean compatibility no-op.
    let dir = tempfile::tempdir().unwrap();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"inverse","dir": dir.path().join("inverse.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms": 0, "label": "x", "include_inverse": true}),
        test_actor(),
    )
    .await;
    assert!(
        r.ok,
        "edit.add_marker should allow deprecated include_inverse: {:?}",
        r.error
    );
}

#[tokio::test]
async fn nested_unknown_args_rejected_when_schema_is_closed() {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"demo","settings":{"fpss":60}}),
        test_actor(),
    )
    .await;
    let e = r
        .error
        .expect("closed nested settings object should reject typo");
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(
        e.message.contains("settings.fpss"),
        "error should name nested typo path: {e:?}"
    );
}

#[tokio::test]
async fn redact_boxes_allow_motion_track_and_faces_use_track_faces_bool() {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "edit.redact",
        json!({
            "clip": "c1",
            "shape": "rect",
            "points": [[0.1,0.1],[0.2,0.2]],
            "boxes": [{
                "shape": "rect",
                "points": [[0.3,0.3],[0.4,0.4]],
                "track": [{"t_ms":0,"cx":0.35,"cy":0.35},{"t_ms":100,"cx":0.36,"cy":0.36}]
            }]
        }),
        test_actor(),
    )
    .await;
    assert_eq!(
        r.error.unwrap().code,
        error_codes::NO_PROJECT,
        "boxes[].track must pass schema validation and then fail only because no project is open"
    );

    let r = dispatch(
        &state,
        "edit.redact",
        json!({"clip":"c1","faces":true,"track":false}),
        test_actor(),
    )
    .await;
    let e = r.error.unwrap();
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(e.message.contains("/track"), "{e:?}");
    assert!(e.message.contains("type"), "{e:?}");

    let r = dispatch(
        &state,
        "edit.redact",
        json!({"clip":"c1","faces":true,"track_faces":false}),
        test_actor(),
    )
    .await;
    assert_eq!(
        r.error.unwrap().code,
        error_codes::NO_PROJECT,
        "track_faces:false must pass schema validation and reach the handler"
    );
}

/// Crossfade pairing guard: crossfading ONE track of the base video/audio pair leaves them
/// at unequal realized lengths (AV desync after the seam) → warn in-band.
/// Crossfading the sibling too re-syncs → no divergence warning.
#[tokio::test]
async fn crossfade_one_track_of_pair_warns_av_diverged() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    // Build a matching media-media seam at 5000ms on BOTH base tracks.
    for track in ["v1", "a1t"] {
        for (at, range) in [(0u64, [0u64, 5000]), (5000, [0, 3000])] {
            let r = dispatch(
                &state,
                "edit.insert",
                json!({"asset":"a1","track":track,"at_ms":at,"src_range_ms":range,"ripple":false}),
                test_actor(),
            )
            .await;
            assert!(r.ok, "{:?}", r.error);
        }
    }
    // Crossfade ONLY v1 → v1 shortens to 7000, a1t stays 8000 → warn.
    let r = dispatch(
        &state,
        "edit.crossfade",
        json!({"track":"v1","at_ms":5000,"duration_ms":1000,"transition":"pixelize"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(
        r.result.as_ref().unwrap()["transition"],
        json!("pixelize"),
        "edit.crossfade result should echo the applied transition"
    );
    let warns = r.warnings.unwrap_or_default();
    assert!(
        warns.iter().any(|w| w.code == "av_length_diverged"),
        "single-track crossfade of the base pair must warn: {warns:?}"
    );
    // Crossfade the sibling too → lengths match again → no divergence warning.
    let r = dispatch(
        &state,
        "edit.crossfade",
        json!({"track":"a1t","at_ms":5000,"duration_ms":1000}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert!(
        r.warnings
            .unwrap_or_default()
            .iter()
            .all(|w| w.code != "av_length_diverged"),
        "after pairing the crossfade, base lengths match → no warning"
    );
}

#[tokio::test]
async fn blend_result_matches_public_contract() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"blend","dir": dir.path().join("blend.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "edit.add_track",
        json!({"kind":"video","id":"overlay"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "edit.blend",
        json!({"track":"overlay","mode":"multiply"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let result = r.result.unwrap();
    assert_eq!(result["track"], json!("overlay"));
    assert_eq!(result["blend_mode"], json!("multiply"));
    assert_eq!(result["old_blend_mode"], Value::Null);
}

#[tokio::test]
async fn trim_edges_undo_restores_both_edges_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("trim.cutproj");
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"trim","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".to_string());
    })
    .await
    .unwrap();
    let receipt_dir = dir.path().join("trim.cutproj").join("receipts");
    std::fs::create_dir_all(&receipt_dir).unwrap();
    std::fs::write(
        receipt_dir.join("a1.words.json"),
        serde_json::to_string(&cut_perception::Transcript {
            asset: "a1".to_string(),
            model: "test".to_string(),
            language: Some("en".to_string()),
            words: vec![
                cut_perception::WordSpan {
                    idx: 0,
                    word: "hello".to_string(),
                    start_ms: 2_000,
                    end_ms: 2_400,
                    confidence: Some(0.9),
                    speaker: None,
                },
                cut_perception::WordSpan {
                    idx: 1,
                    word: "bye".to_string(),
                    start_ms: 7_600,
                    end_ms: 8_000,
                    confidence: Some(0.9),
                    speaker: None,
                },
            ],
        })
        .unwrap(),
    )
    .unwrap();
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,10_000],"ripple":false}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let r = dispatch(
        &state,
        "edit.trim_edges",
        json!({"keep_pad_ms":0,"min_trim_ms":1000}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.op_ids.as_ref().map(Vec::len), Some(2));
    let trimmed: cut_core::Project = serde_json::from_value(
        dispatch(&state, "project.state", json!({}), test_actor())
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(trimmed.duration_ms(), 6_000);

    let r = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let restored: cut_core::Project = serde_json::from_value(
        dispatch(&state, "project.state", json!({}), test_actor())
            .await
            .result
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        restored.duration_ms(),
        10_000,
        "one undo should restore both leading and trailing trims"
    );
}

async fn ramped_single_video_clip_fixture() -> (tempfile::TempDir, AppState, String, String) {
    ramped_single_video_clip_fixture_with_actor(test_actor()).await
}

async fn ramped_single_video_clip_fixture_with_actor(
    ramp_actor: Actor,
) -> (tempfile::TempDir, AppState, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("t.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let asset_id = r.result.unwrap()["asset_id"].as_str().unwrap().to_string();
    update_asset(&state, &asset_id, |a| {
        a.probe = Some(json!({
            "kind":"video",
            "width":1920,
            "height":1080,
            "duration_ms":10000,
            "has_audio":false
        }));
    })
    .await
    .unwrap();

    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":asset_id,"track":"v1","at_ms":0,"src_range_ms":[0,10000],"ripple":false}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let clip_id = r.result.unwrap()["clip_id"].as_str().unwrap().to_string();

    let r = dispatch(
        &state,
        "edit.speed_ramp",
        json!({
            "clip": clip_id,
            "points": [
                {"at_ms":0,"factor":1.0},
                {"at_ms":5000,"factor":3.0},
                {"at_ms":10000,"factor":1.0}
            ],
            "segments": 8
        }),
        ramp_actor,
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    (dir, state, asset_id, clip_id)
}

#[tokio::test]
async fn speed_ramp_timebase_is_identical_for_agent_and_ui_actors() {
    let (_agent_dir, agent_state, _, _) =
        ramped_single_video_clip_fixture_with_actor(test_actor()).await;
    let ui_actor = Actor {
        kind: cut_core::ActorKind::Human,
        name: "ui".into(),
        via: "ui".into(),
        request: None,
    };
    let (_ui_dir, ui_state, _, _) = ramped_single_video_clip_fixture_with_actor(ui_actor).await;
    let agent = {
        let guard = agent_state.project.read().await;
        guard.as_ref().unwrap().project.clone()
    };
    let ui = {
        let guard = ui_state.project.read().await;
        guard.as_ref().unwrap().project.clone()
    };
    let agent_op = {
        let guard = agent_state.project.read().await;
        guard
            .as_ref()
            .unwrap()
            .log
            .read_all()
            .unwrap()
            .into_iter()
            .find(|op| op.verb == "edit.speed_ramp")
            .unwrap()
    };
    let agent_edl = cut_core::edl_from_project(&agent);
    let ui_edl = cut_core::edl_from_project(&ui);
    assert_eq!(agent_edl.duration_ms, ui_edl.duration_ms);
    let agent_ramp = match &agent.track("v1").unwrap().clips[0] {
        cut_core::Clip::Media(clip) => clip.speed_ramp.as_ref().unwrap(),
        _ => unreachable!(),
    };
    let ui_ramp = match &ui.track("v1").unwrap().clips[0] {
        cut_core::Clip::Media(clip) => clip.speed_ramp.as_ref().unwrap(),
        _ => unreachable!(),
    };
    assert_eq!(agent_ramp.timebase_fps, Some(agent.settings.fps));
    assert_eq!(agent_ramp.preferred_segments, Some(8));
    assert_eq!(
        agent_ramp.timebase_audio_rate,
        Some(agent.settings.audio_rate)
    );
    assert_eq!(agent_op.args["timebase_fps"], agent.settings.fps);
    assert_eq!(
        agent_op.args["timebase_audio_rate"],
        agent.settings.audio_rate
    );
    assert_eq!(agent_ramp.timebase_fps, ui_ramp.timebase_fps);
    assert_eq!(agent_ramp.preferred_segments, ui_ramp.preferred_segments);
    assert_eq!(agent_ramp.timebase_audio_rate, ui_ramp.timebase_audio_rate);
}

#[tokio::test]
async fn speed_ramp_overwrites_untrusted_timebase_before_commit() {
    let (_dir, state, _, clip_id) = ramped_single_video_clip_fixture().await;
    edit_speed_ramp(&state, json!({"clip":clip_id,"points":[]}), test_actor())
        .await
        .expect("clear ramp");
    edit_speed_ramp(
        &state,
        json!({
            "clip": clip_id,
            "points":[
                {"at_ms":0,"factor":1.0},
                {"at_ms":2500,"factor":2.0},
                {"at_ms":5000,"factor":1.0}
            ],
            "segments":8,
            "timebase_fps":1.0,
            "timebase_audio_rate":1
        }),
        test_actor(),
    )
    .await
    .expect("dispatch overwrites timebase");
    let guard = state.project.read().await;
    let store = guard.as_ref().unwrap();
    let ops = store.log.read_all().unwrap();
    let clear = ops
        .iter()
        .find(|op| op.verb == "edit.speed_ramp" && op.args["points"] == json!([]))
        .unwrap();
    assert!(clear.args.get("timebase_fps").is_none());
    assert!(clear.args.get("timebase_audio_rate").is_none());
    let op = ops
        .into_iter()
        .rfind(|op| op.verb == "edit.speed_ramp")
        .unwrap();
    assert_eq!(op.args["timebase_fps"], store.project.settings.fps);
    assert_eq!(
        op.args["timebase_audio_rate"],
        store.project.settings.audio_rate
    );
}

/// edit.split_at_scenes: scene cuts at 2/5/8s on a full 10s clip → 3 splits
/// → 4 shots.
#[tokio::test]
async fn edit_split_at_scenes_splits_base_clip() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    update_asset(&state, "a1", |a| {
            a.probe = Some(json!({"kind":"video","width":1920,"height":1080,"duration_ms":10000,"has_audio":false}));
        })
        .await
        .unwrap();
    dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,10000],"ripple":false}),
        test_actor(),
    )
    .await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_string(&json!({
            "schema":"shellx-cut/perception/1","asset_hash":"","source_path":"x",
            "instruments_run":["scenes"],"silences":[],
            "scenes":[{"at_ms":2000},{"at_ms":5000},{"at_ms":8000}],
            "black_spans":[],"frozen_spans":[],"content_bbox":null
        }))
        .unwrap(),
    )
    .unwrap();
    let r = dispatch(
        &state,
        "edit.split_at_scenes",
        json!({"asset":"a1"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["scene_cuts"], 3);
    assert_eq!(res["splits"], 3);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(
        project.track("v1").unwrap().clips.len(),
        4,
        "3 scene cuts → 4 shots"
    );
}

/// The linked A/V split contract the Timeline UI depends on. edit.split has no
/// server-side `linked` arg (unlike edit.trim/edit.move), so the UI dispatches
/// one edit.split per half of an inferred pair. Two guarantees under test:
/// 1. `group_id` is DECLARED on edit.split's public schema. Regression guard:
///    the schema's additionalProperties:false silently REJECTED the UI's
///    grouped linked split (invalid_args at /group_id → "clicked but nothing
///    happened") even though the store has always carried the meta-arg
///    (ops.rs group_id() / store.rs apply).
/// 2. Grouped splits undo as ONE user action: after splitting the video half
///    and its audio sibling with a shared tag, a single project.undo restores
///    both tracks — the same single-Ctrl+Z contract linked delete already has.
#[tokio::test]
async fn linked_split_group_id_is_declared_and_undoes_as_one() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    update_asset(&state, "a1", |a| {
        a.probe = Some(
            json!({"kind":"video","width":1920,"height":1080,"duration_ms":6000,"has_audio":true}),
        );
    })
    .await
    .unwrap();
    // The linked pair exactly as auto-place/insert creates it: same asset,
    // same source window, video on v1 + audio on a1t.
    for track in ["v1", "a1t"] {
        let r = dispatch(
            &state,
            "edit.insert",
            json!({"asset":"a1","track":track,"at_ms":0,"src_range_ms":[0,6000],"ripple":false}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "insert on {track}: {:?}", r.error);
    }
    let media_clips = |project: &cut_core::Project, track: &str| -> usize {
        project
            .track(track)
            .unwrap()
            .clips
            .iter()
            .filter(|c| matches!(c, cut_core::Clip::Media(_)))
            .count()
    };
    // The UI's linked split: one schema-validated edit.split per half at the
    // SAME editorial position, sharing one undo-group tag.
    for track in ["v1", "a1t"] {
        let r = dispatch(
            &state,
            "edit.split",
            json!({"track":track,"at_ms":2000,"group_id":"grp-split-test","rationale":"linked split"}),
            test_actor(),
        )
        .await;
        assert!(
            r.ok,
            "grouped split on {track} must be schema-legal: {:?}",
            r.error
        );
    }
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(media_clips(&project, "v1"), 2, "video half split");
    assert_eq!(media_clips(&project, "a1t"), 2, "audio half split with it");
    // ONE undo step reverts the WHOLE linked cut (group collapse).
    let r = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(r.ok, "undo: {:?}", r.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(media_clips(&project, "v1"), 1, "single undo restores video");
    assert_eq!(
        media_clips(&project, "a1t"),
        1,
        "the SAME undo restores the audio half — one user action, one Ctrl+Z"
    );
}

#[tokio::test]
async fn split_at_scenes_surfaces_non_boundary_split_failure() {
    let (dir, state, asset_id, _clip_id) = ramped_single_video_clip_fixture().await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join(format!("{asset_id}.perception.json")),
        serde_json::to_string(&json!({
            "schema":"shellx-cut/perception/1","asset_hash":"","source_path":"x",
            "instruments_run":["scenes"],"silences":[],
            "scenes":[{"at_ms":2000}],
            "black_spans":[],"frozen_spans":[],"content_bbox":null
        }))
        .unwrap(),
    )
    .unwrap();

    let r = dispatch(
        &state,
        "edit.split_at_scenes",
        json!({"asset":asset_id,"min_shot_ms":0}),
        test_actor(),
    )
    .await;

    assert!(
        !r.ok,
        "speed-ramped clip split failure must be returned, not hidden: {:?}",
        r.result
    );
    let err = r.error.expect("error envelope");
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.message.contains("variable-speed ramp"),
        "unexpected error: {err:?}"
    );
}

#[tokio::test]
async fn cut_to_beat_surfaces_non_boundary_split_failure() {
    let (_dir, state, _asset_id, _clip_id) = ramped_single_video_clip_fixture().await;
    let r = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms":1200,"label":"beat","note":"beat:1"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let r = dispatch(
        &state,
        "edit.cut_to_beat",
        json!({"track":"v1","mode":"split"}),
        test_actor(),
    )
    .await;

    assert!(
        !r.ok,
        "speed-ramped beat split failure must be returned, not hidden: {:?}",
        r.result
    );
    let err = r.error.expect("error envelope");
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.message.contains("variable-speed ramp"),
        "unexpected error: {err:?}"
    );
}

#[test]
fn bundle_caption_sidecars_only_report_written_paths() {
    let dir = tempfile::tempdir().unwrap();
    let (srt_path, vtt_path, err) = write_bundle_caption_sidecars(dir.path(), "1\n", "WEBVTT\n", 1);
    assert!(srt_path.as_deref().is_some_and(|p| p.ends_with("clip.srt")));
    assert!(vtt_path.as_deref().is_some_and(|p| p.ends_with("clip.vtt")));
    assert_eq!(err, None);

    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, b"not a directory").unwrap();
    let (srt_path, vtt_path, err) = write_bundle_caption_sidecars(&blocked, "1\n", "WEBVTT\n", 1);
    assert_eq!(srt_path, None);
    assert_eq!(vtt_path, None);
    assert!(
        err.as_deref()
            .is_some_and(|e| e.contains("clip.srt") && e.contains("clip.vtt")),
        "write failures should be reported without claiming sidecar paths: {err:?}"
    );
}

#[tokio::test]
async fn multicam_switch_does_not_destroy_user_program_track() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"mc","dir": dir.path().join("mc.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    for name in ["cam1.mp4", "cam2.mp4"] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
        let r = dispatch(
            &state,
            "media.import",
            json!({"path": dir.path().join(name)}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "media.import {name}: {:?}", r.error);
    }
    for asset in ["a1", "a2"] {
        update_asset(&state, asset, |a| {
            a.probe = Some(json!({
                "kind":"video","width":1920,"height":1080,"duration_ms":2000,"has_audio":true
            }));
        })
        .await
        .unwrap();
    }

    let r = dispatch(
        &state,
        "edit.add_track",
        json!({"kind":"video","id":"v2"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "add v2: {:?}", r.error);
    let r = dispatch(
        &state,
        "edit.add_track",
        json!({"kind":"video","id":"program"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "add user program: {:?}", r.error);

    for (asset, track) in [("a1", "v1"), ("a2", "v2"), ("a1", "program")] {
        let r = dispatch(
            &state,
            "edit.insert",
            json!({"asset":asset,"track":track,"at_ms":0,"src_range_ms":[0,2000],"ripple":false}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "insert {asset} on {track}: {:?}", r.error);
    }
    let user_program_clip = {
        let guard = state.project.read().await;
        match &guard
            .as_ref()
            .unwrap()
            .project
            .track("program")
            .unwrap()
            .clips[0]
        {
            cut_core::Clip::Media(m) => m.id.clone(),
            _ => unreachable!("program clip should be media"),
        }
    };

    let receipts = dir.path().join("mc.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    for (asset, source, lufs) in [("a1", "cam1.mp4", -10.0), ("a2", "cam2.mp4", -20.0)] {
        std::fs::write(
            receipts.join(format!("{asset}.perception.json")),
            serde_json::to_string(&json!({
                "schema": "shellx-cut/perception/1",
                "asset_hash": "sha256:test",
                "source_path": source,
                "instruments_run": ["loudness"],
                "loudness": {
                    "integrated_lufs": lufs,
                    "true_peak_dbtp": -1.0,
                    "windows": [
                        {"at_ms": 0, "momentary_lufs": lufs},
                        {"at_ms": 1000, "momentary_lufs": lufs}
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();
    }

    let r = dispatch(
        &state,
        "edit.multicam_switch",
        json!({"tracks":["v1","v2"],"mode":"energy","min_shot_ms":250}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "multicam_switch: {:?}", r.error);
    let result = r.result.unwrap();
    assert_ne!(
        result["program_track"], "program",
        "multicam output must not collide with a user-owned program track"
    );

    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    let user_track = project.track("program").expect("user program track kept");
    assert_eq!(user_track.clips.len(), 1, "user program clip kept");
    match &user_track.clips[0] {
        cut_core::Clip::Media(m) => assert_eq!(m.id, user_program_clip),
        _ => unreachable!("program clip should stay media"),
    }
    assert!(
        project
            .track(result["program_track"].as_str().unwrap())
            .is_some(),
        "new reserved multicam output track exists"
    );
}

/// export.chapters: markers → a time-sorted "M:SS Label" chapter file.
#[tokio::test]
async fn export_chapters_writes_sorted_chapter_list() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    // add out of order — export must time-sort.
    dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms": 65000, "label": "Part 2"}),
        test_actor(),
    )
    .await;
    dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms": 0, "label": "Intro"}),
        test_actor(),
    )
    .await;
    let r = dispatch(&state, "export.chapters", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["chapter_count"], 2);
    assert_eq!(res["first_at_zero"], true);
    let content = std::fs::read_to_string(res["path"].as_str().unwrap()).unwrap();
    assert_eq!(
        content, "0:00 Intro\n1:05 Part 2\n",
        "sorted, M:SS formatted"
    );
}

/// transcript.search: find a word + a phrase + a miss, returning word ranges.
#[tokio::test]
async fn transcript_search_finds_word_and_phrase() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset":"a1","model":"test","language":"en","words":[
                {"idx":0,"word":"Hello,","start_ms":100,"end_ms":400},
                {"idx":1,"word":"and","start_ms":500,"end_ms":700},
                {"idx":2,"word":"welcome","start_ms":800,"end_ms":1300},
                {"idx":3,"word":"and","start_ms":1400,"end_ms":1600}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();
    // single word "and" → two matches; punctuation-insensitive "hello" → one
    let r = dispatch(
        &state,
        "transcript.search",
        json!({"asset":"a1","query":"and"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.as_ref().unwrap()["match_count"], 2);
    let r2 = dispatch(
        &state,
        "transcript.search",
        json!({"asset":"a1","query":"hello"}),
        test_actor(),
    )
    .await;
    assert_eq!(
        r2.result.unwrap()["matches"][0]["word_range"],
        json!([0, 0])
    );
    // phrase "and welcome" → one match [1,2]
    let r3 = dispatch(
        &state,
        "transcript.search",
        json!({"asset":"a1","query":"and welcome"}),
        test_actor(),
    )
    .await;
    let m = r3.result.unwrap();
    assert_eq!(m["match_count"], 1);
    assert_eq!(m["matches"][0]["word_range"], json!([1, 2]));
    // miss → 0
    let r4 = dispatch(
        &state,
        "transcript.search",
        json!({"asset":"a1","query":"zebra"}),
        test_actor(),
    )
    .await;
    assert_eq!(r4.result.unwrap()["match_count"], 0);
}

/// transcript.assemble: build a REORDERED, non-contiguous highlight reel
/// ("welcome" then "hello") — spans placed sequentially, audio mirrored.
#[tokio::test]
async fn transcript_assemble_builds_reordered_highlight_reel() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    update_asset(&state, "a1", |a| {
        a.probe = Some(json!({
            "kind":"video","width":1920,"height":1080,"duration_ms":2000,"has_audio":true
        }));
    })
    .await
    .unwrap();
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset":"a1","model":"test","language":"en","words":[
                {"idx":0,"word":"hello","start_ms":100,"end_ms":400},
                {"idx":1,"word":"and","start_ms":500,"end_ms":700},
                {"idx":2,"word":"welcome","start_ms":800,"end_ms":1300}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();
    // Reorder: welcome (idx 2) THEN hello (idx 0) — non-contiguous + reordered.
    let r = dispatch(
        &state,
        "transcript.assemble",
        json!({"asset":"a1","word_ranges":[[2,2],[0,0]]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["spans_placed"], 2);
    assert_eq!(res["audio_mirrored"], true);
    // welcome span = [760,1340] (580ms), hello = [60,440] (380ms) → 960ms.
    assert_eq!(res["total_ms"], 960);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    let v1 = project.track("v1").unwrap();
    assert_eq!(v1.clips.len(), 2, "two highlight clips on v1");
    match (&v1.clips[0], &v1.clips[1]) {
        (cut_core::Clip::Media(first), cut_core::Clip::Media(second)) => {
            assert_eq!(
                (first.src_in_ms, first.src_out_ms),
                (760, 1340),
                "welcome first"
            );
            assert_eq!(
                (second.src_in_ms, second.src_out_ms),
                (60, 440),
                "hello second"
            );
        }
        _ => unreachable!("expected two media clips"),
    }
    assert_eq!(
        project.track("a1t").unwrap().clips.len(),
        2,
        "audio mirrored"
    );
}

/// transcript.ignore_words is non-destructive source-transcript
/// state. Captions and transcript.assemble skip ignored words by default,
/// and project.undo restores the ignore list.
#[tokio::test]
async fn transcript_ignore_words_skips_captions_and_assemble_and_undoes() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    update_asset(&state, "a1", |a| {
        a.probe = Some(json!({
            "kind":"video","width":1920,"height":1080,"duration_ms":2000,"has_audio":true
        }));
    })
    .await
    .unwrap();
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset":"a1","model":"test","language":"en","words":[
                {"idx":0,"word":"hello","start_ms":100,"end_ms":400},
                {"idx":1,"word":"and","start_ms":500,"end_ms":700},
                {"idx":2,"word":"welcome","start_ms":800,"end_ms":1300}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();
    dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,2000],"ripple":false}),
        test_actor(),
    )
    .await;
    dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0,"src_range_ms":[0,2000],"ripple":false}),
        test_actor(),
    )
    .await;

    let ignore = dispatch(
        &state,
        "transcript.ignore_words",
        json!({"asset":"a1","word_range":[1,1],"rationale":"hide connector"}),
        test_actor(),
    )
    .await;
    assert!(ignore.ok, "{:?}", ignore.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert_eq!(
        s.result.unwrap()["transcript_ignores"],
        json!([{"asset":"a1","word_range":[1,1]}])
    );
    let timeline = dispatch(&state, "transcript.timeline", json!({}), test_actor()).await;
    assert!(timeline.ok, "{:?}", timeline.error);
    let timeline_result = timeline.result.unwrap();
    let words = timeline_result["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["word"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(words, vec!["hello", "welcome"]);
    let undo = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undo.ok, "{:?}", undo.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let after_undo = s.result.unwrap();
    assert_eq!(
        after_undo["transcript_ignores"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0),
        0
    );

    let ignore = dispatch(
        &state,
        "transcript.ignore_words",
        json!({"asset":"a1","word_range":[1,1],"rationale":"hide connector"}),
        test_actor(),
    )
    .await;
    assert!(ignore.ok, "{:?}", ignore.error);

    let caps = dispatch(&state, "captions.generate", json!({}), test_actor()).await;
    assert!(caps.ok, "{:?}", caps.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    let cap_text = project
        .track("cap1")
        .unwrap()
        .clips
        .iter()
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => Some(cc.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(cap_text, "hello welcome");

    let reel = dispatch(
        &state,
        "transcript.assemble",
        json!({"asset":"a1","word_ranges":[[0,2]]}),
        test_actor(),
    )
    .await;
    assert!(reel.ok, "{:?}", reel.error);
    let res = reel.result.unwrap();
    assert_eq!(res["spans_placed"], 2);
    assert_eq!(res["total_ms"], 960);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    let v1 = project.track("v1").unwrap();
    let placed: Vec<[u64; 2]> = v1
        .clips
        .iter()
        .filter_map(|c| match c {
            cut_core::Clip::Media(m) => Some([m.src_in_ms, m.src_out_ms]),
            _ => None,
        })
        .collect();
    assert!(
        placed.ends_with(&[[60, 440], [760, 1340]]),
        "assembled spans should skip the ignored connector: {placed:?}"
    );
}

/// transcript.timeline: asset words mapped to timeline positions through
/// the EDL; PROGRAM view de-dups the linked video/audio pair; `clip`/`track`
/// narrow scope; clip-scoped transcript.cut_words trims that clip only.
#[tokio::test]
async fn transcript_timeline_maps_words_dedups_and_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"x").unwrap();
    dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    update_asset(&state, "a1", |a| {
        a.probe = Some(json!({
            "kind":"video","width":1920,"height":1080,"duration_ms":2000,"has_audio":true
        }));
    })
    .await
    .unwrap();
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset":"a1","model":"test","language":"en","words":[
                {"idx":0,"word":"hello","start_ms":100,"end_ms":400},
                {"idx":1,"word":"and","start_ms":500,"end_ms":700},
                {"idx":2,"word":"welcome","start_ms":800,"end_ms":1300}
            ]
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();
    // Place a LINKED video+audio pair (same asset, same position) — the
    // own-line model: video on v1, its sound on a1t.
    dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,2000],"ripple":false}),
        test_actor(),
    )
    .await;
    dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0,"src_range_ms":[0,2000],"ripple":false}),
        test_actor(),
    )
    .await;
    // PROGRAM view: linked V/A de-duplicated → 3 words once, video-preferred.
    let prog = dispatch(&state, "transcript.timeline", json!({}), test_actor()).await;
    assert!(prog.ok, "{:?}", prog.error);
    let pr = prog.result.unwrap();
    assert_eq!(
        pr["word_count"], 3,
        "linked V/A de-duped to one entry per word"
    );
    assert_eq!(pr["entries"][0]["word"], "hello");
    assert_eq!(
        pr["entries"][0]["timeline_start_ms"], 100,
        "src→timeline @ at_ms 0"
    );
    assert_eq!(
        pr["entries"][0]["track_kind"], "video",
        "video preferred in dedup"
    );
    // timeline order
    let ts: Vec<u64> = pr["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["timeline_start_ms"].as_u64().unwrap())
        .collect();
    let mut sorted = ts.clone();
    sorted.sort_unstable();
    assert_eq!(ts, sorted, "program entries in timeline order");
    // track filter: the audio track alone still maps its 3 words.
    let at = dispatch(
        &state,
        "transcript.timeline",
        json!({"track":"a1t"}),
        test_actor(),
    )
    .await;
    assert_eq!(at.result.unwrap()["word_count"], 3);
    // clip filter: the v1 clip's id maps exactly its words.
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    let v1_clip = match &project.track("v1").unwrap().clips[0] {
        cut_core::Clip::Media(m) => m.id.clone(),
        _ => unreachable!("expected a media clip on v1"),
    };
    let cl = dispatch(
        &state,
        "transcript.timeline",
        json!({"clip": v1_clip}),
        test_actor(),
    )
    .await;
    let clr = cl.result.unwrap();
    assert_eq!(clr["word_count"], 3);
    assert!(clr["entries"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["clip_id"] == json!(v1_clip)));
    // unknown clip → empty.
    let none = dispatch(
        &state,
        "transcript.timeline",
        json!({"clip":"nope"}),
        test_actor(),
    )
    .await;
    assert_eq!(none.result.unwrap()["word_count"], 0);
    // clip-scoped cut: trims that clip (removes a timeline range).
    let cut = dispatch(
        &state,
        "transcript.cut_words",
        json!({"asset":"a1","word_range":[1,1],"clip": v1_clip}),
        test_actor(),
    )
    .await;
    assert!(cut.ok, "{:?}", cut.error);
    assert!(
        cut.result.unwrap()["removed_ms"].as_u64().unwrap() > 0,
        "clip-scoped cut removed a timeline range"
    );
}

/// edit.fade (the fade-edit contract) through dispatch: arg validation, the documented
/// result shape, the resolved kind recorded on the op.
#[tokio::test]
async fn edit_fade_dispatch_contract() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0,"src_range_ms":[0,5000],"ripple":false}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // Validation: exactly one target, at least one side, known kind.
    let r = dispatch(&state, "edit.fade", json!({"in_ms":500}), test_actor()).await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    let r = dispatch(
        &state,
        "edit.fade",
        json!({"clip":"c1","track":"a1t","in_ms":500}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    let r = dispatch(&state, "edit.fade", json!({"clip":"c1"}), test_actor()).await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    let r = dispatch(
        &state,
        "edit.fade",
        json!({"clip":"c1","in_ms":500,"kind":"sideways"}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    // Track form on the audio track: result shape + default kind recorded.
    let r = dispatch(
        &state,
        "edit.fade",
        json!({"track":"a1t","in_ms":400,"out_ms":800,"rationale":"music bed in/out"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    let targets = res["targets"].as_array().expect("targets array");
    assert_eq!(targets.len(), 1, "single clip on the track gets both sides");
    assert_eq!(targets[0]["clip"], "c1");
    assert_eq!(targets[0]["fade"]["in_ms"], 400);
    assert_eq!(targets[0]["fade"]["out_ms"], 800);
    assert_eq!(targets[0]["fade"]["kind"], "both");
    assert_eq!(
        res["op"]["args"]["kind"], "both",
        "resolved kind recorded on the op"
    );
    // Kind that cannot render on the track is refused (audio track + video kind).
    let r = dispatch(
        &state,
        "edit.fade",
        json!({"clip":"c1","in_ms":100,"kind":"video"}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    // The fade is undoable like any timeline op (recompute-by-replay).
    let op_id = res["op"]["op_id"].as_str().unwrap().to_string();
    let r = dispatch(
        &state,
        "edit.restore",
        json!({"op_id": op_id}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    match project.track("a1t").unwrap().clips.first().unwrap() {
        cut_core::Clip::Media(c) => assert!(c.fade.is_none(), "restore cleared the fade"),
        _ => unreachable!("media clip expected"),
    }
}

/// Shared fixture for the scope-narrowing tests: project with asset a1
/// (dummy file), v1 holding source [0,5000] and a1t holding source
/// [5000,10000] — so a source-time fact lands on exactly ONE track and
/// track narrowing is distinguishable from timeline-wide.
async fn narrowing_fixture(dir: &std::path::Path) -> AppState {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let media = dir.join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    // ripple:false — this fixture DELIBERATELY places different source
    // windows per track (the ripple-sync contract default would gap-shift v1 when the
    // a1t placement lands; per-track surgery is exactly the opt-out case).
    for (track, range) in [("v1", [0u64, 5000]), ("a1t", [5000, 10000])] {
        let r = dispatch(
            &state,
            "edit.insert",
            json!({"asset":"a1","track":track,"at_ms":0,"src_range_ms":range,"ripple":false}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "insert on {track}: {:?}", r.error);
    }
    state
}

/// the scope contract: `track` on transcript.remove_silences narrows DETECTION to
/// that track's segments. This was ACCEPTED by the schema but silently
/// ignored by dispatch — a real contract violation (the agent narrowed,
/// the server cut timeline-wide facts anyway).
#[tokio::test]
async fn remove_silences_honors_track_narrowing() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    // Perception fact: silence at source [1000,2000] of a1 — present on
    // v1's source window only.
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/perception/1",
            "asset_hash": "sha256:test",
            "source_path": "clip.mp4",
            "instruments_run": ["silence"],
            "silences": [{"start_ms": 1000, "end_ms": 2000, "source": "both"}],
        }))
        .unwrap(),
    )
    .unwrap();
    // Narrowed to a1t (source window [5000,10000]): the silence is not
    // on that track → nothing matched, zero ops.
    let r = dispatch(
        &state,
        "transcript.remove_silences",
        json!({"aggressiveness":"natural","track":"a1t"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.unwrap()["spans_removed"], 0);
    // Unknown track id is an actionable error, not a silent no-match.
    let r = dispatch(
        &state,
        "transcript.remove_silences",
        json!({"aggressiveness":"natural","track":"nope"}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "not_found");
    // Narrowed to v1: silence [1000,2000] − 120ms preset padding →
    // 760ms cut at timeline [1120,1880]; the RIPPLE is timeline-wide
    // (AV sync), so BOTH tracks shrink by 760ms.
    let r = dispatch(
        &state,
        "transcript.remove_silences",
        json!({"aggressiveness":"natural","track":"v1"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["spans_removed"], 1);
    assert_eq!(res["total_removed_ms"], 760);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(project.track("v1").unwrap().duration_ms(), 4240);
    assert_eq!(project.track("a1t").unwrap().duration_ms(), 4240);
}

/// Captions regression: timed text cards land on the
/// dedicated txt1 track with deterministic ids; position synthesizes a
/// built-in style; captions.generate cannot wipe them; cards past the
/// media end warn in-band; edit.restore undoes.
#[tokio::test]
async fn add_text_places_titled_cards() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await; // 5000ms timeline
                                                     // Validation first.
    let r = dispatch(
        &state,
        "captions.add_text",
        json!({"text":"","range_ms":[0,2000]}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    let r = dispatch(
        &state,
        "captions.add_text",
        json!({"text":"x","range_ms":[2000,2000]}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    let r = dispatch(
        &state,
        "captions.add_text",
        json!({"text":"x","range_ms":[0,1000],"style_ref":"s","position":"center"}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    let r = dispatch(
        &state,
        "captions.add_text",
        json!({"text":"x","range_ms":[0,1000],"style_ref":"missing"}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "not_found");
    // Intro card with auto-created center style.
    let r = dispatch(
            &state,
            "captions.add_text",
            json!({"text":"ShellX Cut","range_ms":[0,2500],"position":"center","rationale":"intro card"}),
            test_actor(),
        )
        .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["clip_id"], "txt_0001");
    assert_eq!(res["track_id"], "txt1");
    assert_eq!(res["style_ref"], "txt_center");
    assert!(r.warnings.is_none(), "inside media duration → no warning");
    // The built-in style exists with the right position.
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(
        project.caption_styles["txt_center"].pos.as_deref(),
        Some("center")
    );
    assert_eq!(
        project.track("txt1").unwrap().kind,
        cut_core::TrackKind::Caption
    );
    // Outro card past the 5000ms media end → in-band warning + growth.
    let r = dispatch(
        &state,
        "captions.add_text",
        json!({"text":"theshellx.com","range_ms":[4500,7000],"position":"center"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.as_ref().unwrap()["clip_id"], "txt_0002");
    let w = r.warnings.as_ref().expect("extends past media → warning");
    assert_eq!(w[0].code, "text_extends_composition");
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(
        project.duration_ms(),
        7000,
        "composition grew to the card end"
    );
    // captions.generate must NOT touch txt1 (it owns cap1 only) — here it
    // errors for lack of transcripts, which is fine; txt1 must survive.
    let _ = dispatch(&state, "captions.generate", json!({}), test_actor()).await;
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(project.track("txt1").unwrap().clips.len(), 2);
    // Undo the outro card (the op is restorable like every mutation).
    let s = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let ops = s.result.unwrap()["ops"].as_array().unwrap().clone();
    let outro_op = ops
        .iter()
        .rev()
        .find(|o| o["verb"] == "captions.add_text")
        .unwrap()["op_id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = dispatch(
        &state,
        "edit.restore",
        json!({"op_id": outro_op}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(
        project.track("txt1").unwrap().clips.len(),
        1,
        "outro card undone"
    );
    assert_eq!(project.duration_ms(), 5000);
}

#[tokio::test]
async fn captions_generate_uses_cap1_when_txt1_exists() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let r = dispatch(
        &state,
        "captions.add_text",
        json!({"text":"Intro","range_ms":[0,1000],"position":"center"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset": "a1", "model": "test", "language": "en",
            "words": [
                {"idx": 0, "word": "generated", "start_ms": 5100, "end_ms": 5400},
                {"idx": 1, "word": "caption", "start_ms": 5500, "end_ms": 5900}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();

    let r = dispatch(&state, "captions.generate", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.as_ref().unwrap()["track_id"], "cap1");

    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    let txt_texts: Vec<String> = project
        .track("txt1")
        .unwrap()
        .clips
        .iter()
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => Some(cc.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(txt_texts, vec!["Intro".to_string()]);
    let cap_texts: Vec<String> = project
        .track("cap1")
        .unwrap()
        .clips
        .iter()
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => Some(cc.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(cap_texts.join(" "), "generated caption");
}

#[tokio::test]
async fn captions_kinetic_replace_static_undoes_overlay_and_static_clear_together() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset": "a1", "model": "test", "language": "en",
            "words": [
                {"idx": 0, "word": "animated", "start_ms": 5100, "end_ms": 5400},
                {"idx": 1, "word": "caption", "start_ms": 5500, "end_ms": 5900}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();

    let r = dispatch(&state, "captions.generate", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);

    let r = dispatch(
        &state,
        "captions.kinetic",
        json!({"replace_static": true, "rationale": "animate captions"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.as_ref().unwrap()["cleared_static"], 1);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(
        project.track("cap1").unwrap().clips.len(),
        0,
        "replace_static clears the static caption cue"
    );
    assert!(
        project
            .tracks
            .iter()
            .any(|t| t.id.starts_with("title") && !t.clips.is_empty()),
        "kinetic overlay is present before undo"
    );

    let r = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(
        project.track("cap1").unwrap().clips.len(),
        1,
        "one undo restores the static caption cue"
    );
    assert!(
        !project
            .tracks
            .iter()
            .any(|t| t.id.starts_with("title") && !t.clips.is_empty()),
        "one undo also removes the kinetic overlay"
    );
}

#[tokio::test]
async fn transcript_translate_rejects_path_like_target_lang_before_writing_receipt() {
    let _guard = lock_agent_cli_env();
    let old_runner_py = std::env::var_os("TRANSLATE_RUNNER_PY");
    let old_runner_script = std::env::var_os("TRANSLATE_RUNNER_SCRIPT");

    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset": "a1", "model": "test", "language": "en",
            "words": [
                {"idx": 0, "word": "hello", "start_ms": 0, "end_ms": 500}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();

    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    #[cfg(windows)]
    let fake_py = bin.join("fake-python.cmd");
    #[cfg(not(windows))]
    let fake_py = bin.join("fake-python");
    #[cfg(windows)]
        std::fs::write(
            &fake_py,
            "@echo off\r\necho {\"translations\":[\"Hola\"],\"model\":\"fake-local\",\"backend\":\"opus-mt\"}\r\n",
        )
        .unwrap();
    #[cfg(not(windows))]
        std::fs::write(
            &fake_py,
            "#!/usr/bin/env sh\nprintf '%s\\n' '{\"translations\":[\"Hola\"],\"model\":\"fake-local\",\"backend\":\"opus-mt\"}'\n",
        )
        .unwrap();
    make_executable(&fake_py);
    let fake_script = bin.join("translate_runner.py");
    std::fs::write(&fake_script, "# fake local runner marker\n").unwrap();
    std::env::set_var("TRANSLATE_RUNNER_PY", &fake_py);
    std::env::set_var("TRANSLATE_RUNNER_SCRIPT", &fake_script);

    let outside = dir.path().join("escape.words.json");
    let r = dispatch(
        &state,
        "transcript.translate",
        json!({
            "asset":"a1",
            "source_lang":"en",
            "target_lang":"../../../escape",
            "backend":"local"
        }),
        test_actor(),
    )
    .await;

    match old_runner_py {
        Some(path) => std::env::set_var("TRANSLATE_RUNNER_PY", path),
        None => std::env::remove_var("TRANSLATE_RUNNER_PY"),
    }
    match old_runner_script {
        Some(path) => std::env::set_var("TRANSLATE_RUNNER_SCRIPT", path),
        None => std::env::remove_var("TRANSLATE_RUNNER_SCRIPT"),
    }

    assert!(!r.ok, "path-like target_lang must be rejected");
    let err = r.error.as_ref().unwrap();
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.message.contains("target_lang"),
        "error should name the rejected target_lang: {err:?}"
    );
    assert!(
        !outside.exists(),
        "target_lang traversal must not write a sibling artifact outside the project"
    );
}

#[tokio::test]
async fn multicam_sync_rejects_max_offset_below_schema_minimum() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let r = dispatch(
        &state,
        "edit.multicam_sync",
        json!({"clips":["c1","c2"],"max_offset_ms":0}),
        test_actor(),
    )
    .await;
    let e = r
        .error
        .expect("max_offset_ms below schema minimum must error");
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(
        e.message.contains("max_offset_ms"),
        "error names bad field: {e:?}"
    );
}

#[tokio::test]
async fn captions_translate_rejects_replace_with_reflow_before_backend() {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "captions.translate",
        json!({"target_lang":"es","mode":"replace","reflow":true}),
        test_actor(),
    )
    .await;
    let e = r
        .error
        .expect("replace mode cannot safely combine with reflow");
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(
        e.message.contains("reflow") && e.message.contains("replace"),
        "error explains the invalid combination: {e:?}"
    );
}

#[test]
fn caption_replace_rejects_same_count_identity_mismatch() {
    let source_cues = vec![
        CaptionTranslateSrcCue {
            id: "cap_a".into(),
            range_ms: [0, 1000],
            text: "one".into(),
        },
        CaptionTranslateSrcCue {
            id: "cap_b".into(),
            range_ms: [1000, 2000],
            text: "two".into(),
        },
    ];
    let translated = vec![
        cut_core::CaptionClip {
            id: "xl_0001".into(),
            text: "uno".into(),
            style_ref: None,
            range_ms: [0, 1000],
        },
        cut_core::CaptionClip {
            id: "xl_0002".into(),
            text: "dos".into(),
            style_ref: None,
            range_ms: [1000, 2000],
        },
    ];
    let mut track = cut_core::Track {
        id: "cap1".into(),
        kind: cut_core::TrackKind::Caption,
        clips: vec![
            cut_core::Clip::Caption(cut_core::CaptionClip {
                id: "cap_a".into(),
                text: "one".into(),
                style_ref: None,
                range_ms: [1000, 2000],
            }),
            cut_core::Clip::Caption(cut_core::CaptionClip {
                id: "cap_b".into(),
                text: "two".into(),
                style_ref: None,
                range_ms: [0, 1000],
            }),
        ],
        gain_db: 0.0,
        gain_windows: Vec::new(),
        blend_mode: None,
        visible: true,
        locked: false,
        muted: false,
        solo: false,
        pan: 0.0,
    };

    let err = replace_caption_texts_by_identity(&mut track, &source_cues, &translated)
        .expect_err("same-count cue identity/range drift must conflict");
    assert_eq!(err.code, error_codes::CONFLICT);
    assert!(
        err.message.contains("changed during translation"),
        "error explains concurrent mutation: {err:?}"
    );
}

#[tokio::test]
async fn edit_matte_accepts_rationale_arg_shape() {
    let state = AppState::new();
    let r = dispatch(
        &state,
        "edit.matte",
        json!({"clip":"c1","rationale":"test matte"}),
        test_actor(),
    )
    .await;
    let e = r
        .error
        .expect("no project should be the first runtime error");
    assert_eq!(e.code, error_codes::NO_PROJECT);
}

/// captions.import owns the subtitle `cap1` track. A project may already
/// have `txt1` from timed text cards; imported subtitles must not reuse that
/// title-card track, because captions.translate intentionally ignores txt1.
#[tokio::test]
async fn captions_import_uses_cap1_when_txt1_exists() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;

    let r = dispatch(
        &state,
        "captions.add_text",
        json!({"text":"Intro card","range_ms":[0,1200],"position":"center"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let srt = dir.path().join("t.cutproj").join("sample.srt");
    std::fs::write(
            &srt,
            "1\r\n00:00:00,000 --> 00:00:01,000\r\nFirst cue\r\n\r\n2\r\n00:00:01,200 --> 00:00:02,200\r\nSecond cue\r\n\r\n",
        )
        .unwrap();
    let r = dispatch(
        &state,
        "captions.import",
        json!({"path": srt}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.as_ref().unwrap()["track_id"], "cap1");

    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(
        project.track("txt1").unwrap().clips.len(),
        1,
        "title-card track remains separate"
    );
    let cap1 = project.track("cap1").expect("import creates cap1");
    let texts: Vec<&str> = cap1
        .clips
        .iter()
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => Some(cc.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["First cue", "Second cue"]);
}

#[tokio::test]
async fn captions_import_accepts_external_subtitle_picker_file() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let outside = dir.path().join("outside.srt");
    std::fs::write(&outside, "1\n00:00:00,000 --> 00:00:01,000\noutside cue\n").unwrap();

    let r = dispatch(
        &state,
        "captions.import",
        json!({"path": outside}),
        test_actor(),
    )
    .await;

    assert!(
        r.ok,
        "external subtitle picker path should import: {:?}",
        r.error
    );
    assert_eq!(r.result.as_ref().unwrap()["track_id"], "cap1");
    assert_eq!(r.result.as_ref().unwrap()["caption_count"], 1);
}

#[tokio::test]
async fn captions_import_rejects_non_subtitle_extensions_before_reading() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let txt = dir.path().join("not-subtitles.txt");
    std::fs::write(
        &txt,
        "1\n00:00:00,000 --> 00:00:01,000\nlooks like captions\n",
    )
    .unwrap();

    let r = dispatch(
        &state,
        "captions.import",
        json!({"path": txt}),
        test_actor(),
    )
    .await;

    assert!(!r.ok, "non-subtitle extension must be rejected");
    let e = r.error.unwrap();
    assert_eq!(e.code, error_codes::INVALID_ARGS);
    assert!(
        e.message.contains("extension"),
        "error should name the extension contract: {e:?}"
    );
}

/// caption-deduplication guard (caption doubling): an asset placed on BOTH v1 and a1t —
/// the DEFAULT first-import auto-place shape — must contribute each
/// transcribed word ONCE to generated captions, with non-overlapping
/// cue ranges. Pre-fix, the EDL walk harvested the transcript once per
/// track and every cue read "hello hello and and welcome welcome".
#[tokio::test]
async fn captions_generate_does_not_double_words_on_av_placement() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    // Reproduce the auto-place shape EXACTLY: same asset, same source
    // range, on v1 (video) AND a1t (audio) at 0, ripple:false.
    for track in ["v1", "a1t"] {
        let r = dispatch(
            &state,
            "edit.insert",
            json!({"asset":"a1","track":track,"at_ms":0,"src_range_ms":[0,4000],"ripple":false}),
            test_actor(),
        )
        .await;
        assert!(r.ok, "insert on {track}: {:?}", r.error);
    }
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset": "a1", "model": "test", "language": "en",
            "words": [
                {"idx": 0, "word": "hello",   "start_ms": 100, "end_ms": 400},
                {"idx": 1, "word": "and",     "start_ms": 500, "end_ms": 700},
                {"idx": 2, "word": "welcome", "start_ms": 800, "end_ms": 1300}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();
    let r = dispatch(&state, "captions.generate", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    let cues: Vec<&cut_core::CaptionClip> = project
        .track("cap1")
        .expect("caption track")
        .clips
        .iter()
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => Some(cc),
            _ => None,
        })
        .collect();
    let joined = cues
        .iter()
        .map(|c| c.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        joined, "hello and welcome",
        "each word exactly once, got: {joined:?}"
    );
    // Cue ranges must not overlap (the doubling also produced cue 2
    // starting before cue 1 ended in the exported SRT).
    for pair in cues.windows(2) {
        assert!(
            pair[0].range_ms[1] <= pair[1].range_ms[0],
            "overlapping cues: {:?} then {:?}",
            pair[0].range_ms,
            pair[1].range_ms
        );
    }
}

/// Captions follow the AUDIO placement. With a video-only
/// placement (no audio track carries the asset) the fallback still
/// captions the v1 window; once an audio placement exists, the audio
/// window is the heard speech and is what gets captioned.
#[tokio::test]
async fn captions_generate_follows_audio_placement() {
    let dir = tempfile::tempdir().unwrap();
    // narrowing_fixture: v1 holds src [0,5000], a1t holds src [5000,10000].
    let state = narrowing_fixture(dir.path()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset": "a1", "model": "test", "language": "en",
            "words": [
                {"idx": 0, "word": "seen",  "start_ms": 100,  "end_ms": 400},
                {"idx": 1, "word": "heard", "start_ms": 5500, "end_ms": 5800}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();
    let r = dispatch(&state, "captions.generate", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    let texts: Vec<String> = project
        .track("cap1")
        .expect("caption track")
        .clips
        .iter()
        .filter_map(|c| match c {
            cut_core::Clip::Caption(cc) => Some(cc.text.clone()),
            _ => None,
        })
        .collect();
    // Only the word inside a1t's source window is heard → captioned;
    // the v1-only word is unheard video and must NOT be captioned.
    assert_eq!(
        texts.join(" "),
        "heard",
        "captions follow audio placement: {texts:?}"
    );
}

/// edit.duck end-to-end at the dispatch layer: windows computed from the
/// against-track's perception silences mapped through the EDL, recorded
/// on a self-contained op; honest no-op without speech; arg validation.
#[tokio::test]
async fn duck_computes_windows_from_perception() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await; // a1t holds src [5000,10000] at timeline [0,5000)
                                                     // Music track to duck.
    let r = dispatch(
        &state,
        "edit.add_track",
        json!({"kind":"audio"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.unwrap()["track_id"], "a2t");
    // Validation short-circuits.
    let r = dispatch(
        &state,
        "edit.duck",
        json!({"music_track":"a2t","against_track":"a2t","db":-18.0}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    let r = dispatch(
        &state,
        "edit.duck",
        json!({"music_track":"a2t","against_track":"a1t","db":6.0}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, "invalid_args");
    // No perception report yet → actionable not_found naming the fix.
    let r = dispatch(
        &state,
        "edit.duck",
        json!({"music_track":"a2t","against_track":"a1t","db":-18.0}),
        test_actor(),
    )
    .await;
    let e = r.error.unwrap();
    assert_eq!(e.code, "not_found");
    assert!(e
        .suggested_action
        .as_deref()
        .unwrap_or("")
        .contains("media.perception"));
    // Perception: silences [5500,6500] + [8000,10000] in a1's source →
    // speech complement within a1t's window [5000,10000) =
    // [5000,5500) + [6500,8000) → timeline [0,500) + [1500,3000).
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/perception/1",
            "asset_hash": "sha256:test",
            "source_path": "clip.mp4",
            "instruments_run": ["silence"],
            "silences": [
                {"start_ms": 5500, "end_ms": 6500, "source": "both"},
                {"start_ms": 8000, "end_ms": 10000, "source": "both"}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    let r = dispatch(
            &state,
            "edit.duck",
            json!({"music_track":"a2t","against_track":"a1t","db":-18.0,"attack_ms":250,"rationale":"music under narration"}),
            test_actor(),
        )
        .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["track_id"], "a2t");
    assert_eq!(res["windows_applied"], 2);
    assert_eq!(res["total_ducked_ms"], 2000); // 500 + 1500
                                              // Self-contained op: resolved windows recorded in args.
    let ws = &res["op"]["args"]["windows"];
    assert_eq!(ws[0]["range_ms"], json!([0, 500]));
    assert_eq!(ws[1]["range_ms"], json!([1500, 3000]));
    assert_eq!(ws[0]["db"], -18.0);
    assert_eq!(ws[0]["attack_ms"], 250);
    // State carries the windows on the music track.
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(project.track("a2t").unwrap().gain_windows.len(), 2);
    // Fully-silent against-track → honest no-op, NO op appended.
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/perception/1",
            "asset_hash": "sha256:test",
            "source_path": "clip.mp4",
            "instruments_run": ["silence"],
            "silences": [{"start_ms": 0, "end_ms": 10000, "source": "both"}],
        }))
        .unwrap(),
    )
    .unwrap();
    let ops_before = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let n_before = ops_before.result.unwrap()["ops"].as_array().unwrap().len();
    let r = dispatch(
        &state,
        "edit.duck",
        json!({"music_track":"a2t","against_track":"a1t","db":-18.0}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["windows_applied"], 0);
    assert!(res["note"].as_str().unwrap().contains("no speech"));
    let ops_after = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    assert_eq!(
        ops_after.result.unwrap()["ops"].as_array().unwrap().len(),
        n_before
    );
}

/// Totality guard (the totality guard): a silence pass that would remove
/// >80% of the timeline is REFUSED with code `guardrail` unless
/// allow_extreme:true. On the 76s real screen recording the unguarded
/// verb deleted 99.4% of the timeline.
#[tokio::test]
async fn remove_silences_totality_guard() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await; // v1 src[0,5000] + a1t src[5000,10000] → 5000ms timeline
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    // Perception fact: ONE silence covering the entire 10s source — maps
    // to ~100% of both tracks' timelines (the regression footage shape).
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/perception/1",
            "asset_hash": "sha256:test",
            "source_path": "clip.mp4",
            "instruments_run": ["silence"],
            "silences": [{"start_ms": 0, "end_ms": 10_000, "source": "both"}],
        }))
        .unwrap(),
    )
    .unwrap();
    // Without allow_extreme → guardrail refusal, nothing removed.
    let r = dispatch(
        &state,
        "transcript.remove_silences",
        json!({"aggressiveness":"natural"}),
        test_actor(),
    )
    .await;
    let e = r.error.expect("totality guard must refuse");
    assert_eq!(e.code, "guardrail");
    assert!(
        e.suggested_action
            .as_deref()
            .unwrap_or("")
            .contains("allow_extreme"),
        "suggested_action names the override: {:?}",
        e.suggested_action
    );
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert_eq!(
        project.duration_ms(),
        5000,
        "guard refused → timeline untouched"
    );
    // WITH allow_extreme → the extreme cut proceeds (one op per span).
    let r = dispatch(
        &state,
        "transcript.remove_silences",
        json!({"aggressiveness":"natural","allow_extreme":true}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["spans_removed"], 2); // one span per track's source window
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    assert!(
        project.duration_ms() < 1000,
        "extreme cut applied: {}ms left",
        project.duration_ms()
    );
}

/// The guard stays OUT of the way for normal passes (<80% removal).
#[tokio::test]
async fn remove_silences_guard_allows_normal_pass() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    // One 1s silence inside v1's 5s window → ~15% removal. No guard.
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/perception/1",
            "asset_hash": "sha256:test",
            "source_path": "clip.mp4",
            "instruments_run": ["silence"],
            "silences": [{"start_ms": 1000, "end_ms": 2000, "source": "both"}],
        }))
        .unwrap(),
    )
    .unwrap();
    let r = dispatch(
        &state,
        "transcript.remove_silences",
        json!({"aggressiveness":"natural"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "normal pass must not trip the guard: {:?}", r.error);
    assert_eq!(r.result.unwrap()["spans_removed"], 1);
}

/// missing-silence-facts guard: remove_silences with NO silence facts for the placed
/// asset(s) must error actionably — never the indistinguishable
/// {ok, spans_removed: 0, note: "nothing matched"}. Facts present +
/// genuinely nothing matching stays an ok-zero; assets that are placed
/// but unmeasured surface as an in-band warning when spans were still
/// found elsewhere; imported-but-unplaced assets are ignored entirely.
#[tokio::test]
async fn remove_silences_errors_without_silence_facts() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await; // a1 placed on v1+a1t
                                                     // 1) No perception report at all → actionable error, not ok-zero.
    let r = dispatch(
        &state,
        "transcript.remove_silences",
        json!({"aggressiveness":"natural"}),
        test_actor(),
    )
    .await;
    let e = r.error.expect("must error without facts");
    assert_eq!(e.code, "not_found");
    assert!(e.message.contains("no silence facts"), "{}", e.message);
    assert!(
        e.suggested_action
            .as_deref()
            .unwrap_or("")
            .contains("media.perception"),
        "{:?}",
        e.suggested_action
    );
    // 2) Facts present, nothing matched in scope → honest ok-zero stays.
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/perception/1",
            "asset_hash": "sha256:test",
            "source_path": "clip.mp4",
            "instruments_run": ["silence"],
            "silences": [{"start_ms": 1000, "end_ms": 2000, "source": "both"}],
        }))
        .unwrap(),
    )
    .unwrap();
    let r = dispatch(
        &state,
        "transcript.remove_silences",
        json!({"aggressiveness":"natural","track":"a1t"}), // silence is on v1's window
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.unwrap()["spans_removed"], 0);
    assert!(r.warnings.is_none(), "measured asset, no warning");
    // 3) Second asset imported but UNPLACED → ignored (no error/warning).
    let media2 = dir.path().join("broll.mp4");
    std::fs::write(&media2, b"not-really-video").unwrap();
    let r = dispatch(
        &state,
        "media.import",
        json!({"path": media2}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // 4) Place a2 (no facts) — spans still found from a1 → ok + warning.
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a2","track":"a1t","at_ms":5000,"src_range_ms":[0,1000],"ripple":false}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "transcript.remove_silences",
        json!({"aggressiveness":"natural"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(
        r.result.unwrap()["spans_removed"],
        1,
        "a1's facts still cut"
    );
    let w = r.warnings.expect("unmeasured placed asset must warn");
    assert_eq!(w[0].code, "missing_silence_facts");
    assert!(w[0].message.contains("a2"), "{}", w[0].message);
}

/// the scope contract: `track` on transcript.remove_fillers narrows detection the
/// same way (was silently ignored too).
#[tokio::test]
async fn remove_fillers_honors_track_narrowing() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    // Transcript fact: an "um" at source [5500,5800] of a1 — present on
    // a1t's source window only.
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset": "a1",
            "model": "test",
            "language": "en",
            "words": [
                {"idx": 0, "word": "um", "start_ms": 5500, "end_ms": 5800},
                {"idx": 1, "word": "fine", "start_ms": 5900, "end_ms": 6300}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();
    // Narrowed to v1 (source [0,5000]): the filler is not there.
    let r = dispatch(
        &state,
        "transcript.remove_fillers",
        json!({"track":"v1"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.result.unwrap()["fillers_removed"], 0);
    // Narrowed to a1t: one filler run cut (±40ms word padding).
    let r = dispatch(
        &state,
        "transcript.remove_fillers",
        json!({"track":"a1t"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["fillers_removed"], 1);
    assert_eq!(res["total_removed_ms"], 300 + 80); // word 300ms + 2×40ms pad
}

#[tokio::test]
async fn remove_fillers_normalizes_custom_lexicon_like_words() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(
        receipts.join("a1.words.json"),
        serde_json::to_string(&json!({
            "asset": "a1",
            "model": "test",
            "language": "en",
            "words": [
                {"idx": 0, "word": "uh-huh", "start_ms": 5500, "end_ms": 5800},
                {"idx": 1, "word": "continue", "start_ms": 5900, "end_ms": 6300}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    update_asset(&state, "a1", |a| {
        a.transcript = Some("receipts/a1.words.json".into())
    })
    .await
    .unwrap();

    let r = dispatch(
        &state,
        "transcript.remove_fillers",
        json!({"track":"a1t","lexicon":["uh-huh"]}),
        test_actor(),
    )
    .await;

    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["fillers_removed"], 1);
    assert_eq!(res["total_removed_ms"], 300 + 80);
}

/// Contract: every core edit verb + project.checkpoint returns the result
/// shape schema/verbs.json documents (they used to return only {op} / the
/// bare Checkpoint — a result-shape drift flagged by the docs agent).
/// The op record still rides along as result.op.
#[tokio::test]
async fn edit_results_match_schema_shapes() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // Dummy media file — the asset record is all the edit verbs need
    // (explicit src_range_ms avoids any probe dependency).
    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);

    let go = |verb: &'static str, args: Value| {
        let state = state.clone();
        async move {
            let r = dispatch(&state, verb, args, test_actor()).await;
            assert!(r.ok, "{verb} failed: {:?}", r.error);
            let res = r.result.unwrap();
            assert!(res["op"].is_object(), "{verb}: op record must ride along");
            res
        }
    };

    // edit.insert → {clip_id}
    let res = go(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,5000]}),
    )
    .await;
    assert_eq!(res["clip_id"], "c1");
    // edit.split → {clip_ids: [left, right]}
    let res = go("edit.split", json!({"track":"v1","at_ms":2500})).await;
    assert_eq!(res["clip_ids"], json!(["c1", "c2"]));
    // edit.trim → {clip, src_in_ms, src_out_ms}
    let res = go("edit.trim", json!({"clip":"c1","src_out_ms":2000})).await;
    assert_eq!(res["clip"], "c1");
    assert_eq!(res["src_in_ms"], 0);
    assert_eq!(res["src_out_ms"], 2000);
    // edit.move → {clip, track, at_ms}
    let res = go("edit.move", json!({"clip":"c2","to_track":"v1","at_ms":0})).await;
    assert_eq!(res["clip"], "c2");
    assert_eq!(res["track"], "v1");
    assert_eq!(res["at_ms"], 0);
    for (verb, args) in [
        ("edit.gain", json!({"clip":"c1","db":-3.0})),
        ("edit.mute", json!({"track":"v1","on":true})),
        ("edit.solo", json!({"track":"v1","on":true})),
        ("edit.pan", json!({"track":"v1","pan":0.5})),
    ] {
        let rejected = dispatch(&state, verb, args, test_actor()).await;
        assert!(!rejected.ok, "{verb} must reject a video target");
        assert_eq!(
            rejected.error.as_ref().map(|error| error.code.as_str()),
            Some(cut_core::error::codes::INVALID_ARGS)
        );
    }
    // Audio gain targets the linked AUDIO timeline, never the video clip whose
    // pixels live on v1. Video gain was historically accepted but never entered
    // the render graph.
    let res = go(
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0,"src_range_ms":[0,5000],"ripple":false}),
    )
    .await;
    let audio_clip_id = res["clip_id"].as_str().unwrap().to_string();
    // edit.gain → {target, kind, old_db, new_db}
    let res = go("edit.gain", json!({"clip":audio_clip_id,"db":-3.0})).await;
    assert_eq!(res["target"], audio_clip_id);
    assert_eq!(res["kind"], "clip");
    assert_eq!(res["old_db"], 0.0);
    assert_eq!(res["new_db"], -3.0);
    // edit.add_track → {track_id, kind}
    let res = go(
        "edit.add_track",
        json!({"kind":"audio","rationale":"music bed"}),
    )
    .await;
    assert_eq!(res["track_id"], "a2t");
    assert_eq!(res["kind"], "audio");
    // edit.transform → {clip, transform, old_transform}
    let res = go(
        "edit.transform",
        json!({"clip":"c1","x":0.5,"y":0.5,"scale":0.25}),
    )
    .await;
    assert_eq!(res["clip"], "c1");
    assert_eq!(res["transform"]["scale"], 0.25);
    assert!(res["old_transform"].is_null());
    // Identity clears (transform → null).
    let res = go("edit.transform", json!({"clip":"c1"})).await;
    assert!(res["transform"].is_null());
    assert_eq!(res["old_transform"]["x"], 0.5);
    // edit.add_marker → {marker_id}; edit.remove_marker → {removed}
    let res = go("edit.add_marker", json!({"at_ms":100,"label":"x"})).await;
    let marker_id = res["marker_id"]
        .as_str()
        .expect("marker_id present")
        .to_string();
    let res = go("edit.remove_marker", json!({"id": marker_id})).await;
    assert_eq!(res["removed"], marker_id);
    // edit.ripple_delete → {removed_ms, tracks}
    let res = go("edit.ripple_delete", json!({"range_ms":[0,500]})).await;
    assert_eq!(res["removed_ms"], 500);
    let tracks: Vec<String> =
        serde_json::from_value(res["tracks"].clone()).expect("tracks is a string array");
    assert!(tracks.iter().any(|t| t == "v1"), "v1 touched: {tracks:?}");
    // edit.restore → {restored_op_id, op_ids}
    let gain_op = go("edit.gain", json!({"clip":audio_clip_id,"db":-6.0})).await["op"]["op_id"]
        .as_str()
        .unwrap()
        .to_string();
    let res = go("edit.restore", json!({"op_id": gain_op})).await;
    assert_eq!(res["restored_op_id"], gain_op);
    assert!(res["op_ids"].is_array());
    // project.checkpoint → {checkpoint: {id, name, at_op, ts}} (no op in
    // the result — the envelope's op_ids carries the appended op id).
    let r = dispatch(
        &state,
        "project.checkpoint",
        json!({"name":"cp-test"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    assert_eq!(r.op_ids.as_ref().map(|v| v.len()), Some(1));
    let res = r.result.unwrap();
    let cp = &res["checkpoint"];
    assert_eq!(cp["name"], "cp-test");
    for key in ["id", "at_op", "ts"] {
        assert!(cp[key].is_string(), "checkpoint.{key} present: {res}");
    }
}

/// Regression: a normal timeline drag must not leave an imported clip's exact
/// audio counterpart behind. The linked move is one atomic logged action, and
/// both destination splice ids must remain stable through replay and undo/redo.
#[tokio::test]
async fn edit_move_keeps_exact_av_pair_linked_and_replayable() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"linked-move","dir":dir.path().join("linked-move.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = dir.path().join("linked.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let imported = dispatch(&state, "media.import", json!({"path":media}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);

    for (track, at_ms, src_range_ms) in [
        ("v1", 0, [0, 1000]),
        ("a1t", 0, [0, 1000]),
        ("v1", 2000, [2000, 4000]),
        ("a1t", 2000, [2000, 4000]),
    ] {
        let inserted = dispatch(
            &state,
            "edit.insert",
            json!({
                "asset":"a1",
                "track":track,
                "at_ms":at_ms,
                "src_range_ms":src_range_ms,
                "ripple":false,
            }),
            test_actor(),
        )
        .await;
        assert!(inserted.ok, "insert on {track}: {:?}", inserted.error);
    }

    let moved = dispatch(
        &state,
        "edit.move",
        json!({"clip":"c1","to_track":"v1","at_ms":2500}),
        test_actor(),
    )
    .await;
    assert!(moved.ok, "{:?}", moved.error);
    let result = moved.result.unwrap();
    assert_eq!(result["linked"], true);
    assert_eq!(result["linked_clip"], "c2");
    assert_eq!(result["linked_track"], "a1t");
    assert_eq!(result["ripple"], false);
    assert_eq!(moved.op_ids.as_ref().map(Vec::len), Some(1));
    let split_ids: Vec<&str> = result["op"]["effects"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|effect| effect.get("split_clip")?.as_str())
        .collect();
    assert_eq!(split_ids.len(), 2, "both destination clips were split");
    assert_ne!(split_ids[0], split_ids[1]);

    let (live, log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        (store.project.clone(), store.log.read_all().unwrap())
    };
    let edl = cut_core::edl_from_project(&live);
    for clip in ["c1", "c2"] {
        let segments: Vec<_> = edl
            .segments
            .iter()
            .filter(|segment| segment.clip_id.as_deref() == Some(clip))
            .collect();
        assert_eq!(segments.first().unwrap().timeline_in_ms, 2500);
        assert_eq!(segments.last().unwrap().timeline_out_ms, 3500);
    }
    let rebuilt = cut_core::rebuild_from_log(&log).expect("linked move replay");
    assert_eq!(rebuilt, live, "replay preserves both destination split ids");

    let undo = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undo.ok, "{:?}", undo.error);
    let undone = {
        let guard = state.project.read().await;
        guard.as_ref().unwrap().project.clone()
    };
    let undone_edl = cut_core::edl_from_project(&undone);
    for clip in ["c1", "c2"] {
        let segment = undone_edl
            .segments
            .iter()
            .find(|segment| segment.clip_id.as_deref() == Some(clip))
            .unwrap();
        assert_eq!(segment.timeline_in_ms, 0);
        assert_eq!(segment.timeline_out_ms, 1000);
    }
    let redo = dispatch(&state, "project.redo", json!({}), test_actor()).await;
    assert!(redo.ok, "{:?}", redo.error);
    let redone = {
        let guard = state.project.read().await;
        guard.as_ref().unwrap().project.clone()
    };
    assert_eq!(redone, live, "one redo reapplies the whole linked move");

    let independent = dispatch(
        &state,
        "edit.move",
        json!({
            "clip":"c1",
            "to_track":"v1",
            "at_ms":0,
            "linked":false,
            "ripple":false,
        }),
        test_actor(),
    )
    .await;
    assert!(independent.ok, "{:?}", independent.error);
    let independent_result = independent.result.unwrap();
    assert_eq!(independent_result["linked"], false);
    assert!(independent_result["linked_clip"].is_null());
    let independent_project = {
        let guard = state.project.read().await;
        guard.as_ref().unwrap().project.clone()
    };
    let independent_edl = cut_core::edl_from_project(&independent_project);
    let video_start = independent_edl
        .segments
        .iter()
        .find(|segment| segment.clip_id.as_deref() == Some("c1"))
        .unwrap()
        .timeline_in_ms;
    let audio_start = independent_edl
        .segments
        .iter()
        .find(|segment| segment.clip_id.as_deref() == Some("c2"))
        .unwrap()
        .timeline_in_ms;
    assert_eq!(video_start, 0);
    assert_eq!(audio_start, 2500);
}

/// A timeline edge drag and the Q/W playhead trims must not shorten video while
/// leaving its imported audio at the old duration. The pair is one replayable
/// edit and one undo step; linked:false remains the deliberate unlink escape.
#[tokio::test]
async fn edit_trim_keeps_exact_av_pair_linked_and_replayable() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"linked-trim","dir":dir.path().join("linked-trim.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = dir.path().join("linked.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let imported = dispatch(&state, "media.import", json!({"path":media}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);
    for track in ["v1", "a1t"] {
        let inserted = dispatch(
            &state,
            "edit.insert",
            json!({
                "asset":"a1",
                "track":track,
                "at_ms":0,
                "src_range_ms":[0,5000],
                "ripple":false,
            }),
            test_actor(),
        )
        .await;
        assert!(inserted.ok, "insert on {track}: {:?}", inserted.error);
    }

    let trimmed = dispatch(
        &state,
        "edit.trim",
        json!({"clip":"c1","src_in_ms":1000}),
        test_actor(),
    )
    .await;
    assert!(trimmed.ok, "{:?}", trimmed.error);
    assert_eq!(trimmed.op_ids.as_ref().map(Vec::len), Some(1));
    let result = trimmed.result.as_ref().unwrap();
    assert_eq!(result["clip"], "c1");
    assert_eq!(result["src_in_ms"], 1000);
    assert_eq!(result["src_out_ms"], 5000);
    assert_eq!(result["linked"], true);
    assert_eq!(result["linked_clip"], "c2");
    assert_eq!(result["linked_track"], "a1t");

    let (live, log) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        for clip_id in ["c1", "c2"] {
            let (track_id, index) = store.project.find_clip(clip_id).unwrap();
            let cut_core::Clip::Media(clip) = &store.project.track(track_id).unwrap().clips[index]
            else {
                panic!("{clip_id} must be media")
            };
            assert_eq!([clip.src_in_ms, clip.src_out_ms], [1000, 5000]);
        }
        (store.project.clone(), store.log.read_all().unwrap())
    };
    let rebuilt = cut_core::rebuild_from_log(&log).expect("linked trim replay");
    assert_eq!(rebuilt, live, "replay preserves the atomic linked trim");

    let undo = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undo.ok, "{:?}", undo.error);
    {
        let guard = state.project.read().await;
        let project = &guard.as_ref().unwrap().project;
        for clip_id in ["c1", "c2"] {
            let (track_id, index) = project.find_clip(clip_id).unwrap();
            let cut_core::Clip::Media(clip) = &project.track(track_id).unwrap().clips[index] else {
                panic!("{clip_id} must be media")
            };
            assert_eq!([clip.src_in_ms, clip.src_out_ms], [0, 5000]);
        }
    }
    let redo = dispatch(&state, "project.redo", json!({}), test_actor()).await;
    assert!(redo.ok, "{:?}", redo.error);

    let locked = dispatch(
        &state,
        "edit.track_lock",
        json!({"track":"a1t","on":true}),
        test_actor(),
    )
    .await;
    assert!(locked.ok, "{:?}", locked.error);
    let guarded = dispatch(
        &state,
        "edit.trim",
        json!({"clip":"c1","src_out_ms":4500}),
        test_actor(),
    )
    .await;
    assert!(!guarded.ok, "a locked linked track must block trim");
    assert_eq!(
        guarded.error.as_ref().map(|error| error.code.as_str()),
        Some("guardrail")
    );
    let unlocked = dispatch(
        &state,
        "edit.track_lock",
        json!({"track":"a1t","on":false}),
        test_actor(),
    )
    .await;
    assert!(unlocked.ok, "{:?}", unlocked.error);

    let independent = dispatch(
        &state,
        "edit.trim",
        json!({"clip":"c1","src_out_ms":4000,"linked":false}),
        test_actor(),
    )
    .await;
    assert!(independent.ok, "{:?}", independent.error);
    let independent_result = independent.result.unwrap();
    assert_eq!(independent_result["linked"], false);
    assert!(independent_result["linked_clip"].is_null());
    let guard = state.project.read().await;
    let project = &guard.as_ref().unwrap().project;
    let source_range = |clip_id: &str| {
        let (track_id, index) = project.find_clip(clip_id).unwrap();
        let cut_core::Clip::Media(clip) = &project.track(track_id).unwrap().clips[index] else {
            panic!("{clip_id} must be media")
        };
        [clip.src_in_ms, clip.src_out_ms]
    };
    assert_eq!(source_range("c1"), [1000, 4000]);
    assert_eq!(source_range("c2"), [1000, 5000]);
    drop(guard);

    let no_exact_counterpart = dispatch(
        &state,
        "edit.trim",
        json!({"clip":"c1","src_out_ms":3500}),
        test_actor(),
    )
    .await;
    assert!(no_exact_counterpart.ok, "{:?}", no_exact_counterpart.error);
    let no_exact_counterpart_result = no_exact_counterpart.result.unwrap();
    assert_eq!(no_exact_counterpart_result["linked"], false);
    assert!(no_exact_counterpart_result["linked_clip"].is_null());
}

#[tokio::test]
async fn paste_attributes_video_volume_skips_gain_but_keeps_visual_fade() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"paste-video","dir":dir.path().join("paste-video.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"fixture").unwrap();
    let imported = dispatch(&state, "media.import", json!({"path":media}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);

    for at_ms in [0, 5_000, 10_000] {
        let inserted = dispatch(
            &state,
            "edit.insert",
            json!({"asset":"a1","track":"v1","at_ms":at_ms,"src_range_ms":[0,5000],"ripple":false}),
            test_actor(),
        )
        .await;
        assert!(inserted.ok, "{:?}", inserted.error);
    }
    let faded = dispatch(
        &state,
        "edit.fade",
        json!({"clip":"c1","in_ms":250,"out_ms":400,"kind":"video"}),
        test_actor(),
    )
    .await;
    assert!(faded.ok, "{:?}", faded.error);

    let pasted = dispatch(
        &state,
        "edit.paste_attributes",
        json!({"from_clip":"c1","to_clips":["c2"],"which":["volume"]}),
        test_actor(),
    )
    .await;
    assert!(pasted.ok, "{:?}", pasted.error);
    let result = pasted.result.unwrap();
    assert_eq!(result["status"], "ok");
    assert_eq!(result["applied"][0]["step"], "fade");
    assert!(result["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().is_some_and(|text| text.contains("gain"))));

    let project = state.project.read().await;
    let store = project.as_ref().unwrap();
    let target = store
        .project
        .track("v1")
        .unwrap()
        .clips
        .iter()
        .find_map(|clip| match clip {
            cut_core::Clip::Media(media) if media.id == "c2" => Some(media),
            _ => None,
        })
        .unwrap();
    assert_eq!(target.gain_db, 0.0);
    assert_eq!(target.fade.as_ref().map(|fade| fade.in_ms), Some(250));
    assert_eq!(target.fade.as_ref().map(|fade| fade.out_ms), Some(400));
    drop(project);

    let before_ops = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .len();
    let all_skipped = dispatch(
        &state,
        "edit.paste_attributes",
        json!({"from_clip":"c3","to_clips":["c2"],"which":["volume"]}),
        test_actor(),
    )
    .await;
    assert!(all_skipped.ok, "{:?}", all_skipped.error);
    let skipped_result = all_skipped.result.unwrap();
    assert_eq!(skipped_result["status"], "ok");
    assert_eq!(skipped_result["applied"], json!([]));
    assert!(skipped_result["checkpoint"].is_null());
    let after_ops = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(after_ops, before_ops, "all-skipped paste is read-only");

    let inserted_audio = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"a1t","at_ms":0,"src_range_ms":[0,5000],"ripple":false}),
        test_actor(),
    )
    .await;
    assert!(inserted_audio.ok, "{:?}", inserted_audio.error);
    let audio_clip_id = inserted_audio.result.unwrap()["clip_id"]
        .as_str()
        .unwrap()
        .to_string();
    let gained = dispatch(
        &state,
        "edit.gain",
        json!({"clip":audio_clip_id,"db":-5.0}),
        test_actor(),
    )
    .await;
    assert!(gained.ok, "{:?}", gained.error);
    let before_cross_kind_ops = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .len();
    let cross_kind = dispatch(
        &state,
        "edit.paste_attributes",
        json!({"from_clip":audio_clip_id,"to_clips":["c3"],"which":["volume"]}),
        test_actor(),
    )
    .await;
    assert!(cross_kind.ok, "{:?}", cross_kind.error);
    let cross_kind_result = cross_kind.result.unwrap();
    assert_eq!(cross_kind_result["status"], "ok");
    assert_eq!(cross_kind_result["applied"], json!([]));
    assert!(cross_kind_result["checkpoint"].is_null());
    assert!(cross_kind_result["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item.as_str().is_some_and(|text| text.contains("c3"))));
    let after_cross_kind_ops = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(
        after_cross_kind_ops, before_cross_kind_ops,
        "cross-kind gain-only paste is read-only"
    );
}

#[tokio::test]
async fn import_otio_preserves_project_format_and_commits_one_op() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let otio = dir.path().join("bad-format.otio");
    std::fs::write(
        &otio,
        serde_json::to_string(&json!({
            "OTIO_SCHEMA": "Timeline.1",
            "metadata": {"shellx_cut": {"width": 0, "height": 1080, "fps": 30.0}},
            "global_start_time": {"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":0},
            "tracks": {
                "OTIO_SCHEMA": "Stack.1",
                "children": [{
                    "OTIO_SCHEMA": "Track.1",
                    "name": "v1",
                    "kind": "Video",
                    "children": [{
                        "OTIO_SCHEMA": "Gap.1",
                        "source_range": {
                            "OTIO_SCHEMA": "TimeRange.1",
                            "start_time": {"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":0},
                            "duration": {"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":30}
                        }
                    }]
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let r = dispatch(&state, "import.otio", json!({"path": otio}), test_actor()).await;

    assert!(r.ok, "{:?}", r.error);
    assert_eq!(
        r.op_ids.as_ref().map_or(0, Vec::len),
        1,
        "timeline replacement is one op"
    );
    let res = r.result.unwrap();
    assert_eq!(res["status"], "imported");
    assert_eq!(res["format_policy"], "preserve_project");
    assert!(
        res["source_format"].is_null(),
        "invalid metadata is not adopted"
    );
    let state_now = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(state_now["settings"]["width"], 1920);
    assert_eq!(state_now["settings"]["height"], 1080);
    let ops = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(
        ops.iter().filter(|op| op["verb"] == "import.otio").count(),
        1
    );
    assert!(!ops.iter().any(|op| op["verb"] == "project.checkpoint"));
}

#[tokio::test]
async fn import_otio_preview_is_read_only_and_hash_binds_replace() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir":dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let before_ops = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .len();
    let otio = dir.path().join("missing-media.otio");
    let document = |name: &str| {
        json!({
            "OTIO_SCHEMA":"Timeline.1",
            "name":name,
            "global_start_time":{"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":0},
            "tracks":{"OTIO_SCHEMA":"Stack.1","children":[{
                "OTIO_SCHEMA":"Track.1","name":"Picture 1","kind":"Video","children":[{
                    "OTIO_SCHEMA":"Clip.1","name":"offline",
                    "source_range":{
                        "OTIO_SCHEMA":"TimeRange.1",
                        "start_time":{"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":0},
                        "duration":{"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":60}
                    },
                    "media_reference":{
                        "OTIO_SCHEMA":"ExternalReference.1",
                        "target_url":"file:///definitely/missing/offline.mov"
                    }
                }]}
            ]}
        })
    };
    std::fs::write(&otio, serde_json::to_vec(&document("Preview A")).unwrap()).unwrap();

    let preview = dispatch(
        &state,
        "import.otio",
        json!({"path":otio,"mode":"preview"}),
        test_actor(),
    )
    .await;
    assert!(preview.ok, "{:?}", preview.error);
    assert!(preview.op_ids.as_ref().is_none_or(Vec::is_empty));
    let preview_result = preview.result.unwrap();
    assert_eq!(preview_result["status"], "preview");
    assert_eq!(preview_result["track_count"], 1);
    assert_eq!(preview_result["clips"], 1);
    assert_eq!(preview_result["media_missing"], 1);
    let first_hash = preview_result["source_hash"].as_str().unwrap().to_string();
    let after_preview_ops = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(
        after_preview_ops, before_ops,
        "preview commits no operation"
    );

    std::fs::write(&otio, serde_json::to_vec(&document("Preview B")).unwrap()).unwrap();
    let changed = dispatch(
        &state,
        "import.otio",
        json!({"path":otio,"mode":"replace","expected_hash":first_hash}),
        test_actor(),
    )
    .await;
    assert!(!changed.ok);
    assert_eq!(changed.error.unwrap().code, error_codes::CONFLICT);
    let after_conflict_ops = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(
        after_conflict_ops, before_ops,
        "hash conflict commits nothing"
    );

    let preview_again = dispatch(
        &state,
        "import.otio",
        json!({"path":otio,"mode":"preview"}),
        test_actor(),
    )
    .await
    .result
    .unwrap();
    let imported = dispatch(
        &state,
        "import.otio",
        json!({
            "path":otio,
            "mode":"replace",
            "expected_hash":preview_again["source_hash"],
            "rationale":"confirmed OTIO preview"
        }),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    assert_eq!(imported.op_ids.as_ref().map_or(0, Vec::len), 1);
    assert_eq!(imported.result.unwrap()["missing_clips"], 1);
    assert_eq!(
        imported.warnings.as_ref().unwrap()[0].code,
        "otio_media_missing"
    );
}

#[tokio::test]
async fn import_otio_preflights_real_media_and_replays_exact_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("t.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir":project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = dir.path().join("linked.mp4");
    let bundled_media = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("first-edit-sample.mp4");
    std::fs::copy(&bundled_media, &media).expect("bundled first-edit media must be readable");
    let otio = dir.path().join("linked.otio");
    std::fs::write(
        &otio,
        serde_json::to_vec(&json!({
            "OTIO_SCHEMA":"Timeline.1",
            "name":"Linked",
            "global_start_time":{"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":0},
            "tracks":{"OTIO_SCHEMA":"Stack.1","children":[{
                "OTIO_SCHEMA":"Track.1","name":"Picture 1","kind":"Video","children":[{
                    "OTIO_SCHEMA":"Clip.2","name":"linked",
                    "source_range":{
                        "OTIO_SCHEMA":"TimeRange.1",
                        "start_time":{"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":0},
                        "duration":{"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":30}
                    },
                    "media_references":{
                        "DEFAULT_MEDIA":{
                            "OTIO_SCHEMA":"ExternalReference.1",
                            "target_url":"linked.mp4"
                        }
                    },
                    "active_media_reference_key":"DEFAULT_MEDIA"
                }]}
            ]}
        }))
        .unwrap(),
    )
    .unwrap();
    let preview = dispatch(
        &state,
        "import.otio",
        json!({"path":otio,"mode":"preview"}),
        test_actor(),
    )
    .await;
    assert!(preview.ok, "{:?}", preview.error);
    let preview = preview.result.unwrap();
    assert_eq!(preview["media_available"], 1);
    assert_eq!(preview["media_missing"], 0);
    let imported = dispatch(
        &state,
        "import.otio",
        json!({
            "path":otio,
            "mode":"replace",
            "expected_hash":preview["source_hash"],
            "rationale":"real linked-media import test"
        }),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    assert!(imported.warnings.is_none());
    assert_eq!(imported.op_ids.as_ref().map_or(0, Vec::len), 1);
    let result = imported.result.unwrap();
    assert_eq!(result["clips_inserted"], 1);
    assert_eq!(result["assets_imported"], 1);

    let (project, ops) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        (store.project.clone(), store.log.read_all().unwrap())
    };
    assert_eq!(project.tracks[0].id, "Picture1");
    assert_eq!(project.tracks[0].clips.len(), 1);
    assert!(project.assets["a1"].probe.is_some());
    let rebuilt = cut_core::rebuild_from_log(&ops).unwrap();
    assert_eq!(rebuilt.tracks, project.tracks);
    assert_eq!(rebuilt.assets["a1"].path, project.assets["a1"].path);
}

#[tokio::test]
async fn import_otio_replaces_complex_timeline_atomically_and_undoes() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,5000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "edit.speed_ramp",
        json!({
            "clip":"c1",
            "points":[
                {"at_ms":0,"factor":1.0},
                {"at_ms":2500,"factor":2.0},
                {"at_ms":5000,"factor":1.0}
            ],
            "segments":12
        }),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let otio = dir.path().join("gap.otio");
    std::fs::write(
        &otio,
        serde_json::to_string(&json!({
            "OTIO_SCHEMA": "Timeline.1",
            "global_start_time": {"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":0},
            "tracks": {
                "OTIO_SCHEMA": "Stack.1",
                "children": [{
                    "OTIO_SCHEMA": "Track.1",
                    "name": "v1",
                    "kind": "Video",
                    "children": [{
                        "OTIO_SCHEMA": "Gap.1",
                        "source_range": {
                            "OTIO_SCHEMA": "TimeRange.1",
                            "start_time": {"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":0},
                            "duration": {"OTIO_SCHEMA":"RationalTime.1","rate":30.0,"value":30}
                        }
                    }]
                }]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let r = dispatch(&state, "import.otio", json!({"path": otio}), test_actor()).await;

    assert!(r.ok, "{:?}", r.error);
    assert_eq!(
        r.op_ids.as_ref().map_or(0, Vec::len),
        1,
        "timeline replacement is one op"
    );
    let res = r.result.unwrap();
    assert_eq!(res["status"], "imported");
    let imported = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    let clips = imported["tracks"][0]["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0]["kind"], "gap");

    let undone = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undone.ok, "{:?}", undone.error);
    let restored = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    let restored_clip = &restored["tracks"][0]["clips"][0];
    assert_eq!(restored_clip["asset"], "a1");
    assert!(restored_clip["speed_ramp"].is_object());
}

#[tokio::test]
async fn generate_storyboard_insert_error_preserves_partial_ops() {
    let _guard = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"story","dir": dir.path().join("story.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let adapter = dir.path().join("storyboard_adapter.py");
    std::fs::write(
        &adapter,
        r##"
import json, sys
sys.stdin.read()
print(json.dumps({
  "schema": "shellx-cut/generate-storyboard-result/1",
  "status": "completed",
  "backend": {"provider": "stub", "model": "stub/storyboard"},
  "questions": [],
  "warnings": [],
  "storyboard": {
    "schema": "shellx-cut/generate-storyboard/1",
    "storyboard_id": "partial-test",
    "mode": "quick_prompt",
    "status": "valid",
    "scenes": [
      {
        "scene_id": "s1",
        "index": 1,
        "role": "lower-third",
        "source": "generate_template",
        "template_id": "builtin.lower-third.clean",
        "range_ms": [0, 4000],
        "params": {"name": "First", "accent": "#FFD24A"}
      },
      {
        "scene_id": "s2",
        "index": 2,
        "role": "lower-third",
        "source": "generate_template",
        "template_id": "builtin.lower-third.clean",
        "range_ms": [4000, 8000],
        "params": {"accent": "#FFD24A"}
      }
    ]
  }
}))
"##,
    )
    .unwrap();

    let old_adapter = std::env::var_os("CUTD_GENERATE_STORYBOARD_ADAPTER");
    let old_python = std::env::var_os(ENV_ADAPTER_PYTHON);
    std::env::set_var("CUTD_GENERATE_STORYBOARD_ADAPTER", &adapter);
    std::env::set_var(ENV_ADAPTER_PYTHON, "python3");

    let r = dispatch(
        &state,
        "generate.storyboard",
        json!({"input":"partial storyboard", "mode":"quick_prompt", "policy":"insert"}),
        test_actor(),
    )
    .await;

    match old_adapter {
        Some(value) => std::env::set_var("CUTD_GENERATE_STORYBOARD_ADAPTER", value),
        None => std::env::remove_var("CUTD_GENERATE_STORYBOARD_ADAPTER"),
    }
    match old_python {
        Some(value) => std::env::set_var(ENV_ADAPTER_PYTHON, value),
        None => std::env::remove_var(ENV_ADAPTER_PYTHON),
    }

    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["status"], "error");
    assert!(
        res["insert"]["op_ids"]
            .as_array()
            .is_some_and(|ids| !ids.is_empty()),
        "partial insert op_ids must survive scene failure: {res}"
    );
    assert_eq!(res["insert"]["scenes"].as_array().map(Vec::len), Some(1));
    assert_eq!(res["insert"]["failed_scene"], "s2");
    assert!(res["insert"]["restore_hint"]
        .as_str()
        .unwrap_or("")
        .contains("project.revert"));
}

/// Historic snapshot-era records can still be rendered without their legacy
/// inverse by default; the deprecated compatibility option retains it only for
/// callers that explicitly ask to inspect such a record. Fresh records never
/// take this path because recompute-by-replay omits inverse payloads.
#[test]
fn op_for_result_strips_legacy_inverse_unless_compat_requested() {
    // An artificial historic op WITH an inverse.
    let op = OpRecord {
        op_id: "op_000002".into(),
        ts: OpRecord::now_ts(),
        actor: test_actor(),
        verb: "edit.crop".into(),
        args: json!({"clip": "c1"}),
        rationale: None,
        effects: vec![],
        inverse: Some(InverseOp {
            verb: "edit._set_timeline".into(),
            args: json!({"tracks": "…a big snapshot…"}),
        }),
        status: cut_core::OpStatus::Applied,
    };
    // Default: inverse stripped, marker left so a reader can tell
    // "trimmed" from "no inverse".
    let trimmed = op_for_result(&op, false);
    assert!(
        trimmed.get("inverse").is_none(),
        "inverse dropped by default"
    );
    assert_eq!(trimmed["inverse_omitted"], json!(true), "omission marked");
    assert_eq!(trimmed["op_id"], "op_000002", "the rest of the op survives");
    // Compatibility opt-in: historic inverse preserved, no marker.
    let full = op_for_result(&op, true);
    assert!(
        full.get("inverse").is_some(),
        "deprecated include_inverse preserves a historic payload"
    );
    assert!(full.get("inverse_omitted").is_none(), "no marker when kept");
    // A current inverse-free op is untouched — no marker.
    let mut import_op = op.clone();
    import_op.inverse = None;
    let imp = op_for_result(&import_op, false);
    assert!(imp.get("inverse").is_none());
    assert!(
        imp.get("inverse_omitted").is_none(),
        "no marker — there was nothing to drop"
    );
}

/// The deprecated compatibility option remains strict: only boolean true asks
/// to retain an already historic payload; the public schema rejects stringly
/// booleans before dispatch.
#[test]
fn wants_legacy_inverse_parsing() {
    assert!(wants_legacy_inverse(&json!({"include_inverse": true})));
    assert!(!wants_legacy_inverse(&json!({})));
    assert!(!wants_legacy_inverse(&json!({"include_inverse": false})));
    assert!(!wants_legacy_inverse(&json!({"include_inverse": "true"})));
    assert!(!wants_legacy_inverse(&json!({"include_inverse": 1})));
    assert!(!wants_legacy_inverse(&json!({"include_inverse": "no"})));
}

/// Recompute-by-replay model: a NEW op carries NO snapshot
/// inverse at all (that per-op full-timeline copy was the O(N²) disk
/// growth), yet the engine still undoes it via edit.restore — which now
/// recomputes the pre-op timeline by replaying the log prefix. Because there
/// is no stored inverse to drop, there is no `inverse_omitted` marker, and
/// include_inverse:true remains a deprecated compatibility no-op for fresh
/// records. The legacy rendering path is separately covered by the synthetic
/// historic op test above.
#[tokio::test]
async fn add_marker_result_has_no_inverse_but_restore_works() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    // New op: no inverse stored (recompute model), so no omission marker.
    let r = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms":100,"label":"x"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    let op = &res["op"];
    assert!(
        op["inverse"].is_null(),
        "new ops carry no legacy inverse: {op}"
    );
    assert!(
        op.get("inverse_omitted").is_none(),
        "nothing was omitted — there is no inverse to drop"
    );
    let marker_op_id = op["op_id"].as_str().unwrap().to_string();

    // The engine still undoes it — proof undo is recomputed from the op log,
    // not read from a stored snapshot. add_marker is the tip op, so a
    // default tip-restore applies.
    let r = dispatch(
        &state,
        "edit.restore",
        json!({"op_id": marker_op_id}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "restore must work via recompute: {:?}", r.error);

    // include_inverse:true has nothing extra to echo for a new op — it is a
    // compatibility no-op under the recompute model.
    let r = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms":200,"label":"y","include_inverse":true}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let op = &r.result.unwrap()["op"];
    assert!(
        op["inverse"].is_null(),
        "no snapshot exists to include: {op}"
    );
}

/// comment.draft spawns the drafting adapter and stores the
/// proposed change set on the comment, validates the verbs against the real
/// registry (unknown verbs flagged, never trusted), and is honest when no
/// backend is available (not_run, no draft stored). Uses a stub adapter via
/// CUTD_DRAFT_ADAPTER — no live CLI / no quota.
#[tokio::test]
async fn comment_draft_fake_adapter_stores_validated_draft() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"cm","dir": dir.path().join("cm.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "comment.add",
        json!({"at_ms":1000,"text":"tighten the intro"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let cmid = r.result.unwrap()["comment"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Stub adapter: ignores stdin, prints a canned completed envelope whose
    // draft references a REAL verb (edit.ripple_delete) so validation passes.
    let stub = dir.path().join("draft_stub.py");
    std::fs::write(&stub, "import sys, json\nsys.stdin.read()\nprint(json.dumps({\"schema\":\"shellx-cut/comment-draft/1\",\"status\":\"completed\",\"backend\":{\"provider\":\"stub\",\"model\":\"stub/x\"},\"draft\":{\"verbs\":[{\"verb\":\"edit.ripple_delete\",\"args\":{\"range_ms\":[800,1200]}}],\"rationale\":\"remove the dead air\",\"confidence\":0.8},\"reason\":None}))\n").unwrap();
    std::env::set_var("CUTD_DRAFT_ADAPTER", &stub);
    let r = dispatch(
        &state,
        "comment.draft",
        json!({"comment_id": cmid}),
        test_actor(),
    )
    .await;
    std::env::remove_var("CUTD_DRAFT_ADAPTER");
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["status"], "completed");
    assert_eq!(res["draft"]["verbs"][0]["verb"], "edit.ripple_delete");
    assert_eq!(res["draft"]["validation"]["ok"], true);
    assert_eq!(res["draft"]["confidence"], 0.8);
    // The draft persisted on the comment (read back via comment.list).
    let r = dispatch(&state, "comment.list", json!({}), test_actor()).await;
    let c = r.result.unwrap();
    assert_eq!(
        c["comments"][0]["draft"]["verbs"][0]["verb"],
        "edit.ripple_delete"
    );

    // An UNKNOWN verb in a draft is flagged invalid (model never trusted).
    let stub2 = dir.path().join("draft_stub2.py");
    std::fs::write(&stub2, "import sys, json\nsys.stdin.read()\nprint(json.dumps({\"status\":\"completed\",\"backend\":{\"provider\":\"stub\"},\"draft\":{\"verbs\":[{\"verb\":\"edit.teleport\",\"args\":{}}],\"rationale\":\"x\",\"confidence\":0.5},\"reason\":None}))\n").unwrap();
    std::env::set_var("CUTD_DRAFT_ADAPTER", &stub2);
    let r = dispatch(
        &state,
        "comment.draft",
        json!({"comment_id": cmid}),
        test_actor(),
    )
    .await;
    std::env::remove_var("CUTD_DRAFT_ADAPTER");
    let v = r.result.unwrap();
    assert_eq!(
        v["draft"]["validation"]["ok"], false,
        "unknown verb must be flagged"
    );
    assert_eq!(
        v["draft"]["validation"]["invalid"][0]["verb"],
        "edit.teleport"
    );

    // No adapter → honest not_run, no draft stored / no op.
    std::env::set_var("CUTD_DRAFT_ADAPTER", "/nonexistent/draft_adapter.py");
    let r = dispatch(
        &state,
        "comment.draft",
        json!({"comment_id": cmid}),
        test_actor(),
    )
    .await;
    std::env::remove_var("CUTD_DRAFT_ADAPTER");
    assert_eq!(r.result.unwrap()["status"], "not_run");

    // Unknown comment id → not_found.
    std::env::set_var("CUTD_DRAFT_ADAPTER", &stub);
    let r = dispatch(
        &state,
        "comment.draft",
        json!({"comment_id":"cm99"}),
        test_actor(),
    )
    .await;
    std::env::remove_var("CUTD_DRAFT_ADAPTER");
    assert_eq!(r.error.unwrap().code, "not_found");
}

#[tokio::test]
async fn comment_add_rejects_inverted_range() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"cm","dir": dir.path().join("cm.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "comment.add",
        json!({"at_ms":2000,"end_ms":1000,"text":"bad range"}),
        test_actor(),
    )
    .await;
    assert_eq!(r.error.unwrap().code, error_codes::INVALID_ARGS);
}

#[tokio::test]
async fn plugins_call_propagates_inner_failure_and_records_plugin_actor() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();

    let missing_project = dispatch(
        &state,
        "plugins.call",
        json!({
            "plugin": "openverse-assets",
            "verb": "assets.fetch",
            "args": {"provider":"local_folder","id":"/definitely/missing.wav","kind":"audio"}
        }),
        test_actor(),
    )
    .await;
    assert!(!missing_project.ok);

    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"pl","dir": dir.path().join("pl.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let media = dir.path().join("tone.wav");
    std::fs::write(&media, b"RIFF....WAVE").unwrap();
    let r = dispatch(
        &state,
        "plugins.call",
        json!({
            "plugin": "openverse-assets",
            "verb": "assets.fetch",
            "args": {"provider":"local_folder","id": media, "kind":"audio", "dir": dir.path()}
        }),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let ops = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap();
    let last = ops["ops"].as_array().unwrap().last().unwrap();
    assert_eq!(last["actor"]["name"], "plugin:openverse-assets");
    assert_eq!(last["actor"]["via"], "plugins.call/test");
}

#[tokio::test]
async fn assets_fetch_local_folder_requires_search_dir_scope() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"lf","dir": dir.path().join("lf.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let search_dir = dir.path().join("search");
    let outside_dir = dir.path().join("outside");
    std::fs::create_dir_all(&search_dir).unwrap();
    std::fs::create_dir_all(&outside_dir).unwrap();
    let inside = search_dir.join("inside.wav");
    let outside = outside_dir.join("outside.wav");
    std::fs::write(&inside, b"RIFF....WAVE").unwrap();
    std::fs::write(&outside, b"RIFF....WAVE").unwrap();

    let missing_dir = dispatch(
        &state,
        "assets.fetch",
        json!({"provider":"local_folder","id": inside, "kind":"audio"}),
        test_actor(),
    )
    .await;
    assert_eq!(missing_dir.error.unwrap().code, error_codes::INVALID_ARGS);

    let escaped = dispatch(
        &state,
        "assets.fetch",
        json!({"provider":"local_folder","id": outside, "kind":"audio", "dir": search_dir}),
        test_actor(),
    )
    .await;
    assert_eq!(escaped.error.unwrap().code, error_codes::INVALID_ARGS);

    let accepted = dispatch(
        &state,
        "assets.fetch",
        json!({"provider":"local_folder","id": inside, "kind":"audio", "dir": search_dir}),
        test_actor(),
    )
    .await;
    assert!(accepted.ok, "{:?}", accepted.error);
    assert_eq!(accepted.result.unwrap()["provider"], "local_folder");
}

/// comment.apply executes the drafted verbs (each a real op with
/// the comment as rationale), wraps them in an auto-checkpoint, returns the
/// review diff, marks the comment addressed — and the whole apply reverts in
/// one step. Also: apply without a draft refuses honestly.
#[tokio::test]
async fn comment_apply_executes_draft_checkpoint_and_reverts() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"ap","dir": dir.path().join("ap.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let r = dispatch(
        &state,
        "comment.add",
        json!({"at_ms":2000,"text":"mark the intro beat"}),
        test_actor(),
    )
    .await;
    let cmid = r.result.unwrap()["comment"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Draft a REAL applicable verb (edit.add_marker works on any project).
    let stub = dir.path().join("ap_stub.py");
    std::fs::write(&stub, "import sys,json\nsys.stdin.read()\nprint(json.dumps({\"status\":\"completed\",\"backend\":{\"provider\":\"stub\"},\"draft\":{\"verbs\":[{\"verb\":\"edit.add_marker\",\"args\":{\"at_ms\":2000,\"label\":\"intro\"}}],\"rationale\":\"mark it\",\"confidence\":0.9},\"reason\":None}))\n").unwrap();
    std::env::set_var("CUTD_DRAFT_ADAPTER", &stub);
    let r = dispatch(
        &state,
        "comment.draft",
        json!({"comment_id": cmid}),
        test_actor(),
    )
    .await;
    std::env::remove_var("CUTD_DRAFT_ADAPTER");
    assert_eq!(r.result.unwrap()["status"], "completed");

    // APPLY — execute the draft.
    let r = dispatch(
        &state,
        "comment.apply",
        json!({"comment_id": cmid}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["status"], "addressed");
    assert_eq!(res["applied"][0]["verb"], "edit.add_marker");
    assert_eq!(res["applied"][0]["ok"], true);
    let checkpoint = res["checkpoint"].as_str().unwrap().to_string();
    assert!(
        checkpoint.starts_with("cp"),
        "auto-checkpoint returned: {checkpoint}"
    );
    assert!(
        res["diff"].is_object(),
        "diff present as the review artifact"
    );

    // The marker was actually added + the comment marked addressed.
    let st = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(st["markers"].as_array().unwrap().len(), 1);
    assert_eq!(st["markers"][0]["label"], "intro");
    let cl = dispatch(&state, "comment.list", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(cl["comments"][0]["status"], "addressed");

    // One-click revert undoes the whole apply (the marker is gone).
    let rv = dispatch(
        &state,
        "project.revert",
        json!({"to": checkpoint}),
        test_actor(),
    )
    .await;
    assert!(rv.ok, "{:?}", rv.error);
    let st = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(
        st["markers"].as_array().unwrap().len(),
        0,
        "revert undid the applied marker"
    );

    // Apply without a draft → honest refusal.
    let r2 = dispatch(
        &state,
        "comment.add",
        json!({"at_ms":0,"text":"no draft yet"}),
        test_actor(),
    )
    .await;
    let cm2 = r2.result.unwrap()["comment"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = dispatch(
        &state,
        "comment.apply",
        json!({"comment_id": cm2}),
        test_actor(),
    )
    .await;
    assert_eq!(
        r.error.unwrap().code,
        "invalid_args",
        "apply without a draft must refuse"
    );
}

#[tokio::test]
async fn autopilot_run_surfaces_comment_apply_failure_before_job() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"ap","dir": dir.path().join("ap.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let r = dispatch(
        &state,
        "autopilot.run",
        json!({"comment_id":"cm_missing","policy":"preview"}),
        test_actor(),
    )
    .await;
    let e = r
        .error
        .expect("autopilot must not ignore comment.apply failure");
    assert_eq!(e.code, error_codes::NOT_FOUND);
    assert!(
        e.message.contains("comment"),
        "error should surface comment.apply failure: {e:?}"
    );
}

/// assets.generate queues the user's generation CLI and then imports the written
/// file through the normal media.import path. media.import is
/// strict about its args, so assets.generate must not pass private metadata
/// like `source` into that sub-call; generated provenance belongs on the
/// outer result.
#[tokio::test]
async fn assets_generate_imports_generated_file_without_extra_media_import_args() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project = dir.path().join("gen.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"gen","dir": project}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    // Real PNG fixture, generated the same way as the still-image import
    // regression above, then copied by the fake codex CLI to the requested path.
    let fixture = dir.path().join("fixture.png");
    let ffmpeg = std::env::var("SHELLX_CUT_FFMPEG")
        .or_else(|_| std::env::var("FFMPEG_BIN"))
        .unwrap_or_else(|_| "ffmpeg".to_string());
    let ff = std::process::Command::new(ffmpeg)
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:s=320x240",
            "-frames:v",
            "1",
        ])
        .arg(&fixture)
        .status()
        .expect("ffmpeg present");
    assert!(ff.success());
    let variation_fixture = dir.path().join("variation.png");
    let ffmpeg = std::env::var("SHELLX_CUT_FFMPEG")
        .or_else(|_| std::env::var("FFMPEG_BIN"))
        .unwrap_or_else(|_| "ffmpeg".to_string());
    let variation_ff = std::process::Command::new(ffmpeg)
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=red:s=320x240",
            "-frames:v",
            "1",
        ])
        .arg(&variation_fixture)
        .status()
        .expect("ffmpeg present");
    assert!(variation_ff.success());

    let invocations = dir.path().join("codex-invocations.txt");
    let _fake_cli = generation_cli::FakeGenerationCli::install(
        dir.path(),
        generation_cli::FakeGenerationCliConfig::copying(fixture.clone())
            .with_variation("reference variation", variation_fixture.clone())
            .require_reference_for("reference variation")
            .log_invocations(invocations.clone()),
    );
    let queued = dispatch(
        &state,
        "assets.generate",
        json!({"prompt":"blue card","provider":"codex","kind":"image","timeout_ms":30000}),
        test_actor(),
    )
    .await;
    assert!(queued.ok, "{:?}", queued.error);
    let queued_result = queued.result.unwrap();
    assert_eq!(queued_result["state"], "queued");
    let first_job_id = queued_result["job_id"].as_str().unwrap();
    let early_generation_id = queued_result["generation_id"].as_str().unwrap();
    let first_job = wait_job(&state, first_job_id, 30).await;
    assert_eq!(
        first_job.state,
        crate::jobs::JobState::Done,
        "{:?}",
        first_job.error
    );
    let res = first_job.result.unwrap();

    let reused_queued = dispatch(
        &state,
        "assets.generate",
        json!({"prompt":"blue card","provider":"codex","kind":"image","timeout_ms":30000}),
        test_actor(),
    )
    .await;
    assert!(reused_queued.ok, "{:?}", reused_queued.error);
    let reused_job_id = reused_queued.result.as_ref().unwrap()["job_id"]
        .as_str()
        .unwrap();
    let reused_job = wait_job(&state, reused_job_id, 30).await;
    assert_eq!(
        reused_job.state,
        crate::jobs::JobState::Done,
        "{:?}",
        reused_job.error
    );
    let reused = reused_job.result.unwrap();

    let early_provenance_path = project
        .join("assets/generated")
        .join(format!("{early_generation_id}.json"));
    let original_provenance = std::fs::read(&early_provenance_path).unwrap();
    let mut altered_provenance: Value = serde_json::from_slice(&original_provenance).unwrap();
    altered_provenance["prompt"] = json!("different request");
    std::fs::write(
        &early_provenance_path,
        serde_json::to_vec(&altered_provenance).unwrap(),
    )
    .unwrap();
    let untrusted_reuse_queued = dispatch(
        &state,
        "assets.generate",
        json!({"prompt":"blue card","provider":"codex","kind":"image","timeout_ms":30000}),
        test_actor(),
    )
    .await;
    assert!(
        untrusted_reuse_queued.ok,
        "{:?}",
        untrusted_reuse_queued.error
    );
    let untrusted_reuse_id = untrusted_reuse_queued.result.as_ref().unwrap()["job_id"]
        .as_str()
        .unwrap();
    let untrusted_reuse = wait_job(&state, untrusted_reuse_id, 30).await;
    std::fs::write(&early_provenance_path, &original_provenance).unwrap();

    let early_generated_path = project
        .join("assets/generated")
        .join(format!("{early_generation_id}.png"));
    std::fs::write(&early_generated_path, b"tampered").unwrap();
    let tampered_queued = dispatch(
        &state,
        "assets.generate",
        json!({"prompt":"blue card","provider":"codex","kind":"image","timeout_ms":30000}),
        test_actor(),
    )
    .await;
    assert!(tampered_queued.ok, "{:?}", tampered_queued.error);
    let tampered_job_id = tampered_queued.result.as_ref().unwrap()["job_id"]
        .as_str()
        .unwrap();
    let tampered = wait_job(&state, tampered_job_id, 30).await;
    std::fs::copy(&fixture, &early_generated_path).unwrap();
    let reference_asset = res["asset_id"].as_str().unwrap();
    let variation_queued = dispatch(
        &state,
        "assets.generate",
        json!({
            "prompt":"reference variation",
            "provider":"codex",
            "kind":"image",
            "references":[reference_asset],
            "variation":"take-2",
            "timeout_ms":30000
        }),
        test_actor(),
    )
    .await;
    assert!(variation_queued.ok, "{:?}", variation_queued.error);
    let variation_job_id = variation_queued.result.as_ref().unwrap()["job_id"]
        .as_str()
        .unwrap();
    let variation_job = wait_job(&state, variation_job_id, 30).await;
    assert_eq!(
        variation_job.state,
        crate::jobs::JobState::Done,
        "{:?}",
        variation_job.error
    );
    let variation = variation_job.result.unwrap();
    assert_ne!(res["ok"], false, "assets.generate degraded: {res}");
    assert!(res["asset_id"].as_str().is_some(), "asset imported: {res}");
    assert_eq!(res["generated"]["provider"], "codex");
    assert_eq!(res["generated"]["kind"], "image");
    assert_eq!(res["generated"]["schema"], "shellx-cut/generated-asset/2");
    assert_eq!(res["generated"]["references"], json!([]));
    assert_eq!(res["generated"]["reused"], false);
    let generation_id = res["generated"]["generation_id"].as_str().unwrap();
    assert_eq!(generation_id.len(), 24);
    assert_eq!(res["generated"]["cost_usd"], Value::Null);
    let generated_path = project
        .join("assets/generated")
        .join(format!("{generation_id}.png"));
    let provenance_path = project
        .join("assets/generated")
        .join(format!("{generation_id}.json"));
    assert!(generated_path.is_file(), "generated source is durable");
    assert!(provenance_path.is_file(), "provenance sidecar is durable");
    let canonical_project = project.canonicalize().unwrap();
    let canonical_provenance = provenance_path.canonicalize().unwrap();
    assert!(canonical_provenance.starts_with(&canonical_project));
    let provenance: Value =
        serde_json::from_slice(&std::fs::read(canonical_provenance).unwrap()).unwrap();
    assert_eq!(provenance["generation_id"], generation_id);
    assert_eq!(provenance["prompt"], "blue card");
    assert!(provenance["content_hash"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));

    assert_ne!(variation["generated"]["generation_id"], generation_id);
    assert_ne!(variation["generated"]["family_id"], generation_id);
    assert_eq!(variation["generated"]["variation"], "take-2");
    assert_eq!(
        variation["generated"]["references"][0]["asset_id"],
        reference_asset
    );

    let history = dispatch(
        &state,
        "assets.generated_list",
        json!({"limit":10}),
        test_actor(),
    )
    .await;
    assert!(history.ok, "{:?}", history.error);
    let history = history.result.unwrap();
    assert_eq!(history["total"], 2);
    assert_eq!(history["verified"], 2);
    assert_eq!(history["items"].as_array().unwrap().len(), 2);
    let history_text = serde_json::to_string(&history).unwrap();
    assert!(!history_text.contains(project.to_string_lossy().as_ref()));
    assert!(!history_text.contains("provenance_path"));

    assert_eq!(reused["asset_id"], res["asset_id"]);
    assert_eq!(reused["generated"]["reused"], true);
    assert_eq!(untrusted_reuse.state, crate::jobs::JobState::Failed);
    assert!(untrusted_reuse
        .error
        .as_ref()
        .unwrap()
        .message
        .contains("provenance"));
    assert_eq!(tampered.state, crate::jobs::JobState::Failed);
    assert_eq!(tampered.error.as_ref().unwrap().code, "generation_failed");
    assert!(tampered
        .error
        .as_ref()
        .unwrap()
        .message
        .contains("changed after import"));
    assert_eq!(
        std::fs::read_to_string(invocations)
            .unwrap()
            .lines()
            .count(),
        2,
        "the unchanged base request reuses while the explicit variation runs once"
    );
    let runs = project.join("cache/gen/runs");
    assert!(
        !runs.exists() || std::fs::read_dir(runs).unwrap().next().is_none(),
        "generation scratch directories are cleaned"
    );
}

#[tokio::test]
async fn assets_generate_reserves_replaces_and_retains_pending_timeline_slots() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_dir = dir.path().join("gen-placement.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"gen-placement","dir": project_dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let fixture = dir.path().join("generated.png");
    let ffmpeg = std::env::var("SHELLX_CUT_FFMPEG")
        .or_else(|_| std::env::var("FFMPEG_BIN"))
        .unwrap_or_else(|_| "ffmpeg".to_string());
    let rendered = std::process::Command::new(ffmpeg)
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=green:s=320x240",
            "-frames:v",
            "1",
        ])
        .arg(&fixture)
        .status()
        .expect("ffmpeg present");
    assert!(rendered.success());

    let _fake_cli = generation_cli::FakeGenerationCli::install(
        dir.path(),
        generation_cli::FakeGenerationCliConfig::copying(fixture.clone())
            .with_default_delay_ms(1_000)
            .with_extra_delay_if_prompt("cancelled slot", 30_000),
    );

    let queued = dispatch(
        &state,
        "assets.generate",
        json!({
            "prompt":"visible pending slot",
            "provider":"codex",
            "kind":"image",
            "timeout_ms":30000,
            "placement":{"mode":"insert","track":"v1","at_ms":0,"duration_ms":1200}
        }),
        test_actor(),
    )
    .await;
    assert!(queued.ok, "{:?}", queued.error);
    let queued = queued.result.unwrap();
    assert_eq!(queued["placement"]["state"], "pending");
    assert_eq!(queued["placement"]["mode"], "insert");
    let target_clip = queued["placement"]["target_clip"]
        .as_str()
        .unwrap()
        .to_string();
    let job_id = queued["job_id"].as_str().unwrap().to_string();

    let (placeholder_asset, placeholder_path) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&target_clip).unwrap();
        assert_eq!(track_id, "v1");
        let cut_core::Clip::Media(clip) = &store.project.track(track_id).unwrap().clips[index]
        else {
            panic!("pending slot must be a media clip")
        };
        assert_eq!(clip.src_out_ms - clip.src_in_ms, 1200);
        let path = PathBuf::from(&store.project.assets[&clip.asset].path);
        assert_eq!(
            path.parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str()),
            Some("placeholders")
        );
        (clip.asset.clone(), path)
    };
    assert!(
        placeholder_path.is_file(),
        "pending placeholder must be visible on disk"
    );

    let finished = wait_job(&state, &job_id, 30).await;
    assert_eq!(
        finished.state,
        crate::jobs::JobState::Done,
        "{:?}",
        finished.error
    );
    let result = finished.result.unwrap();
    assert_eq!(result["placement"]["state"], "applied");
    assert_eq!(result["placement"]["target_clip"], target_clip);
    assert_eq!(result["placement"]["cleanup"]["removed"], true);
    assert_eq!(result["placement"]["cleanup"]["source_deleted"], true);
    let generated_asset = result["asset_id"].as_str().unwrap().to_string();
    {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&target_clip).unwrap();
        let cut_core::Clip::Media(clip) = &store.project.track(track_id).unwrap().clips[index]
        else {
            panic!("completed slot must remain a media clip")
        };
        assert_eq!(
            clip.id, target_clip,
            "replacement preserves the reserved clip id"
        );
        assert_eq!(clip.asset, generated_asset);
        assert_eq!(clip.src_out_ms - clip.src_in_ms, 1200);
        assert!(!store.project.assets.contains_key(&placeholder_asset));
    }
    assert!(
        !placeholder_path.exists(),
        "replaced placeholder source is deleted"
    );

    let stale_target = dispatch(
        &state,
        "assets.generate",
        json!({
            "prompt":"deleted pending slot",
            "provider":"codex",
            "kind":"image",
            "timeout_ms":30000,
            "placement":{"mode":"insert","track":"v1","at_ms":1200,"duration_ms":700}
        }),
        test_actor(),
    )
    .await;
    assert!(stale_target.ok, "{:?}", stale_target.error);
    let stale_target = stale_target.result.unwrap();
    let stale_job = stale_target["job_id"].as_str().unwrap().to_string();
    let stale_clip = stale_target["placement"]["target_clip"]
        .as_str()
        .unwrap()
        .to_string();
    let stale_placeholder = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&stale_clip).unwrap();
        let cut_core::Clip::Media(clip) = &store.project.track(track_id).unwrap().clips[index]
        else {
            panic!("stale target must begin as a media clip")
        };
        PathBuf::from(&store.project.assets[&clip.asset].path)
    };
    let deleted = dispatch(
        &state,
        "edit.ripple_delete",
        json!({"track":"v1","range_ms":[1200,1900],"ripple":false,"rationale":"test target race"}),
        test_actor(),
    )
    .await;
    assert!(deleted.ok, "{:?}", deleted.error);
    let stale_finished = wait_job(&state, &stale_job, 30).await;
    assert_eq!(stale_finished.state, crate::jobs::JobState::Done);
    let stale_result = stale_finished.result.unwrap();
    assert_eq!(stale_result["placement"]["state"], "failed");
    assert!(
        stale_result["asset_id"].as_str().is_some(),
        "provider result is retained"
    );
    assert_eq!(stale_result["placement"]["cleanup"]["removed"], true);
    assert!(
        !stale_placeholder.exists(),
        "deleted target does not leave an orphan placeholder"
    );

    let cancelled = dispatch(
        &state,
        "assets.generate",
        json!({
            "prompt":"cancelled slot",
            "provider":"codex",
            "kind":"image",
            "variation":"cancelled",
            "timeout_ms":30000,
            "placement":{"mode":"insert","track":"v1","at_ms":1200,"duration_ms":900}
        }),
        test_actor(),
    )
    .await;
    assert!(cancelled.ok, "{:?}", cancelled.error);
    let cancelled = cancelled.result.unwrap();
    let cancelled_job = cancelled["job_id"].as_str().unwrap();
    let cancelled_clip = cancelled["placement"]["target_clip"]
        .as_str()
        .unwrap()
        .to_string();
    let cancelled_path = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        let (track_id, index) = store.project.find_clip(&cancelled_clip).unwrap();
        let cut_core::Clip::Media(clip) = &store.project.track(track_id).unwrap().clips[index]
        else {
            panic!("cancel target must be a media clip")
        };
        PathBuf::from(&store.project.assets[&clip.asset].path)
    };
    let cancelled_result = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id":cancelled_job}),
        test_actor(),
    )
    .await;
    assert!(cancelled_result.ok, "{:?}", cancelled_result.error);
    let cancelled_record = state.jobs.get(cancelled_job).unwrap();
    assert_eq!(cancelled_record.state, crate::jobs::JobState::Failed);
    assert_eq!(
        cancelled_record.error.as_ref().unwrap().code,
        "job_cancelled"
    );
    {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        assert!(store.project.find_clip(&cancelled_clip).is_some());
    }
    assert!(
        cancelled_path.is_file(),
        "cancelled generation keeps its pending slot source"
    );
}

#[tokio::test]
async fn assets_generate_queues_and_cancels_provider_work() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project = dir.path().join("gen-cancel.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"gen-cancel","dir": project}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let invocations = dir.path().join("codex-invocations.txt");
    let _fake_cli = generation_cli::FakeGenerationCli::install(
        dir.path(),
        generation_cli::FakeGenerationCliConfig::waiting()
            .log_invocations(invocations.clone())
            .with_default_delay_ms(30_000),
    );

    let first = dispatch(
        &state,
        "assets.generate",
        json!({"prompt":"first queued image","provider":"codex","kind":"image","timeout_ms":30000}),
        test_actor(),
    )
    .await;
    let second = dispatch(
        &state,
        "assets.generate",
        json!({"prompt":"second queued image","provider":"codex","kind":"image","timeout_ms":30000}),
        test_actor(),
    )
    .await;
    let first_id = first.result.as_ref().unwrap()["job_id"].as_str().unwrap();
    let second_id = second.result.as_ref().unwrap()["job_id"].as_str().unwrap();

    for _ in 0..100 {
        if invocations.is_file() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(invocations.is_file(), "first provider run never started");
    assert_eq!(
        state.jobs.get(second_id).unwrap().state,
        crate::jobs::JobState::Queued
    );

    let cancel_second = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id": second_id}),
        test_actor(),
    )
    .await;
    let cancel_first = dispatch(
        &state,
        "jobs.cancel",
        json!({"job_id": first_id}),
        test_actor(),
    )
    .await;
    assert!(cancel_second.ok, "{:?}", cancel_second.error);
    assert!(cancel_first.ok, "{:?}", cancel_first.error);
    for job_id in [first_id, second_id] {
        let record = state.jobs.get(job_id).unwrap();
        assert_eq!(record.state, crate::jobs::JobState::Failed);
        assert_eq!(record.error.as_ref().unwrap().code, "job_cancelled");
    }
    assert_eq!(
        std::fs::read_to_string(invocations)
            .unwrap()
            .lines()
            .count(),
        1,
        "the queued job must not invoke the provider"
    );
    let runs = project.join("cache/gen/runs");
    assert!(
        !runs.exists() || std::fs::read_dir(runs).unwrap().next().is_none(),
        "cancelling the running job must clean generation scratch"
    );
}

#[tokio::test]
async fn generated_media_import_refuses_a_different_open_project_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let first = dir.path().join("first.cutproj");
    let second = dir.path().join("second.cutproj");
    for (name, path) in [("first", &first), ("second", &second)] {
        let created = dispatch(
            &state,
            "project.create",
            json!({"name":name,"dir":path}),
            test_actor(),
        )
        .await;
        assert!(created.ok, "{:?}", created.error);
    }
    let generated = dir.path().join("generated.png");
    std::fs::write(&generated, b"not reached by probe").unwrap();

    let result = media_import(
        &state,
        json!({
            "path": generated,
            "expected_project_dir": first.display().to_string(),
            "rationale": "generated-media project binding test",
        }),
        test_actor(),
    )
    .await;
    let error = result.expect_err("import must not cross the project boundary");
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.message.contains("project changed"));
    let guard = state.project.read().await;
    assert!(guard.as_ref().unwrap().project.assets.is_empty());
}

#[tokio::test]
async fn media_remove_cleans_only_project_owned_generated_sources() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project = dir.path().join("cleanup.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"cleanup","dir":project}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let generated_dir = project.join("assets/generated");
    std::fs::create_dir_all(&generated_dir).unwrap();
    let generated = generated_dir.join("gen-test.png");
    let provenance = generated_dir.join("gen-test.json");
    std::fs::write(&generated, b"generated").unwrap();
    std::fs::write(&provenance, b"{}").unwrap();
    let external = dir.path().join("external.png");
    std::fs::write(&external, b"external").unwrap();

    let make_asset = |path: &Path, hash: &str| cut_core::Asset {
        path: path.display().to_string(),
        hash: hash.into(),
        probe: None,
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store
            .record_import(
                Some("generated".into()),
                make_asset(&generated, "sha256:generated"),
                test_actor(),
                None,
            )
            .unwrap();
        store
            .record_import(
                Some("external".into()),
                make_asset(&external, "sha256:external"),
                test_actor(),
                None,
            )
            .unwrap();
    }

    let removed_generated = dispatch(
        &state,
        "media.remove",
        json!({"asset":"generated"}),
        test_actor(),
    )
    .await;
    assert!(removed_generated.ok, "{:?}", removed_generated.error);
    assert_eq!(
        removed_generated.result.as_ref().unwrap()["source_deleted"],
        true
    );
    assert!(!generated.exists() && !provenance.exists());

    let removed_external = dispatch(
        &state,
        "media.remove",
        json!({"asset":"external"}),
        test_actor(),
    )
    .await;
    assert!(removed_external.ok, "{:?}", removed_external.error);
    assert_eq!(
        removed_external.result.as_ref().unwrap()["source_deleted"],
        false
    );
    assert_eq!(
        removed_external.result.as_ref().unwrap()["source_kept"],
        external.display().to_string()
    );
    assert!(external.is_file(), "ordinary imported source must be kept");
}

/// the checkpoint-cursor contract — project.ops{since} accepts a checkpoint id/name (not just a raw
/// op id), consistent with project.diff{from}. A checkpoint resolves to its
/// at_op, so the returned ops are those AFTER the checkpoint; an unknown ref
/// gives resolve_ref's actionable error; a raw op id still works.
#[tokio::test]
async fn project_ops_since_accepts_checkpoint_ref() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let go = |verb: &'static str, args: Value| {
        let state = state.clone();
        async move {
            let r = dispatch(&state, verb, args, test_actor()).await;
            assert!(r.ok, "{verb} failed: {:?}", r.error);
            r.result.unwrap()
        }
    };
    go(
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
    )
    .await;
    // One op, then a named checkpoint, then two more ops.
    go("edit.add_marker", json!({"at_ms":10,"label":"before"})).await;
    let cp = go("project.checkpoint", json!({"name":"cp-mid"})).await;
    let cp_id = cp["checkpoint"]["id"].as_str().unwrap().to_string();
    go("edit.add_marker", json!({"at_ms":20,"label":"after1"})).await;
    go("edit.add_marker", json!({"at_ms":30,"label":"after2"})).await;

    // since = checkpoint NAME → only the two ops after the checkpoint.
    let by_name = go("project.ops", json!({"since":"cp-mid"})).await;
    let ops = by_name["ops"].as_array().expect("ops array");
    let labels: Vec<&str> = ops
        .iter()
        .filter(|o| o["verb"] == "edit.add_marker")
        .filter_map(|o| o["args"]["label"].as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["after1", "after2"],
        "checkpoint name ref → ops after it"
    );
    assert!(
        !ops.iter().any(|o| o["verb"] == "project.checkpoint"),
        "the checkpoint op itself is excluded (since is exclusive)"
    );

    // since = checkpoint ID resolves identically.
    let by_id = go("project.ops", json!({"since": cp_id})).await;
    assert_eq!(
        by_id["ops"].as_array().unwrap().len(),
        ops.len(),
        "id ref == name ref"
    );

    // since = a raw op id still works (the pre-the checkpoint-cursor contract path).
    let all = go("project.ops", json!({})).await;
    let first_op = all["ops"][0]["op_id"].as_str().unwrap().to_string();
    let since_first = go("project.ops", json!({"since": first_op})).await;
    assert!(
        (since_first["ops"].as_array().unwrap().len() as i64)
            == (all["ops"].as_array().unwrap().len() as i64 - 1),
        "raw op id ref skips exactly that op"
    );

    // Unknown ref → actionable resolve_ref error (not the raw read_since one).
    let r = dispatch(&state, "project.ops", json!({"since":"nope"}), test_actor()).await;
    assert!(!r.ok);
    assert_eq!(r.error.as_ref().unwrap().code, "not_found");
    assert!(
        r.error
            .unwrap()
            .message
            .contains("neither a checkpoint nor an op id"),
        "unknown ref → resolve_ref's message"
    );
}

/// render.final preset arg: unknown names fail FAST (before any encode)
/// with the preset list in the cause; the schema enum and the media-crate
/// registry must agree (registry-sync pattern).
#[tokio::test]
async fn render_final_preset_validation() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // Unknown preset → invalid_args naming the valid tiers, no job spawned.
    let r = dispatch(
        &state,
        "render.final",
        json!({"preset":"h264_1080p30"}),
        test_actor(),
    )
    .await;
    let e = r.error.expect("unknown preset must error");
    assert_eq!(e.code, "invalid_args");
    for name in cut_media::render::PRESET_NAMES {
        assert!(e.cause.contains(name), "cause lists '{name}': {}", e.cause);
    }
    // Schema enum == media registry (drift tripwire both directions).
    let reg = crate::registry::VerbRegistry::load();
    let v = reg.get("render.final").expect("render.final in registry");
    let schema_names: Vec<String> = v.args["properties"]["preset"]["enum"]
        .as_array()
        .expect("preset enum declared in schema")
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    assert_eq!(schema_names, cut_media::render::PRESET_NAMES.to_vec());
    assert_eq!(
        v.args["properties"]["preset"]["default"].as_str(),
        Some(cut_media::render::RenderPreset::default().name.as_str()),
        "schema default preset must match RenderPreset::default()"
    );
    // Footage profile handoff: unknown profile
    // fails fast too, and every schema enum value parses in perception.
    let r = dispatch(
        &state,
        "render.final",
        json!({"profile":"vlog"}),
        test_actor(),
    )
    .await;
    let e = r.error.expect("unknown profile must error");
    assert_eq!(e.code, "invalid_args");
    assert!(
        e.cause.contains("talking_head"),
        "cause lists valid profiles: {}",
        e.cause
    );
    for p in v.args["properties"]["profile"]["enum"]
        .as_array()
        .expect("profile enum")
    {
        p.as_str()
            .unwrap()
            .parse::<cut_perception::FootageProfile>()
            .expect("schema profile enum parses in cut-perception");
    }

    // Fit/resolution: unknown values fail fast, every schema enum value
    // parses in cut-media, and the schema `default` matches the Rust
    // RenderOptions::default() — the byte-identical-replay guarantee
    // depends on the default staying contain + project.
    let r = dispatch(
        &state,
        "render.final",
        json!({"fit":"squish"}),
        test_actor(),
    )
    .await;
    let e = r.error.expect("unknown fit must error");
    assert_eq!(e.code, "invalid_args");
    assert!(
        e.cause.contains("contain"),
        "cause lists valid fit modes: {}",
        e.cause
    );
    let r = dispatch(
        &state,
        "render.final",
        json!({"resolution":"4k"}),
        test_actor(),
    )
    .await;
    let e = r.error.expect("unknown resolution must error");
    assert_eq!(e.code, "invalid_args");
    for f in v.args["properties"]["fit"]["enum"]
        .as_array()
        .expect("fit enum")
    {
        f.as_str()
            .unwrap()
            .parse::<cut_media::render::Fit>()
            .expect("schema fit enum parses");
    }
    for rsn in v.args["properties"]["resolution"]["enum"]
        .as_array()
        .expect("resolution enum")
    {
        rsn.as_str()
            .unwrap()
            .parse::<cut_media::render::Resolution>()
            .expect("schema resolution enum parses");
    }
    let defaults = cut_media::render::RenderOptions::default();
    assert_eq!(
        v.args["properties"]["fit"]["default"].as_str(),
        Some(defaults.fit.as_str()),
        "schema fit default must match RenderOptions::default() (byte-identical replay)"
    );
    assert_eq!(
        v.args["properties"]["resolution"]["default"].as_str(),
        Some(defaults.resolution.as_str()),
        "schema resolution default must match RenderOptions::default()"
    );
}

/// Contract sync: the dispatch default for render.preview duration_ms
/// must equal the `default` schema/verbs.json advertises (it silently
/// drifted to 3000 once — agents got shorter previews than documented).
#[test]
fn preview_default_matches_schema() {
    let reg = crate::registry::VerbRegistry::load();
    let v = reg
        .get("render.preview")
        .expect("render.preview in registry");
    let schema_default = v.args["properties"]["duration_ms"]["default"]
        .as_u64()
        .expect("schema declares a numeric default for duration_ms");
    assert_eq!(
        PREVIEW_DEFAULT_DURATION_MS, schema_default,
        "dispatch default and schema/verbs.json default must match"
    );
}

#[test]
fn target_size_bitrate_rejects_values_that_would_wrap() {
    let kbps = target_size_video_kbps(25.0, 10_000, 192).expect("normal target size bitrate");
    assert_eq!(kbps, 18_259);

    let err = target_size_video_kbps(f64::from(u32::MAX), 100, 0)
        .expect_err("computed video bitrate must not wrap u32");
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.cause.contains("exceeds u32::MAX"),
        "cause explains overflow guard: {}",
        err.cause
    );
}

#[tokio::test]
async fn storyboard_tiles_use_black_placeholder_for_single_frame_failure() {
    let dir = tempfile::tempdir().unwrap();
    let good_tile = cut_media::render::black_frame_jpeg(32, 18).unwrap();
    let warnings = write_storyboard_tiles(
        3,
        3000,
        90,
        false,
        dir.path(),
        |index, _at, _h, _compose| {
            let good_tile = good_tile.clone();
            async move {
                if index == 1 {
                    Err(CutError::new(
                        error_codes::FFMPEG,
                        "simulated frame extraction failure",
                        "decode failed",
                    ))
                } else {
                    Ok(good_tile)
                }
            }
        },
    )
    .await
    .expect("placeholder tile keeps storyboard generation alive");

    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0]["index"], 1);
    assert_eq!(warnings[0]["at_ms"], 1500);
    assert_eq!(warnings[0]["code"], error_codes::FFMPEG);
    for i in 0..3 {
        let p = dir.path().join(format!("f{i:03}.jpg"));
        assert!(
            std::fs::metadata(&p).map(|m| m.len() > 0).unwrap_or(false),
            "tile {i} should exist even when one extractor call fails"
        );
    }
}

#[test]
fn reframe_qc_resolves_output_path_from_receipt() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("exports")).unwrap();
    std::fs::create_dir_all(dir.path().join("receipts")).unwrap();
    let custom = dir.path().join("exports/custom.mp4");
    std::fs::write(&custom, b"not a real mp4 but exists").unwrap();
    std::fs::write(
        dir.path().join("receipts/reframe_007.json"),
        serde_json::to_vec_pretty(&json!({
            "reframe_id": "reframe_007",
            "output_path": custom,
        }))
        .unwrap(),
    )
    .unwrap();

    let resolved = resolve_reframe_output_for_qc(dir.path(), "reframe_007").unwrap();
    assert_eq!(resolved, custom.canonicalize().unwrap());
}

/// Contract: render.frame returns {path, mime, at_ms, width, height}
/// exactly as schema/verbs.json promises (at_ms/width/height were
/// missing). Real ffmpeg single-frame extract over a tiny lavfi asset.
#[tokio::test]
async fn render_frame_result_shape_matches_schema() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": dir.path().join("t.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // Tiny real input — same lavfi pattern as cut-media's engine tests.
    let media = dir.path().join("tiny.mp4");
    let ff = std::process::Command::new("ffmpeg")
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=1",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&media)
        .status()
        .expect("ffmpeg present (cut-media dependency)");
    assert!(ff.success(), "lavfi asset generation failed");
    let r = dispatch(&state, "media.import", json!({"path": media}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    // Explicit src_range_ms — no probe dependency, no auto-place race.
    let r = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,1000]}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    // h:1080 → full project geometry (render.frame DEFAULTS to the 540-high fast-scrub preview
    // since SCRUB_DEFAULT_HEIGHT=540, which yields 960×540; pass h to get the full frame).
    let r = dispatch(
        &state,
        "render.frame",
        json!({"at_ms": 0, "h": 1080}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["mime"], "image/jpeg");
    assert_eq!(res["at_ms"], 0);
    // Geometry conforms to project settings (aspect preserved): h:1080 on a 16:9 project → 1920×1080.
    assert_eq!(res["width"], 1920);
    assert_eq!(res["height"], 1080);
    let p = res["path"].as_str().unwrap();
    assert!(
        std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false),
        "frame file written"
    );
    assert!(
        res.get("base64").is_none(),
        "base64 only with inline:true (the binary-output contract)"
    );
    // The `fast` field reports which path served the frame. With no
    // proxy built (the import chain is async + unawaited here) the fast
    // scrub plan can't resolve, so it falls back to the composed path.
    assert_eq!(
        res["fast"], false,
        "no proxy → composed fallback, fast:false"
    );
}

/// the output-fencing contract: output paths are fenced — traversal, foreign dirs and
/// non-media suffixes are refused.
#[test]
fn output_path_fencing() {
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("p.cutproj");
    std::fs::create_dir_all(&proj).unwrap();
    // Default path inside the project is fine.
    assert!(fence_output_path(&proj, None, "exports/out.mp4", OutputPathPolicy::MP4).is_ok());
    // Traversal refused.
    assert!(fence_output_path(&proj, Some("../evil.mp4"), "x.mp4", OutputPathPolicy::MP4).is_err());
    // Foreign absolute dir refused.
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("evil.mp4");
    assert!(
        fence_output_path(&proj, outside_file.to_str(), "x.mp4", OutputPathPolicy::MP4).is_err()
    );
    // Overwriting an EXISTING non-export-suffix file refused (the output-fencing contract
    // invariant: a render/export verb must not clobber project data files;
    // PathFence allows CREATING new files of any suffix inside the fence,
    // and refuses only unsafe overwrites).
    std::fs::create_dir_all(proj.join("exports")).unwrap();
    let inside = proj.join("exports/project.json");
    std::fs::write(&inside, b"existing project data file").unwrap();
    assert!(fence_output_path(
        &proj,
        Some(inside.to_str().unwrap()),
        "x.mp4",
        OutputPathPolicy::MP4
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn atomic_output_write_replaces_late_symlink_instead_of_following_it() {
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("p.cutproj");
    std::fs::create_dir_all(proj.join("exports")).unwrap();
    let final_path = fence_output_path(
        &proj,
        Some("exports/out.srt"),
        "exports/out.srt",
        OutputPathPolicy::SRT,
    )
    .expect("initial fence passes before attacker swap");

    let outside = dir.path().join("outside.txt");
    std::fs::write(&outside, b"keep me").unwrap();
    std::os::unix::fs::symlink(&outside, &final_path).unwrap();

    write_output_atomic(&final_path, b"caption export").expect("atomic output write");

    assert_eq!(std::fs::read(&outside).unwrap(), b"keep me");
    assert_eq!(std::fs::read(&final_path).unwrap(), b"caption export");
    assert!(
        !std::fs::symlink_metadata(&final_path)
            .unwrap()
            .file_type()
            .is_symlink(),
        "final path should be a regular export file, not the late symlink"
    );
}

#[test]
fn default_output_paths_avoid_existing_files() {
    let _output_dir_guard = crate::output_paths::SESSION_OUTPUT_DIR_TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    crate::output_paths::set_session_output_dir(None);
    let dir = tempfile::tempdir().unwrap();
    let proj = dir.path().join("p.cutproj");
    std::fs::create_dir_all(proj.join("exports")).unwrap();
    let first =
        fence_output_path(&proj, None, "exports/recording.mp4", OutputPathPolicy::MP4).unwrap();
    assert!(first.ends_with("recording.mp4"));
    let first_path = first.to_path_buf();
    std::fs::write(&first_path, b"existing recording").unwrap();
    drop(first);
    let second =
        fence_output_path(&proj, None, "exports/recording.mp4", OutputPathPolicy::MP4).unwrap();
    assert!(second.ends_with("recording-2.mp4"));
    assert!(!second.exists());
    let explicit = fence_output_path(
        &proj,
        Some(first_path.to_str().unwrap()),
        "exports/recording.mp4",
        OutputPathPolicy::MP4,
    )
    .unwrap();
    assert_eq!(
        explicit.as_ref(),
        first_path,
        "explicit Save As paths stay exact"
    );
    crate::output_paths::set_session_output_dir(None);
}

#[test]
fn resolve_receipt_path_rejects_traversal_render_id() {
    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let outside = dir.path().join("outside.json");
    std::fs::write(&outside, "{}").unwrap();

    let err = resolve_receipt_path(&receipts, Some("../outside"))
        .expect_err("render_id must not escape receipts dir");
    assert_eq!(err.code, error_codes::INVALID_ARGS);
}

#[test]
fn resolve_receipt_path_with_explicit_id_never_falls_back_to_latest() {
    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    std::fs::write(receipts.join("render_001.json"), "{}").unwrap();

    let latest = resolve_receipt_path(&receipts, None).expect("latest receipt");
    assert!(latest.ends_with("render_001.json"));
    let err = resolve_receipt_path(&receipts, Some("render_002"))
        .expect_err("explicit missing receipt must stay missing");
    assert_eq!(err.code, error_codes::NOT_FOUND);
}

#[test]
fn reserve_receipt_id_prevents_reuse_before_receipt_exists() {
    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    let (first, _first_marker) = reserve_receipt_id(&receipts, "render").unwrap();
    let (second, _second_marker) = reserve_receipt_id(&receipts, "render").unwrap();
    assert_eq!(first, "render_001");
    assert_eq!(second, "render_002");
    assert_eq!(next_receipt_id_preview(&receipts, "render"), "render_003");
}

#[test]
fn attach_judge_to_receipt_rejects_receipt_outside_receipts_dir() {
    let dir = tempfile::tempdir().unwrap();
    let receipts = dir.path().join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let outside = dir.path().join("outside.json");
    let receipt = cut_core::RenderReceipt {
        render_id: "render_001".into(),
        ts: OpRecord::now_ts(),
        output_path: dir.path().join("render.mp4").display().to_string(),
        output_hash: "sha256:fake".into(),
        duration_ms: 1000,
        preset: "standard".into(),
        at_op: "op_000001".into(),
        checks: vec![],
        pass: false,
        judge: None,
        fix_actions: vec![],
    };
    std::fs::write(&outside, serde_json::to_string_pretty(&receipt).unwrap()).unwrap();
    let state = AppState::new();

    let err = attach_judge_to_receipt(&state, &receipts, &outside, json!({"status":"test"}))
        .expect_err("judge attachment must not write receipts outside receipts dir");
    assert_eq!(err.code, error_codes::INVALID_ARGS);
}

// -----------------------------------------------------------------------
// verify.judge wiring — deterministic fake adapter coverage.
// -----------------------------------------------------------------------

/// CUTD_JUDGE_ADAPTER is process-global env — judge tests serialize on
/// this lock so parallel test threads never see each other's stub. Tests that
/// execute a Python adapter also take `AGENT_CLI_ENV_LOCK`, because adapter
/// interpreter discovery reads the shared `CUTD_ADAPTER_PYTHON`/PATH state.
static JUDGE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Project + manufactured receipt + dummy render output. render.final
/// needs real media; the judge WIRING only needs a persisted receipt and
/// a file at output_path — manufacturing both keeps the test hermetic.
async fn judge_fixture(dir: &std::path::Path) -> (AppState, PathBuf) {
    let state = AppState::new();
    let proj = dir.join("t.cutproj");
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"t","dir": &proj}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let receipts = proj.join("receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    let render = proj.join("exports/render_001.mp4");
    std::fs::create_dir_all(render.parent().unwrap()).unwrap();
    std::fs::write(&render, b"fake-render-bytes").unwrap();
    let receipt = cut_core::RenderReceipt {
        render_id: "render_001".into(),
        ts: OpRecord::now_ts(),
        output_path: render.display().to_string(),
        output_hash: "sha256:fake".into(),
        duration_ms: 1000,
        preset: "standard".into(),
        at_op: "op_000001".into(),
        checks: vec![],
        pass: false,
        judge: None,
        fix_actions: vec![],
    };
    let rpath = receipts.join("render_001.json");
    std::fs::write(&rpath, serde_json::to_string_pretty(&receipt).unwrap()).unwrap();
    (state, rpath)
}

/// Write a python stub that speaks the adapter contract (review command,
/// --out file, exit 0) and emits `envelope_py` (a python dict literal).
/// The stub records the argv it received into the envelope so tests can
/// assert the wiring passed the right render/intent/perception.
fn write_stub_adapter(path: &std::path::Path, envelope_py: &str) -> String {
    let code = format!(
        r#"import json, sys
args = sys.argv[1:]
def val(flag):
    return args[args.index(flag) + 1] if flag in args else None
env = {envelope_py}
env["stub_args"] = {{"command": args[0] if args else None, "render": val("--render"),
                     "intent": val("--intent"), "perception": val("--perception"),
                     "bundle_dir": val("--bundle-dir"), "provider": val("--provider")}}
out = val("--out")
if out:
    open(out, "w").write(json.dumps(env))
print(json.dumps(env))
"#
    );
    std::fs::write(path, code).unwrap();
    path.display().to_string()
}

/// Poll a job to a terminal state (50 ms cadence) or panic after `secs`.
async fn wait_job(state: &AppState, job_id: &str, secs: u64) -> crate::jobs::JobRecord {
    use crate::jobs::JobState;
    for _ in 0..(secs * 20) {
        if let Some(rec) = state.jobs.get(job_id) {
            if matches!(rec.state, JobState::Done | JobState::Failed) {
                return rec;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    unreachable!("job {job_id} did not reach a terminal state within {secs}s");
}

/// Wiring happy path: the adapter envelope lands at receipt.judge, the
/// job result carries the verdict, and the adapter received the render
/// path + perception + op-derived intent.
#[tokio::test]
async fn verify_judge_fake_adapter_completed() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    // A perception sidecar file for the render → must be passed through.
    let pcept = rpath
        .parent()
        .unwrap()
        .join("render_001.output.perception.json");
    std::fs::write(&pcept, b"{}").unwrap();
    let stub = write_stub_adapter(
        &dir.path().join("stub.py"),
        r#"{"schema": "shellx-cut/judge-review/1", "status": "completed",
                "backend": {"name": "cli", "provider": "stub", "watched": True, "listened": False},
                "review": {"verdict": "pass", "issues": [], "confidence": 0.91, "summary": "stub review"},
                "not_run_reason": None}"#,
    );
    std::env::set_var("CUTD_JUDGE_ADAPTER", &stub);
    let r = dispatch(&state, "verify.judge", json!({}), test_actor()).await;
    std::env::remove_var("CUTD_JUDGE_ADAPTER");
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["render_id"], "render_001");
    let job_id = res["job_id"].as_str().unwrap().to_string();
    let rec = wait_job(&state, &job_id, 30).await;
    assert_eq!(rec.state, crate::jobs::JobState::Done, "{:?}", rec.error);
    let result = rec.result.unwrap();
    assert_eq!(result["status"], "completed");
    assert_eq!(result["verdict"], "pass");
    assert_eq!(result["confidence"], 0.91);
    // Normalized gate outcome (additive): pass -> approve.
    assert_eq!(result["outcome"], "approve");
    assert!(result["outcome_reason"].is_string());
    // Receipt persisted with the judge section; checks untouched.
    let receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(&rpath).unwrap()).unwrap();
    let judge = receipt.judge.expect("judge envelope attached");
    assert_eq!(judge["status"], "completed");
    assert_eq!(judge["review"]["verdict"], "pass");
    // The wiring passed the right inputs to the adapter.
    assert_eq!(
        judge["stub_args"]["render"].as_str().unwrap(),
        receipt.output_path,
        "adapter must receive the receipt's output path"
    );
    // Compare CANONICAL forms: ProjectStore canonicalizes the project root,
    // so the adapter receives the perception path under the RESOLVED root
    // (e.g. /private/var on macOS, where tempdir() hands back the /var
    // symlink). Both point at the same file the fixture wrote, so
    // canonicalize matches regardless of the platform's symlink/prefix
    // quirks — asserting the raw display() string only held on Linux.
    assert_eq!(
        std::fs::canonicalize(judge["stub_args"]["perception"].as_str().unwrap()).unwrap(),
        std::fs::canonicalize(&pcept).unwrap(),
        "render's own output-perception must be passed explicitly"
    );
    let intent = judge["stub_args"]["intent"].as_str().unwrap();
    assert!(
        intent.contains("render_001") && intent.contains("op_000001"),
        "intent names the render + at_op: {intent}"
    );
    // Bundle dir is project-local so the configured review CLI can read it
    // within the same workspace boundary as the project.
    // Compare with separators normalised: Windows hands back `.scratch\judge\…`
    // (and a `\\?\C:\…` extended-length prefix from canonicalize), so a literal
    // forward-slash `contains` asserted the separator rather than the location.
    let bundle = judge["stub_args"]["bundle_dir"].as_str().unwrap();
    let bundle_slashes = bundle.replace('\\', "/");
    assert!(
        bundle_slashes.contains(".cutproj") && bundle_slashes.contains(".scratch/judge/render_001"),
        "bundle under <proj>/.scratch/judge/: {bundle}"
    );
    // With no backend arg, the wiring passes the ladder default
    // (--provider auto) so ladder_judge.py walks claude->codex->antigravity->grok.
    assert_eq!(
        judge["stub_args"]["provider"].as_str().unwrap(),
        "auto",
        "default backend maps to --provider auto (walk the ladder)"
    );
}

/// A judge that FAILS the render normalizes to outcome=reject (additive),
/// carrying the review summary as the reason — so a gate/autopilot can act
/// on a rejection without parsing each CLI's verdict vocabulary.
#[tokio::test]
async fn verify_judge_fail_verdict_normalizes_to_reject() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let (state, _rpath) = judge_fixture(dir.path()).await;
    let stub = write_stub_adapter(
        &dir.path().join("stub.py"),
        r#"{"schema": "shellx-cut/judge-review/1", "status": "completed",
                "backend": {"name": "cli", "provider": "stub", "watched": True, "listened": False},
                "review": {"verdict": "fail", "issues": ["frozen tail"], "confidence": 0.77,
                           "summary": "the last 3s are frozen"},
                "not_run_reason": None}"#,
    );
    std::env::set_var("CUTD_JUDGE_ADAPTER", &stub);
    let r = dispatch(&state, "verify.judge", json!({}), test_actor()).await;
    std::env::remove_var("CUTD_JUDGE_ADAPTER");
    assert!(r.ok, "{:?}", r.error);
    let job_id = r.result.unwrap()["job_id"].as_str().unwrap().to_string();
    let rec = wait_job(&state, &job_id, 30).await;
    assert_eq!(rec.state, crate::jobs::JobState::Done, "{:?}", rec.error);
    let result = rec.result.unwrap();
    assert_eq!(result["verdict"], "fail", "raw model verdict preserved");
    assert_eq!(result["outcome"], "reject", "fail normalizes to reject");
    assert!(
        result["outcome_reason"]
            .as_str()
            .unwrap()
            .contains("frozen"),
        "reject reason carries the review summary: {}",
        result["outcome_reason"]
    );
}

/// An explicit backend (codex) is threaded through as --provider codex
/// so the ladder forces that rung (honest not_run if its CLI is absent).
#[tokio::test]
async fn verify_judge_explicit_backend_threads_provider() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let stub = write_stub_adapter(
        &dir.path().join("stub.py"),
        r#"{"schema": "shellx-cut/judge-review/1", "status": "completed",
                "backend": {"name": "cli", "provider": "codex", "watched": True, "listened": False},
                "review": {"verdict": "pass", "issues": [], "confidence": 0.8, "summary": "x"},
                "not_run_reason": None}"#,
    );
    std::env::set_var("CUTD_JUDGE_ADAPTER", &stub);
    let r = dispatch(
        &state,
        "verify.judge",
        json!({"backend": "codex"}),
        test_actor(),
    )
    .await;
    std::env::remove_var("CUTD_JUDGE_ADAPTER");
    assert!(r.ok, "{:?}", r.error);
    let job_id = r.result.unwrap()["job_id"].as_str().unwrap().to_string();
    let rec = wait_job(&state, &job_id, 30).await;
    assert_eq!(rec.state, crate::jobs::JobState::Done, "{:?}", rec.error);
    let receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(&rpath).unwrap()).unwrap();
    let judge = receipt.judge.expect("judge envelope attached");
    assert_eq!(
        judge["stub_args"]["provider"].as_str().unwrap(),
        "codex",
        "backend=codex threads through as --provider codex"
    );
}

/// No backend available (adapter script missing) → the job COMPLETES
/// with an honest structured not_run attached to the receipt — never an
/// error, never a fake pass (public verb contract / verification contract).
#[tokio::test]
async fn verify_judge_missing_adapter_is_honest_not_run() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    std::env::set_var("CUTD_JUDGE_ADAPTER", "/nonexistent/cli_judge.py");
    let r = dispatch(&state, "verify.judge", json!({}), test_actor()).await;
    std::env::remove_var("CUTD_JUDGE_ADAPTER");
    assert!(r.ok, "{:?}", r.error);
    let job_id = r.result.unwrap()["job_id"].as_str().unwrap().to_string();
    let rec = wait_job(&state, &job_id, 30).await;
    assert_eq!(
        rec.state,
        crate::jobs::JobState::Done,
        "not_run is a COMPLETED job"
    );
    let result = rec.result.unwrap();
    assert_eq!(result["status"], "not_run");
    assert!(result["reason"]
        .as_str()
        .unwrap()
        .contains("CUTD_JUDGE_ADAPTER"));
    let receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(&rpath).unwrap()).unwrap();
    let judge = receipt.judge.expect("not_run envelope still attached");
    assert_eq!(judge["status"], "not_run");
    assert!(judge["review"].is_null(), "not_run never carries a verdict");
}

/// Adapter crash (nonzero exit) → error envelope attached to the receipt
/// (the attempt is honest history) AND the job FAILS with every labelled
/// process stream as the actionable cause.
#[tokio::test]
async fn verify_judge_adapter_crash_fails_job_with_all_streams() {
    let _env = JUDGE_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _agent_env = lock_agent_cli_env();
    let dir = tempfile::tempdir().unwrap();
    let (state, rpath) = judge_fixture(dir.path()).await;
    let stub = dir.path().join("crash.py");
    std::fs::write(
        &stub,
        "import sys; print('{\"is_error\":true,\"result\":\"OAuth expired\"}'); print('judge exploded: flames', file=sys.stderr); sys.exit(3)\n",
    )
    .unwrap();
    std::env::set_var("CUTD_JUDGE_ADAPTER", stub.display().to_string());
    let r = dispatch(
        &state,
        "verify.judge",
        json!({"render_id":"render_001"}),
        test_actor(),
    )
    .await;
    std::env::remove_var("CUTD_JUDGE_ADAPTER");
    assert!(r.ok, "{:?}", r.error);
    let job_id = r.result.unwrap()["job_id"].as_str().unwrap().to_string();
    let rec = wait_job(&state, &job_id, 30).await;
    assert_eq!(rec.state, crate::jobs::JobState::Failed);
    let err = rec.error.unwrap();
    assert!(
        err.cause.contains("judge exploded: flames"),
        "stderr captured into the cause: {}",
        err.cause
    );
    assert!(
        err.cause.contains("stdout: {\"is_error\":true"),
        "stdout error envelope captured into the cause: {}",
        err.cause
    );
    let receipt: cut_core::RenderReceipt =
        serde_json::from_str(&std::fs::read_to_string(&rpath).unwrap()).unwrap();
    let judge = receipt.judge.expect("error envelope attached");
    assert_eq!(judge["status"], "error");
    assert!(judge["review"].is_null());
}

/// Unknown backend ids fail FAST (no job, no quota).
#[tokio::test]
async fn verify_judge_unknown_backend_fails_fast() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _rpath) = judge_fixture(dir.path()).await;
    // "gemini" itself is now unknown — its rung was REPLACED by grok
    // (the ladder mirrors shellX's provider set).
    let r = dispatch(
        &state,
        "verify.judge",
        json!({"backend":"gemini"}),
        test_actor(),
    )
    .await;
    let e = r.error.expect("unknown backend must error");
    assert_eq!(e.code, "invalid_args");
    // The cause lists the valid ladder backends so the caller can correct.
    assert!(
        e.cause.contains("auto") && e.cause.contains("grok"),
        "cause lists the valid backends: {}",
        e.cause
    );
}

/// audio.add_music: places a bed on an auto-created music1 track,
/// auto-ducks under the base speech track using perception silences
/// (windows RECORDED on the lowered edit.duck step), surfaces beat:N
/// markers from the music asset's perception BeatGrid, and applies the bed
/// gain. Proves the whole lowered chain end-to-end at the dispatch layer.
#[tokio::test]
async fn add_music_beds_ducks_and_marks_beats() {
    let dir = tempfile::tempdir().unwrap();
    // narrowing_fixture: a1 placed on v1 [src 0-5000] + a1t [src 5000-10000]
    // at timeline [0,5000). a1t is the speech track the bed ducks under.
    let state = narrowing_fixture(dir.path()).await;
    let receipts = dir.path().join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    // Speech facts on a1 (the VO): silences [5500,6500]+[8000,10000] in
    // source → speech complement within a1t's window [5000,10000) maps to
    // timeline [0,500)+[1500,3000) (same maths as the duck test).
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/perception/1",
            "asset_hash": "sha256:test",
            "source_path": "clip.mp4",
            "instruments_run": ["silence"],
            "silences": [
                {"start_ms": 5500, "end_ms": 6500, "source": "both"},
                {"start_ms": 8000, "end_ms": 10000, "source": "both"}
            ],
        }))
        .unwrap(),
    )
    .unwrap();
    // Import a "music" asset (dummy file) + give it a probe duration and a
    // perception report with a beat grid (the import chain doesn't run in
    // tests, so we wire the facts directly — the same pattern the duck test
    // uses for silences).
    let music = dir.path().join("music.wav");
    std::fs::write(&music, b"not-really-audio").unwrap();
    let r = dispatch(&state, "media.import", json!({"path": music}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let music_asset = r.result.unwrap()["asset_id"].as_str().unwrap().to_string();
    update_asset(&state, &music_asset, |a| {
        a.probe = Some(json!({"kind":"audio","duration_ms":4000,"has_audio":true}));
    })
    .await
    .unwrap();
    std::fs::write(
        receipts.join(format!("{music_asset}.perception.json")),
        serde_json::to_string(&json!({
            "schema": "shellx-cut/perception/1",
            "asset_hash": "sha256:music",
            "source_path": "music.wav",
            "instruments_run": ["beats"],
            // Beats every 500ms (120 bpm) within [0,4000): 0,500...,3500.
            "beats": {"bpm": 120.0, "beats_ms": [0, 500, 1000, 1500, 2000, 2500, 3000, 3500]},
        }))
        .unwrap(),
    )
    .unwrap();

    // Add the music bed (defaults: music1 track, bed_gain -18, auto-duck
    // against the base speech track a1t at -15, beat markers on).
    let r = dispatch(
        &state,
        "audio.add_music",
        json!({"asset": music_asset, "rationale": "podcast bed"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(
        res["track_id"], "music1",
        "auto-created the dedicated bed track"
    );
    assert_eq!(res["created_track"], true);
    assert_eq!(res["bed_gain_db"], -18.0);
    assert_eq!(res["bed_duration_ms"], 4000);
    // Auto-duck produced 2 windows from the VO speech (same as duck test).
    assert_eq!(
        res["ducked_windows"], 2,
        "ducked under the two speech spans"
    );
    // 8 beats within [0,4000) → 8 markers.
    assert_eq!(res["beats_marked"], 8);

    // State checks: the bed clip exists on music1, gain set, duck windows
    // recorded on the track, and 8 beat markers landed.
    let s = dispatch(&state, "project.state", json!({}), test_actor()).await;
    let project: cut_core::Project = serde_json::from_value(s.result.unwrap()).unwrap();
    let bed_track = project.track("music1").expect("music1 track created");
    match bed_track.clips.first().expect("bed clip placed") {
        cut_core::Clip::Media(c) => {
            assert_eq!(c.asset, music_asset);
            assert_eq!(c.gain_db, -18.0, "bed gain applied");
        }
        _ => unreachable!("bed clip should be media"),
    }
    assert_eq!(
        bed_track.gain_windows.len(),
        2,
        "auto-duck windows on the bed track"
    );
    assert_eq!(bed_track.gain_windows[0].db, -15.0, "default duck depth");
    let beats: Vec<&cut_core::Marker> = project
        .markers
        .iter()
        .filter(|m| m.label == "beat")
        .collect();
    assert_eq!(beats.len(), 8, "one marker per beat in range");
    assert_eq!(beats[0].at_ms, 0);
    assert_eq!(
        beats[1].at_ms, 500,
        "beat at source 500 → timeline 500 (bed at 0)"
    );

    // Replay determinism: the whole lowered chain (add_track + insert +
    // gain + duck + 8 add_marker) reproduces the TIMELINE byte-identically
    // from the log. (Asset.probe is a CACHE field written by update_asset,
    // not an op — replay legitimately lacks it; compare tracks+markers, the
    // state the lowered steps actually produce.)
    let ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
    let log: Vec<cut_core::OpRecord> =
        serde_json::from_value(ops.result.unwrap()["ops"].clone()).unwrap();
    let rebuilt = cut_core::rebuild_from_log(&log).expect("replay");
    assert_eq!(rebuilt.tracks, project.tracks, "tracks replay identically");
    assert_eq!(
        rebuilt.markers, project.markers,
        "beat markers replay identically"
    );

    // duck:false skips ducking; beat_markers:false skips markers.
    let r = dispatch(
            &state,
            "audio.add_music",
            json!({"asset": music_asset, "track": "music1", "at_ms": 5000, "duck": false, "beat_markers": false}),
            test_actor(),
        )
        .await;
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["ducked_windows"], 0, "duck:false skips ducking");
    assert_eq!(res["beats_marked"], 0, "beat_markers:false skips markers");
    assert_eq!(
        res["created_track"], false,
        "reused the existing music1 track"
    );
}
