//! Imported request receipt roots cannot redirect publication or replay.

use super::{prepare, receipt};
use cut_core::{error_codes, Actor, ActorKind, ProjectStore, VerbResult};
use serde_json::json;

fn fixture(with_ops: bool) -> (tempfile::TempDir, ProjectStore, Actor, VerbResult) {
    let temp = tempfile::tempdir().unwrap();
    let mut store = ProjectStore::create(temp.path(), "request-boundary", None).unwrap();
    let actor = prepare(
        "edit.add_marker",
        json!({"request_id":"request-1","at_ms":1}),
        Actor {
            kind: ActorKind::Agent,
            name: "fixture".into(),
            via: "rest".into(),
            request: None,
        },
    )
    .unwrap()
    .actor;
    let mut response = if with_ops {
        let op = store
            .apply(
                "edit.add_marker",
                json!({"at_ms":1,"label":"fixture"}),
                actor.clone(),
                None,
            )
            .unwrap();
        VerbResult::ok_with_ops(json!({"marker":"fixture"}), vec![op.op_id])
    } else {
        VerbResult::ok(json!({"status":"cancelled"}))
    };
    response.project_revision = store.log.current_revision().unwrap();
    (temp, store, actor, response)
}

fn write(
    store: &ProjectStore,
    actor: &Actor,
    response: &VerbResult,
    with_ops: bool,
) -> Result<(), cut_core::CutError> {
    let revision = response.project_revision.as_deref().unwrap();
    if with_ops {
        receipt::write(store, "edit.add_marker", actor, revision, response)
    } else {
        receipt::write_without_ops(store, "edit.add_marker", actor, revision, response)
    }
}

#[test]
fn request_receipts_preserve_publication_replay_and_idempotent_bytes() {
    for with_ops in [false, true] {
        let (_temp, store, actor, response) = fixture(with_ops);
        assert!(
            receipt::replay_without_ops(&store, "edit.add_marker", &actor)
                .unwrap()
                .is_none()
        );
        assert!(
            !store.dir.join("request-receipts").exists(),
            "lookup must not create its root"
        );
        write(&store, &actor, &response, with_ops).unwrap();
        let root = store.dir.join("request-receipts");
        let path = std::fs::read_dir(&root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(path.extension().unwrap(), "json");
        assert_eq!(path.file_stem().unwrap().to_str().unwrap().len(), 64);
        let bytes = std::fs::read(&path).unwrap();
        write(&store, &actor, &response, with_ops).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            1,
            "no temporary files remain"
        );
        let replay = if with_ops {
            let ids = store.log.request_ops(&actor).unwrap().unwrap();
            receipt::replay(&store, "edit.add_marker", &actor, &ids).unwrap()
        } else {
            receipt::replay_without_ops(&store, "edit.add_marker", &actor)
                .unwrap()
                .unwrap()
        };
        assert_eq!(
            serde_json::to_value(replay).unwrap(),
            serde_json::to_value(&response).unwrap()
        );
        let mut conflicting = actor.clone();
        conflicting.request.as_mut().unwrap().fingerprint = "different".into();
        assert!(write(&store, &conflicting, &response, with_ops).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[cfg(unix)]
#[test]
fn attached_request_receipt_roots_reject_links_before_read_or_write() {
    for with_ops in [false, true] {
        for dangling in [false, true] {
            let (temp, store, actor, response) = fixture(with_ops);
            write(&store, &actor, &response, with_ops).unwrap();
            let root = store.dir.join("request-receipts");
            let outside = temp.path().join("outside");
            std::fs::rename(&root, &outside).unwrap();
            let outside_path = std::fs::read_dir(&outside)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let bytes = std::fs::read(&outside_path).unwrap();
            let target = if dangling {
                temp.path().join("absent")
            } else {
                outside.clone()
            };
            std::os::unix::fs::symlink(&target, &root).unwrap();
            let error = write(&store, &actor, &response, with_ops).unwrap_err();
            assert_eq!(error.code, error_codes::INVALID_ARGS);
            assert!(receipt::replay_without_ops(&store, "edit.add_marker", &actor).is_err());
            if with_ops {
                let ids = store.log.request_ops(&actor).unwrap().unwrap();
                assert!(receipt::replay(&store, "edit.add_marker", &actor, &ids).is_err());
            }
            assert_eq!(std::fs::read(&outside_path).unwrap(), bytes);
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
            assert!(!temp.path().join("absent").exists());
        }
    }
}

#[test]
fn attached_request_receipt_roots_reject_non_directories() {
    for with_ops in [false, true] {
        let (_temp, store, actor, response) = fixture(with_ops);
        let root = store.dir.join("request-receipts");
        std::fs::write(&root, b"keep").unwrap();
        assert!(write(&store, &actor, &response, with_ops).is_err());
        assert!(receipt::replay_without_ops(&store, "edit.add_marker", &actor).is_err());
        assert_eq!(std::fs::read(&root).unwrap(), b"keep");
    }
}

#[cfg(unix)]
#[test]
fn request_receipt_leaf_links_cannot_supply_replay_or_redirect_write() {
    for with_ops in [false, true] {
        let (temp, store, actor, response) = fixture(with_ops);
        write(&store, &actor, &response, with_ops).unwrap();
        let root = store.dir.join("request-receipts");
        let leaf = std::fs::read_dir(&root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let outside = temp.path().join("outside.json");
        std::fs::rename(&leaf, &outside).unwrap();
        let bytes = std::fs::read(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, &leaf).unwrap();
        assert!(write(&store, &actor, &response, with_ops).is_err());
        assert!(receipt::replay_without_ops(&store, "edit.add_marker", &actor).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), bytes);
    }
}

#[cfg(unix)]
#[test]
fn attached_linked_root_cannot_publish_a_new_request_receipt() {
    for with_ops in [false, true] {
        let (temp, store, actor, response) = fixture(with_ops);
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("sentinel"), b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, store.dir.join("request-receipts")).unwrap();
        assert!(write(&store, &actor, &response, with_ops).is_err());
        assert!(receipt::replay_without_ops(&store, "edit.add_marker", &actor).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
        assert_eq!(std::fs::read(outside.join("sentinel")).unwrap(), b"keep");
    }
}
