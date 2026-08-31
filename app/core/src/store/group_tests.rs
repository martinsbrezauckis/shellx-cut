use super::*;
use crate::ops::ActorKind;

fn actor() -> Actor {
    Actor {
        kind: ActorKind::Agent,
        name: "group-test".into(),
        via: "test".into(),
        request: None,
    }
}

#[test]
fn preview_and_reject_restore_one_adjacent_group_and_undo_restores_it() {
    let root = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(root.path(), "group-test", None).unwrap();
    let first = store
        .apply(
            "edit.add_marker",
            json!({"at_ms": 100, "label": "A", "group_id": "linked-cut-01"}),
            actor(),
            None,
        )
        .unwrap();
    let last = store
        .apply(
            "edit.add_marker",
            json!({"at_ms": 200, "label": "B", "group_id": "linked-cut-01"}),
            actor(),
            None,
        )
        .unwrap();
    let preview = store.atomic_group_preview(&first.op_id).unwrap();
    assert_eq!(preview.first_op_id, first.op_id);
    assert_eq!(preview.last_op_id, last.op_id);
    assert_eq!(
        preview.op_ids,
        vec![first.op_id.clone(), last.op_id.clone()]
    );
    assert!(preview.is_reject_ready());

    let rejected = store
        .restore_atomic_group_tip(&preview, actor(), Some("reject linked cut".into()))
        .unwrap();
    assert_eq!(store.project.markers.len(), 0);
    let detail = &rejected.effects[0].detail;
    assert_eq!(detail["restored_group"]["group_id"], "linked-cut-01");
    assert_eq!(
        detail["restored_group"]["op_ids"],
        json!([first.op_id, last.op_id])
    );

    // The one restore op is itself one history step, so one undo restores
    // the complete original compound action rather than a partial member.
    store.undo(actor()).unwrap();
    assert_eq!(store.project.markers.len(), 2);
    let reopened = ProjectStore::open(&store.dir).unwrap();
    assert_eq!(reopened.project, store.project);
}

#[test]
fn preview_keeps_historical_group_visible_but_refuses_rejection() {
    let root = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(root.path(), "group-test", None).unwrap();
    let first = store
        .apply(
            "edit.add_marker",
            json!({"at_ms": 100, "label": "A", "group_id": "linked-cut-02"}),
            actor(),
            None,
        )
        .unwrap();
    store
        .apply(
            "edit.add_marker",
            json!({"at_ms": 200, "label": "B", "group_id": "linked-cut-02"}),
            actor(),
            None,
        )
        .unwrap();
    store
        .apply(
            "edit.add_marker",
            json!({"at_ms": 300, "label": "later"}),
            actor(),
            None,
        )
        .unwrap();
    let preview = store.atomic_group_preview(&first.op_id).unwrap();
    assert!(matches!(
        preview.reject_status,
        AtomicGroupRejectStatus::NotCurrentTip { .. }
    ));
    let before = store.project.clone();
    let before_len = store.log.read_all().unwrap().len();
    let error = store
        .restore_atomic_group_tip(&preview, actor(), None)
        .unwrap_err();
    assert_eq!(error.code, codes::GUARDRAIL);
    assert_eq!(store.project, before);
    assert_eq!(store.log.read_all().unwrap().len(), before_len);
}
