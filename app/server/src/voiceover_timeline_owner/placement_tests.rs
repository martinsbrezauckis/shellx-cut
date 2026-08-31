use super::super::VoiceoverMaterialization;
use super::{commit, commit_with_actor, VoiceoverPreviewPlaybackRequest, PREVIEW_COMMAND};
use cut_core::{
    apply_record, error_codes, Actor, MutationRequest, Project, ProjectCacheHealth,
    ProjectSettings, ProjectStore,
};
use record_capture::VoiceoverArtifact;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

fn materialization(
    path: &Path,
    revision: &str,
    request_id: &str,
    start_ms: u64,
) -> VoiceoverMaterialization {
    let bytes = std::fs::read(path).unwrap();
    VoiceoverMaterialization {
        take: super::super::AcceptedVoiceoverTimeline {
            request_id: request_id.into(),
            revision: revision.into(),
            audio_track: "a1t".into(),
            start_ms,
            out_ms: None,
        },
        artifact: VoiceoverArtifact {
            path: path.into(),
            sha256: format!("{:x}", Sha256::digest(bytes.as_slice())),
            bytes: bytes.len() as u64,
            duration_ms: 160,
            sample_rate_hz: 48_000,
            channels: 1,
        },
        device_lost_after_prefix: false,
    }
}

fn store() -> (tempfile::TempDir, ProjectStore, std::path::PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("sealed-voiceover.wav");
    std::fs::write(&source, b"sealed WAV fixture bytes").unwrap();
    let store =
        ProjectStore::create(root.path(), "voiceover", Some(ProjectSettings::default())).unwrap();
    (root, store, source)
}

fn audio_media_count(store: &ProjectStore) -> usize {
    store
        .project
        .track("a1t")
        .unwrap()
        .clips
        .iter()
        .filter(|clip| clip.id().is_some())
        .count()
}

#[test]
fn placement_retries_exactly_and_one_undo_removes_its_asset_and_clip() {
    let (_root, mut store, source) = store();
    let take = materialization(&source, "op_000001", "voiceover-placement-1", 120);
    let first = commit(&mut store, &take).unwrap();
    assert_eq!(
        store
            .log
            .request_ops(&super::placement_actor(
                &take,
                &super::placement_fingerprint(&take),
            ))
            .unwrap(),
        Some(vec![first.op_id.clone()])
    );
    assert_eq!(store.project.assets.len(), 1);
    assert_eq!(audio_media_count(&store), 1);
    let retry = commit(&mut store, &take).unwrap();
    assert_eq!(retry.asset_id, first.asset_id);
    assert_eq!(retry.clip_id, first.clip_id);
    assert_eq!(retry.op_id, first.op_id);
    assert!(retry.already_applied);
    assert_eq!(store.project.assets.len(), 1);
    assert_eq!(audio_media_count(&store), 1);

    store.undo(Actor::system()).unwrap();
    assert!(store.project.assets.is_empty());
    assert!(store.project.track("a1t").unwrap().clips.is_empty());
    assert!(
        !store.undo_available(),
        "placement must consume exactly one Undo"
    );
    assert_eq!(
        store.undo(Actor::system()).unwrap_err().code,
        error_codes::GUARDRAIL
    );
}

#[test]
fn placement_preserves_original_controlled_actor_for_lost_response_replay() {
    let (_root, mut store, source) = store();
    let take = materialization(&source, "op_000001", "voiceover-placement-replay", 120);
    let actor = Actor {
        kind: cut_core::ActorKind::Human,
        name: "ui-owner".into(),
        via: "ui".into(),
        request: Some(MutationRequest {
            caller: "voiceover-owner-test".into(),
            request_id: "voiceover-placement-replay".into(),
            fingerprint: "a".repeat(64),
            expected_revision: Some("op_000001".into()),
        }),
    };
    let first = commit_with_actor(&mut store, &take, actor.clone()).unwrap();
    assert_eq!(
        store.log.request_ops(&actor).unwrap(),
        Some(vec![first.op_id.clone()])
    );
    let replay = commit_with_actor(&mut store, &take, actor.clone()).unwrap();
    assert!(replay.already_applied);
    assert_eq!(replay.op_id, first.op_id);
    assert_eq!(
        store.log.request_ops(&actor).unwrap(),
        Some(vec![first.op_id])
    );
}

