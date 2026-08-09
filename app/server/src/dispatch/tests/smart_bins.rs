use super::test_actor;
use crate::dispatch::{dispatch, update_asset};
use crate::state::AppState;
use serde_json::{json, Value};

async fn create_project(state: &AppState, dir: &std::path::Path) {
    let r = dispatch(
        state,
        "project.create",
        json!({"name":"smart-bins","dir": dir.join("smart-bins.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
}

async fn import_stub(state: &AppState, path: &std::path::Path, probe: Value) -> String {
    let r = dispatch(
        state,
        "media.import",
        json!({"path": path, "proxy": false, "rationale": "test import"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let asset = r.result.unwrap()["asset_id"].as_str().unwrap().to_string();
    update_asset(state, &asset, |a| a.probe = Some(probe))
        .await
        .unwrap();
    asset
}

async fn save_bin(state: &AppState, args: Value) {
    let r = dispatch(state, "media.bin_save", args, test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
}

async fn bin_matches(state: &AppState, name: &str) -> Vec<String> {
    let r = dispatch(state, "media.bin_list", json!({}), test_actor()).await;
    assert!(r.ok, "{:?}", r.error);
    let bins = r.result.unwrap()["bins"].as_array().unwrap().clone();
    let bin = bins
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("missing bin {name}: {bins:?}"));
    bin["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn smart_bins_filter_resolution_offline_and_modified_time() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    create_project(&state, dir.path()).await;

    let large_path = dir.path().join("large-4k.mp4");
    let small_path = dir.path().join("small-hd.mp4");
    let missing_path = dir.path().join("missing-camera.mp4");
    std::fs::write(&large_path, b"large").unwrap();
    std::fs::write(&small_path, b"small").unwrap();
    std::fs::write(&missing_path, b"missing").unwrap();

    let large = import_stub(
        &state,
        &large_path,
        json!({"kind":"video","width":3840,"height":2160,"duration_ms":1000,"has_audio":false}),
    )
    .await;
    let small = import_stub(
        &state,
        &small_path,
        json!({"kind":"video","width":1920,"height":1080,"duration_ms":1000,"has_audio":false}),
    )
    .await;
    let missing = import_stub(
        &state,
        &missing_path,
        json!({"kind":"video","width":3840,"height":2160,"duration_ms":1000,"has_audio":false}),
    )
    .await;
    std::fs::remove_file(&missing_path).unwrap();

    save_bin(
        &state,
        json!({
            "name": "large online recent",
            "min_width": 3840,
            "min_height": 2160,
            "offline": false,
            "modified_after_ms": 0
        }),
    )
    .await;
    save_bin(&state, json!({"name": "missing only", "offline": true})).await;
    save_bin(&state, json!({"name": "old none", "modified_before_ms": 1})).await;

    assert_eq!(
        bin_matches(&state, "large online recent").await,
        vec![large]
    );
    assert_eq!(bin_matches(&state, "missing only").await, vec![missing]);
    assert_eq!(bin_matches(&state, "old none").await, Vec::<String>::new());
    assert!(
        !bin_matches(&state, "large online recent")
            .await
            .contains(&small),
        "HD assets must not match a 4K smart bin"
    );
}
