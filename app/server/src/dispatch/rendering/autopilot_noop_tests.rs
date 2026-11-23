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

    // Without a transcript trim_edges is refused with the reason (it is no
    // longer an ok that reads like "no dead air"), and is not a fix either.
    let no_words = dispatch(&state, "edit.trim_edges", json!({}), actor()).await;
    assert!(!no_words.ok, "trim_edges without a transcript must refuse");
    assert!(no_words.op_ids.as_ref().is_none_or(Vec::is_empty));
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

#[test]
fn autopilot_summary_requires_passing_checks_and_distinguishes_not_measured() {
    let base = AutopilotSummaryInput {
        policy: "preview".into(),
        pass: false,
        auto_fixable_planned: 0,
        unverified: false,
        stalled: false,
        iterations: 0,
        applied_fixes: 0,
        failed_checks: 0,
        unmeasured_checks: 0,
        measured_checks: 2,
    };

    assert_eq!(
        autopilot_summary(&AutopilotSummaryInput {
            pass: true,
            ..base.clone()
        }),
        "All checks already pass — nothing to fix."
    );
    assert_eq!(
        autopilot_summary(&AutopilotSummaryInput {
            failed_checks: 1,
            ..base.clone()
        }),
        "Some checks did not pass and none can be fixed automatically. Open Inspect to see them."
    );
    assert_eq!(
        autopilot_summary(&AutopilotSummaryInput {
            unmeasured_checks: 3,
            ..base.clone()
        }),
        "Measured checks passed, and 3 check(s) were not measured. Open Inspect to see them."
    );
    assert_eq!(
        autopilot_summary(&AutopilotSummaryInput {
            unmeasured_checks: 2,
            measured_checks: 0,
            ..base.clone()
        }),
        "2 check(s) were not measured. Open Inspect to see them."
    );
    assert!(autopilot_summary(&AutopilotSummaryInput {
        unverified: true,
        ..base.clone()
    })
    .contains("not measured"));
    assert!(autopilot_summary(&AutopilotSummaryInput {
        auto_fixable_planned: 2,
        failed_checks: 2,
        ..base.clone()
    })
    .starts_with("Preview: 2 issue(s) fixable"));
    assert!(autopilot_summary(&AutopilotSummaryInput {
        policy: "auto_low_risk".into(),
        unmeasured_checks: 4,
        ..base
    })
    .contains("not measured"));
}