#[test]
fn undone_placement_stays_refused_until_redo_restores_both_from_the_log_prefix() {
    let (_root, mut store, source) = store();
    let take = materialization(&source, "op_000001", "voiceover-placement-undo", 120);
    let first = commit(&mut store, &take).unwrap();
    store.undo(Actor::system()).unwrap();

    let rejected = commit(&mut store, &take).unwrap_err();
    assert_eq!(rejected.code, error_codes::CONFLICT);
    assert!(rejected.message.contains("not currently active"));

    let dir = store.dir.clone();
    drop(store);
    let mut reopened = ProjectStore::open(&dir).unwrap();
    assert!(reopened.project.assets.is_empty());
    assert!(reopened.project.track("a1t").unwrap().clips.is_empty());
    assert_eq!(
        commit(&mut reopened, &take).unwrap_err().code,
        error_codes::CONFLICT
    );

    reopened.redo(Actor::system()).unwrap();
    assert!(reopened.project.assets.contains_key(&first.asset_id));
    assert!(reopened
        .project
        .track("a1t")
        .unwrap()
        .clips
        .iter()
        .any(|clip| clip.id() == Some(first.clip_id.as_str())));
    let retried = commit(&mut reopened, &take).unwrap();
    assert!(retried.already_applied);
    assert_eq!(retried.asset_id, first.asset_id);
    assert_eq!(retried.clip_id, first.clip_id);

    let replayed = ProjectStore::open(&dir).unwrap();
    assert!(replayed.project.assets.contains_key(&first.asset_id));
    assert!(replayed
        .project
        .track("a1t")
        .unwrap()
        .clips
        .iter()
        .any(|clip| clip.id() == Some(first.clip_id.as_str())));
}

#[test]
fn stale_or_semantically_reused_voiceover_request_fails_closed() {
    let (_root, mut store, source) = store();
    store
        .apply(
            "edit.add_marker",
            json!({"at_ms": 1, "label": "advance revision"}),
            Actor::system(),
            None,
        )
        .unwrap();
    let stale = materialization(&source, "op_000001", "voiceover-placement-2", 20);
    assert_eq!(
        commit(&mut store, &stale).unwrap_err().code,
        error_codes::CONFLICT
    );
    assert!(store.project.assets.is_empty());

    let current = store.log.current_revision().unwrap().unwrap();
    let accepted = materialization(&source, &current, "voiceover-placement-2", 20);
    let first = commit(&mut store, &accepted).unwrap();
    let changed = materialization(&source, &first.op_id, "voiceover-placement-2", 21);
    assert_eq!(
        commit(&mut store, &changed).unwrap_err().code,
        error_codes::CONFLICT
    );
    assert_eq!(store.project.assets.len(), 1);
    assert_eq!(audio_media_count(&store), 1);
}

#[test]
fn cache_lag_after_durable_append_reopens_and_retries_without_duplicates() {
    let (_root, mut store, source) = store();
    let take = materialization(&source, "op_000001", "voiceover-placement-3", 0);
    let stale_cache = std::fs::read(store.dir.join("project.json")).unwrap();
    let first = commit(&mut store, &take).unwrap();
    // Simulate process loss after ops.jsonl is durable but before its disposable
    // project.json cache is refreshed. Recovery must replay the atomic record.
    std::fs::write(store.dir.join("project.json"), stale_cache).unwrap();
    let reopened = ProjectStore::open(&store.dir).unwrap();
    let mut reopened = reopened;
    assert_eq!(reopened.open_health().cache, ProjectCacheHealth::Rebuilt);
    let retry = commit(&mut reopened, &take).unwrap();
    assert_eq!(retry.asset_id, first.asset_id);
    assert_eq!(retry.clip_id, first.clip_id);
    assert!(retry.already_applied);
    assert_eq!(reopened.project.assets.len(), 1);
    assert_eq!(reopened.project.track("a1t").unwrap().clips.len(), 1);
}

