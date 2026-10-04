//! End-to-end OTIO EOF rounding admission with the bundled, probed media.

use super::*;

async fn project_with_media() -> (tempfile::TempDir, AppState, std::path::PathBuf, u64) {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"otio-rounding", "dir":root.path().join("otio-rounding.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let media = root.path().join("source.mp4");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/first-edit-sample.mp4"),
        &media,
    )
    .unwrap();
    let duration = cut_media::probe(&media).unwrap().duration_ms.unwrap();
    (root, state, media, duration)
}

#[tokio::test]
async fn otio_exact_low_fps_overrun_refuses_before_any_project_mutation() {
    let (root, state, media, duration) = project_with_media().await;
    let timeline = json!({
        "settings":{"width":1920,"height":1080,"fps":1.0,"audio_rate":48000},
        "assets":{"source":{"path":media,"probe":{"duration_ms":duration,"width":1920,"height":1080,"has_audio":true}}},
        "tracks":[{"id":"v1","kind":"video","clips":[{"id":"c1","asset":"source","src_in_ms":0,"src_out_ms":duration}]}]
    });
    let cut_export = cut_export::otio::export_otio(&timeline, "low fps rounding").unwrap();
    let cut_doc: serde_json::Value = serde_json::from_str(&cut_export).unwrap();
    let cut_clip = cut_doc.pointer("/tracks/children/0/children/0").unwrap();
    let frame_end = cut_clip
        .pointer("/source_range/duration/value")
        .unwrap()
        .as_u64()
        .unwrap();
    let requested_end = frame_end * 1000;
    assert!(
        requested_end > duration + 50,
        "genuine Cut export must exercise the capped overrun"
    );
    let (before_project, before_ops) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        (store.project.clone(), store.log.read_all().unwrap())
    };
    let before_revision = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap()["project_revision"]
        .clone();
    let fractional_frames = frame_end as f64 - 0.1;
    for (name, mut doc) in [
        ("cut-low-fps", cut_doc.clone()),
        ("foreign-fractional", cut_doc.clone()),
        ("foreign-mixed-rate", cut_doc.clone()),
    ] {
        let clip = doc.pointer_mut("/tracks/children/0/children/0").unwrap();
        match name {
            "foreign-fractional" => {
                clip["source_range"]["duration"]["value"] = json!(fractional_frames)
            }
            "foreign-mixed-rate" => clip["source_range"]["start_time"]["rate"] = json!(2.0),
            _ => {}
        }
        let otio = root.path().join(format!("{name}.otio"));
        std::fs::write(&otio, serde_json::to_vec(&doc).unwrap()).unwrap();
        let parsed =
            cut_export::otio::parse_otio(&std::fs::read_to_string(&otio).unwrap()).unwrap();
        let clip = &parsed[0].clips[0];
        assert!(
            clip.src_in_ms + clip.dur_ms > duration + 50,
            "{name} must materially exceed EOF"
        );
        let preview = dispatch(
            &state,
            "import.otio",
            json!({"path":otio,"mode":"preview"}),
            test_actor(),
        )
        .await;
        assert!(preview.ok, "{name}: {:?}", preview.error);
        let source_hash = preview.result.unwrap()["source_hash"].clone();
        let result = dispatch(
            &state,
            "import.otio",
            json!({"path":otio,"mode":"replace","expected_hash":source_hash}),
            test_actor(),
        )
        .await;
        assert!(!result.ok, "{name} must refuse");
        let error = result.error.unwrap();
        assert_eq!(error.code, error_codes::INVALID_ARGS, "{name}");
        assert!(
            error
                .suggested_action
                .as_deref()
                .unwrap_or("")
                .contains("retrim"),
            "{name}: {error:?}"
        );
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        assert_eq!(store.project, before_project, "{name}");
        assert_eq!(store.log.read_all().unwrap(), before_ops, "{name}");
        drop(guard);
        let after_revision = dispatch(&state, "project.state", json!({}), test_actor())
            .await
            .result
            .unwrap()["project_revision"]
            .clone();
        assert_eq!(after_revision, before_revision, "{name}");
        assert!(
            state.jobs.list().is_empty(),
            "{name} must not start enrichment"
        );
    }
}

