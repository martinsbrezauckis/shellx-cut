use super::*;

#[test]
fn recording_edits_replay_undo_and_preserve_clip_fields() {
    let d = tempfile::tempdir().unwrap();
    let mut s = ProjectStore::create(d.path(), "demo", None).unwrap();
    s.apply_atomic_media_insert_plan(
        &"a".repeat(64),
        vec![asset()],
        vec![json!({"track":"v1","at_ms":0,"src_range_ms":[0,1000]})],
        actor(),
        None,
    )
    .unwrap();
    let take = json!({"id":"take-1","capture_folder":"cache/take","source":"source.mp4","events":"events.json","pending_plan":"p1","applied_plan":"p1","clip_ids":["c1"],"asset":"a1","segments":[]});
    let before = serde_json::to_vec(&s.project).unwrap();
    assert!(!String::from_utf8(before.clone())
        .unwrap()
        .contains("recordings"));
    assert_eq!(
        before,
        serde_json::to_vec(&serde_json::from_slice::<Project>(&before).unwrap()).unwrap()
    );
    let checkpoint = s.checkpoint("before-take", actor(), None).unwrap();
    s.apply(
        "screen_record.take_link",
        json!({"recording":take}),
        actor(),
        None,
    )
    .unwrap();
    s.undo(actor()).unwrap();
    assert!(s.project.recordings.is_empty());
    s.redo(actor()).unwrap();
    s.apply(
        "screen_record.zoom_set",
        json!({"take":"take-1","segments":[],"pending_plan":"p2"}),
        actor(),
        None,
    )
    .unwrap();
    assert!(s.project.recordings[0].stale());
    s.undo(actor()).unwrap();
    assert!(!s.project.recordings[0].stale());
    s.redo(actor()).unwrap();
    s.apply(
        "edit.split",
        json!({"track":"v1","at_ms":500}),
        actor(),
        None,
    )
    .unwrap();
    s.apply(
        "edit.speed",
        json!({"clip":"c1","factor":2.0}),
        actor(),
        None,
    )
    .unwrap();
    let old = s.project.tracks.clone();
    let refresh = json!({"take":"take-1","asset_id":"a2","asset":Asset { path:"/new.mp4".into(), hash:"sha256:new".into(), ..asset() },"applied_plan":"p2"});
    s.apply("screen_record.take_refresh", refresh, actor(), None)
        .unwrap();
    assert!(!s.project.recordings[0].stale());
    let mut expected = old.clone();
    for track in &mut expected {
        for clip in &mut track.clips {
            if let Clip::Media(m) = clip {
                m.asset = "a2".into();
            }
        }
    }
    assert_eq!(s.project.tracks, expected);
    let dir = s.dir.clone();
    drop(s);
    let mut s = ProjectStore::open(&dir).unwrap();
    assert_eq!(s.project.tracks, expected);
    s.undo(actor()).unwrap();
    assert_eq!(s.project.tracks, old);
    assert!(s.project.recordings[0].stale());
    s.redo(actor()).unwrap();
    assert_eq!(s.project.tracks, expected);
    s.revert(&checkpoint.0.id, actor()).unwrap();
    assert!(s.project.recordings.is_empty());
}
