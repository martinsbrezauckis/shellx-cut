//! Core contract tests for Premiere/Resolve-style clip markers on MediaClip:
//! 1. Pre-0.6.122 projects load unchanged and re-save byte-for-byte identically.
//! 2. Move, trim, split, splice, duplicate, speed, reverse, undo/redo, and log
//!    replay preserve or map clip markers in source-media milliseconds.

use cut_core::{
    edit::{self, make_media_clip},
    rebuild_from_log, Actor, ActorKind, Asset, Clip, ClipMarker, Project, ProjectSettings,
    ProjectStore,
};
use serde_json::json;

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Agent,
        name: "clip-markers-test".into(),
        via: "test".into(),
        request: None,
    }
}

fn video_asset(dur_ms: u64) -> Asset {
    Asset {
        path: "/fixtures/recording.mp4".into(),
        hash: "sha256:clipmarkers".into(),
        probe: Some(json!({
            "kind": "video",
            "width": 1280,
            "height": 720,
            "fps": 30.0,
            "duration_ms": dur_ms,
            "has_audio": false
        })),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    }
}

#[test]
fn pre_clip_marker_fixture_loads_and_saves_byte_identically() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(dir.path(), "legacy_06121", None).unwrap();
    let proj_dir = store.dir.clone();
    store
        .record_import(Some("a1".into()), video_asset(6_000), actor(), None)
        .unwrap();
    store
        .apply(
            "edit.insert",
            json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 6_000], "ripple": false}),
            actor(),
            None,
        )
        .unwrap();

    // Verify serialized clip JSON has no `"markers"` key on MediaClip and re-saves byte-identically.
    let project_json_path = proj_dir.join("project.json");
    let ops_jsonl_path = proj_dir.join("ops.jsonl");
    let orig_project_bytes = std::fs::read(&project_json_path).unwrap();
    let orig_ops_bytes = std::fs::read(&ops_jsonl_path).unwrap();
    let orig_val: serde_json::Value = serde_json::from_slice(&orig_project_bytes).unwrap();
    let clip_obj = orig_val["tracks"][0]["clips"][0].as_object().unwrap();
    assert!(
        !clip_obj.contains_key("markers"),
        "empty clip markers must be serde-skipped on clip: {clip_obj:?}"
    );
    drop(store);

    let roundtrip_project: Project = serde_json::from_slice(&orig_project_bytes).unwrap();
    let reserialized = serde_json::to_vec_pretty(&roundtrip_project).unwrap();
    assert_eq!(
        reserialized, orig_project_bytes,
        "load + save of a project with no clip markers must produce byte-for-byte identical JSON"
    );

    // Real published legacy journal fixture (overlay96.jsonl) replays and serializes with zero clip markers.
    let legacy_ops: Vec<cut_core::OpRecord> =
        include_str!("../../server/src/dispatch/tests/fixtures/legacy_keyframes/overlay96.jsonl")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
    let legacy_project = rebuild_from_log(&legacy_ops).unwrap();
    let legacy_val = serde_json::to_value(&legacy_project).unwrap();
    for track in legacy_val["tracks"].as_array().unwrap() {
        for clip in track["clips"].as_array().unwrap() {
            assert!(
                clip.as_object().is_some_and(|m| !m.contains_key("markers")),
                "legacy fixture clip must not emit markers key: {clip}"
            );
        }
    }
    let legacy_bytes = serde_json::to_vec_pretty(&legacy_project).unwrap();
    let legacy_roundtrip: Project = serde_json::from_slice(&legacy_bytes).unwrap();
    assert_eq!(
        serde_json::to_vec_pretty(&legacy_roundtrip).unwrap(),
        legacy_bytes,
        "legacy fixture load+save must be byte-identical"
    );

    // Opening an existing 0.6.121 project must not rewrite project.json or ops.jsonl.
    let reopened = ProjectStore::open(&proj_dir).unwrap();
    assert_eq!(
        std::fs::read(&project_json_path).unwrap(),
        orig_project_bytes
    );
    assert_eq!(std::fs::read(&ops_jsonl_path).unwrap(), orig_ops_bytes);
    drop(reopened);
}

