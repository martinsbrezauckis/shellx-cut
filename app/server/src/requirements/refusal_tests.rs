use super::refusal::{complete, refusal, way_forward};
use cut_core::{error_codes, CutError};
use serde_json::json;

#[test]
fn a_tagged_refusal_gets_the_registry_name_and_way_forward() {
    let error = complete(
        CutError::new(error_codes::SIDECAR, "x", "y").with_requirement("captions_runtime"),
    );
    let tag = error.requirement.as_ref().expect("tagged");
    assert_eq!(tag.name.as_deref(), Some("Captions and transcription"));
    assert_eq!(tag.get.as_ref().unwrap()["verb"], "system.setup_perception");
    let action = error.suggested_action.expect("way forward filled");
    assert!(
        action.starts_with("call system.setup_perception"),
        "{action}"
    );
    assert!(
        action.contains("Install captions in Settings > AI & transcription"),
        "{action}"
    );
}

#[test]
fn a_refusal_that_says_what_to_do_keeps_its_own_words() {
    let error = complete(
        CutError::new(error_codes::NO_PROJECT, "no project is open", "c")
            .with_suggested_action("open the project you were editing")
            .with_requirement("open_project"),
    );
    assert_eq!(
        error.suggested_action.as_deref(),
        Some("open the project you were editing")
    );
    assert_eq!(
        error.requirement.unwrap().name.as_deref(),
        Some("An open project")
    );
}

#[test]
fn an_untagged_or_unknown_refusal_is_left_alone() {
    let plain = complete(CutError::new(error_codes::INVALID_ARGS, "m", "c"));
    assert!(plain.requirement.is_none() && plain.suggested_action.is_none());
    let unknown =
        complete(CutError::new(error_codes::INVALID_ARGS, "m", "c").with_requirement("no_such"));
    assert!(unknown.suggested_action.is_none());
    assert_eq!(unknown.requirement.unwrap().name, None);
}

#[test]
fn the_builder_names_what_only_a_person_can_do() {
    let error = refusal(
        error_codes::NOT_FOUND,
        "screen_permission",
        "screen recording is not allowed",
        "c",
    );
    let action = error.suggested_action.unwrap();
    assert!(action.contains("a person must allow it"), "{action}");
    assert_eq!(error.requirement.unwrap().id, "screen_permission");
    let ffmpeg =
        way_forward(&json!({"verb": "system.fetch_tool", "args": {"tool": "ffmpeg"}})).unwrap();
    assert_eq!(ffmpeg, "call system.fetch_tool {\"tool\":\"ffmpeg\"}");
}

#[tokio::test]
async fn refusals_leaving_dispatch_name_the_missing_requirement() {
    let state = crate::state::AppState::new();
    let actor = cut_core::Actor::system();
    let no_project = crate::dispatch::dispatch(
        &state,
        "transcript.get",
        json!({"asset": "a1"}),
        actor.clone(),
    )
    .await;
    let error = no_project.error.expect("refused without a project");
    let tag = error.requirement.expect("names the requirement");
    assert_eq!(tag.id, "open_project");
    assert_eq!(tag.get.unwrap()["verb"], "project.open");

    let no_ui = crate::dispatch::dispatch(&state, "ui.state", json!({}), actor.clone()).await;
    assert_eq!(
        no_ui
            .error
            .expect("no editor window")
            .requirement
            .unwrap()
            .id,
        "ui_client"
    );

    let _missing = crate::media_tool_gate::test_presence::missing();
    let gated =
        crate::dispatch::dispatch(&state, "media.probe", json!({"asset": "a1"}), actor).await;
    let error = gated.error.expect("refused without ffmpeg");
    assert_eq!(error.code, error_codes::TOOL_MISSING);
    let tag = error.requirement.expect("names ffmpeg");
    assert_eq!(tag.id, "ffmpeg");
    assert_eq!(tag.name.as_deref(), Some("Video processing (FFmpeg)"));
    assert_eq!(tag.get.unwrap()["verb"], "system.fetch_tool");
}
