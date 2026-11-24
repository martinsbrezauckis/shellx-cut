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
    let points = (0..=cut_core::keyframe_limit::MAX_EDIT_KEYFRAME_POINTS)
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
        "clip 'c2' has 10001 keyframe points for PosX; the limit is 10000 per track"
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

#[tokio::test]
async fn oversized_movement_data_refuses_before_any_render_job_starts() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = root.path().join("oversized-cmd.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "oversized-cmd", "dir": dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let two_pts = |cx0: f64, cx1: f64| {
        json!([
            {"t_ms": 0, "cx": cx0, "cy": 0.5},
            {"t_ms": 480_000, "cx": cx1, "cy": 0.5}
        ])
    };
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        let (asset_id, _) = store
            .record_import(
                Some("a1".into()),
                Asset {
                    path: "/nonexistent/video.mp4".into(),
                    hash: "sha256:v1".into(),
                    probe: Some(
                        json!({"kind":"video","width":1280,"height":720,"fps":30.0,"duration_ms":480_000}),
                    ),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                test_actor(),
                None,
            )
            .unwrap();
        let ins = store
            .apply(
                "edit.insert",
                json!({"asset": asset_id, "track": "v1", "at_ms": 0, "src_range_ms": [0, 480_000], "ripple": false}),
                test_actor(),
                None,
            )
            .unwrap();
        let clip_id = ins.effects[0].detail["added_clip"]
            .as_str()
            .unwrap()
            .to_string();
        let boxes: Vec<_> = (1..256)
            .map(|i| {
                json!({
                    "shape": "rect",
                    "points": [[0.1, 0.1], [0.2, 0.2]],
                    "track": two_pts(0.1 + (i as f64) * 0.001, 0.8 - (i as f64) * 0.001),
                })
            })
            .collect();
        store
            .apply(
                "edit.redact",
                json!({
                    "clip": clip_id,
                    "shape": "rect",
                    "mode": "blur",
                    "points": [[0.1, 0.1], [0.2, 0.2]],
                    "track": two_pts(0.1, 0.9),
                    "boxes": boxes,
                }),
                test_actor(),
                None,
            )
            .unwrap();
    }
    // Saved/received project still opens cleanly.
    let reopened = dispatch(&state, "project.open", json!({"path": dir}), test_actor()).await;
    assert!(reopened.ok, "{:?}", reopened.error);
    let jobs_before = state.jobs.list().len();

    for (verb, args) in [
        ("render.final", json!({"dry_run": true, "hardware": "off"})),
        ("render.final", json!({"hardware": "off"})),
        ("render.queue", json!({"jobs": [{"hardware": "off"}]})),
        ("render.preview", json!({"at_ms": 0, "duration_ms": 1000})),
        (
            "export.range",
            json!({"range_ms": [0, 1000], "to_asset": false}),
        ),
        (
            "export.gif",
            json!({"range_ms": [0, 1000], "to_asset": false}),
        ),
        ("render.direct", json!({})),
        ("render.reframe", json!({"aspect": "9:16"})),
        ("render.bundle", json!({"platforms": ["9:16"]})),
        ("autopilot.run", json!({})),
    ] {
        let res = dispatch(&state, verb, args, test_actor()).await;
        let err = res
            .error
            .unwrap_or_else(|| panic!("{verb} must refuse before starting any job"));
        assert_eq!(err.code, error_codes::INVALID_ARGS, "{verb}: {err:?}");
        assert!(
            err.message.contains("This export would need ")
                && err
                    .message
                    .contains("MB of movement data; split the clip or reduce the moving regions"),
            "{verb}: {err:?}"
        );
    }
    assert_eq!(
        state.jobs.list().len(),
        jobs_before,
        "no render or autopilot job may start: {:?}",
        state.jobs.list()
    );
    assert!(
        state.jobs.list().iter().all(|j| !matches!(
            j.kind.as_str(),
            "render" | "render_queue" | "reframe" | "reframe-direct" | "bundle" | "autopilot"
        )),
        "unexpected render job: {:?}",
        state.jobs.list()
    );
    let exports = dir.join("exports");
    if exports.exists() {
        assert_eq!(std::fs::read_dir(&exports).unwrap().count(), 0);
    }
}

