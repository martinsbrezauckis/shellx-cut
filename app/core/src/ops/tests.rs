use super::*;

fn op(verb: &str) -> OpRecord {
    OpRecord {
        op_id: "op_000001".into(),
        ts: "2026-07-01T00:00:00.000Z".into(),
        actor: crate::Actor {
            kind: crate::ActorKind::Agent,
            name: "test".into(),
            via: "test".into(),
            request: None,
        },
        verb: verb.into(),
        args: serde_json::json!({}),
        rationale: None,
        effects: Vec::new(),
        inverse: None,
        status: OpStatus::Applied,
    }
}

#[test]
fn project_metadata_ops_are_not_timeline_mutations() {
    assert!(!op("project.rename").mutates_timeline().unwrap());
    assert!(!op("project.format").mutates_timeline().unwrap());
    assert!(!op("project.color").mutates_timeline().unwrap());
    assert!(!op("project.brand").mutates_timeline().unwrap());
    assert!(!op("comment.import").mutates_timeline().unwrap());
    assert!(op("edit.insert").mutates_timeline().unwrap());
}

#[test]
fn unknown_journal_verbs_fail_closed_instead_of_becoming_timeline_mutations() {
    let error = op("future.unclassified").mutates_timeline().unwrap_err();
    assert_eq!(error.code, crate::error::codes::INVALID_ARGS);
    assert!(error.message.contains("future.unclassified"));
}

#[test]
fn request_identity_deduplicates_and_revision_guard_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ops.jsonl");
    let log = OpLog::open(&path).unwrap();
    let initial = op("project.create");
    log.append(&initial).unwrap();

    let request = crate::MutationRequest {
        caller: "[\"agent\",\"test\",\"test\"]".into(),
        request_id: "request-0001".into(),
        fingerprint: format!("sha256:{}", "a".repeat(64)),
        expected_revision: Some("op_000001".into()),
    };
    let mut edit = op("edit.add_marker");
    edit.op_id = "op_000002".into();
    edit.actor.request = Some(request.clone());
    log.append(&edit).unwrap();
    assert_eq!(
        log.current_revision().unwrap().as_deref(),
        Some("op_000002")
    );
    assert_eq!(
        log.request_ops(&edit.actor).unwrap(),
        Some(vec!["op_000002".into()])
    );

    let reopened = OpLog::open(&path).unwrap();
    assert_eq!(
        reopened.request_ops(&edit.actor).unwrap(),
        Some(vec!["op_000002".into()])
    );

    let mut changed = edit.actor.clone();
    changed.request.as_mut().unwrap().fingerprint = format!("sha256:{}", "b".repeat(64));
    assert_eq!(
        reopened.request_ops(&changed).unwrap_err().code,
        crate::error::codes::CONFLICT
    );

    let mut stale = op("edit.add_marker");
    stale.op_id = "op_000003".into();
    stale.actor.request = Some(crate::MutationRequest {
        caller: request.caller,
        request_id: "request-0002".into(),
        fingerprint: format!("sha256:{}", "c".repeat(64)),
        expected_revision: Some("op_000001".into()),
    });
    assert_eq!(
        reopened.append(&stale).unwrap_err().code,
        crate::error::codes::CONFLICT
    );
    assert_eq!(
        reopened.current_revision().unwrap().as_deref(),
        Some("op_000002")
    );
}

#[test]
fn replay_view_refuses_an_externally_changed_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ops.jsonl");
    let log = OpLog::open(&path).unwrap();
    log.append(&op("project.create")).unwrap();
    std::fs::write(&path, b" \n").unwrap();

    let error = log.replay_view().unwrap_err();
    assert_eq!(error.code, crate::error::codes::CONFLICT);
    assert!(error.message.contains("changed outside"));

    let page_error = log.page_after(None, 1, 1024).unwrap_err();
    assert_eq!(page_error.code, crate::error::codes::CONFLICT);
}
