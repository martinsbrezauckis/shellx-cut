use super::*;
use crate::mcp::{handle_sync_method, McpMode};

#[test]
fn initialize_advertises_resources_and_summary_instructions() {
    let state = AppState::new();
    let msg = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {}}
    });
    let reply = handle_sync_method(&state, McpMode::Proxy, &msg, false)
        .expect("initialize returns a JSON-RPC reply");
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    assert!(reply["result"]["capabilities"]["tools"].is_object());
    assert!(reply["result"]["capabilities"]["resources"].is_object());
    assert!(reply["result"]["instructions"]
        .as_str()
        .is_some_and(|text| text.contains("shellx-cut://verbs")));
}

#[test]
fn resources_list_and_templates_expose_full_registry_and_respect_containment() {
    let unrestricted = list_resources(false);
    let uris: Vec<_> = unrestricted["resources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["uri"].as_str())
        .collect();
    assert!(uris.contains(&VERBS_URI));
    assert!(uris.contains(&SKILL_DOC_URI));
    assert!(uris.contains(&REFERENCE_DOC_URI));

    let restricted = list_resources(true);
    let restricted_uris: Vec<_> = restricted["resources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["uri"].as_str())
        .collect();
    assert_eq!(restricted_uris, vec![VERBS_URI]);

    let templates = list_resource_templates();
    assert_eq!(
        templates["resourceTemplates"][0]["uriTemplate"],
        "shellx-cut://verb/{name}"
    );
}

#[test]
fn per_verb_resource_returns_untruncated_contract_for_dot_or_mcp_name() {
    let state = AppState::new();
    let by_dot = read_resource(&state, "shellx-cut://verb/screen_record.start", false)
        .expect("dot verb resource must resolve");
    let by_mcp = read_resource(&state, "shellx-cut://verb/screen_record_start", false)
        .expect("MCP tool name resource must resolve");

    let dot_text = by_dot["contents"][0]["text"].as_str().unwrap();
    let mcp_text = by_mcp["contents"][0]["text"].as_str().unwrap();
    let dot_val: Value = serde_json::from_str(dot_text).unwrap();
    let mcp_val: Value = serde_json::from_str(mcp_text).unwrap();
    assert_eq!(dot_val, mcp_val);
    assert_eq!(dot_val["name"], "screen_record.start");
    assert_eq!(dot_val["mcp_tool_name"], "screen_record_start");
    assert!(
        dot_val["description"].as_str().unwrap().len() > 1_000,
        "resource must retain the untruncated description"
    );
    assert!(
        dot_val["args"]["properties"]["request_id"]["description"]
            .as_str()
            .is_some_and(|text| text.contains("Caller-generated retry identity")),
        "resource must retain shared mutation control descriptions"
    );
}

#[test]
fn full_verbs_resource_and_docs_enforce_containment_and_reject_unknown_uris() {
    let state = AppState::new();
    let all_verbs = read_resource(&state, VERBS_URI, false).unwrap();
    let all_json: Value =
        serde_json::from_str(all_verbs["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        all_json["verbs"].as_array().unwrap().len(),
        state.registry.verbs.len()
    );

    let restricted_verbs = read_resource(&state, VERBS_URI, true).unwrap();
    let restricted_json: Value =
        serde_json::from_str(restricted_verbs["contents"][0]["text"].as_str().unwrap()).unwrap();
    let restricted_names: Vec<_> = restricted_json["verbs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|verb| verb["name"].as_str())
        .collect();
    assert!(restricted_names.contains(&"edit.add_marker"));
    assert!(!restricted_names.contains(&"project.open"));

    let denied_verb = read_resource(&state, "shellx-cut://verb/project.open", true).unwrap_err();
    assert_eq!(denied_verb.0, -32002);
    let denied_doc = read_resource(&state, SKILL_DOC_URI, true).unwrap_err();
    assert_eq!(denied_doc.0, -32002);

    let skill_doc = read_resource(&state, SKILL_DOC_URI, false).unwrap();
    assert!(skill_doc["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("ShellX Cut"));

    for bad_uri in [
        "shellx-cut://verb/",
        "shellx-cut://verb/does.not.exist",
        "shellx-cut://verb/../Cargo.toml",
        "file:///etc/passwd",
    ] {
        let err = read_resource(&state, bad_uri, false).unwrap_err();
        assert_eq!(err.0, -32002, "unexpected error for {bad_uri}: {err:?}");
    }

    let missing_uri = handle_sync_method(
        &state,
        McpMode::Proxy,
        &json!({"jsonrpc": "2.0", "id": 9, "method": "resources/read", "params": {}}),
        false,
    )
    .unwrap();
    assert_eq!(missing_uri["error"]["code"], -32602);
}
