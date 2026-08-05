use super::*;

// recipe.* — declarative pipeline-manifest layer. The runner is a PURE
// orchestrator in the autopilot.run + audio.cleanup_voice mould: it records NO
// op of its own, wraps the run in ONE auto-checkpoint, dispatches each stage
// through the normal dispatch() path (real, replay-safe ops), polls sub-jobs,
// evaluates a declarative gate after each stage, and STOPS + reports on the
// first failed verb or gate (handing back the checkpoint). No new mutation path
// under the zero-local-mutation contract; no new replay arm (the sub-ops replay).
// ===========================================================================

/// `NOT_FOUND` for an unknown recipe name (actionable: names come from recipe.list).
fn recipe_not_found(name: &str) -> CutError {
    CutError::new(
        error_codes::NOT_FOUND,
        format!("no recipe '{name}'"),
        "recipe names come from recipe.list",
    )
}

/// A recipe's params as a compact list (`recipe.list`): name/type/required/default.
fn param_summaries(r: &recipes::Recipe) -> Vec<Value> {
    r.params
        .iter()
        .map(|(name, p)| {
            json!({
                "name": name,
                "type": p.ty,
                "required": p.required,
                "default": p.default,
            })
        })
        .collect()
}

/// A gate as JSON for recipe.describe / the dry-run plan (None → null).
fn gate_to_json(g: Option<&recipes::Gate>) -> Value {
    match g {
        None => Value::Null,
        Some(g) => json!({
            "checks": g.checks,
            "state": g.state.iter().map(|sp| json!({
                "fact": sp.fact, "op": sp.op, "value": sp.value,
            })).collect::<Vec<_>>(),
        }),
    }
}

/// The full resolved manifest (`recipe.describe`): params (with description +
/// enum) + the ordered stages (verb, templated args, rationale, gate).
fn describe_recipe(r: &recipes::Recipe) -> Value {
    let params: Vec<Value> = r
        .params
        .iter()
        .map(|(name, p)| {
            let mut o = Map::new();
            o.insert("name".into(), json!(name));
            o.insert("type".into(), json!(p.ty));
            o.insert("required".into(), json!(p.required));
            o.insert("default".into(), p.default.clone().unwrap_or(Value::Null));
            if let Some(d) = &p.description {
                o.insert("description".into(), json!(d));
            }
            if let Some(e) = &p.allowed {
                o.insert("enum".into(), json!(e));
            }
            Value::Object(o)
        })
        .collect();
    let stages: Vec<Value> = r
        .stages
        .iter()
        .map(|st| {
            json!({
                "id": st.id,
                "verb": st.verb,
                "args": st.args,
                "rationale": st.rationale,
                "await_job": st.await_job,
                "gate": gate_to_json(st.gate.as_ref()),
            })
        })
        .collect();
    json!({
        "name": r.name,
        "title": r.title,
        "description": r.description,
        "params": params,
        "stages": stages,
    })
}

/// recipe.list{} — discovery. Pure read (no project, no op, no checkpoint).
pub(super) async fn recipe_list(_args: Value) -> Result<VerbResult, CutError> {
    let list: Vec<Value> = recipes::registry()
        .recipes
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "title": r.title,
                "description": r.description,
                "params": param_summaries(r),
                "stage_count": r.stages.len(),
            })
        })
        .collect();
    Ok(VerbResult::ok(json!({ "recipes": list })))
}

/// recipe.describe{name} — the full manifest for inspection BEFORE running it.
/// Pure read; `NOT_FOUND` for an unknown name.
pub(super) async fn recipe_describe(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        name: String,
    }
    let a: Args = parse_args(args)?;
    let recipe = recipes::registry()
        .get(&a.name)
        .ok_or_else(|| recipe_not_found(&a.name))?;
    Ok(VerbResult::ok(describe_recipe(recipe)))
}

/// Compute the closed-vocab RecipeFacts from the open project (scoped read guard,
/// dropped immediately — the single-state-holder contract: the runner never holds a guard across dispatch).
async fn recipe_facts_now(state: &AppState) -> Result<recipes::RecipeFacts, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    Ok(recipes::RecipeFacts::compute(&store.project, &store.dir))
}

