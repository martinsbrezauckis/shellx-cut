use cut_core::{Actor, ActorKind, Asset, ClipTransform, ProjectStore};
use serde_json::json;

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Agent,
        name: "test".into(),
        via: "test".into(),
        request: None,
    }
}

#[test]
fn overlay_shape_roundtrip_and_old_transform_bytes() {
    let old = r#"{"x":0.1,"y":0.2,"scale":0.3,"opacity":1.0}"#;
    let t: ClipTransform = serde_json::from_str(old).unwrap();
    assert_eq!(serde_json::to_string(&t).unwrap(), old);
    for shape in ["circle", "rounded"] {
        let value = json!({"x":0.1,"y":0.2,"scale":0.3,"opacity":1.0,"shape":shape,"ring":{"width":0.02,"colour":"#ffffffe6"}});
        let t: ClipTransform = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(t).unwrap(), value);
    }
}

#[test]
fn overlay_shape_replay_undo_redo_and_validation() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "shape", None).unwrap();
    s.record_import(
        None,
        Asset {
            path: "/camera.mp4".into(),
            hash: "sha256:shape".into(),
            probe: Some(json!({"duration_ms":1000})),
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        },
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0}),
        actor(),
        None,
    )
    .unwrap();
    let refused = s.apply(
        "edit.transform",
        json!({"clip":"c1","shape":"circle"}),
        actor(),
        None,
    );
    assert!(
        refused.is_err(),
        "shapes must not be silently ignored on the base canvas"
    );
    s.apply("edit.add_track", json!({"kind":"video"}), actor(), None)
        .unwrap();
    s.apply(
        "edit.insert",
        json!({"asset":"a1","track":"v2","at_ms":0}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.transform",
        json!({"clip":"c2","x":0.1,"y":0.2,"scale":0.3}),
        actor(),
        None,
    )
    .unwrap();
    let old_project = serde_json::to_vec(&s.project).unwrap();
    assert!(!String::from_utf8_lossy(&old_project).contains("\"shape\":"));
    let old_roundtrip: cut_core::Project = serde_json::from_slice(&old_project).unwrap();
    assert_eq!(serde_json::to_vec(&old_roundtrip).unwrap(), old_project);
    let before = serde_json::to_value(&s.project).unwrap();
    let r = s
        .apply(
            "edit.transform",
            json!({"clip":"c2","shape":"circle","scale":0.3}),
            actor(),
            None,
        )
        .unwrap();
    let shaped = serde_json::to_value(&s.project).unwrap();
    assert!(shaped.to_string().contains("circle"));
    let undo = s
        .apply("edit.restore", json!({"op_id":r.op_id}), actor(), None)
        .unwrap();
    assert_eq!(serde_json::to_value(&s.project).unwrap(), before);
    s.apply("edit.restore", json!({"op_id":undo.op_id}), actor(), None)
        .unwrap();
    assert_eq!(serde_json::to_value(&s.project).unwrap(), shaped);
    for args in [
        json!({"shape":"triangle"}),
        json!({"shape":"circle","radius":0.2}),
        json!({"shape":"rounded","radius":0.6}),
        json!({"shape":"circle","ring":{"width":-1,"colour":"#ffffff"}}),
        json!({"shape":"circle","ring":{"width":0.02,"colour":"not-a-colour"}}),
    ] {
        let mut args = args.as_object().unwrap().clone();
        args.insert("clip".into(), json!("c2"));
        assert!(s
            .apply("edit.transform", args.into(), actor(), None)
            .is_err());
    }
    let project_dir = s.dir.clone();
    drop(s);
    let reopened = ProjectStore::open(&project_dir).unwrap();
    assert_eq!(serde_json::to_value(&reopened.project).unwrap(), shaped);
}
