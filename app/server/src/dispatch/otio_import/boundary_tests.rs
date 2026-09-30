//! Imported paths must fail in the parser before targets performs filesystem I/O.

use super::*;

const HOSTILE: &[&str] = &[
    "file:///%5C%5C%3F%5CUNC%5Cattacker%5Cshare%5Ca.mov",
    r"\\attacker\share\a.mov",
    "//attacker/share/a.mov",
    r"/\attacker/share/a.mov",
    "file:////attacker/share/a.mov",
    "file://localhost//attacker/share/a.mov",
    r"\\?\uNc\attacker\share\a.mov",
    r"file:///\\?\unc\attacker\share\a.mov",
    r"\\.\UNC\attacker\share\a.mov",
    r"\??\UNC\attacker\share\a.mov",
    r"\\?\GLOBALROOT\Device\Mup\attacker\share\a.mov",
    r"\\?\Volume{123}\clip.mov",
    "%2f%5cattacker%2fshare%2fa.mov",
    "%68ttps%3a%2f%2fattacker/a.mov",
    "file:///%68ttps%3a%2f%2fattacker/a.mov",
    "https:attacker/a.mov",
    "file%3a%2f%2fattacker/share/a.mov",
    "clip%00.mov",
    "clip\0.mov",
    "NUL.mov",
    r"C:\media\aux .mov",
    "folder/COM1.mp4",
    "folder/LPT².wav",
    r"\\?\C:\media\CON.mp4",
];

#[test]
fn imported_media_parser_rejects_network_devices_and_decoded_schemes() {
    for target in HOSTILE {
        let error = media_path(target).expect_err(target);
        assert_eq!(error.code, error_codes::INVALID_ARGS, "{target}");
    }
}

#[test]
fn imported_media_parser_preserves_local_references() {
    for (target, expected) in [
        ("relative/A%20B.mov", "relative/A B.mov"),
        (r"C:\media\clip.mov", r"C:\media\clip.mov"),
        (r"\\?\C:\media\clip.mov", r"C:\media\clip.mov"),
        (
            "file:///%5C%5C%3F%5CC:%5Cmedia%5Cclip.mov",
            r"C:\media\clip.mov",
        ),
        ("/media/clip.mov", "/media/clip.mov"),
        ("FILE://LOCALHOST/media/clip.mov", "/media/clip.mov"),
        ("folder/COM10.mov", "folder/COM10.mov"),
    ] {
        assert_eq!(
            media_path(target).unwrap(),
            PathBuf::from(expected),
            "{target}"
        );
    }
    let expected = if cfg!(windows) {
        "C:/media/clip.mov"
    } else {
        "/C:/media/clip.mov"
    };
    assert_eq!(
        media_path("file:///C:/media/clip.mov").unwrap(),
        PathBuf::from(expected)
    );
}

fn document(target: &str) -> Value {
    json!({
        "OTIO_SCHEMA":"Timeline.1",
        "tracks":{"OTIO_SCHEMA":"Stack.1","children":[{
            "OTIO_SCHEMA":"Track.1","name":"Picture","kind":"Video","children":[{
                "OTIO_SCHEMA":"Clip.1",
                "source_range":{
                    "OTIO_SCHEMA":"TimeRange.1",
                    "start_time":{"OTIO_SCHEMA":"RationalTime.1","rate":30,"value":0},
                    "duration":{"OTIO_SCHEMA":"RationalTime.1","rate":30,"value":30}
                },
                "media_reference":{"OTIO_SCHEMA":"ExternalReference.1","target_url":target}
            }]}
        ]}
    })
}

fn actor() -> Actor {
    Actor {
        kind: cut_core::ActorKind::Agent,
        name: "otio-boundary-test".into(),
        via: "test".into(),
        request: None,
    }
}

#[tokio::test]
async fn import_otio_refuses_hostile_references_in_preview_and_replace() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"local", "dir":root.path().join("local.cutproj")}),
        actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let project_dir = state.project.read().await.as_ref().unwrap().dir.clone();
    let before_ops = std::fs::read(project_dir.join("ops.jsonl")).unwrap();
    let before_cache = std::fs::read(project_dir.join("project.json")).unwrap();
    let otio = root.path().join("import.otio");
    for target in HOSTILE {
        std::fs::write(&otio, serde_json::to_vec(&document(target)).unwrap()).unwrap();
        for mode in ["preview", "replace"] {
            let result = crate::dispatch::dispatch(
                &state,
                "import.otio",
                json!({"path":otio, "mode":mode}),
                actor(),
            )
            .await;
            assert!(!result.ok, "{mode} accepted {target}");
            assert_eq!(
                result.error.unwrap().code,
                error_codes::INVALID_ARGS,
                "{target}"
            );
        }
    }
    assert_eq!(
        std::fs::read(project_dir.join("ops.jsonl")).unwrap(),
        before_ops
    );
    assert_eq!(
        std::fs::read(project_dir.join("project.json")).unwrap(),
        before_cache
    );
    assert!(state.jobs.list().is_empty());

    // A real local file still resolves during read-only preview without probing.
    std::fs::write(root.path().join("local clip.mov"), b"local preview fixture").unwrap();
    std::fs::write(
        &otio,
        serde_json::to_vec(&document("local%20clip.mov")).unwrap(),
    )
    .unwrap();
    let preview = crate::dispatch::dispatch(
        &state,
        "import.otio",
        json!({"path":otio,"mode":"preview"}),
        actor(),
    )
    .await;
    assert!(preview.ok, "{:?}", preview.error);
    assert_eq!(preview.result.unwrap()["media_available"], 1);
    assert_eq!(
        std::fs::read(project_dir.join("ops.jsonl")).unwrap(),
        before_ops
    );
}