#[test]
fn move_preserves_clip_markers_and_source_positions() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "move", None).unwrap();
    s.record_import(Some("a1".into()), video_asset(6_000), actor(), None)
        .unwrap();
    s.apply_lowered(
        "edit.insert",
        json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 6_000], "ripple": false}),
        actor(),
        None,
        vec![
            cut_core::InverseOp {
                verb: "edit.insert".into(),
                args: json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 6_000], "ripple": false}),
            },
            cut_core::InverseOp {
                verb: "edit._set_clip_markers".into(),
                args: json!({
                    "clip": "c1",
                    "markers": [
                        {"source_ms": 1_200, "label": "First"},
                        {"source_ms": 4_100, "label": "Second"}
                    ]
                }),
            },
        ],
        vec![],
    )
    .unwrap();

    s.apply(
        "edit.move",
        json!({"clip": "c1", "to_track": "v1", "at_ms": 3_000, "ripple": false}),
        actor(),
        None,
    )
    .unwrap();

    let Clip::Media(c) = s
        .project
        .track("v1")
        .unwrap()
        .clips
        .iter()
        .find(|c| c.id() == Some("c1"))
        .unwrap()
    else {
        panic!("expected media clip c1");
    };
    assert_eq!(c.markers.len(), 2);
    assert_eq!(c.markers[0].source_ms, 1_200);
    assert_eq!(c.markers[1].source_ms, 4_100);
    assert_eq!(c.marker_timeline_ms(3_000, 1_200), Some(4_200));
    assert_eq!(c.marker_timeline_ms(3_000, 4_100), Some(7_100));
}

#[test]
fn trim_keeps_out_of_range_markers_in_data_and_hides_from_visible() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "trim", None).unwrap();
    s.record_import(Some("a1".into()), video_asset(6_000), actor(), None)
        .unwrap();
    s.apply_lowered(
        "edit.insert",
        json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 6_000], "ripple": false}),
        actor(),
        None,
        vec![
            cut_core::InverseOp {
                verb: "edit.insert".into(),
                args: json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 6_000], "ripple": false}),
            },
            cut_core::InverseOp {
                verb: "edit._set_clip_markers".into(),
                args: json!({
                    "clip": "c1",
                    "markers": [
                        {"source_ms": 900, "label": "Early"},
                        {"source_ms": 3_400, "label": "Middle"}
                    ]
                }),
            },
        ],
        vec![],
    )
    .unwrap();

    s.apply(
        "edit.trim",
        json!({"clip": "c1", "src_in_ms": 2_000, "src_out_ms": 5_000}),
        actor(),
        None,
    )
    .unwrap();

    let Clip::Media(trimmed) = &s.project.track("v1").unwrap().clips[0] else {
        panic!("expected media clip");
    };
    assert_eq!(
        trimmed.markers.len(),
        2,
        "out-of-range markers stay in data"
    );
    let vis: Vec<&str> = trimmed
        .visible_markers()
        .into_iter()
        .map(|m| m.label.as_str())
        .collect();
    assert_eq!(vis, vec!["Middle"]);
    assert_eq!(trimmed.marker_timeline_ms(0, 900), None);
    assert_eq!(trimmed.marker_timeline_ms(0, 3_400), Some(1_400));

    // Widen trim back: hidden marker comes back.
    s.apply(
        "edit.trim",
        json!({"clip": "c1", "src_in_ms": 0, "src_out_ms": 6_000}),
        actor(),
        None,
    )
    .unwrap();
    let Clip::Media(widened) = &s.project.track("v1").unwrap().clips[0] else {
        panic!("expected media clip");
    };
    let vis_widened: Vec<&str> = widened
        .visible_markers()
        .into_iter()
        .map(|m| m.label.as_str())
        .collect();
    assert_eq!(vis_widened, vec!["Early", "Middle"]);

    // Undo the widening: the trimmed state (one visible marker) comes back;
    // undo the trim too: both markers are visible again.
    s.undo(actor()).unwrap();
    let Clip::Media(retrimmed) = &s.project.track("v1").unwrap().clips[0] else {
        panic!("expected media clip");
    };
    assert_eq!(retrimmed.markers.len(), 2);
    assert_eq!(retrimmed.visible_markers().len(), 1);
    s.undo(actor()).unwrap();
    let Clip::Media(restored) = &s.project.track("v1").unwrap().clips[0] else {
        panic!("expected media clip");
    };
    assert_eq!((restored.src_in_ms, restored.src_out_ms), (0, 6_000));
    assert_eq!(restored.visible_markers().len(), 2);
}

