use super::test_actor;
use crate::dispatch::dispatch;
use crate::state::AppState;
use cut_core::error_codes;
use serde_json::json;

fn make_test_media(path: &std::path::Path) {
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-nostats",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x240:rate=30:duration=1",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .status()
        .expect("ffmpeg present (cut-media dependency)");
    assert!(status.success(), "lavfi asset generation failed");
}

#[tokio::test]
async fn render_compare_refuses_without_an_earlier_durable_state() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"compare","dir":dir.path().join("compare.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:#?}", created.error);

    let before = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap();
    let revision = before["ops"][0]["op_id"].as_str().unwrap();
    let stale = dispatch(
        &state,
        "render.compare",
        json!({"at_ms":0,"revision":"op_999999"}),
        test_actor(),
    )
    .await;
    assert_eq!(stale.error.unwrap().code, error_codes::CONFLICT);
    let compared = dispatch(
        &state,
        "render.compare",
        json!({"at_ms":0,"revision":revision}),
        test_actor(),
    )
    .await;
    assert!(!compared.ok, "a one-revision project cannot compare itself");
    assert_eq!(compared.error.unwrap().code, error_codes::NOT_FOUND);
    let after = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(
        after["ops"], before["ops"],
        "comparison must never append or undo an op"
    );
}

#[tokio::test]
async fn render_compare_uses_the_prefix_before_the_latest_timeline_edit_even_with_trailing_metadata(
) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let created = dispatch(
        &state,
        "project.create",
        json!({"name":"compare","dir":dir.path().join("compare.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(created.ok, "{:#?}", created.error);
    let media = dir.path().join("source.mp4");
    make_test_media(&media);
    for (verb, args) in [
        ("media.import", json!({"path":media})),
        (
            "edit.insert",
            json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,1000]}),
        ),
        ("edit.grade", json!({"clip":"c1","brightness":0.25})),
    ] {
        let result = dispatch(&state, verb, args, test_actor()).await;
        assert!(result.ok, "{verb}: {:#?}", result.error);
    }
    let renamed = dispatch(
        &state,
        "project.rename",
        json!({"name":"compare metadata"}),
        test_actor(),
    )
    .await;
    assert!(renamed.ok, "project.rename: {:#?}", renamed.error);
    let ops_before = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap()["ops"]
        .as_array()
        .unwrap()
        .to_vec();
    let current_revision = ops_before.last().unwrap()["op_id"]
        .as_str()
        .unwrap()
        .to_string();
    let prior_revision = ops_before[ops_before.len() - 3]["op_id"]
        .as_str()
        .unwrap()
        .to_string();

    let compared = dispatch(
        &state,
        "render.compare",
        json!({"at_ms":0,"revision":current_revision,"h":90}),
        test_actor(),
    )
    .await;
    assert!(compared.ok, "{:#?}", compared.error);
    let pair = compared.result.unwrap();
    assert_eq!(pair["schema"], "shellx-cut/preview-comparison/1");
    assert_eq!(pair["at_ms"], 0);
    assert_eq!(pair["current_revision"], current_revision);
    assert_eq!(pair["prior_revision"], prior_revision);
    assert_eq!(pair["compared_operation"]["verb"], "edit.grade");
    assert_eq!(
        pair["compared_operation"]["id"],
        ops_before[ops_before.len() - 2]["op_id"]
    );
    assert_eq!(pair["before"]["revision"], prior_revision);
    assert_eq!(pair["current"]["revision"], current_revision);
    assert_eq!(pair["before"]["mime"], "image/jpeg");
    assert_eq!(pair["current"]["mime"], "image/jpeg");
    assert!(pair["before"]["base64"].as_str().unwrap().len() > 100);
    assert!(pair["current"]["base64"].as_str().unwrap().len() > 100);
    assert_ne!(
        pair["before"]["base64"], pair["current"]["base64"],
        "the grade must be visible in the current composed frame"
    );

    let ops_after = dispatch(&state, "project.ops", json!({}), test_actor())
        .await
        .result
        .unwrap();
    assert_eq!(
        ops_after["ops"],
        json!(ops_before),
        "comparison is strictly read-only"
    );
}
