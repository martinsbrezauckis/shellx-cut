//! Every verb that builds the timeline picture refuses up front, in plain
//! words, when an HDR clip needs the tone map and no installed FFmpeg has zimg.
//!
//! Only `render.final` did before; previews, composed frames, scopes, GIFs and
//! ranges failed later with ffmpeg's raw "No such filter: 'zscale'".

use super::test_actor;
use crate::dispatch::dispatch;
use crate::media_tool_gate::test_presence;
use crate::state::AppState;
use cut_media::toolpath::FfmpegFeature;
use serde_json::{json, Value};

fn hlg_probe() -> Value {
    json!({"kind": "video", "width": 320, "height": 180, "duration_ms": 2000, "fps": 30.0,
        "has_audio": false, "format": "mov,mp4 / hevc",
        "raw": {"streams": [{"codec_type": "video", "pix_fmt": "yuv420p10le",
            "color_transfer": "arib-std-b67", "color_primaries": "bt2020",
            "color_space": "bt2020nc"}]}})
}

/// A project whose timeline holds one clip with the given probe.
async fn project_with_clip(root: &std::path::Path, probe: Value) -> AppState {
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "hdr-gate", "dir": root.join("hdr-gate.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = root.join("hlg.mp4");
    std::fs::write(
        &media,
        b"stand-in bytes: the gate reads only the stored probe",
    )
    .unwrap();
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        let asset = cut_core::Asset {
            path: media.display().to_string(),
            hash: "sha256:00".into(),
            probe: Some(probe),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        };
        store
            .record_import(None, asset, test_actor(), None)
            .unwrap();
    }
    let inserted = dispatch(
        &state,
        "edit.insert",
        json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 2000]}),
        test_actor(),
    )
    .await;
    assert!(inserted.ok, "{:?}", inserted.error);
    state
}

/// Each verb (with valid arguments) that composes the timeline picture.
fn picture_verbs() -> Vec<(&'static str, Value)> {
    vec![
        ("render.final", json!({"dry_run": true})),
        ("render.preview", json!({"at_ms": 0, "duration_ms": 1000})),
        ("render.frame", json!({"at_ms": 500, "compose": true})),
        ("render.storyboard", json!({"compose": true})),
        ("render.direct", json!({})),
        ("render.reframe", json!({"aspect": "9:16"})),
        ("export.frame", json!({"at_ms": 500})),
        ("export.range", json!({"range_ms": [0, 1000]})),
        ("export.gif", json!({"range_ms": [0, 1000]})),
        ("verify.scopes", json!({"at_ms": 500})),
    ]
}

#[tokio::test]
async fn every_picture_verb_refuses_an_hdr_tone_map_without_zimg_by_name() {
    let root = tempfile::tempdir().unwrap();
    let state = project_with_clip(root.path(), hlg_probe()).await;
    let _missing = test_presence::features_missing(&[FfmpegFeature::Zscale]);
    for (verb, args) in picture_verbs() {
        let r = dispatch(&state, verb, args, test_actor()).await;
        let error = r
            .error
            .unwrap_or_else(|| panic!("{verb} must refuse up front"));
        assert_eq!(error.code, "tool_missing", "{verb}: {error:?}");
        assert!(
            error
                .message
                .starts_with("Converting the HDR video (hlg.mp4) to SDR needs FFmpeg with zimg"),
            "{verb}: {}",
            error.message
        );
        assert!(
            error.cause.contains("edit.color_space"),
            "{verb}: {}",
            error.cause
        );
        assert!(error.suggested_action.is_some(), "{verb}");
    }
    // The fast (proxy) frame does not compose, so it is not refused for this.
    let r = dispatch(&state, "render.frame", json!({"at_ms": 500}), test_actor()).await;
    assert_ne!(
        r.error.map(|e| e.message).unwrap_or_default().get(..33),
        Some("Converting the HDR video (hlg.mp4)")
    );
}

#[tokio::test]
async fn an_sdr_timeline_is_not_refused_for_zimg() {
    let root = tempfile::tempdir().unwrap();
    let mut probe = hlg_probe();
    probe["raw"]["streams"][0] = json!({"codec_type": "video", "pix_fmt": "yuv420p"});
    let state = project_with_clip(root.path(), probe).await;
    let _missing = test_presence::features_missing(&[FfmpegFeature::Zscale]);
    let r = dispatch(
        &state,
        "render.final",
        json!({"dry_run": true}),
        test_actor(),
    )
    .await;
    assert!(
        r.error.as_ref().map(|e| e.code.as_str()) != Some("tool_missing"),
        "{:?}",
        r.error
    );
}