#[tokio::test]
async fn otio_cut_export_roundtrip_clamps_small_nonframe_aligned_eof() {
    let (root, state, media, duration) = project_with_media().await;
    let timeline = json!({
        "settings":{"width":1920,"height":1080,"fps":25.0,"audio_rate":48000},
        "assets":{"source":{"path":media,"probe":{"duration_ms":duration,"width":1920,"height":1080,"has_audio":true}}},
        "tracks":[{"id":"v1","kind":"video","clips":[{"id":"c1","asset":"source","src_in_ms":1000,"src_out_ms":duration}]}]
    });
    let exported = cut_export::otio::export_otio(&timeline, "rounding").unwrap();
    let parsed = cut_export::otio::parse_otio(&exported).unwrap();
    let requested_end = parsed[0].clips[0].src_in_ms + parsed[0].clips[0].dur_ms;
    assert!(
        requested_end > duration && requested_end - duration <= 50,
        "fixture must round just past EOF"
    );
    let (before_project, before_ops) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        (store.project.clone(), store.log.read_all().unwrap())
    };
    let before_revision = dispatch(&state, "project.state", json!({}), test_actor())
        .await
        .result
        .unwrap()["project_revision"]
        .clone();
    let exported_doc: serde_json::Value = serde_json::from_str(&exported).unwrap();
    for (name, mut doc) in [
        ("near-eof-fractional", exported_doc.clone()),
        ("near-eof-mixed-rate", exported_doc.clone()),
    ] {
        let clip = doc.pointer_mut("/tracks/children/0/children/0").unwrap();
        match name {
            "near-eof-fractional" => {
                let frames = clip["source_range"]["duration"]["value"].as_f64().unwrap();
                clip["source_range"]["duration"]["value"] = json!(frames + 0.1);
            }
            "near-eof-mixed-rate" => {
                clip["source_range"]["duration"]["rate"] = json!(24.99);
            }
            _ => unreachable!(),
        }
        let foreign = root.path().join(format!("{name}.otio"));
        std::fs::write(&foreign, serde_json::to_vec(&doc).unwrap()).unwrap();
        let parsed =
            cut_export::otio::parse_otio(&std::fs::read_to_string(&foreign).unwrap()).unwrap();
        let foreign_end = parsed[0].clips[0].src_in_ms + parsed[0].clips[0].dur_ms;
        assert!(
            foreign_end > duration && foreign_end - duration <= 50,
            "{name} must test raw timing, not the cap"
        );
        let preview = dispatch(
            &state,
            "import.otio",
            json!({"path":foreign,"mode":"preview"}),
            test_actor(),
        )
        .await;
        assert!(preview.ok, "{name}: {:?}", preview.error);
        let result = dispatch(
            &state,
            "import.otio",
            json!({"path":foreign,"mode":"replace","expected_hash":preview.result.unwrap()["source_hash"]}),
            test_actor(),
        )
        .await;
        assert!(!result.ok, "{name} must refuse a non-Cut source range");
        assert_eq!(
            result.error.unwrap().code,
            error_codes::INVALID_ARGS,
            "{name}"
        );
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        assert_eq!(store.project, before_project, "{name}");
        assert_eq!(store.log.read_all().unwrap(), before_ops, "{name}");
        drop(guard);
        let after_revision = dispatch(&state, "project.state", json!({}), test_actor())
            .await
            .result
            .unwrap()["project_revision"]
            .clone();
        assert_eq!(after_revision, before_revision, "{name}");
        assert!(
            state.jobs.list().is_empty(),
            "{name} must not start enrichment"
        );
    }
    let otio = root.path().join("cut.otio");
    std::fs::write(&otio, exported).unwrap();
    let result = dispatch(
        &state,
        "import.otio",
        json!({"path":otio,"mode":"replace"}),
        test_actor(),
    )
    .await;
    assert!(result.ok, "{:?}", result.error);
    assert!(result
        .warnings
        .as_ref()
        .unwrap()
        .iter()
        .any(|warning| warning.code == "otio_time_clamped"));
    assert_eq!(result.result.as_ref().unwrap()["time_clamped_clips"], 1);
    let project = state.project.read().await.as_ref().unwrap().project.clone();
    let cut_core::Clip::Media(clip) = &project.tracks[0].clips[0] else {
        panic!("expected media clip")
    };
    assert_eq!(clip.src_out_ms, duration);
}
