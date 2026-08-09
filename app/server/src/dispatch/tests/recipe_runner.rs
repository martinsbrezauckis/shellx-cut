use super::super::{dispatch, run_resolved_recipe};
use super::{lock_agent_cli_env, test_actor, wait_job};
use crate::recipes;
use crate::state::AppState;
use cut_core::{error_codes, VerbResult};
use serde_json::json;

// ----------------------------------------------------------------------
// recipe.* runner tests. The built-in registry holds shipped recipes
// whose heavy stages need media/perception/render sidecars, so these
// drive a constructed test recipe over edit.add_marker — a verb that
// accepts a rationale (threading) AND adds a project marker (the
// marker_count state fact), so the whole runner is exercised with NO
// sidecar / media dependency.
// ----------------------------------------------------------------------

/// A 2-stage test recipe over edit.add_marker with marker_count state gates.
fn marker_test_recipe(name: &str) -> recipes::Recipe {
    serde_json::from_value(json!({
        "name": name,
        "title": "Test markers",
        "description": "two add_marker stages with state gates",
        "params": { "label": { "type": "string", "default": "m" } },
        "stages": [
            { "id": "m1", "verb": "edit.add_marker", "args": {"at_ms": 0, "label": "{{label}}"},
              "rationale": "first marker",
              "gate": { "state": [ {"fact":"marker_count","op":"gte","value":1} ] } },
            { "id": "m2", "verb": "edit.add_marker", "args": {"at_ms": 1000, "label": "second"},
              "rationale": "second marker",
              "gate": { "state": [ {"fact":"marker_count","op":"gte","value":2} ] } }
        ]
    }))
    .expect("test recipe parses")
}