/// Evaluate one StatePredicate `measured <op> value` (or vs the run-start
/// `baseline` for the `*_start` ops). A missing measured fact = fail (never panic).
fn eval_predicate(
    measured: Option<i64>,
    op: &str,
    value: Option<&Value>,
    baseline: Option<i64>,
) -> bool {
    let Some(m) = measured else {
        return false;
    };
    let v = || value.and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f.round() as i64)));
    match op {
        "gt" => v().map(|x| m > x).unwrap_or(false),
        "gte" => v().map(|x| m >= x).unwrap_or(false),
        "lt" => v().map(|x| m < x).unwrap_or(false),
        "lte" => v().map(|x| m <= x).unwrap_or(false),
        "eq" => v().map(|x| m == x).unwrap_or(false),
        "lt_start" => baseline.map(|b| m < b).unwrap_or(false),
        "gt_start" => baseline.map(|b| m > b).unwrap_or(false),
        _ => false,
    }
}

/// Evaluate a stage gate: `checks` over the tip RenderReceipt + `state` over
/// the closed RecipeFacts. Returns a GateReport `{pass, checks[], state[]}` with
/// the measured evidence so a failure says WHY. A gate passes iff ALL checks pass
/// AND ALL state predicates hold (AND semantics).
async fn eval_gate(
    state: &AppState,
    gate: &recipes::Gate,
    baseline: &recipes::RecipeFacts,
) -> Result<Value, CutError> {
    let mut all_pass = true;
    let mut check_reports: Vec<Value> = Vec::new();
    if !gate.checks.is_empty() {
        // A `checks` gate is load-validated to sit only on a render.final stage,
        // so a receipt exists at the tip — read it (the autopilot helper).
        let receipts_dir = {
            let g = state.project.read().await;
            g.as_ref().ok_or_else(no_project)?.receipts_dir()
        };
        let receipt = read_receipt(&receipts_dir, None)?;
        for name in &gate.checks {
            let found = receipt.checks.iter().find(|c| c.name == *name);
            // A named check that did not appear in the receipt = fail (honest:
            // never assert a check the render did not emit).
            let pass = found.map(|c| c.pass).unwrap_or(false);
            if !pass {
                all_pass = false;
            }
            check_reports.push(json!({
                "name": name,
                "pass": pass,
                "found": found.is_some(),
                "evidence": found.map(|c| c.evidence.clone()),
            }));
        }
    }
    let mut state_reports: Vec<Value> = Vec::new();
    if !gate.state.is_empty() {
        let facts = recipe_facts_now(state).await?;
        for sp in &gate.state {
            let measured = facts.get(&sp.fact);
            let pass = eval_predicate(measured, &sp.op, sp.value.as_ref(), baseline.get(&sp.fact));
            if !pass {
                all_pass = false;
            }
            state_reports.push(json!({
                "fact": sp.fact,
                "op": sp.op,
                "value": sp.value,
                "measured": measured,
                "pass": pass,
            }));
        }
    }
    Ok(json!({ "pass": all_pass, "checks": check_reports, "state": state_reports }))
}

