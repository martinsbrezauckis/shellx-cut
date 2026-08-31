//! Optional atomic-insert topology decoding and replay regression coverage.
use super::*;
use serde_json::Map;

pub(super) fn track_topology(
    detail: &Map<String, Value>,
    op: &OpRecord,
) -> Result<Option<AtomicMediaInsertTrack>, CutError> {
    detail
        .get("track")
        .filter(|track| !track.is_null())
        .map(|track| serde_json::from_value(track.clone()))
        .transpose()
        .map_err(|_| replay_corrupt(op, "atomic media insert track topology is malformed"))
}

#[cfg(test)]
mod tests {
    use super::super::atomic_media_insert::replay;
    use super::super::atomic_media_insert_request::AtomicMediaInsert;
    use super::*;
    use crate::MutationRequest;

    #[test]
    fn null_track_topology_replays_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ProjectStore::create(dir.path(), "atomic", None).unwrap();
        let request = AtomicMediaInsert {
            idempotency_key: "a".repeat(64),
            asset: Asset {
                path: "/sealed/voiceover.wav".into(),
                hash: format!("sha256:{}", "b".repeat(64)),
                probe: Some(json!({"duration_ms": 160})),
                transcript: None,
                perception: None,
                proxy: None,
                filmstrip: None,
            },
            insert: json!({
                "track": "a1t",
                "at_ms": 0,
                "src_range_ms": [0, 160],
                "ripple": false,
            }),
            binding: json!({"schema": "test"}),
            track: None,
        };
        let actor = Actor::system().with_request(MutationRequest {
            caller: "test/atomic".into(),
            request_id: "atomic-replay".into(),
            fingerprint: "test-fingerprint".into(),
            expected_revision: Some("op_000001".into()),
        });
        let committed = store
            .apply_atomic_media_insert(request, actor, None)
            .unwrap();

        let mut replayed = Project::new("atomic", ProjectSettings::default());
        replay(&mut replayed, &committed.op).unwrap();
        assert!(replayed.assets.contains_key(&committed.asset_id));
        assert!(replayed
            .track("a1t")
            .unwrap()
            .clips
            .iter()
            .any(|clip| clip.id() == Some(committed.clip_id.as_str())));
    }
}
