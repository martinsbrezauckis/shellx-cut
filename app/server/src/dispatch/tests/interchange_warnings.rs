use super::*;

#[test]
fn interchange_warns_for_nondefault_playback_state_on_nonempty_tracks() {
    let mut project = cut_core::Project::new("playback", cut_core::ProjectSettings::default());
    let mut video = cut_core::edit::make_media_clip("v", "source", 0, 4000);
    video.mute_ranges = vec![[100, 300]];
    project.tracks[0].clips.push(cut_core::Clip::Media(video));
    project.tracks[0].visible = false;
    project.tracks[1]
        .clips
        .push(cut_core::Clip::Media(cut_core::edit::make_media_clip(
            "a", "source", 0, 4000,
        )));
    project.tracks[1].muted = true;
    project.tracks[1].solo = true;
    project.tracks[1].gain_db = -6.0;
    project.tracks[1].gain_windows.push(cut_core::GainWindow {
        range_ms: [200, 800],
        db: -9.0,
        attack_ms: 50,
    });
    project.tracks[1].pan = 0.5;
    let mut overlay = project.tracks[0].clone();
    overlay.id = "v2".into();
    overlay.visible = true;
    overlay.blend_mode = Some("multiply".into());
    project.tracks.push(overlay);

    let expected = [
        "audio ducking",
        "audio pan",
        "clip mute ranges",
        "hidden video tracks",
        "muted or soloed audio",
        "track gain",
        "video blend modes",
    ];
    for target in [
        ExportWarningTarget::Otio,
        ExportWarningTarget::Edl,
        ExportWarningTarget::Xml(cut_export::XmlFormat::Fcpxml),
        ExportWarningTarget::Xml(cut_export::XmlFormat::Premiere),
        ExportWarningTarget::Xml(cut_export::XmlFormat::Resolve),
        ExportWarningTarget::Xml(cut_export::XmlFormat::Mlt),
    ] {
        let warnings = export_richness_warnings(&project, target);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "richness_dropped");
        let dropped = warnings[0].detail["dropped"].as_array().unwrap();
        for label in expected {
            assert!(
                dropped.iter().any(|entry| entry == label),
                "missing {label}"
            );
        }
        assert!(dropped
            .windows(2)
            .all(|pair| pair[0].as_str() < pair[1].as_str()));
    }

    project.tracks[0].clips[0] =
        cut_core::Clip::Media(cut_core::edit::make_media_clip("v", "source", 0, 4000));
    project.tracks[0].visible = true;
    project.tracks[1].muted = false;
    project.tracks[1].solo = false;
    project.tracks[1].gain_db = 0.0;
    project.tracks[1].gain_windows.clear();
    project.tracks[1].pan = 0.0;
    project.tracks[2].blend_mode = Some("normal".into());
    project.tracks[2].clips[0] =
        cut_core::Clip::Media(cut_core::edit::make_media_clip("v2", "source", 0, 4000));
    assert!(export_richness_warnings(&project, ExportWarningTarget::Otio).is_empty());
    project.tracks[1].solo = true;
    assert_eq!(
        export_richness_warnings(&project, ExportWarningTarget::Otio)[0].detail["dropped"],
        json!(["muted or soloed audio"]),
    );
    project.tracks[1].solo = false;
    project.tracks[1].muted = true;
    assert_eq!(
        export_richness_warnings(&project, ExportWarningTarget::Otio)[0].detail["dropped"],
        json!(["muted or soloed audio"]),
    );
    project.tracks[1].muted = false;
    project.tracks[1].clips.clear();
    project.tracks[1].muted = true;
    project.tracks[1].gain_db = -3.0;
    assert!(export_richness_warnings(&project, ExportWarningTarget::Otio).is_empty());
    project.tracks[0].clips.clear();
    project.tracks[2].blend_mode = Some("multiply".into());
    assert_eq!(
        export_richness_warnings(&project, ExportWarningTarget::Otio)[0].detail["dropped"],
        json!(["video blend modes"]),
        "a nondefault blend on a nonempty v2 must be disclosed even with empty v1",
    );
    project.tracks[2].clips.clear();
    assert!(export_richness_warnings(&project, ExportWarningTarget::Otio).is_empty());
}

