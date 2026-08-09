use super::test_actor;
use crate::dispatch::dispatch;
use crate::state::AppState;
use serde_json::json;

#[tokio::test]
async fn searches_all_sequences_without_mutating_history() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_path = dir.path().join("sequence-index.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"sequence-index","dir":project_path.clone()}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let media = dir.path().join("Interview Hero.mp4");
    std::fs::write(&media, b"stub-video").unwrap();
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path":media,"proxy":false}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    let asset = imported.result.unwrap()["asset_id"]
        .as_str()
        .unwrap()
        .to_string();
    let inserted = dispatch(
        &state,
        "edit.insert",
        json!({"asset":asset,"track":"v1","at_ms":0,"src_range_ms":[0,2000]}),
        test_actor(),
    )
    .await;
    assert!(inserted.ok, "{:?}", inserted.error);
    let main_marker = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms":900,"label":"Review hook","note":"client approved"}),
        test_actor(),
    )
    .await;
    assert!(main_marker.ok, "{:?}", main_marker.error);

    let social = dispatch(
        &state,
        "project.sequence_create",
        json!({"name":"Social Cut","from":"active"}),
        test_actor(),
    )
    .await;
    assert!(social.ok, "{:?}", social.error);
    let social_id = social.result.unwrap()["active_sequence"]
        .as_str()
        .unwrap()
        .to_string();
    let social_marker = dispatch(
        &state,
        "edit.add_marker",
        json!({"at_ms":1500,"label":"TikTok ending"}),
        test_actor(),
    )
    .await;
    assert!(social_marker.ok, "{:?}", social_marker.error);
    let social_marker_id = social_marker.result.unwrap()["marker_id"]
        .as_str()
        .unwrap()
        .to_string();
    let colored = dispatch(
        &state,
        "edit.update_marker",
        json!({"id":social_marker_id,"color":"purple"}),
        test_actor(),
    )
    .await;
    assert!(colored.ok, "{:?}", colored.error);

    let before = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .len();
    let all = dispatch(
        &state,
        "project.sequence_index",
        json!({"query":"interview hero","kind":"clip"}),
        test_actor(),
    )
    .await;
    assert!(all.ok, "{:?}", all.error);
    let all = all.result.unwrap();
    assert_eq!(all["total"], 2, "duplicated clip appears in both sequences");
    assert_eq!(all["clip_count"], 2);
    assert_eq!(all["marker_count"], 0);
    assert_eq!(all["results"][0]["sequence_id"], "seq1");
    assert_eq!(all["results"][1]["sequence_id"], social_id);
    assert_eq!(all["results"][0]["label"], "Interview Hero.mp4");
    assert!(all["results"][0].get("path").is_none());

    let markers = dispatch(
        &state,
        "project.sequence_index",
        json!({"query":"tiktok purple","kind":"marker","sequence":social_id}),
        test_actor(),
    )
    .await;
    assert!(markers.ok, "{:?}", markers.error);
    let markers = markers.result.unwrap();
    assert_eq!(markers["total"], 1);
    assert_eq!(markers["results"][0]["label"], "TikTok ending");
    assert_eq!(markers["results"][0]["at_ms"], 1500);

    let notes = dispatch(
        &state,
        "project.sequence_index",
        json!({"query":"client approved","kind":"marker"}),
        test_actor(),
    )
    .await;
    assert!(notes.ok, "{:?}", notes.error);
    assert_eq!(
        notes.result.unwrap()["total"],
        2,
        "duplicated marker note is searchable"
    );

    let after = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(after, before, "Sequence Index is a pure read");

    let missing = dispatch(
        &state,
        "project.sequence_index",
        json!({"sequence":"missing"}),
        test_actor(),
    )
    .await;
    assert!(!missing.ok);
    assert_eq!(missing.error.unwrap().code, "not_found");

    let closed = dispatch(&state, "project.close", json!({}), test_actor()).await;
    assert!(closed.ok, "{:?}", closed.error);
    let reopened = dispatch(
        &state,
        "project.open",
        json!({"path":project_path}),
        test_actor(),
    )
    .await;
    assert!(reopened.ok, "{:?}", reopened.error);
    let persisted = dispatch(
        &state,
        "project.sequence_index",
        json!({"query":"tiktok ending","kind":"marker"}),
        test_actor(),
    )
    .await;
    assert!(persisted.ok, "{:?}", persisted.error);
    assert_eq!(persisted.result.unwrap()["total"], 1);
}