#[tokio::test]
async fn oversized_audio_volume_curve_refuses_export_audio_before_work() {
    use cut_core::{Clip, Keyframe, KfInterp, KfParam, KfPoint};
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let dir = root.path().join("oversized-audio-cmd.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name": "oversized-audio-cmd", "dir": dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().unwrap();
        store.project.assets.insert(
            "a1".into(),
            Asset {
                path: "/nonexistent/audio.wav".into(),
                hash: "sha256:a1".into(),
                probe: Some(json!({"kind":"audio","duration_ms":200_000_000,"has_audio":true})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
        );
        let mut clip = cut_core::edit::make_media_clip("ca", "a1", 0, 200_000_000);
        clip.keyframes.push(Keyframe {
            lead_in: None,
            param: KfParam::Volume,
            points: (0..=65)
                .map(|i| KfPoint {
                    t_ms: i * 1_000,
                    value: 0.2 + 0.01 * i as f64,
                })
                .collect(),
            interp: KfInterp::Linear,
        });
        store
            .project
            .track_mut("a1t")
            .unwrap()
            .clips
            .push(Clip::Media(clip));
        store.save().unwrap();
    }
    let res = dispatch(
        &state,
        "export.audio",
        json!({"format": "mp3"}),
        test_actor(),
    )
    .await;
    let err = res
        .error
        .expect("export.audio must refuse oversized volume curve before running ffmpeg");
    assert_eq!(err.code, error_codes::INVALID_ARGS);
    assert!(
        err.message.starts_with("This export would need ")
            && err.message.contains("split the clip"),
        "{err:?}"
    );
}

#[tokio::test]
async fn nested_and_concurrent_restart_oversized_projects_refuse_before_work() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("nested-oversized.cutproj");
    let state1 = AppState::new();
    let created = dispatch(
        &state1,
        "project.create",
        json!({"name": "nested-oversized", "dir": dir}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);
    let two_pts = |cx0: f64, cx1: f64| {
        json!([
            {"t_ms": 0, "cx": cx0, "cy": 0.5},
            {"t_ms": 480_000, "cx": cx1, "cy": 0.5}
        ])
    };
    {
        let mut guard = state1.project.write().await;
        let store = guard.as_mut().unwrap();
        let (asset_id, _) = store
            .record_import(
                Some("a1".into()),
                Asset {
                    path: "/nonexistent/video.mp4".into(),
                    hash: "sha256:v1".into(),
                    probe: Some(
                        json!({"kind":"video","width":1280,"height":720,"fps":30.0,"duration_ms":480_000}),
                    ),
                    transcript: None,
                    perception: None,
                    proxy: None,
                    filmstrip: None,
                },
                test_actor(),
                None,
            )
            .unwrap();
        let ins = store
            .apply(
                "edit.insert",
                json!({"asset": asset_id, "track": "v1", "at_ms": 0, "src_range_ms": [0, 480_000], "ripple": false}),
                test_actor(),
                None,
            )
            .unwrap();
        let clip_id = ins.effects[0].detail["added_clip"]
            .as_str()
            .unwrap()
            .to_string();
        let boxes: Vec<_> = (1..256)
            .map(|i| {
                json!({
                    "shape": "rect",
                    "points": [[0.1, 0.1], [0.2, 0.2]],
                    "track": two_pts(0.1 + (i as f64) * 0.001, 0.8 - (i as f64) * 0.001),
                })
            })
            .collect();
        store
            .apply(
                "edit.redact",
                json!({
                    "clip": clip_id,
                    "shape": "rect",
                    "mode": "blur",
                    "points": [[0.1, 0.1], [0.2, 0.2]],
                    "track": two_pts(0.1, 0.9),
                    "boxes": boxes,
                }),
                test_actor(),
                None,
            )
            .unwrap();
        store
            .apply(
                "edit.nest",
                json!({"clips": [clip_id], "name": "nested blur"}),
                test_actor(),
                None,
            )
            .unwrap();
    }

    // Reopen on two independent clients concurrently (simulating restart + second client).
    let state_a = AppState::new();
    let state_b = AppState::new();
    let (open_a, open_b) = tokio::join!(
        dispatch(&state_a, "project.open", json!({"path": dir}), test_actor()),
        dispatch(&state_b, "project.open", json!({"path": dir}), test_actor()),
    );
    assert!(open_a.ok, "{:?}", open_a.error);
    assert!(open_b.ok, "{:?}", open_b.error);
    let jobs_before_a = state_a.jobs.list().len();
    let jobs_before_b = state_b.jobs.list().len();

    let (res_a, res_b) = tokio::join!(
        dispatch(
            &state_a,
            "render.final",
            json!({"dry_run": true, "hardware": "off"}),
            test_actor()
        ),
        dispatch(
            &state_b,
            "render.final",
            json!({"hardware": "off"}),
            test_actor()
        ),
    );
    for res in [res_a, res_b] {
        let err = res
            .error
            .expect("oversized nest must refuse before baking or spawning job");
        assert_eq!(err.code, error_codes::INVALID_ARGS);
        assert!(
            err.message.starts_with("This export would need ")
                && err
                    .message
                    .contains("MB of movement data; split the clip or reduce the moving regions"),
            "{err:?}"
        );
    }
    assert_eq!(state_a.jobs.list().len(), jobs_before_a);
    assert_eq!(state_b.jobs.list().len(), jobs_before_b);
    assert!(state_a.jobs.list().iter().all(|j| j.kind != "render"));
    assert!(state_b.jobs.list().iter().all(|j| j.kind != "render"));
}
