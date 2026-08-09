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
