//! Assemble b-roll import-terminal-state regression.

use super::transition_test_support::{
    broll_actor, create_project, transition_gate_test_lock, TEST_TIMEOUT,
};
use super::*;

#[tokio::test]
async fn assemble_broll_refuses_insert_when_the_import_job_fails() {
    let _test_lock = transition_gate_test_lock().lock().await;
    // Stills fail before admission; video keeps its asynchronous import job.
    for (suffix, kind, failed_step, admitted) in [
        ("png", "image", "assets.fetch", false),
        ("mp4", "video", "assets.fetch.import", true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let project_path = root.path().join("failed-import.cutproj");
        let search_dir = root.path().join("search");
        std::fs::create_dir_all(&search_dir).unwrap();
        std::fs::write(
            search_dir.join(format!("broken-broll.{suffix}")),
            b"not media",
        )
        .unwrap();

        let state = AppState::new();
        create_project(&state, "failed-import", &project_path).await;
        let assembled = tokio::time::timeout(
            TEST_TIMEOUT,
            dispatch(
                &state,
                "assemble.broll",
                json!({
                    "slots": [{"query": "broken-broll", "at_ms": 100, "duration_ms": 500}],
                    "provider": "local_folder",
                    "kind": kind,
                    "dir": search_dir,
                }),
                broll_actor(),
            ),
        )
        .await
        .expect("failed image import did not settle before test timeout");
        assert!(
            assembled.ok,
            "the orchestrator must return its truthful partial-state result: {:?}",
            assembled.error
        );
        let result = assembled.result.unwrap();
        assert_eq!(result["status"], "failed");
        assert_eq!(result["failed_step"], failed_step);
        assert_eq!(result["placed"], json!([]));
        assert!(
            result["checkpoint"]
                .as_str()
                .is_some_and(|id| !id.is_empty()),
            "a failed assembly must preserve the checkpoint needed to undo setup work"
        );

        let ops = dispatch(&state, "project.ops", json!({}), test_actor()).await;
        assert!(ops.ok, "project.ops failed: {:?}", ops.error);
        let ops_result = ops.result.unwrap();
        let ops = ops_result["ops"].as_array().unwrap();
        assert_eq!(
            ops.iter().any(|op| op["verb"] == "media.import"),
            admitted,
            "damaged pictures must never be registered; video retains its asynchronous admission"
        );
        assert!(
            !ops.iter().any(|op| op["verb"] == "edit.insert"),
            "a failed import must never be inserted into the timeline"
        );
        let project = dispatch(&state, "project.state", json!({}), test_actor()).await;
        assert!(project.ok, "project.state failed: {:?}", project.error);
        let project_state = project.result.unwrap();
        let broll_track = project_state["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["id"] == "broll")
            .expect(
                "partial assembly keeps its created b-roll track for checkpoint/revert semantics",
            );
        assert!(
            broll_track["clips"].as_array().unwrap().is_empty(),
            "the failed import must leave no timeline clip behind"
        );
    }
}