#[test]
fn split_and_splice_give_both_halves_the_markers_each_visible_in_its_range() {
    let mut p = Project::new("split", ProjectSettings::default());
    p.assets.insert("a1".into(), video_asset(8_000));
    let mut c1 = make_media_clip("c1", "a1", 0, 6_000);
    c1.markers = vec![
        ClipMarker {
            id: "cm1".into(),
            source_ms: 1_000,
            label: "Left half".into(),
            note: None,
            color: None,
        },
        ClipMarker {
            id: "cm2".into(),
            source_ms: 4_500,
            label: "Right half".into(),
            note: None,
            color: None,
        },
    ];
    p.track_mut("v1").unwrap().clips.push(Clip::Media(c1));

    edit::split(&mut p, "v1", 3_000).unwrap();
    let v1 = p.track("v1").unwrap();
    assert_eq!(v1.clips.len(), 2);
    let (Clip::Media(left), Clip::Media(right)) = (&v1.clips[0], &v1.clips[1]) else {
        panic!("expected two media halves");
    };
    assert_eq!(left.markers.len(), 2, "left keeps both source markers");
    assert_eq!(
        right.markers.len(),
        2,
        "split must copy source markers onto right half too"
    );
    assert_eq!(
        left.visible_markers()
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec!["cm1"]
    );
    assert_eq!(
        right
            .visible_markers()
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec!["cm2"]
    );
    assert_eq!(right.marker_timeline_ms(3_000, 4_500), Some(4_500));
    let (left_id, right_id) = (left.id.clone(), right.id.clone());

    // Splice-split via insert into middle of left half (`[0, 3000)` at 1500ms).
    edit::insert(&mut p, "a1", "v1", 1_500, Some([6_000, 7_000]), false).unwrap();
    let spliced_right = p
        .track("v1")
        .unwrap()
        .clips
        .iter()
        .filter_map(|c| match c {
            Clip::Media(m) if m.id != left_id && m.id != right_id && m.src_in_ms == 1_500 => {
                Some(m)
            }
            _ => None,
        })
        .next()
        .expect("splice should create right half starting at 1500ms");
    assert_eq!(
        spliced_right.markers.len(),
        2,
        "splice_into_track right half must carry clip markers"
    );
}

#[test]
fn duplicate_copies_markers_with_new_deterministic_ids() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "dup", None).unwrap();
    s.record_import(Some("a1".into()), video_asset(6_000), actor(), None)
        .unwrap();
    s.apply_lowered(
        "edit.insert",
        json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 6_000], "ripple": false}),
        actor(),
        None,
        vec![
            cut_core::InverseOp {
                verb: "edit.insert".into(),
                args: json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 6_000], "ripple": false}),
            },
            cut_core::InverseOp {
                verb: "edit._set_clip_markers".into(),
                args: json!({
                    "clip": "c1",
                    "markers": [
                        {"source_ms": 1_000, "label": "One"},
                        {"source_ms": 4_000, "label": "Two"}
                    ]
                }),
            },
        ],
        vec![],
    )
    .unwrap();

    s.apply(
        "edit.duplicate",
        json!({"clip": "c1", "ripple": false}),
        actor(),
        None,
    )
    .unwrap();

    let v1 = s.project.track("v1").unwrap();
    let (Clip::Media(orig), Clip::Media(copy)) = (&v1.clips[0], &v1.clips[1]) else {
        panic!("expected original and duplicate clips");
    };
    assert_eq!(copy.markers.len(), 2);
    assert_ne!(
        copy.markers[0].id, orig.markers[0].id,
        "duplicate must mint new marker ids"
    );
    assert_ne!(copy.markers[1].id, orig.markers[1].id);
    assert_ne!(copy.markers[0].id, copy.markers[1].id);
    assert_eq!(copy.markers[0].source_ms, 1_000);
    assert_eq!(copy.markers[1].label, "Two");

    let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_string(&rebuilt).unwrap(),
        serde_json::to_string(&s.project).unwrap(),
        "duplicate marker ids must replay byte-identically"
    );
}

