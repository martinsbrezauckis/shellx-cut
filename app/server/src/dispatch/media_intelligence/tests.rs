use super::*;
use crate::dispatch::media_intelligence::model::{
    finalize_index, load_index, opaque_id, EvidenceEntry, MediaEvidenceIndex,
};
use cut_core::{Asset, Clip, Marker, Project, ProjectSettings};

fn fixture() -> (tempfile::TempDir, ProjectSnapshot) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("receipts")).unwrap();
    let source = directory.path().join("interview.mp4");
    std::fs::write(&source, b"fixture video").unwrap();
    std::fs::write(
        directory.path().join("receipts/a1.words.json"),
        serde_json::to_vec_pretty(&json!({
            "asset": "a1",
            "model": "fixture",
            "language": "en",
            "words": [
                {"idx":0,"word":"Launch","start_ms":900,"end_ms":1200,"speaker":"S1"},
                {"idx":1,"word":"the","start_ms":1250,"end_ms":1400,"speaker":"S1"},
                {"idx":2,"word":"product.","start_ms":1450,"end_ms":1900,"speaker":"S1"},
                {"idx":3,"word":"Thank","start_ms":2600,"end_ms":2900,"speaker":"S2"},
                {"idx":4,"word":"you.","start_ms":2950,"end_ms":3200,"speaker":"S2"}
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let mut project = Project::new("Evidence fixture", ProjectSettings::default());
    project.assets.insert(
        "a1".into(),
        Asset {
            path: source.display().to_string(),
            hash: "sha256:fixture-a1".into(),
            probe: Some(json!({
                "kind": "video",
                "has_video": true,
                "has_audio": true,
                "duration_ms": 5000
            })),
            transcript: Some("receipts/a1.words.json".into()),
            perception: None,
            proxy: None,
            filmstrip: None,
        },
    );
    let clip: Clip = serde_json::from_value(json!({
        "id": "c1",
        "asset": "a1",
        "src_in_ms": 0,
        "src_out_ms": 5000
    }))
    .expect("minimal media clip fixture");
    project.tracks[0].clips.push(clip);
    project.markers.push(Marker {
        id: "m1".into(),
        at_ms: 1000,
        label: "Launch line".into(),
        note: Some("Use in trailer".into()),
        color: Some("blue".into()),
    });
    let snapshot = ProjectSnapshot {
        dir: directory.path().to_path_buf(),
        revision: Some("op_001".into()),
        project,
    };
    (directory, snapshot)
}

fn kinds(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn build_fixture_index(snapshot: &ProjectSnapshot) -> MediaEvidenceIndex {
    let bindings = current_bindings(snapshot, None).unwrap();
    build_index(
        snapshot,
        bindings,
        kinds(&["transcript", "marker", "metadata"]),
        crate::jobs::JobCancellation::test_active(),
    )
    .unwrap()
}

#[test]
fn rebuild_is_deterministic_and_path_light() {
    let (_directory, snapshot) = fixture();
    let first = build_fixture_index(&snapshot);
    let second = build_fixture_index(&snapshot);
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap(),
        "same authorities produce one deterministic index",
    );
    let serialized = serde_json::to_string(&first).unwrap();
    assert!(!serialized.contains(snapshot.dir.to_string_lossy().as_ref()));
    assert!(!serialized.contains("receipts/a1.words.json"));
    assert!(first.entries.iter().any(|entry| entry.kind == "transcript"));
    assert!(first.entries.iter().any(|entry| entry.kind == "marker"));
}

#[test]
fn publication_round_trips_the_versioned_index() {
    let (_directory, snapshot) = fixture();
    let index = build_fixture_index(&snapshot);
    publish_index(&snapshot, &index).unwrap();
    let loaded = load_index(&snapshot.dir).expect("published index");
    assert_eq!(loaded.index_id, index.index_id);
    assert_eq!(loaded.entries.len(), index.entries.len());
}

#[test]
fn scoped_rebuild_preserves_untouched_evidence_kinds() {
    let (_directory, snapshot) = fixture();
    let current = current_bindings(&snapshot, None).unwrap();
    let existing = build_fixture_index(&snapshot);
    let replaced_kinds = kinds(&["metadata"]);
    let rebuilt = build_index(
        &snapshot,
        current.clone(),
        replaced_kinds.clone(),
        crate::jobs::JobCancellation::test_active(),
    )
    .unwrap();
    let selected = kinds(&["a1"]);
    let merged = merge_index(
        &snapshot,
        current,
        Some(existing),
        rebuilt,
        Some(&selected),
        &replaced_kinds,
    )
    .unwrap();
    assert!(merged
        .entries
        .iter()
        .any(|entry| entry.kind == "transcript"));
    assert!(merged.entries.iter().any(|entry| entry.kind == "marker"));
    assert!(merged.entries.iter().any(|entry| entry.kind == "metadata"));
}

#[tokio::test]
async fn search_returns_exact_citations_and_live_occurrences() {
    let (_directory, snapshot) = fixture();
    publish_index(&snapshot, &build_fixture_index(&snapshot)).unwrap();
    let result = search_value(
        snapshot,
        SearchArgs {
            query: "launch product".into(),
            asset_ids: None,
            kinds: Some(vec![
                "transcript".into(),
                "marker".into(),
                "metadata".into(),
            ]),
            scope: None,
            limit: Some(20),
            cursor: None,
        },
    )
    .await
    .unwrap();
    let hit = &result["hits"][0];
    assert_eq!(hit["kind"], "transcript");
    assert_eq!(hit["asset_id"], "a1");
    assert_eq!(hit["source_start_ms"], 900);
    assert_eq!(hit["occurrences"][0]["clip_id"], "c1");
    assert_eq!(hit["occurrences"][0]["timeline_start_ms"], 900);
    assert!(hit.get("path").is_none());
}

#[tokio::test]
async fn changed_authority_is_excluded_until_rebuild() {
    let (directory, snapshot) = fixture();
    publish_index(&snapshot, &build_fixture_index(&snapshot)).unwrap();
    std::fs::write(
        directory.path().join("receipts/a1.words.json"),
        br#"{"asset":"a1","model":"fixture","words":[]}"#,
    )
    .unwrap();
    let result = search_value(
        snapshot.clone(),
        SearchArgs {
            query: "launch".into(),
            asset_ids: None,
            kinds: Some(vec!["transcript".into()]),
            scope: None,
            limit: None,
            cursor: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(result["count"], 0);
    assert!(result["stale_excluded"].as_u64().unwrap() > 0);
    assert_eq!(status_value(&snapshot, None).unwrap()["stale"], true);
}

#[tokio::test]
async fn sparse_large_index_pages_stably_without_duplicate_or_unbounded_rows() {
    let (_directory, snapshot) = fixture();
    let mut index = build_fixture_index(&snapshot);
    let provenance = index.bindings[0].transcript_sha256.clone().unwrap();
    let mut expected = Vec::new();
    for ordinal in 0..25_000u64 {
        let is_match = ordinal % 811 == 0;
        let excerpt = if is_match {
            format!("needle cited moment {ordinal}")
        } else {
            format!("ordinary evidence row {ordinal}")
        };
        let evidence_id = opaque_id(&["sparse", &ordinal.to_string(), &excerpt]);
        if is_match {
            expected.push(evidence_id.clone());
        }
        index.entries.push(EvidenceEntry {
            evidence_id,
            asset_id: "a1".into(),
            kind: "transcript".into(),
            source_start_ms: ordinal.saturating_mul(10),
            source_end_ms: ordinal.saturating_mul(10).saturating_add(5),
            anchor_ms: ordinal.saturating_mul(10),
            excerpt,
            speaker: None,
            provenance_sha256: provenance.clone(),
            sequence_id: None,
            timeline_anchor_ms: None,
            marker_id: None,
        });
    }
    let duplicate = index
        .entries
        .iter()
        .find(|entry| entry.excerpt.starts_with("needle"))
        .unwrap()
        .clone();
    index.entries.push(duplicate);
    let index = finalize_index(index).unwrap();
    publish_index(&snapshot, &index).unwrap();

    let mut cursor = None;
    let mut observed = Vec::new();
    let mut first_cursor = None;
    loop {
        let page = search_value(
            snapshot.clone(),
            SearchArgs {
                query: "needle".into(),
                asset_ids: None,
                kinds: Some(vec!["transcript".into()]),
                scope: None,
                limit: Some(7),
                cursor,
            },
        )
        .await
        .unwrap();
        assert!(page["count"].as_u64().unwrap() <= 7);
        observed.extend(
            page["hits"]
                .as_array()
                .unwrap()
                .iter()
                .map(|hit| hit["evidence_id"].as_str().unwrap().to_string()),
        );
        cursor = page["next_cursor"].as_str().map(str::to_string);
        first_cursor.get_or_insert_with(|| cursor.clone().unwrap());
        if cursor.is_none() {
            break;
        }
    }
    expected.sort();
    observed.sort();
    assert_eq!(
        observed, expected,
        "cursor pages are complete and deduplicated"
    );

    let seed = first_cursor
        .unwrap()
        .rsplit_once(':')
        .unwrap()
        .0
        .to_string();
    let error = search_value(
        snapshot,
        SearchArgs {
            query: "needle".into(),
            asset_ids: None,
            kinds: Some(vec!["transcript".into()]),
            scope: None,
            limit: Some(7),
            cursor: Some(format!("{seed}:999999")),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
}

#[tokio::test]
async fn bounded_background_derivation_keeps_foreground_evidence_reads_responsive() {
    use tokio::sync::Notify;
    use tokio::time::{timeout, Duration};

    let (_directory, snapshot) = fixture();
    publish_index(&snapshot, &build_fixture_index(&snapshot)).unwrap();
    let jobs = crate::jobs::JobManager::new(crate::events::EventBus::new());
    let job = jobs.create("media_intelligence");
    let started = std::sync::Arc::new(Notify::new());
    let finished = std::sync::Arc::new(Notify::new());
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let started_worker = started.clone();
    let finished_worker = finished.clone();
    let jobs_worker = jobs.clone();
    let job_id = job.job_id.clone();
    jobs.spawn_limited(&job.job_id, "analysis", ANALYSIS_MAX_RUNNING, async move {
        crate::dispatch::run_blocking_cancellable("media intelligence load fixture", move |_| {
            started_worker.notify_one();
            release_rx.recv().map_err(|error| {
                CutError::new(
                    error_codes::JOB_FAILED,
                    "load fixture stopped",
                    error.to_string(),
                )
            })
        })
        .await
        .unwrap();
        jobs_worker.finish(&job_id, json!({"ok": true}));
        finished_worker.notify_one();
    });
    timeout(Duration::from_secs(1), started.notified())
        .await
        .expect("bounded background worker should start");

    let foreground = timeout(
        Duration::from_millis(250),
        search_value(
            snapshot,
            SearchArgs {
                query: "launch".into(),
                asset_ids: None,
                kinds: Some(vec!["transcript".into()]),
                scope: None,
                limit: Some(10),
                cursor: None,
            },
        ),
    )
    .await
    .expect("foreground evidence read must not wait for the analysis slot")
    .unwrap();
    assert_eq!(foreground["count"], 1);
    assert_eq!(
        jobs.get(&job.job_id).unwrap().state,
        crate::jobs::JobState::Running
    );

    release_tx.send(()).unwrap();
    timeout(Duration::from_secs(1), finished.notified())
        .await
        .expect("background fixture should finish cleanly");
}

#[test]
fn inspect_media_is_bounded_current_and_path_light() {
    let (_directory, snapshot) = fixture();
    publish_index(&snapshot, &build_fixture_index(&snapshot)).unwrap();
    let result = inspect_media_value(&snapshot, Some(vec!["a1".into()])).unwrap();
    assert_eq!(result["schema"], "shellx-cut/media-inspection/1");
    assert_eq!(result["count"], 1);
    assert_eq!(result["assets"][0]["asset_id"], "a1");
    assert_eq!(result["assets"][0]["media_kind"], "video");
    assert_eq!(
        result["assets"][0]["evidence"]["transcript"]["state"],
        "ready"
    );
    let serialized = result.to_string();
    assert!(!serialized.contains("interview.mp4"));
    assert!(!serialized.contains(snapshot.dir.to_string_lossy().as_ref()));
}

#[test]
fn inspect_range_resolves_the_same_current_evidence_identity() {
    let (_directory, snapshot) = fixture();
    let index = build_fixture_index(&snapshot);
    let evidence_id = index
        .entries
        .iter()
        .find(|entry| entry.kind == "transcript")
        .unwrap()
        .evidence_id
        .clone();
    publish_index(&snapshot, &index).unwrap();
    let result = inspect_range_value(
        &snapshot,
        Some(&index.index_id),
        std::slice::from_ref(&evidence_id),
    )
    .unwrap();
    assert_eq!(result["schema"], "shellx-cut/evidence-inspection/1");
    assert_eq!(result["hits"][0]["evidence_id"], evidence_id);
    assert_eq!(result["hits"][0]["match"], "inspect");
    assert_eq!(result["hits"][0]["occurrences"][0]["clip_id"], "c1");

    let error = inspect_range_value(
        &snapshot,
        Some("idx_000000000000000000000000"),
        &[evidence_id],
    )
    .unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
}

#[test]
fn inspect_range_refuses_evidence_after_its_authority_changes() {
    let (directory, snapshot) = fixture();
    let index = build_fixture_index(&snapshot);
    let evidence_id = index
        .entries
        .iter()
        .find(|entry| entry.kind == "transcript")
        .unwrap()
        .evidence_id
        .clone();
    publish_index(&snapshot, &index).unwrap();
    std::fs::write(
        directory.path().join("receipts/a1.words.json"),
        br#"{"asset":"a1","model":"changed","words":[]}"#,
    )
    .unwrap();
    let error = inspect_range_value(&snapshot, Some(&index.index_id), &[evidence_id]).unwrap_err();
    assert_eq!(error.code, error_codes::CONFLICT);
    assert!(error.message.contains("stale"));
}

#[test]
fn cancellation_stops_before_publication() {
    let (_directory, snapshot) = fixture();
    let cancellation = crate::jobs::JobCancellation::test_active();
    cancellation.request_cancel();
    let error = build_index(
        &snapshot,
        current_bindings(&snapshot, None).unwrap(),
        kinds(&["transcript"]),
        cancellation,
    )
    .unwrap_err();
    assert_eq!(error.code, error_codes::JOB_FAILED);
    assert!(load_index(&snapshot.dir).is_none());
}