#[tokio::test]
async fn filters_live_sequence_status_without_polluting_the_default_index() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let project_path = dir.path().join("sequence-status.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"sequence-status","dir":project_path}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:?}", created.error);

    let media = dir.path().join("Status Hero.mp4");
    std::fs::write(&media, b"stub-video").unwrap();
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path":media,"proxy":false}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "{:?}", imported.error);
    let asset = imported.result.unwrap()["asset_id"]
        .as_str()
        .unwrap()
        .to_string();

    let video = dispatch(
        &state,
        "edit.insert",
        json!({"asset":asset,"track":"v1","at_ms":1000,"src_range_ms":[0,2000]}),
        test_actor(),
    )
    .await;
    assert!(video.ok, "{:?}", video.error);
    let video_clip = video.result.unwrap()["clip_id"]
        .as_str()
        .unwrap()
        .to_string();
    let audio = dispatch(
        &state,
        "edit.insert",
        json!({"asset":asset,"track":"a1t","at_ms":0,"src_range_ms":[0,2000]}),
        test_actor(),
    )
    .await;
    assert!(audio.ok, "{:?}", audio.error);
    let effected = dispatch(
        &state,
        "edit.effect",
        json!({"clip":video_clip,"effects":[{"type":"vignette","amount":0.4}]}),
        test_actor(),
    )
    .await;
    assert!(effected.ok, "{:?}", effected.error);

    for (verb, args) in [
        ("edit.track_lock", json!({"track":"v1","on":true})),
        ("edit.track_visible", json!({"track":"v1","on":false})),
        ("edit.mute", json!({"track":"a1t","on":true})),
    ] {
        let changed = dispatch(&state, verb, args, test_actor()).await;
        assert!(changed.ok, "{verb}: {:?}", changed.error);
    }
    std::fs::remove_file(&media).unwrap();

    let all = dispatch(
        &state,
        "project.sequence_index",
        json!({"kind":"clip"}),
        test_actor(),
    )
    .await;
    assert!(all.ok, "{:?}", all.error);
    let all = all.result.unwrap();
    assert_eq!(all["total"], 2, "default index omits anonymous gaps");
    assert_eq!(all["issue_count"], 2, "both media rows are now offline");
    assert_eq!(all["effect_clip_count"], 1);
    assert!(all["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["clip_kind"] != "gap"));

    for (status, total) in [
        ("gaps", 2),
        ("issues", 4),
        ("offline", 2),
        ("effects", 1),
        ("hidden", 1),
        ("locked", 1),
        ("muted", 1),
    ] {
        let filtered = dispatch(
            &state,
            "project.sequence_index",
            json!({"status":status}),
            test_actor(),
        )
        .await;
        assert!(filtered.ok, "{status}: {:?}", filtered.error);
        assert_eq!(filtered.result.unwrap()["total"], total, "status={status}");
    }

    let gap = dispatch(
        &state,
        "project.sequence_index",
        json!({"status":"gaps"}),
        test_actor(),
    )
    .await
    .result
    .unwrap();
    assert!(gap["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["clip_kind"] == "gap" && row["issues"] == json!(["gap"])));

    let searchable = dispatch(
        &state,
        "project.sequence_index",
        json!({"query":"vignette offline"}),
        test_actor(),
    )
    .await;
    assert!(searchable.ok, "{:?}", searchable.error);
    let searchable = searchable.result.unwrap();
    assert_eq!(searchable["total"], 1);
    assert_eq!(searchable["results"][0]["effects"], json!(["vignette"]));
    assert_eq!(searchable["results"][0]["offline"], true);

    let invalid = dispatch(
        &state,
        "project.sequence_index",
        json!({"status":"broken"}),
        test_actor(),
    )
    .await;
    assert!(!invalid.ok);
    assert_eq!(invalid.error.unwrap().code, "invalid_args");
}