#[test]
fn speed_and_reverse_map_source_ms_to_timeline_time() {
    let mut c = make_media_clip("c1", "a1", 0, 4_000);
    c.markers = vec![
        ClipMarker {
            id: "cm1".into(),
            source_ms: 1_000,
            label: "A".into(),
            note: None,
            color: None,
        },
        ClipMarker {
            id: "cm2".into(),
            source_ms: 3_000,
            label: "B".into(),
            note: None,
            color: None,
        },
    ];

    c.speed = 2.0;
    assert_eq!(c.marker_timeline_ms(1_000, 1_000), Some(1_500));
    assert_eq!(c.marker_timeline_ms(1_000, 3_000), Some(2_500));

    c.speed = 1.0;
    c.reverse = true;
    assert_eq!(c.marker_timeline_ms(1_000, 1_000), Some(4_000));
    assert_eq!(c.marker_timeline_ms(1_000, 3_000), Some(2_000));

    c.speed = 2.0;
    c.reverse = true;
    assert_eq!(c.marker_timeline_ms(1_000, 1_000), Some(2_500));
    assert_eq!(c.marker_timeline_ms(1_000, 3_000), Some(1_500));
}

#[test]
fn undo_redo_and_replay_restore_clip_markers_byte_identically() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(dir.path(), "history", None).unwrap();
    s.record_import(Some("a1".into()), video_asset(8_000), actor(), None)
        .unwrap();
    s.apply_lowered(
        "edit.insert",
        json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 8_000], "ripple": false}),
        actor(),
        None,
        vec![
            cut_core::InverseOp {
                verb: "edit.insert".into(),
                args: json!({"asset": "a1", "track": "v1", "at_ms": 0, "src_range_ms": [0, 8_000], "ripple": false}),
            },
            cut_core::InverseOp {
                verb: "edit._set_clip_markers".into(),
                args: json!({
                    "clip": "c1",
                    "markers": [
                        {"source_ms": 1_500, "label": "Mark 1"},
                        {"source_ms": 5_500, "label": "Mark 2"}
                    ]
                }),
            },
        ],
        vec![],
    )
    .unwrap();

    s.apply(
        "edit.trim",
        json!({"clip": "c1", "src_in_ms": 2_000, "src_out_ms": 7_000}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.split",
        json!({"track": "v1", "at_ms": 2_000}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.reverse",
        json!({"clip": "c1", "reverse": true}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.reverse",
        json!({"clip": "c2", "reverse": true}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.speed",
        json!({"clip": "c2", "factor": 2.0}),
        actor(),
        None,
    )
    .unwrap();
    // Speed and reverse keep the source-time markers; only their timeline
    // position changes (right half [4_000,7_000) reversed at 2x: source 5_500
    // -> (7_000 - 5_500) / 2 = 750 ms into the clip).
    let Clip::Media(fast) = s
        .project
        .track("v1")
        .unwrap()
        .clips
        .iter()
        .find(|c| c.id() == Some("c2"))
        .unwrap()
    else {
        panic!("expected media clip c2");
    };
    assert_eq!((fast.src_in_ms, fast.src_out_ms), (4_000, 7_000));
    assert_eq!(fast.markers.len(), 2);
    assert_eq!(fast.markers[1].source_ms, 5_500);
    assert_eq!(fast.marker_timeline_ms(0, 5_500), Some(750));

    let after_edits = serde_json::to_string(&s.project).unwrap();
    for _ in 0..4 {
        s.undo(actor()).unwrap();
    }
    assert_eq!(s.project.track("v1").unwrap().clips.len(), 1);
    for _ in 0..4 {
        s.redo(actor()).unwrap();
    }
    assert_eq!(serde_json::to_string(&s.project).unwrap(), after_edits);

    let rebuilt = rebuild_from_log(&s.log.read_all().unwrap()).unwrap();
    assert_eq!(serde_json::to_string(&rebuilt).unwrap(), after_edits);
}
