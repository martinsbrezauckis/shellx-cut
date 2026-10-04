//! Delivery admissions bind the observed origin before output/job side effects.

use super::*;
use std::sync::Arc;

async fn create(state: &AppState, name: &str, root: &std::path::Path) {
    let result = dispatch(
        state,
        "project.create",
        json!({"name":name,"dir":root.join(format!("{name}.cutproj"))}),
        test_actor(),
    )
    .await;
    assert!(result.ok, "{:?}", result.error);
}

async fn digest(state: &AppState) -> String {
    let result = dispatch(state, "project.state", json!({}), test_actor()).await;
    result.result.unwrap()["project_identity"]["origin_path_sha256"]
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn foreign_delivery_origin_refuses_before_input_output_or_job_admission() {
    let root = tempfile::tempdir().unwrap();
    let state = AppState::new();
    create(&state, "a", root.path()).await;
    let a_digest = digest(&state).await;
    create(&state, "b", root.path()).await;
    let b_digest = digest(&state).await;
    assert_ne!(a_digest, b_digest);
    for (verb, args) in [
        (
            "render.queue",
            json!({"jobs":[{"output":"a.mp4","path":"b.mp4"}]}),
        ),
        (
            "screen_record.export",
            json!({"source":"/not-present.mp4","plan":"/not-present.json"}),
        ),
        (
            "screen_record.copy_raw",
            json!({"source":"/not-present.mp4"}),
        ),
    ] {
        let before = state.jobs.list().len();
        let mut foreign = args.clone();
        foreign["expected_origin_path_sha256"] = json!(a_digest);
        let result = dispatch(&state, verb, foreign, test_actor()).await;
        assert_eq!(result.error.unwrap().code, error_codes::CONFLICT, "{verb}");
        assert_eq!(
            state.jobs.list().len(),
            before,
            "{verb} must not make a job"
        );
        assert!(
            !root.path().join("b.cutproj/exports").exists(),
            "{verb} must not reserve output"
        );

        let mut current = args.clone();
        current["expected_origin_path_sha256"] = json!(b_digest);
        let result = dispatch(&state, verb, current, test_actor()).await;
        assert_ne!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some(error_codes::CONFLICT),
            "{verb} current origin proceeds to ordinary input checks"
        );
        let result = dispatch(&state, verb, args.clone(), test_actor()).await;
        assert_ne!(
            result.error.as_ref().map(|error| error.code.as_str()),
            Some(error_codes::CONFLICT),
            "{verb} legacy path stays accepted"
        );
    }
    let renamed = dispatch(
        &state,
        "project.rename",
        json!({"name":"b renamed"}),
        test_actor(),
    )
    .await;
    assert!(renamed.ok, "{:?}", renamed.error);
    assert_eq!(digest(&state).await, b_digest);
    let renamed = dispatch(
        &state,
        "screen_record.copy_raw",
        json!({"source":"/not-present.mp4","expected_origin_path_sha256":b_digest}),
        test_actor(),
    )
    .await;
    assert_ne!(renamed.error.unwrap().code, error_codes::CONFLICT);
    for (verb, args) in [
        ("render.queue", json!({"jobs":[{}]})),
        ("screen_record.export", json!({"source":"x","plan":"y"})),
        ("screen_record.copy_raw", json!({"source":"x"})),
    ] {
        for malformed in ["", "sha256:bad", "/private/project.cutproj"] {
            let mut request = args.clone();
            request["expected_origin_path_sha256"] = json!(malformed);
            let refused = dispatch(&state, verb, request, test_actor()).await;
            assert_eq!(
                refused.error.unwrap().code,
                error_codes::INVALID_ARGS,
                "{verb}: {malformed}"
            );
        }
    }
}

#[tokio::test]
async fn real_queue_preflight_holds_transition_until_it_finishes() {
    use crate::dispatch::rendering::{
        install_render_queue_transition_gate, RenderQueueTransitionGate,
    };
    struct ResetGate;
    impl Drop for ResetGate {
        fn drop(&mut self) {
            install_render_queue_transition_gate(None);
        }
    }
    let _reset = ResetGate;
    let root = tempfile::tempdir().unwrap();
    let state = Arc::new(AppState::new());
    create(&state, "a", root.path()).await;
    let a_digest = digest(&state).await;
    let media = root.path().join("fixture.mp4");
    std::fs::write(&media, b"x").unwrap();
    let imported = dispatch(&state, "media.import", json!({"path":media}), test_actor()).await;
    assert!(imported.ok, "{:?}", imported.error);
    update_asset(&state, "a1", |asset| {
        asset.probe = Some(
            json!({"kind":"video","width":160,"height":90,"duration_ms":5000,"has_audio":false}),
        );
    })
    .await
    .unwrap();
    let inserted = dispatch(
        &state,
        "edit.insert",
        json!({"asset":"a1","track":"v1","at_ms":0,"src_range_ms":[0,5000],"ripple":false}),
        test_actor(),
    )
    .await;
    assert!(inserted.ok, "{:?}", inserted.error);

    let gate = RenderQueueTransitionGate::new("b04c-preflight-gate");
    install_render_queue_transition_gate(Some(gate.clone()));
    let queue_state = Arc::clone(&state);
    let queue = tokio::spawn(async move {
        dispatch(
            &queue_state,
            "render.queue",
            json!({
                "jobs":[{"hardware":"off"},{"output":"a.mp4","path":"b.mp4"}],
                "rationale":"b04c-preflight-gate",
                "expected_origin_path_sha256":a_digest,
            }),
            test_actor(),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(10), gate.reached.notified())
        .await
        .unwrap();
    assert!(
        state.project_transition.try_lock().is_err(),
        "real dry-run keeps project transition pinned"
    );
    let switch_state = Arc::clone(&state);
    let root_path = root.path().to_path_buf();
    let switch = tokio::spawn(async move {
        create(&switch_state, "b", &root_path).await;
    });
    tokio::task::yield_now().await;
    assert!(!switch.is_finished(), "switch waits during queue preflight");
    gate.release.notify_one();
    let result = queue.await.unwrap();
    assert_eq!(
        result.error.unwrap().code,
        error_codes::INVALID_ARGS,
        "second entry refuses before job spawn"
    );
    switch.await.unwrap();
    let current = dispatch(&state, "project.state", json!({}), test_actor()).await;
    assert!(
        current.ok,
        "project.state after switch: {:?}",
        current.error
    );
    assert_eq!(current.result.unwrap()["name"], "b");
    assert!(state
        .jobs
        .list()
        .iter()
        .all(|job| job.kind != "render_queue"));
}