/// Poll a stage's sub-job (render/transcribe) to a terminal state, capped at
/// `cap_ms`. Mirrors the autopilot render-poll loop. Failed/timeout → Err.
pub(super) async fn poll_sub_job(
    state: &AppState,
    job_id: &str,
    cap_ms: u64,
) -> Result<crate::jobs::JobRecord, CutError> {
    let mut waited = 0u64;
    loop {
        match state.jobs.get(job_id) {
            Some(j) if matches!(j.state, crate::jobs::JobState::Done) => return Ok(j),
            Some(j) if matches!(j.state, crate::jobs::JobState::Failed) => {
                return Err(j.error.unwrap_or_else(|| {
                    CutError::new(
                        error_codes::JOB_FAILED,
                        format!("recipe sub-job {job_id} failed"),
                        "the stage's verb job ended in Failed",
                    )
                }));
            }
            _ => {
                if waited > cap_ms {
                    return Err(CutError::new(
                        error_codes::JOB_FAILED,
                        format!("recipe sub-job {job_id} timed out"),
                        format!("exceeded {}s", cap_ms / 1000),
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                waited += 500;
            }
        }
    }
}

/// Thread the recipe-level provenance into a stage's args `rationale` —
/// but ONLY for verbs whose schema declares a `rationale` property. media.transcribe
/// / edit.trim_edges do NOT accept one (additionalProperties:false), so passing it
/// would be invalid_args and the stage would fail — the exact trap autopilot
/// documents for trim_edges/captions.reflow.
fn thread_recipe_rationale(
    state: &AppState,
    args: &mut Value,
    recipe: &str,
    i: usize,
    n: usize,
    stage: &recipes::RecipeStage,
    run_rationale: Option<&str>,
) {
    let Some(spec) = state.registry.get(&stage.verb) else {
        return;
    };
    let accepts = spec
        .args
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|p| p.contains_key("rationale"))
        .unwrap_or(false);
    if !accepts {
        return;
    }
    let why = stage.rationale.as_deref().or(run_rationale).unwrap_or("");
    let text = format!("recipe {recipe} · stage {i}/{n} ({}): {why}", stage.verb);
    if let Value::Object(m) = args {
        m.insert("rationale".into(), json!(text));
    }
}

/// Finish a recipe job: compute the start→tip diff (the review artifact), gather
/// the render receipt ids, and finish the job with the clean report.
#[allow(clippy::too_many_arguments)]
async fn finish_recipe(
    state: &AppState,
    jid: &str,
    recipe: &str,
    policy: &str,
    status: &str,
    ran: usize,
    stage_results: Vec<Value>,
    start_at_op: &str,
    start_checkpoint: &str,
    actor: Actor,
    stop_reason: Option<String>,
) {
    let tip = snapshot(state)
        .await
        .map(|(_, _, _, at)| at)
        .unwrap_or_default();
    let changed = dispatch_send(
        state,
        "project.diff",
        json!({"from": start_at_op, "to": tip}),
        actor,
    )
    .await
    .result
    .unwrap_or(Value::Null);
    // Receipt ids come from any render stage's polled job_result.
    let receipt_ids: Vec<String> = stage_results
        .iter()
        .filter_map(|s| s.get("job_result"))
        .filter_map(|jr| jr.get("render_id"))
        .filter_map(|v| v.as_str())
        .map(String::from)
        .collect();
    let summary_line = match status {
        "completed" => format!("Done: {ran}/{ran} stages, all gates pass."),
        "completed_with_warnings" => format!(
            "Done with warnings: {ran}/{ran} stages and selected gates pass; review the render receipt."
        ),
        "gate_failed" => format!(
            "Stopped at stage {ran}: {}",
            stop_reason
                .clone()
                .unwrap_or_else(|| "a gate did not pass".into())
        ),
        _ => format!(
            "Stopped at stage {ran}: {}",
            stop_reason
                .clone()
                .unwrap_or_else(|| "a stage failed".into())
        ),
    };
    state.jobs.finish(
        jid,
        json!({
            "summary_line": summary_line,
            "recipe": recipe,
            "status": status,
            "policy": policy,
            "stages_run": ran,
            "stage_results": stage_results,
            "changed": changed,
            "checkpoint": start_checkpoint,
            "receipt_ids": receipt_ids,
            "restore_hint": format!("project.revert{{to:\"{start_checkpoint}\"}} undoes the whole recipe run"),
        }),
    );
}

/// recipe.run{name, args?, policy?, rationale?} — execute (or dry_run-plan) a
/// recipe. `run` (default) returns {job_id, checkpoint, recipe, stages}; the
/// clean report lands in the job result. `dry_run` returns the resolved PLAN
/// without a checkpoint or any dispatch (the pre-render-gate seam).
pub(super) async fn recipe_run(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        name: String,
        #[serde(default)]
        args: Value,
        #[serde(default)]
        policy: Option<String>,
        #[serde(default)]
        rationale: Option<String>,
    }
    let a: Args = parse_args(args)?;
    let recipe = recipes::registry()
        .get(&a.name)
        .cloned()
        .ok_or_else(|| recipe_not_found(&a.name))?;
    run_resolved_recipe(state, recipe, a.args, a.policy, a.rationale, actor).await
}

/// The runner core, factored out so tests can drive a constructed `Recipe`
/// directly without depending on slow perception/render stages in built-ins.
pub(super) async fn run_resolved_recipe(
    state: &AppState,
    recipe: recipes::Recipe,
    arg_overrides: Value,
    policy: Option<String>,
    run_rationale: Option<String>,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let policy = policy.unwrap_or_else(|| "run".into());
    if !matches!(policy.as_str(), "run" | "dry_run") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("unknown policy '{policy}'"),
            "valid: run (default) | dry_run",
        ));
    }
    // Resolve params (defaults ⊕ overrides; missing required → INVALID_ARGS).
    let params = recipes::resolve_params(&recipe, &arg_overrides).map_err(|missing| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("recipe '{}' requires param '{missing}'", recipe.name),
            "pass it under args, e.g. recipe.run{name, args:{asset:\"a1\"}}",
        )
    })?;
    // Interpolate every stage's args ONCE (used by both dry_run and run).
    let resolved_stages: Vec<(recipes::RecipeStage, Value)> = recipe
        .stages
        .iter()
        .map(|st| (st.clone(), recipes::interpolate(&st.args, &params)))
        .collect();
    // Require an open project up front (fail fast, not inside the job).
    {
        let g = state.project.read().await;
        g.as_ref().ok_or_else(no_project)?;
    }

    // policy=dry_run: return the PLAN without a checkpoint or any dispatch. STOP.
    if policy == "dry_run" {
        let stages: Vec<Value> = resolved_stages
            .iter()
            .map(|(st, iargs)| {
                json!({
                    "id": st.id,
                    "verb": st.verb,
                    "args": iargs,
                    "rationale": st.rationale,
                    "gate": gate_to_json(st.gate.as_ref()),
                })
            })
            .collect();
        return Ok(VerbResult::ok(json!({
            "recipe": recipe.name,
            "policy": "dry_run",
            "status": "planned",
            "params": Value::Object(params.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            "stages": stages,
        })));
    }

    // Auto-checkpoint: the whole run reverts to here in ONE step.
    let cp_rationale = match &run_rationale {
        Some(r) => format!("recipe {}: {r}", recipe.name),
        None => format!("recipe {}: start", recipe.name),
    };
    let cp = Box::pin(dispatch(
        state,
        "project.checkpoint",
        json!({"name": format!("recipe-{}-start", recipe.name), "rationale": cp_rationale}),
        actor.clone(),
    ))
    .await;
    if !cp.ok {
        return Ok(cp);
    }
    let cp_obj = cp.result.unwrap_or(Value::Null);
    let start_checkpoint = cp_obj
        .get("checkpoint")
        .and_then(|c| c.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let start_at_op = cp_obj
        .get("checkpoint")
        .and_then(|c| c.get("at_op"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Run-start facts baseline (for *_start gates).
    let baseline = recipe_facts_now(state).await?;
    let stage_overview: Vec<Value> = resolved_stages
        .iter()
        .map(|(st, _)| json!({"id": st.id, "verb": st.verb}))
        .collect();

    let job = state.jobs.create("recipe");
    let job_id = job.job_id.clone();
    let start_checkpoint_out = start_checkpoint.clone();
    let recipe_out = recipe.name.clone();
    let st = state.clone();
    let n = resolved_stages.len();
    let recipe_name = recipe.name.clone();
    let jobs = state.jobs.clone();
    jobs.spawn(&job_id, async move {
        let jid = job.job_id.clone();
        let mut stage_results: Vec<Value> = Vec::new();
        let mut ran = 0usize;
        let mut had_warnings = false;
        for (i, (stage, iargs)) in resolved_stages.iter().enumerate() {
            st.jobs.progress(
                &jid,
                0.05 + (i as f32 / n.max(1) as f32) * 0.9,
                Some(format!(
                    "recipe {recipe_name} · stage {}/{n} ({})",
                    i + 1,
                    stage.verb
                )),
            );
            // Thread the recipe rationale (for verbs that accept it).
            let mut sargs = iargs.clone();
            thread_recipe_rationale(
                &st,
                &mut sargs,
                &recipe_name,
                i + 1,
                n,
                stage,
                run_rationale.as_deref(),
            );
            // THE single dispatch path (real, replay-safe ops).
            let res = dispatch_send(&st, &stage.verb, sargs, actor.clone()).await;
            ran += 1;
            let mut sr = json!({
                "id": stage.id,
                "verb": stage.verb,
                "ok": res.ok,
                "op_ids": res.op_ids.clone().unwrap_or_default(),
            });
            if !res.ok {
                sr["error"] = json!(res.error);
                stage_results.push(sr);
                return finish_recipe(
                    &st,
                    &jid,
                    &recipe_name,
                    &policy,
                    "failed",
                    ran,
                    stage_results,
                    &start_at_op,
                    &start_checkpoint,
                    actor.clone(),
                    Some(format!("stage '{}' ({}) failed", stage.id, stage.verb)),
                )
                .await;
            }
            // Poll a sub-job to completion if the stage produced one (auto-detect
            // from a {job_id} result unless await_job is explicitly false).
            let sub = res
                .result
                .as_ref()
                .and_then(|r| r.get("job_id"))
                .and_then(|v| v.as_str())
                .map(String::from);
            if let Some(sub) = sub {
                if stage.await_job != Some(false) {
                    sr["job_id"] = json!(sub);
                    let cap_ms = if stage.verb == "render.final" {
                        180_000u64
                    } else {
                        600_000
                    };
                    match poll_sub_job(&st, &sub, cap_ms).await {
                        Ok(jr) => {
                            let completion_warning = matches!(
                                jr.completion,
                                Some(crate::jobs::JobCompletion::DoneWithWarnings)
                            );
                            let job_result = jr.result.unwrap_or(Value::Null);
                            if completion_warning
                                || job_result.get("verified").and_then(Value::as_bool)
                                    == Some(false)
                                || job_result.get("pass").and_then(Value::as_bool) == Some(false)
                            {
                                had_warnings = true;
                            }
                            sr["job_result"] = job_result;
                        }
                        Err(e) => {
                            sr["error"] = json!(e);
                            stage_results.push(sr);
                            return finish_recipe(
                                &st,
                                &jid,
                                &recipe_name,
                                &policy,
                                "failed",
                                ran,
                                stage_results,
                                &start_at_op,
                                &start_checkpoint,
                                actor.clone(),
                                Some(format!(
                                    "stage '{}' sub-job ({}) did not complete",
                                    stage.id, stage.verb
                                )),
                            )
                            .await;
                        }
                    }
                }
            }
            // Evaluate the gate.
            if let Some(gate) = &stage.gate {
                match eval_gate(&st, gate, &baseline).await {
                    Ok(report) => {
                        let pass = report["pass"].as_bool().unwrap_or(false);
                        sr["gate"] = report;
                        if !pass {
                            stage_results.push(sr);
                            return finish_recipe(
                                &st,
                                &jid,
                                &recipe_name,
                                &policy,
                                "gate_failed",
                                ran,
                                stage_results,
                                &start_at_op,
                                &start_checkpoint,
                                actor.clone(),
                                Some(format!("stage '{}' gate did not pass", stage.id)),
                            )
                            .await;
                        }
                    }
                    Err(e) => {
                        sr["error"] = json!(e);
                        stage_results.push(sr);
                        return finish_recipe(
                            &st,
                            &jid,
                            &recipe_name,
                            &policy,
                            "failed",
                            ran,
                            stage_results,
                            &start_at_op,
                            &start_checkpoint,
                            actor.clone(),
                            Some(format!("stage '{}' gate could not be evaluated", stage.id)),
                        )
                        .await;
                    }
                }
            }
            stage_results.push(sr);
        }
        let final_status = if had_warnings {
            "completed_with_warnings"
        } else {
            "completed"
        };
        finish_recipe(
            &st,
            &jid,
            &recipe_name,
            &policy,
            final_status,
            ran,
            stage_results,
            &start_at_op,
            &start_checkpoint,
            actor.clone(),
            None,
        )
        .await;
    });
    Ok(VerbResult::ok(json!({
        "job_id": job_id,
        "checkpoint": start_checkpoint_out,
        "recipe": recipe_out,
        "stages": stage_overview,
    })))
}
