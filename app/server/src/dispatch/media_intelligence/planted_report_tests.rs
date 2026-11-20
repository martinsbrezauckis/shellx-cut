//! Item 155: search reads an analysis report only when this engine wrote it.

use super::tests::{fixture, kinds};
use super::*;
use crate::dispatch::media_intelligence::model::current_bindings_for_visual_runtime;
use crate::vissearch::VisualCacheRuntime;

/// Item 155: a report that came with the project does not feed search; the
/// index says the asset's scenes are not ready instead.
#[test]
fn planted_report_does_not_feed_search() {
    let (directory, mut snapshot) = fixture();
    std::fs::write(
        directory.path().join("receipts/a1.perception.json"),
        br#"{"asset":"a1","scenes":[]}"#,
    )
    .unwrap();
    snapshot.project.assets.get_mut("a1").unwrap().perception =
        Some("receipts/a1.perception.json".into());
    let bindings =
        current_bindings_for_visual_runtime(&snapshot, None, &VisualCacheRuntime::Unavailable)
            .unwrap();
    assert!(bindings.iter().all(|b| b.perception_sha256.is_none()));
    let evidence_index = build_index(
        &snapshot,
        bindings,
        kinds(&["scene"]),
        crate::jobs::JobCancellation::test_active(),
    )
    .unwrap();
    assert!(
        evidence_index.coverage["a1"].is_empty(),
        "{:?}",
        evidence_index.coverage
    );
}

/// Row 201b sibling: a bound transcript that cannot be read when the index is
/// built is NOT listed as ready (it was, while no entry came from it).
#[test]
fn an_unreadable_bound_transcript_is_not_reported_ready() {
    let (directory, snapshot) = fixture();
    let bindings =
        current_bindings_for_visual_runtime(&snapshot, None, &VisualCacheRuntime::Unavailable)
            .unwrap();
    assert!(
        bindings[0].transcript_sha256.is_some(),
        "bound before the read"
    );
    std::fs::write(
        directory.path().join("receipts/a1.words.json"),
        b"{not json",
    )
    .unwrap();
    let evidence_index = build_index(
        &snapshot,
        bindings,
        kinds(&["transcript"]),
        crate::jobs::JobCancellation::test_active(),
    )
    .unwrap();
    assert!(
        evidence_index.coverage["a1"].is_empty(),
        "{:?}",
        evidence_index.coverage
    );
}

/// One asset's analysis report never stands for another asset: a project that
/// points asset a1 at asset a2's report (one this engine made) does not get
/// a2's scenes indexed as a1's, while a2's own report still feeds search.
#[test]
fn another_assets_report_is_never_indexed_for_this_asset() {
    let (directory, mut snapshot) = fixture();
    let mut second = snapshot.project.assets["a1"].clone();
    second.hash = "sha256:fixture-a2".into();
    second.transcript = None;
    snapshot.project.assets.insert("a2".into(), second);
    let report = directory.path().join("receipts/a2.perception.json");
    std::fs::write(
        &report,
        br#"{"schema":"shellx-cut/perception/1","asset_hash":"sha256:fixture-a2","source_path":"interview.mp4","instruments_run":["scenes"],"silences":[],"scenes":[{"at_ms":1500},{"at_ms":3500}],"black_spans":[],"frozen_spans":[],"content_bbox":null}"#,
    )
    .unwrap();
    // Stands in for this engine's own analysis write of a2.
    cut_media::cache_trust::remember(&report);
    for id in ["a1", "a2"] {
        snapshot.project.assets.get_mut(id).unwrap().perception =
            Some("receipts/a2.perception.json".into());
    }

    let bindings =
        current_bindings_for_visual_runtime(&snapshot, None, &VisualCacheRuntime::Unavailable)
            .unwrap();
    let evidence_index = build_index(
        &snapshot,
        bindings,
        kinds(&["scene"]),
        crate::jobs::JobCancellation::test_active(),
    )
    .unwrap();

    assert!(
        evidence_index
            .entries
            .iter()
            .all(|entry| entry.asset_id != "a1"),
        "a2's scenes were indexed under a1"
    );
    assert!(evidence_index.coverage["a1"].is_empty());
    assert!(
        evidence_index
            .entries
            .iter()
            .any(|entry| entry.asset_id == "a2" && entry.kind == "scene"),
        "a2's own report still feeds search"
    );
    assert_eq!(evidence_index.coverage["a2"], ["scene"]);
}

/// The report must describe this asset's source: an engine-made report whose
/// `asset_hash` is another source (for example made before the asset was
/// relinked) is not used.
#[test]
fn a_report_for_another_source_is_not_indexed() {
    let (directory, mut snapshot) = fixture();
    let report = directory.path().join("receipts/a1.perception.json");
    std::fs::write(
        &report,
        br#"{"schema":"shellx-cut/perception/1","asset_hash":"sha256:some-other-source","source_path":"interview.mp4","instruments_run":["scenes"],"silences":[],"scenes":[{"at_ms":1500}],"black_spans":[],"frozen_spans":[],"content_bbox":null}"#,
    )
    .unwrap();
    cut_media::cache_trust::remember(&report);
    snapshot.project.assets.get_mut("a1").unwrap().perception =
        Some("receipts/a1.perception.json".into());
    let bindings =
        current_bindings_for_visual_runtime(&snapshot, None, &VisualCacheRuntime::Unavailable)
            .unwrap();
    assert!(bindings[0].perception_sha256.is_none());
}
