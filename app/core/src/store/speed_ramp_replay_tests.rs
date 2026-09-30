//! Durable replay boundaries for speed-ramp timebase journal arguments.

use super::*;
use crate::types::{Clip, ColorConfig, SpeedRamp};

fn settings(fps: f64) -> ProjectSettings {
    ProjectSettings {
        width: 320,
        height: 240,
        fps,
        audio_rate: 48_000,
        color: ColorConfig::default(),
    }
}

fn asset() -> Asset {
    Asset {
        path: "source.mp4".into(),
        hash: "sha256:test".into(),
        probe: None,
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

fn ramp_args(timebase: Option<(f64, u32)>) -> Value {
    let mut args = json!({
        "clip": "c1",
        "points": [
            {"at_ms": 0, "factor": 1.0},
            {"at_ms": 2500, "factor": 3.0},
            {"at_ms": 5000, "factor": 1.0}
        ],
        "segments": 80
    });
    if let (Some((fps, audio_rate)), Value::Object(map)) = (timebase, &mut args) {
        map.insert("timebase_fps".into(), json!(fps));
        map.insert("timebase_audio_rate".into(), json!(audio_rate));
    }
    args
}

fn add_media_and_clip(store: &mut ProjectStore) {
    store
        .record_import(Some("a1".into()), asset(), Actor::system(), None)
        .unwrap();
    store
        .apply(
            "edit.insert",
            json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,5000]}),
            Actor::system(),
            None,
        )
        .unwrap();
}

fn ramp(project: &Project) -> &SpeedRamp {
    match &project.track("v1").unwrap().clips[0] {
        Clip::Media(clip) => clip.speed_ramp.as_ref().unwrap(),
        _ => unreachable!(),
    }
}

fn reopen_from_journal(dir: &std::path::Path) -> ProjectStore {
    std::fs::remove_file(dir.join("project.json")).unwrap();
    ProjectStore::open(dir).unwrap()
}

#[test]
fn invalid_project_cache_ramp_counts_rebuild_from_supported_journal() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(temp.path(), "cache", Some(settings(30.0))).unwrap();
    add_media_and_clip(&mut store);
    store
        .apply("edit.speed_ramp", ramp_args(None), Actor::system(), None)
        .unwrap();
    let before = store.project.clone();
    let dir = store.dir.clone();
    drop(store);
    for field in ["segments", "preferred_segments"] {
        for count in [121, usize::MAX] {
            let mut cache = serde_json::to_value(&before).unwrap();
            cache["tracks"][0]["clips"][0]["speed_ramp"][field] = json!(count);
            std::fs::write(
                dir.join("project.json"),
                serde_json::to_vec(&cache).unwrap(),
            )
            .unwrap();
            let reopened = ProjectStore::open(&dir).unwrap();
            assert_eq!(reopened.project, before);
        }
    }
}

#[test]
fn imported_ramp_counts_are_rejected_without_mutating_restore_state() {
    for field in ["segments", "preferred_segments"] {
        for count in [121, usize::MAX] {
            let temp = tempfile::tempdir().unwrap();
            let mut store =
                ProjectStore::create(temp.path(), "counts", Some(settings(30.0))).unwrap();
            add_media_and_clip(&mut store);
            let before = store.project.clone();
            let prior = store.log.read_all().unwrap();
            let mut snapshot = timeline_snapshot(&before).args;
            let mut bad_ramp = json!({
                "segments": 2,
                "points": [{"at_ms":0,"factor":1.0},{"at_ms":5000,"factor":1.0}]
            });
            bad_ramp[field] = json!(count);
            snapshot["tracks"][0]["clips"][0]["speed_ramp"] = bad_ramp;
            for detail_key in ["restored_timeline", "rebase_new_timeline"] {
                let mut detail = serde_json::Map::new();
                detail.insert(detail_key.into(), snapshot.clone());
                let record = OpRecord {
                    op_id: store.log.next_id().unwrap(),
                    ts: OpRecord::now_ts(),
                    actor: Actor::system(),
                    verb: "edit.restore".into(),
                    args: json!({}),
                    rationale: None,
                    effects: vec![crate::ops::OpEffect {
                        track: None,
                        detail,
                    }],
                    inverse: None,
                    status: OpStatus::Applied,
                };
                let mut candidate = before.clone();
                assert!(apply_record(&mut candidate, &record, &prior).is_err());
                assert_eq!(candidate, before);
            }
            let mut original = prior.last().unwrap().clone();
            original.inverse = Some(crate::ops::InverseOp {
                verb: "edit._set_timeline".into(),
                args: snapshot.clone(),
            });
            let legacy_restore = OpRecord {
                op_id: store.log.next_id().unwrap(),
                ts: OpRecord::now_ts(),
                actor: Actor::system(),
                verb: "edit.restore".into(),
                args: json!({"op_id": original.op_id}),
                rationale: None,
                effects: vec![],
                inverse: None,
                status: OpStatus::Applied,
            };
            let mut candidate = before.clone();
            assert!(apply_record(&mut candidate, &legacy_restore, &[original]).is_err());
            assert_eq!(candidate, before);

            let imported = OpRecord {
                effects: vec![edit::fx(None, json!({"restored_timeline": snapshot}))],
                ..legacy_restore
            };
            store.log.append(&imported).unwrap();
            let dir = store.dir.clone();
            drop(store);
            let error = ProjectStore::open(&dir).unwrap_err();
            assert_eq!(error.code, codes::INVALID_ARGS, "{error}");
        }
    }
}

