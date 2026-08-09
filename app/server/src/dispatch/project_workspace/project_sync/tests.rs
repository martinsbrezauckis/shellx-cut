use super::*;
use crate::state::AppState;

fn op(sequence: u64, verb: &str, args: Value, effects: Vec<cut_core::OpEffect>) -> OpRecord {
    OpRecord {
        op_id: OpRecord::format_id(sequence),
        ts: "2026-08-08T00:00:00.000Z".into(),
        actor: cut_core::Actor::system(),
        verb: verb.into(),
        args,
        rationale: None,
        effects,
        inverse: None,
        status: cut_core::OpStatus::Applied,
    }
}

#[test]
fn bounded_delta_discloses_affected_markers_without_a_project_snapshot() {
    let records = vec![
        op(
            1,
            "edit.add_marker",
            json!({"at_ms": 200, "label": "keep"}),
            vec![effect(
                None,
                json!({"added_marker": {"id": "m1", "at_ms": 200, "label": "keep"}}),
            )],
        ),
        op(2, "edit.remove_marker", json!({"id": "m1"}), vec![]),
    ];

    let (changes, affected) = bounded_changes(&records).expect("marker changes are lossless");
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0]["kind"], "marker_upsert");
    assert_eq!(changes[1], json!({"kind":"marker_remove", "id":"m1"}));
    assert_eq!(affected, json!({"markers": 2, "assets": 0, "project": 0}));
}

#[test]
fn unsupported_timeline_edit_requires_snapshot_fallback() {
    let records = vec![op(1, "edit.insert", json!({"asset": "a1"}), vec![])];
    assert!(bounded_changes(&records).is_none());
}

#[test]
fn limit_validation_is_closed_and_bounded() {
    assert_eq!(bounded_limit(None).unwrap(), DEFAULT_SYNC_LIMIT);
    assert_eq!(bounded_limit(Some(MAX_SYNC_LIMIT)).unwrap(), MAX_SYNC_LIMIT);
    assert_eq!(
        bounded_limit(Some(0)).unwrap_err().code,
        error_codes::INVALID_ARGS
    );
    assert_eq!(
        bounded_limit(Some(MAX_SYNC_LIMIT + 1)).unwrap_err().code,
        error_codes::INVALID_ARGS
    );
}

#[tokio::test]
async fn state_and_ops_use_the_same_bounded_revision_cursor_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = crate::dispatch::dispatch(
        &state,
        "project.create",
        json!({"name":"sync", "dir": dir.path().join("sync.cutproj")}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(created.ok, "project create: {:?}", created.error);
    let baseline = crate::dispatch::dispatch(
        &state,
        "project.state",
        json!({}),
        cut_core::Actor::system(),
    )
    .await;
    let revision = baseline.result.unwrap()["project_revision"]
        .as_str()
        .unwrap()
        .to_string();

    let marker = crate::dispatch::dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms": 200, "label": "reconnect"}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(marker.ok, "marker: {:?}", marker.error);

    let delta = crate::dispatch::dispatch(
        &state,
        "project.state",
        json!({"since_revision": revision, "limit": 1}),
        cut_core::Actor::system(),
    )
    .await;
    let delta = delta.result.unwrap();
    assert_eq!(delta["sync"]["mode"], "delta");
    assert_eq!(delta["sync"]["ops"].as_array().unwrap().len(), 1);
    assert_eq!(delta["sync"]["changes"][0]["kind"], "marker_upsert");

    let page = crate::dispatch::dispatch(
        &state,
        "project.ops",
        json!({"cursor": revision, "limit": 1}),
        cut_core::Actor::system(),
    )
    .await;
    let page = page.result.unwrap();
    assert_eq!(page["ops"].as_array().unwrap().len(), 1);
    assert_eq!(page["has_more"], false);
    assert_eq!(page["next_cursor"], Value::Null);
    assert_eq!(page["project_revision"], page["ops"][0]["op_id"]);
    assert!(page["encoded_bytes"].as_u64().unwrap() > 0);

    let second_marker = crate::dispatch::dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms": 300, "label": "too old for one page"}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(second_marker.ok, "second marker: {:?}", second_marker.error);
    let too_old = crate::dispatch::dispatch(
        &state,
        "project.state",
        json!({"since_revision": revision, "limit": 1}),
        cut_core::Actor::system(),
    )
    .await
    .result
    .unwrap();
    assert_eq!(too_old["sync"]["mode"], "snapshot");
    assert_eq!(too_old["sync"]["reason"], "too_old");

    let invalid = crate::dispatch::dispatch(
        &state,
        "project.state",
        json!({"since_revision": "op_999999"}),
        cut_core::Actor::system(),
    )
    .await
    .result
    .unwrap();
    assert_eq!(invalid["sync"]["mode"], "snapshot");
    assert_eq!(invalid["sync"]["reason"], "invalid_revision");

    let before_large = too_old["project_revision"].as_str().unwrap().to_string();
    let large = crate::dispatch::dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms": 400, "label": "x".repeat(MAX_SYNC_BYTES)}),
        cut_core::Actor::system(),
    )
    .await;
    assert!(large.ok, "large marker: {:?}", large.error);
    let too_large = crate::dispatch::dispatch(
        &state,
        "project.state",
        json!({"since_revision": before_large}),
        cut_core::Actor::system(),
    )
    .await
    .result
    .unwrap();
    assert_eq!(too_large["sync"]["mode"], "snapshot");
    assert_eq!(too_large["sync"]["reason"], "delta_too_large");
}