/// The runner drives every stage through dispatch(), records a single
/// auto-checkpoint, threads the recipe rationale onto each sub-op, reports
/// op_ids + passing gates, and reverts the WHOLE run as one unit.
#[tokio::test]
async fn recipe_run_drives_stages_checkpoints_and_reverts() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"rec","dir": dir.path().join("rec.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    let recipe = marker_test_recipe("test-markers");
    let r: VerbResult = run_resolved_recipe(&state, recipe, json!({}), None, None, test_actor())
        .await
        .into();
    assert!(r.ok, "recipe.run dispatch: {:?}", r.error);
    let res = r.result.unwrap();
    let job_id = res["job_id"].as_str().unwrap().to_string();
    let checkpoint = res["checkpoint"].as_str().unwrap().to_string();
    assert!(!checkpoint.is_empty(), "auto-checkpoint recorded");
    assert_eq!(res["recipe"], "test-markers");

    let rec = wait_job(&state, &job_id, 30).await;
    assert_eq!(rec.state, crate::jobs::JobState::Done, "{:?}", rec.error);
    let report = rec.result.unwrap();
    assert_eq!(report["status"], "completed", "report: {report}");
    assert_eq!(report["stages_run"], 2);
    let sr = report["stage_results"].as_array().unwrap();
    assert_eq!(sr.len(), 2);
    for s in sr {
        assert!(s["ok"].as_bool().unwrap(), "stage ok: {s}");
        assert!(
            !s["op_ids"].as_array().unwrap().is_empty(),
            "stage appended an op: {s}"
        );
        assert!(s["gate"]["pass"].as_bool().unwrap(), "stage gate pass: {s}");
    }

    // A checkpoint op was appended; each marker op carries the threaded
    // recipe rationale; the markers actually landed.
    {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        assert_eq!(store.project.markers.len(), 2, "two markers landed");
        let ops = store.log.read_all().unwrap();
        assert!(
            ops.iter().any(|o| o.verb == "project.checkpoint"),
            "auto-checkpoint op present"
        );
        let marker_ops: Vec<_> = ops.iter().filter(|o| o.verb == "edit.add_marker").collect();
        assert_eq!(marker_ops.len(), 2);
        for o in marker_ops {
            let why = o.rationale.as_deref().unwrap_or("");
            assert!(
                why.starts_with("recipe test-markers · stage"),
                "threaded recipe rationale: {why}"
            );
        }
    }

    // One-step revert undoes the WHOLE run.
    let r = dispatch(
        &state,
        "project.revert",
        json!({"to": checkpoint}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "revert: {:?}", r.error);
    let guard = state.project.read().await;
    assert_eq!(
        guard.as_ref().unwrap().project.markers.len(),
        0,
        "revert cleared the whole run"
    );
}

/// A failing state gate stops the run honestly: status=gate_failed, the next
/// stage's verb never runs, the report names the failing predicate + measured
/// value, and the checkpoint reverts the partial run.
#[tokio::test]
async fn recipe_run_stops_and_reports_on_gate_fail() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"recf","dir": dir.path().join("recf.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);

    // Stage 1 adds ONE marker but its gate demands >= 5 -> gate_failed.
    let recipe: recipes::Recipe = serde_json::from_value(json!({
        "name": "fail-gate", "title": "fail", "description": "gate fails",
        "params": {},
        "stages": [
            { "id": "m1", "verb": "edit.add_marker", "args": {"at_ms": 0, "label": "x"},
              "rationale": "only marker",
              "gate": { "state": [ {"fact":"marker_count","op":"gte","value":5} ] } },
            { "id": "m2", "verb": "edit.add_marker", "args": {"at_ms": 1000, "label": "never"} }
        ]
    }))
    .unwrap();
    let r: VerbResult = run_resolved_recipe(&state, recipe, json!({}), None, None, test_actor())
        .await
        .into();
    assert!(r.ok, "{:?}", r.error);
    let job_id = r.result.unwrap()["job_id"].as_str().unwrap().to_string();
    let rec = wait_job(&state, &job_id, 30).await;
    assert_eq!(rec.state, crate::jobs::JobState::Done);
    let report = rec.result.unwrap();
    assert_eq!(report["status"], "gate_failed", "report: {report}");
    assert_eq!(report["stages_run"], 1, "stopped after stage 1");
    let sr = report["stage_results"].as_array().unwrap();
    assert_eq!(sr.len(), 1, "stage 2 never ran");
    let gate = &sr[0]["gate"];
    assert!(!gate["pass"].as_bool().unwrap());
    let pred = &gate["state"][0];
    assert_eq!(pred["fact"], "marker_count");
    assert_eq!(pred["measured"], 1, "measured value reported");
    assert!(!pred["pass"].as_bool().unwrap());

    let checkpoint = report["checkpoint"].as_str().unwrap().to_string();
    {
        let guard = state.project.read().await;
        let store = guard.as_ref().unwrap();
        assert_eq!(store.project.markers.len(), 1, "only stage 1's marker");
        let ops = store.log.read_all().unwrap();
        assert_eq!(
            ops.iter().filter(|o| o.verb == "edit.add_marker").count(),
            1,
            "stage 2 verb never dispatched"
        );
    }
    let r = dispatch(
        &state,
        "project.revert",
        json!({"to": checkpoint}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "revert: {:?}", r.error);
    let guard = state.project.read().await;
    assert_eq!(guard.as_ref().unwrap().project.markers.len(), 0);
}

/// policy:dry_run resolves + interpolates the plan and returns it WITHOUT a
/// checkpoint or any dispatch — the op-log head is unchanged.
#[tokio::test]
async fn recipe_dry_run_plans_without_mutating() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new();
    let r = dispatch(
        &state,
        "project.create",
        json!({"name":"recd","dir": dir.path().join("recd.cutproj")}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let head_before = {
        let g = state.project.read().await;
        g.as_ref()
            .unwrap()
            .log
            .read_all()
            .unwrap()
            .last()
            .map(|o| o.op_id.clone())
    };
    let recipe = marker_test_recipe("test-markers");
    let r: VerbResult = run_resolved_recipe(
        &state,
        recipe,
        json!({"label":"hello"}),
        Some("dry_run".into()),
        None,
        test_actor(),
    )
    .await
    .into();
    assert!(r.ok, "{:?}", r.error);
    let res = r.result.unwrap();
    assert_eq!(res["policy"], "dry_run");
    assert_eq!(res["status"], "planned");
    assert!(res["job_id"].is_null(), "dry_run returns no job");
    // params resolved + interpolated into the plan.
    assert_eq!(res["params"]["label"], "hello");
    assert_eq!(res["stages"][0]["args"]["label"], "hello");
    assert_eq!(res["stages"].as_array().unwrap().len(), 2);
    // No checkpoint, no ops appended.
    let head_after = {
        let g = state.project.read().await;
        g.as_ref()
            .unwrap()
            .log
            .read_all()
            .unwrap()
            .last()
            .map(|o| o.op_id.clone())
    };
    assert_eq!(head_before, head_after, "dry_run must not append any op");
    let guard = state.project.read().await;
    assert_eq!(
        guard.as_ref().unwrap().project.markers.len(),
        0,
        "dry_run must not mutate"
    );
}

/// recipe.list / recipe.describe are pure reads: they succeed with NO project
/// open and append no op; an unknown describe name is NOT_FOUND.
#[tokio::test]
async fn recipe_list_describe_are_pure_reads() {
    let state = AppState::new();
    let r = dispatch(&state, "recipe.list", json!({}), test_actor()).await;
    assert!(r.ok, "recipe.list with no project: {:?}", r.error);
    let recs = r.result.unwrap();
    assert_eq!(
        recs["recipes"].as_array().unwrap().len(),
        11,
        "the shipped built-ins"
    );
    assert!(r.op_ids.is_none(), "pure read appends no op");

    let r = dispatch(
        &state,
        "recipe.describe",
        json!({"name":"talking-head-cleanup"}),
        test_actor(),
    )
    .await;
    assert!(r.ok, "{:?}", r.error);
    let d = r.result.unwrap();
    assert_eq!(d["name"], "talking-head-cleanup");
    assert!(!d["stages"].as_array().unwrap().is_empty());

    let r = dispatch(
        &state,
        "recipe.describe",
        json!({"name":"nope"}),
        test_actor(),
    )
    .await;
    assert!(!r.ok);
    assert_eq!(r.error.unwrap().code, error_codes::NOT_FOUND);
}

#[tokio::test]
async fn bundled_first_edit_completes_with_an_honest_degraded_render_receipt() {
    let _env_guard = lock_agent_cli_env();
    let old_sidecar = std::env::var_os("SHELLX_CUT_SIDECAR_DIR");
    let old_python = std::env::var_os("SHELLX_CUT_PYTHON");
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("SHELLX_CUT_SIDECAR_DIR", dir.path().join("missing-sidecar"));
    std::env::set_var("SHELLX_CUT_PYTHON", dir.path().join("missing-python"));

    let state = AppState::new();
    let project_dir = dir.path().join("first-edit.cutproj");
    let created = dispatch(
        &state,
        "project.create",
        json!({
            "name": "first-edit",
            "dir": project_dir,
            "settings": {"width": 640, "height": 360, "fps": 24},
            "starter": "first-edit"
        }),
        test_actor(),
    )
    .await;
    assert!(created.ok, "starter create: {:?}", created.error);
    let starter_path = created.result.unwrap()["starter_asset_path"]
        .as_str()
        .unwrap()
        .to_string();
    let imported = dispatch(
        &state,
        "media.import",
        json!({"path": starter_path, "proxy": false}),
        test_actor(),
    )
    .await;
    assert!(imported.ok, "starter import: {:?}", imported.error);
    let import_job = imported.result.unwrap()["job_id"]
        .as_str()
        .unwrap()
        .to_string();
    let imported = wait_job(&state, &import_job, 30).await;
    assert_eq!(imported.state, crate::jobs::JobState::Done, "{imported:?}");

    let run = dispatch(
        &state,
        "recipe.run",
        json!({"name": "first-project", "args": {"asset": "a1"}}),
        test_actor(),
    )
    .await;
    assert!(run.ok, "recipe start: {:?}", run.error);
    let recipe_job = run.result.unwrap()["job_id"].as_str().unwrap().to_string();
    let finished = wait_job(&state, &recipe_job, 120).await;

    match old_sidecar {
        Some(value) => std::env::set_var("SHELLX_CUT_SIDECAR_DIR", value),
        None => std::env::remove_var("SHELLX_CUT_SIDECAR_DIR"),
    }
    match old_python {
        Some(value) => std::env::set_var("SHELLX_CUT_PYTHON", value),
        None => std::env::remove_var("SHELLX_CUT_PYTHON"),
    }

    assert_eq!(finished.state, crate::jobs::JobState::Done, "{finished:?}");
    let report = finished.result.unwrap();
    assert_eq!(report["status"], "completed_with_warnings", "{report}");
    let render_stage = report["stage_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stage| stage["id"] == "render")
        .unwrap();
    assert_eq!(render_stage["gate"]["pass"], true, "{render_stage}");
    assert_eq!(render_stage["job_result"]["verified"], false);
    let render_job_id = render_stage["job_id"].as_str().unwrap();
    assert_eq!(
        state.jobs.get(render_job_id).unwrap().completion,
        Some(crate::jobs::JobCompletion::DoneWithWarnings)
    );
    let receipt_path = render_stage["job_result"]["receipt"].as_str().unwrap();
    let receipt: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(receipt_path).unwrap()).unwrap();
    let check = |name: &str| {
        receipt["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["name"] == name)
            .unwrap()["pass"]
            .as_bool()
            .unwrap()
    };
    assert!(check("cut_on_word"));
    assert!(check("caption_presence"));
    assert!(check("duration_matches_edl"));
    assert!(
        !check("lufs"),
        "unmeasured output loudness must not fake a pass"
    );
}