#[test]
fn corrupt_atomic_record_fails_closed_without_partially_replaying_asset_or_clip() {
    let (_root, mut store, source) = store();
    let take = materialization(&source, "op_000001", "voiceover-placement-corrupt", 0);
    let first = commit(&mut store, &take).unwrap();
    let dir = store.dir.clone();
    let mut records = store.log.read_all().unwrap();
    let corrupt = {
        let corrupt = records.get_mut(1).unwrap();
        corrupt.args["track"] = json!("missing-audio-track");
        corrupt
            .effects
            .iter_mut()
            .find(|effect| {
                effect
                    .detail
                    .get("atomic_media_insert")
                    .and_then(Value::as_bool)
                    == Some(true)
            })
            .unwrap()
            .detail["insert"]["track"] = json!("missing-audio-track");
        corrupt.clone()
    };

    let mut replay_probe = Project::new("replay-probe", ProjectSettings::default());
    assert_eq!(
        apply_record(&mut replay_probe, &corrupt, &records[..1])
            .unwrap_err()
            .code,
        error_codes::NOT_FOUND
    );
    assert!(!replay_probe.assets.contains_key(&first.asset_id));
    assert!(replay_probe
        .all_sequence_tracks()
        .flat_map(|track| track.clips.iter())
        .all(|clip| clip.id() != Some(first.clip_id.as_str())));

    let journal = records
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .join("\n");
    std::fs::write(dir.join("ops.jsonl"), format!("{journal}\n")).unwrap();
    drop(store);
    assert_eq!(
        ProjectStore::open(&dir).unwrap_err().code,
        error_codes::NOT_FOUND
    );
}

#[test]
fn retry_after_clip_and_asset_removal_refuses_without_reusing_or_duplicating_ids() {
    let (_root, mut store, source) = store();
    let take = materialization(&source, "op_000001", "voiceover-placement-removed", 0);
    let first = commit(&mut store, &take).unwrap();
    store
        .apply(
            "edit.ripple_delete",
            json!({"track": "a1t", "range_ms": [0, 160], "ripple": false}),
            Actor::system(),
            None,
        )
        .unwrap();
    assert!(store
        .project
        .all_sequence_tracks()
        .flat_map(|track| track.clips.iter())
        .all(|clip| clip.id() != Some(first.clip_id.as_str())));
    store
        .record_remove_asset(&first.asset_id, Actor::system(), None)
        .unwrap();
    let before_retry = store.log.read_all().unwrap();

    let error = commit(&mut store, &take).unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.message.contains("not currently active"));
    assert!(!store.project.assets.contains_key(&first.asset_id));
    assert!(store
        .project
        .all_sequence_tracks()
        .flat_map(|track| track.clips.iter())
        .all(|clip| clip.id() != Some(first.clip_id.as_str())));
    assert_eq!(store.log.read_all().unwrap(), before_retry);
}

#[tokio::test]
async fn stale_preview_ack_is_rejected_after_ws_command_correlation() {
    let bridge = crate::ui_bridge::UiBridge::default();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let client = bridge.register(tx);
    let answer_bridge = bridge.clone();
    let answerer = tokio::spawn(async move {
        let outbound: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
        assert_eq!(outbound["verb"], PREVIEW_COMMAND);
        assert_eq!(outbound["args"]["start_ms"], 40);
        let bridge_request_id = outbound["request_id"].as_u64().unwrap();
        assert!(answer_bridge.resolve(
            client,
            json!({
                "type": "ui_command_result",
                "request_id": bridge_request_id,
                "verb": PREVIEW_COMMAND,
                "applied": true,
                "voiceover_playback": {
                    "request_id": "voiceover-placement-4",
                    "request_fingerprint": "f".repeat(64),
                    "bridge_epoch": 6,
                },
            }),
        ));
    });
    let request = VoiceoverPreviewPlaybackRequest {
        request_id: "voiceover-placement-4".into(),
        request_fingerprint: "f".repeat(64),
        bridge_epoch: 7,
        accepted_revision: "op_000001".into(),
        audio_track: "a1t".into(),
        start_ms: 40,
        out_ms: Some(80),
    };
    assert_eq!(
        request.await_ack(&bridge).await.unwrap_err().code,
        error_codes::CONFLICT
    );
    answerer.await.unwrap();
}
