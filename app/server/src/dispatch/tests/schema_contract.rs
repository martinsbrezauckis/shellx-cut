use super::*;

fn assert_schema_error(result: VerbResult, verb: &str, path: &str, keyword: &str) {
    let error = result.error.expect("schema-invalid args must fail");
    assert_eq!(error.code, error_codes::INVALID_ARGS);
    assert!(error.message.contains(verb), "{error:?}");
    assert!(error.message.contains(path), "{error:?}");
    assert!(error.message.contains(keyword), "{error:?}");
    assert!(error.cause.contains(keyword), "{error:?}");
    assert!(
        error
            .suggested_action
            .as_deref()
            .is_some_and(|action| action.contains("GET /api/verbs")),
        "{error:?}"
    );
}

#[tokio::test]
async fn ui_commands_obey_the_executable_input_schema_before_ui_delivery() {
    let state = AppState::new();

    let no_ui = dispatch(&state, "ui.state", json!({}), test_actor()).await;
    let no_ui_error = no_ui.error.expect("headless ui.state must fail honestly");
    assert_eq!(no_ui_error.code, error_codes::NO_UI_CLIENT);
    let action = no_ui_error
        .suggested_action
        .expect("headless UI recovery must remain actionable");
    assert!(action.contains("system.doctor"));
    assert!(!action.contains("127.0.0.1:6161"));

    assert_schema_error(
        dispatch(&state, "ui.open", json!({}), test_actor()).await,
        "ui.open",
        "/panel",
        "required",
    );
    assert_schema_error(
        dispatch(
            &state,
            "ui.open",
            json!({"panel":"not-a-real-surface"}),
            test_actor(),
        )
        .await,
        "ui.open",
        "/panel",
        "enum",
    );
    assert_schema_error(
        dispatch(&state, "ui.playhead", json!({}), test_actor()).await,
        "ui.playhead",
        "/at_ms",
        "required",
    );
    assert_schema_error(
        dispatch(&state, "ui.playhead", json!({"at_ms":"100"}), test_actor()).await,
        "ui.playhead",
        "/at_ms",
        "type",
    );
    assert_schema_error(
        dispatch(&state, "ui.playhead", json!({"at_ms":-1}), test_actor()).await,
        "ui.playhead",
        "/at_ms",
        "minimum",
    );

    for at_ms in [0, 1, u32::MAX as u64] {
        let result = dispatch(&state, "ui.playhead", json!({"at_ms":at_ms}), test_actor()).await;
        assert_eq!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some(error_codes::NO_UI_CLIENT),
            "valid playhead {at_ms} must pass schema validation and reach the handler: {result:?}"
        );
    }
}

#[tokio::test]
async fn nested_required_and_unknown_paths_are_exact() {
    let state = AppState::new();
    assert_schema_error(
        dispatch(
            &state,
            "project.create",
            json!({
                "name":"demo",
                "settings":{
                    "width":1280,
                    "height":720,
                    "fps":30,
                    "bogus":true
                }
            }),
            test_actor(),
        )
        .await,
        "project.create",
        "/settings/bogus",
        "additionalProperties",
    );

    assert_schema_error(
        dispatch(
            &state,
            "edit.speed_ramp",
            json!({
                "clip":"c1",
                "points":[{"at_ms":0}]
            }),
            test_actor(),
        )
        .await,
        "edit.speed_ramp",
        "/points/0/factor",
        "required",
    );
}

#[tokio::test]
async fn common_inverse_result_modifier_is_declared_and_type_checked() {
    let state = AppState::new();
    let spec = state
        .registry
        .get("edit.add_marker")
        .expect("edit.add_marker schema");
    assert_eq!(
        spec.args["properties"]["include_inverse"]["type"],
        "boolean"
    );
    assert_schema_error(
        dispatch(
            &state,
            "edit.add_marker",
            json!({"at_ms":0,"label":"x","include_inverse":"true"}),
            test_actor(),
        )
        .await,
        "edit.add_marker",
        "/include_inverse",
        "type",
    );
}
