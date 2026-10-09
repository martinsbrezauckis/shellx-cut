use super::*;

fn actor() -> Actor {
    Actor {
        kind: cut_core::ActorKind::Agent,
        name: "autopilot-noop-test".into(),
        via: "test".into(),
        request: None,
    }
}

#[tokio::test]
async fn timeline_fix_count_uses_real_dispatch_operations() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"autopilot-noop", "dir": dir.path().join("autopilot-noop.cutproj")}),
        actor(),
    )
    .await;
    assert!(created.ok, "project.create: {:?}", created.error);

    let media = dir.path().join("clip.mp4");
    std::fs::write(&media, b"not-really-video").unwrap();
    let imported = dispatch(&state, "media.import", json!({"path": media}), actor()).await;
    assert!(imported.ok, "media.import: {:?}", imported.error);
    let inserted = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1", "track":"v1", "at_ms":0, "src_range_ms":[0,5000]}),
        actor(),
    )
    .await;
    assert!(inserted.ok, "edit.insert: {:?}", inserted.error);
    let clip = inserted.result.as_ref().unwrap()["clip_id"]
        .as_str()
        .unwrap();

    let no_words = dispatch(&state, "edit.trim_edges", json!({}), actor()).await;
    assert!(
        no_words.ok,
        "trim_edges without words: {:?}",
        no_words.error
    );
    assert!(no_words.op_ids.as_ref().is_none_or(Vec::is_empty));
    assert_eq!(no_words.result.as_ref().unwrap()["leading_trimmed_ms"], 0);
    assert!(!autopilot_timeline_fix_applied(&no_words));

    let missing_captions = dispatch(&state, "captions.reflow", json!({}), actor()).await;
    assert!(!missing_captions.ok);
    assert!(!autopilot_timeline_fix_applied(&missing_captions));

    let trimmed = dispatch(
        &state,
        "edit.trim",
        json!({"clip": clip, "src_out_ms": 4000}),
        actor(),
    )
    .await;
    assert!(trimmed.ok, "edit.trim: {:?}", trimmed.error);
    assert!(autopilot_timeline_fix_applied(&trimmed));
    assert_eq!(trimmed.op_ids.as_ref().map(Vec::len), Some(1));

    let added = dispatch(
        &state,
        "captions.add_text",
        json!({"text":"A deliberately long timed text card for reflow", "range_ms":[1000,3000], "position":"center"}),
        actor(),
    )
    .await;
    assert!(added.ok, "captions.add_text: {:?}", added.error);
    let reflowed = dispatch(&state, "captions.reflow", json!({"max_chars":12}), actor()).await;
    assert!(reflowed.ok, "captions.reflow: {:?}", reflowed.error);
    assert!(
        reflowed.result.as_ref().unwrap()["cues_after"]
            .as_u64()
            .unwrap()
            > 1
    );
    assert!(autopilot_timeline_fix_applied(&reflowed));
    assert_eq!(reflowed.op_ids.as_ref().map(Vec::len), Some(1));

    // Build the same report-list decision from the real verb envelopes.
    let mut report = vec![json!({"check":"lufs", "via":"render.final"})];
    if autopilot_timeline_fix_applied(&no_words) {
        report.push(json!({"check":"silence_at_edges", "via":"edit.trim_edges"}));
    }
    if !missing_captions.ok {
        report.push(json!({"check":"caption_presence", "via":"captions.reflow", "failed":missing_captions.error.unwrap().message}));
    }
    if autopilot_timeline_fix_applied(&trimmed) {
        report.push(json!({"check":"cut_on_word", "via":"edit.trim"}));
    }
    if autopilot_timeline_fix_applied(&reflowed) {
        report.push(json!({"check":"caption_overlap", "via":"captions.reflow"}));
    }
    assert_eq!(report.len(), 4);
    assert!(!report.iter().any(|fix| fix["via"] == "edit.trim_edges"));
    assert!(report.iter().any(|fix| fix.get("failed").is_some()));
    assert_eq!(autopilot_applied_fix_count(&report), 3);
    assert_eq!(autopilot_applied_fix_count(&[json!({"failed":null})]), 0);
}
