//! Item 155: an analysis report that came with the project (planted, or made
//! on another computer) is never used as data by an edit; the verb answers as
//! if the asset were not analysed and names the way forward (media.perception).
//! The gate is `cut_perception::load_report`; these tests drive three of its
//! callers through their verbs.

use super::*;

fn plant(dir: &std::path::Path, body: serde_json::Value) {
    let receipts = dir.join("t.cutproj/receipts");
    std::fs::create_dir_all(&receipts).unwrap();
    // A plain write: NOT recorded as this engine's report.
    std::fs::write(
        receipts.join("a1.perception.json"),
        serde_json::to_string(&body).unwrap(),
    )
    .unwrap();
}

fn not_analysed(r: &VerbResult, verb: &str) {
    let e = r
        .error
        .as_ref()
        .unwrap_or_else(|| panic!("{verb} used a planted report: {:?}", r.result));
    assert_eq!(e.code, "not_found", "{verb}: {e:?}");
    assert!(
        e.suggested_action
            .as_deref()
            .unwrap_or("")
            .contains("media.perception"),
        "{verb}: {e:?}"
    );
}

#[tokio::test]
async fn remove_silences_ignores_a_planted_report() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    plant(
        dir.path(),
        json!({
            "schema": "shellx-cut/perception/1", "asset_hash": "sha256:test",
            "source_path": "clip.mp4", "instruments_run": ["silence"],
            "silences": [{"start_ms": 1000, "end_ms": 2000, "source": "both"}],
        }),
    );
    let r = dispatch(
        &state,
        "transcript.remove_silences",
        json!({"aggressiveness":"natural"}),
        test_actor(),
    )
    .await;
    not_analysed(&r, "transcript.remove_silences");
}

#[tokio::test]
async fn duck_ignores_a_planted_report() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    let r = dispatch(
        &state,
        "edit.add_track",
        json!({"kind":"audio"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    plant(
        dir.path(),
        json!({
            "schema": "shellx-cut/perception/1", "asset_hash": "sha256:test",
            "source_path": "clip.mp4", "instruments_run": ["silence"],
            "silences": [{"start_ms": 5500, "end_ms": 6500, "source": "both"}],
        }),
    );
    let r = dispatch(
        &state,
        "edit.duck",
        json!({"music_track":"a2t","against_track":"a1t","db":-18.0}),
        test_actor(),
    )
    .await;
    not_analysed(&r, "edit.duck");
}

#[tokio::test]
async fn split_at_scenes_ignores_a_planted_report() {
    let dir = tempfile::tempdir().unwrap();
    let state = narrowing_fixture(dir.path()).await;
    plant(
        dir.path(),
        json!({
            "schema":"shellx-cut/perception/1","asset_hash":"","source_path":"x",
            "instruments_run":["scenes"],"silences":[],
            "scenes":[{"at_ms":2000},{"at_ms":5000},{"at_ms":8000}],
            "black_spans":[],"frozen_spans":[],"content_bbox":null
        }),
    );
    let r = dispatch(
        &state,
        "edit.split_at_scenes",
        json!({"asset":"a1"}),
        test_actor(),
    )
    .await;
    not_analysed(&r, "edit.split_at_scenes");
}
