//! Published 0.6.118 journals must cross the server's full project.open route.
use super::*;
use cut_core::{Actor, Asset, InverseOp, OpRecord, ProjectStore};

fn old_ops(points: usize) -> Vec<OpRecord> {
    let text = match points {
        96 => include_str!("fixtures/legacy_keyframes/overlay96.jsonl"),
        300 => include_str!("fixtures/legacy_keyframes/overlay300.jsonl"),
        _ => unreachable!(),
    };
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn write_journal(dir: &std::path::Path, ops: &[OpRecord]) -> Vec<u8> {
    std::fs::create_dir_all(dir).unwrap();
    let bytes = ops
        .iter()
        .map(|op| format!("{}\n", serde_json::to_string(op).unwrap()))
        .collect::<String>()
        .into_bytes();
    std::fs::write(dir.join("ops.jsonl"), &bytes).unwrap();
    bytes
}

fn title_or_shape_ops(root: &std::path::Path, kind: &str, points: usize) -> Vec<OpRecord> {
    let mut store = ProjectStore::create(root, kind, None).unwrap();
    let (asset, _) = store
        .record_import(
            Some("a1".into()),
            Asset {
                path: "/legacy/title.mov".into(),
                hash: "sha256:legacy-title".into(),
                probe: Some(json!({"duration_ms": 30000})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
            Actor::system(),
            None,
        )
        .unwrap();
    let verb = if kind == "title" {
        "title.add"
    } else {
        "edit.add_shape"
    };
    store
        .apply_lowered(
            verb,
            if kind == "title" {
                json!({"text":"Legacy title","range_ms":[0,30000]})
            } else {
                json!({"shape":"rect","range_ms":[0,30000]})
            },
            Actor::system(),
            None,
            vec![
                InverseOp {
                    verb: "edit.add_track".into(),
                    args: json!({"kind":"video","id":"v2"}),
                },
                InverseOp {
                    verb: "edit.insert".into(),
                    args: json!({"asset":asset,"track":"v2","at_ms":0,"src_range_ms":[0,30000],"ripple":false}),
                },
            ],
            vec![],
        )
        .unwrap();
    let mut ops = store.log.read_all().unwrap();
    let mut keyframe = old_ops(points).pop().unwrap();
    keyframe.op_id = format!("op_{:06}", ops.len() + 1);
    keyframe.args["clip"] = json!("c1");
    for effect in &mut keyframe.effects {
        effect.track = Some("v2".into());
        effect.detail.insert("clip".into(), json!("c1"));
    }
    ops.push(keyframe);
    ops
}

#[tokio::test]
async fn published_keyframe_journals_open_on_overlay_base_title_and_shape() {
    for points in [96, 300] {
        for kind in ["overlay", "base", "title", "shape"] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join(format!("legacy-{kind}-{points}.cutproj"));
            let mut ops = if kind == "title" || kind == "shape" {
                title_or_shape_ops(root.path(), kind, points)
            } else {
                old_ops(points)
            };
            let clip = if kind == "base" || kind == "title" || kind == "shape" {
                "c1"
            } else {
                "c2"
            };
            if kind == "base" {
                let op = ops.last_mut().unwrap();
                op.args["clip"] = json!("c1");
                for effect in &mut op.effects {
                    effect.track = Some("v1".into());
                    effect.detail.insert("clip".into(), json!("c1"));
                }
            }
            let before = if kind == "overlay" {
                let bytes: &[u8] = if points == 96 {
                    include_bytes!("fixtures/legacy_keyframes/overlay96.jsonl")
                } else {
                    include_bytes!("fixtures/legacy_keyframes/overlay300.jsonl")
                };
                std::fs::create_dir_all(&path).unwrap();
                std::fs::write(path.join("ops.jsonl"), bytes).unwrap();
                bytes.to_vec()
            } else {
                write_journal(&path, &ops)
            };
            let state = AppState::new();
            let opened = dispatch(&state, "project.open", json!({"path":path}), test_actor()).await;
            assert!(opened.ok, "{kind} {points}: {:?}", opened.error);
            let project = opened.result.unwrap()["project"].clone();
            let expected = cut_core::rebuild_from_log(&ops).unwrap();
            assert_eq!(
                project["tracks"],
                serde_json::to_value(expected.tracks).unwrap()
            );
            let tracks = project["tracks"].as_array().unwrap();
            let saved = tracks
                .iter()
                .flat_map(|track| track["clips"].as_array().unwrap())
                .find(|entry| entry["id"] == clip)
                .unwrap();
            assert_eq!(
                saved["keyframes"][0]["points"].as_array().unwrap().len(),
                points
            );
            assert_eq!(
                saved["keyframes"][0]["points"],
                ops.last().unwrap().args["points"]
            );
            assert_eq!(
                saved["keyframes"][0]["param"],
                ops.last().unwrap().args["param"]
            );
            assert_eq!(
                saved["keyframes"][0]["interp"].as_str().unwrap_or("linear"),
                ops.last().unwrap().args["interp"].as_str().unwrap()
            );
            assert_eq!(std::fs::read(path.join("ops.jsonl")).unwrap(), before);
        }
    }
}

#[tokio::test]
async fn old_keyframe_operation_undo_redo_and_new_edit_limit() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("old-overlay.cutproj");
    let journal = write_journal(&path, &old_ops(96));
    let state = AppState::new();
    let opened = dispatch(&state, "project.open", json!({"path":path}), test_actor()).await;
    assert!(opened.ok, "{:?}", opened.error);
    let points = (0..65)
        .map(|t_ms| json!({"t_ms":t_ms,"value":0.5}))
        .collect::<Vec<_>>();
    let refused = dispatch(
        &state,
        "edit.keyframe",
        json!({"clip":"c2","param":"pos_x","interp":"linear","points":points}),
        test_actor(),
    )
    .await;
    assert!(!refused.ok);
    assert_eq!(
        refused.error.unwrap().message,
        "clip 'c2' has 65 keyframe points for PosX; the limit is 64 per track"
    );
    assert_eq!(std::fs::read(path.join("ops.jsonl")).unwrap(), journal);
    let undone = dispatch(&state, "project.undo", json!({}), test_actor()).await;
    assert!(undone.ok, "{:?}", undone.error);
    let undone_points = {
        let guard = state.project.read().await;
        let clip = &guard.as_ref().unwrap().project.track("v2").unwrap().clips[0];
        let cut_core::Clip::Media(media) = clip else {
            panic!("media clip")
        };
        media.keyframes.len()
    };
    assert_eq!(undone_points, 0);
    let redone = dispatch(&state, "project.redo", json!({}), test_actor()).await;
    assert!(redone.ok, "{:?}", redone.error);
    let redone_points = {
        let guard = state.project.read().await;
        let clip = &guard.as_ref().unwrap().project.track("v2").unwrap().clips[0];
        let cut_core::Clip::Media(media) = clip else {
            panic!("media clip")
        };
        media.keyframes[0].points.len()
    };
    assert_eq!(redone_points, 96);
    let reopened = ProjectStore::open(&path).unwrap();
    let cut_core::Clip::Media(media) = &reopened.project.track("v2").unwrap().clips[0] else {
        panic!("media clip")
    };
    assert_eq!(media.keyframes[0].points.len(), 96);
}