#[tokio::test]
async fn interchange_verbs_warn_without_changing_serialized_cuts_or_project_history() {
    let _output_fixture = crate::output_paths::test_fixture::SessionOutputDirFixture::new();
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"warning", "dir":root.path().join("warning.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    {
        let mut guard = state.project.write().await;
        let project = &mut guard.as_mut().unwrap().project;
        project.assets.insert(
            "source".into(),
            cut_core::Asset {
                path: root.path().join("source.mp4").to_string_lossy().into_owned(),
                hash: "sha256:fixture".into(),
                probe: Some(json!({"kind":"video", "duration_ms":4000, "width":1920, "height":1080, "has_audio":true})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        project.tracks[0]
            .clips
            .push(cut_core::Clip::Media(cut_core::edit::make_media_clip(
                "v", "source", 0, 4000,
            )));
        project.tracks[1]
            .clips
            .push(cut_core::Clip::Media(cut_core::edit::make_media_clip(
                "a", "source", 0, 4000,
            )));
    }

    let targets = [
        (
            "export.otio",
            json!({"path":"exports/plain.otio"}),
            json!({"path":"exports/rich.otio"}),
        ),
        (
            "export.xml",
            json!({"format":"premiere", "path":"exports/plain.xml"}),
            json!({"format":"premiere", "path":"exports/rich.xml"}),
        ),
        (
            "export.xml",
            json!({"format":"fcpxml", "path":"exports/plain.fcpxml"}),
            json!({"format":"fcpxml", "path":"exports/rich.fcpxml"}),
        ),
        (
            "export.edl",
            json!({"path":"exports/plain.edl"}),
            json!({"path":"exports/rich.edl"}),
        ),
    ];
    let mut plain_files = Vec::new();
    for (verb, plain_args, _) in &targets {
        let response = dispatch(&state, verb, plain_args.clone(), test_actor()).await;
        assert!(response.ok, "{verb}: {:?}", response.error);
        assert!(response.warnings.is_none(), "plain {verb} must not warn");
        let path = response.result.unwrap()["path"]
            .as_str()
            .unwrap()
            .to_owned();
        plain_files.push(std::fs::read_to_string(path).unwrap());
    }
    {
        let mut guard = state.project.write().await;
        let project = &mut guard.as_mut().unwrap().project;
        let cut_core::Clip::Media(clip) = &mut project.tracks[0].clips[0] else {
            unreachable!()
        };
        clip.mute_ranges = vec![[100, 300]];
        project.tracks[0].visible = false;
        project.tracks[1].muted = true;
        project.tracks[1].gain_db = -6.0;
    }
    let before_project = state.project.read().await.as_ref().unwrap().project.clone();
    let before_ops = state
        .project
        .read()
        .await
        .as_ref()
        .unwrap()
        .log
        .read_all()
        .unwrap();
    for ((verb, _, rich_args), plain) in targets.iter().zip(plain_files.iter()) {
        let response = dispatch(&state, verb, rich_args.clone(), test_actor()).await;
        assert!(response.ok, "{verb}: {:?}", response.error);
        let warnings = response.warnings.as_ref().expect("rich state must warn");
        let warning = warnings
            .iter()
            .find(|warning| warning.code == "richness_dropped")
            .unwrap();
        let labels = warning.detail["dropped"].as_array().unwrap();
        for expected in [
            "clip mute ranges",
            "hidden video tracks",
            "muted or soloed audio",
            "track gain",
        ] {
            assert!(
                labels.iter().any(|label| label == expected),
                "{verb} omitted {expected}"
            );
        }
        let path = response.result.unwrap()["path"]
            .as_str()
            .unwrap()
            .to_owned();
        let rich = std::fs::read_to_string(path).unwrap();
        assert_eq!(
            &rich, plain,
            "{verb} serializer bytes changed with warning-only state"
        );
        match *verb {
            "export.otio" => {
                let otio: Value = serde_json::from_str(&rich).unwrap();
                assert_eq!(
                    otio["tracks"]["children"][0]["children"][0]["OTIO_SCHEMA"],
                    "Clip.1"
                );
            }
            "export.edl" => assert!(rich.contains("* FROM CLIP NAME: source.mp4")),
            "export.xml" => assert!(rich.contains("source.mp4")),
            _ => unreachable!(),
        }
    }
    let after_ops = state
        .project
        .read()
        .await
        .as_ref()
        .unwrap()
        .log
        .read_all()
        .unwrap();
    assert_eq!(
        before_ops, after_ops,
        "export must not mutate project history"
    );
    assert_eq!(
        before_project,
        state.project.read().await.as_ref().unwrap().project,
        "export must not mutate the open project"
    );
}