#[test]
fn historic_ramp_op_reopens_with_millisecond_semantics() {
    let temp = tempfile::tempdir().unwrap();
    let (dir, journal, legacy_duration) = {
        let mut store = ProjectStore::create(temp.path(), "legacy", Some(settings(30.0))).unwrap();
        add_media_and_clip(&mut store);
        store
            .set_format(None, None, Some(24.0), Actor::system(), None)
            .unwrap();
        let legacy = store
            .apply("edit.speed_ramp", ramp_args(None), Actor::system(), None)
            .unwrap();
        assert!(legacy.args.get("timebase_fps").is_none());
        assert!(legacy.args.get("timebase_audio_rate").is_none());
        let duration = store.project.track("v1").unwrap().duration_ms();
        store
            .set_format(None, None, Some(60.0), Actor::system(), None)
            .unwrap();
        let journal = store.log.read_all().unwrap();
        assert_eq!(
            journal
                .iter()
                .map(|op| op.verb.as_str())
                .collect::<Vec<_>>(),
            [
                "project.create",
                "media.import",
                "edit.insert",
                "project.format",
                "edit.speed_ramp",
                "project.format",
            ]
        );
        (store.dir.clone(), journal, duration)
    };

    let reopened = reopen_from_journal(&dir);
    let restored = ramp(&reopened.project);
    assert_eq!(restored.timebase_fps, None);
    assert_eq!(restored.timebase_audio_rate, None);
    assert_eq!(reopened.project.settings.fps, 60.0);
    assert_eq!(
        reopened.project.track("v1").unwrap().duration_ms(),
        legacy_duration
    );
    assert_eq!(reopened.project, rebuild_from_log(&journal).unwrap());
}

#[test]
fn timebased_ramp_regrids_across_format_changes_and_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let (dir, journal, persisted_duration) = {
        let mut store = ProjectStore::create(temp.path(), "grid", Some(settings(30.0))).unwrap();
        add_media_and_clip(&mut store);
        store
            .set_format(None, None, Some(60.0), Actor::system(), None)
            .unwrap();
        let committed = store
            .apply(
                "edit.speed_ramp",
                ramp_args(Some((60.0, 48_000))),
                Actor::system(),
                None,
            )
            .unwrap();
        assert_eq!(committed.args["timebase_fps"], 60.0);
        assert_eq!(committed.args["timebase_audio_rate"], 48_000);
        assert_eq!(ramp(&store.project).preferred_segments, Some(80));
        assert_eq!(ramp(&store.project).segments, 25, "60fps frame cap wins");
        store
            .set_format(None, None, Some(24.0), Actor::system(), None)
            .unwrap();
        assert_eq!(ramp(&store.project).timebase_fps, Some(24.0));
        assert_eq!(ramp(&store.project).preferred_segments, Some(80));
        assert_eq!(
            ramp(&store.project).segments,
            10,
            "24fps regrid clamps safely"
        );
        store
            .set_format(None, None, Some(60.0), Actor::system(), None)
            .unwrap();
        assert_eq!(ramp(&store.project).timebase_fps, Some(60.0));
        assert_eq!(ramp(&store.project).timebase_audio_rate, Some(48_000));
        assert_eq!(ramp(&store.project).preferred_segments, Some(80));
        assert_eq!(
            ramp(&store.project).segments,
            25,
            "restoring the output grid restores the retained request"
        );
        let duration = store.project.track("v1").unwrap().duration_ms();
        (store.dir.clone(), store.log.read_all().unwrap(), duration)
    };

    let reopened = reopen_from_journal(&dir);
    let restored = ramp(&reopened.project);
    assert_eq!(reopened.project.settings.fps, 60.0);
    assert_eq!(restored.timebase_fps, Some(60.0));
    assert_eq!(restored.timebase_audio_rate, Some(48_000));
    assert_eq!(restored.preferred_segments, Some(80));
    assert_eq!(
        restored.segments, 25,
        "replay restores the retained request when the 60fps cap permits it"
    );
    assert_eq!(
        reopened.project.track("v1").unwrap().duration_ms(),
        persisted_duration
    );
    assert_eq!(reopened.project, rebuild_from_log(&journal).unwrap());
}
