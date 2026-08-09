use super::*;
use tokio::sync::mpsc;

#[tokio::test]
async fn confirmed_ui_command_returns_only_the_correlated_applied_result() {
    let state = AppState::new();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let client_id = state.ui_bridge.register(tx);
    let worker_state = state.clone();
    let worker = tokio::spawn(async move {
        dispatch(
            &worker_state,
            "ui.open",
            json!({"panel":"timeline"}),
            test_actor(),
        )
        .await
    });

    let outbound: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
    assert_eq!(outbound["type"], "ui_command");
    assert_eq!(outbound["verb"], "ui.open");
    let request_id = outbound["request_id"].as_u64().unwrap();
    assert!(state.ui_bridge.resolve(
        client_id,
        json!({
            "type":"ui_command_result",
            "request_id":request_id,
            "verb":"ui.open",
            "applied":true,
            "requested":{"panel":"timeline"},
            "surface":"timeline",
            "selector":"[data-cut-panel=\"timeline\"]",
            "state":{"schema":"shellx-cut/ui-state/2","state_revision":7}
        }),
    ));

    let result = worker.await.unwrap();
    assert!(result.ok, "{result:?}");
    let payload = result.result.unwrap();
    assert_eq!(payload["applied"], true);
    assert_eq!(payload["verb"], "ui.open");
    assert_eq!(payload["request_id"], request_id);
    assert_eq!(payload["requested"], json!({"panel":"timeline"}));
    assert_eq!(payload["state"]["state_revision"], 7);
}

#[tokio::test]
async fn rejected_or_noop_ui_command_is_not_a_success_envelope() {
    let state = AppState::new();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let client_id = state.ui_bridge.register(tx);
    let worker_state = state.clone();
    let worker = tokio::spawn(async move {
        dispatch(
            &worker_state,
            "ui.playhead",
            json!({"at_ms":100}),
            test_actor(),
        )
        .await
    });

    let outbound: Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
    let request_id = outbound["request_id"].as_u64().unwrap();
    assert!(state.ui_bridge.resolve(
        client_id,
        json!({
            "type":"ui_command_result",
            "request_id":request_id,
            "verb":"ui.playhead",
            "applied":false,
            "requested":{"at_ms":100},
            "state":{"schema":"shellx-cut/ui-state/2","state_revision":9},
            "error":{"code":"conflict","message":"playhead is already at 100 ms"}
        }),
    ));

    let result = worker.await.unwrap();
    assert!(!result.ok, "{result:?}");
    assert_eq!(result.error.as_ref().unwrap().code, error_codes::CONFLICT);
    let payload = result.result.unwrap();
    assert_eq!(payload["applied"], false);
    assert_eq!(payload["request_id"], request_id);
    assert_eq!(payload["error"]["code"], "conflict");
}
