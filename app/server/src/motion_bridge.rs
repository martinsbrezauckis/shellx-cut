//! motion_bridge.rs - ShellX Motion connector bridge for Cut.
//!
//! Owns the external Motion CLI boundary and replay-safe Cut import handoff.

#[path = "motion_connector_contract.rs"]
mod connector_contract;

use crate::dispatch::{
    dispatch_send, no_project, spawn_plain_import_chain, verify_attested_media_source,
};
use crate::motion_artifact::verify_motion_artifact_reference;
use crate::motion_editable_import::{
    apply_editable_motion_import, editable_planned_steps, parse_editable_motion_import_plan,
    EditableMotionImportPlan,
};
use crate::motion_package::duration_ms as motion_source_duration_ms;
pub(crate) use crate::motion_package::{
    identity as motion_package_identity, revision as motion_package_revision,
};
pub(crate) use crate::motion_runtime::motion_available;
use crate::motion_runtime::{build_motion_cli_command, run_motion_command_spec, MotionCommandSpec};
#[cfg(test)]
use crate::motion_runtime::{
    parse_motion_connector_stdout, tail, ENV_MOTION_BIN, ENV_MOTION_ROOT, ENV_MOTION_TIMEOUT_MS,
};
use crate::motion_template_catalog::resolve_motion_template_package;
#[cfg(test)]
use crate::motion_template_catalog::ENV_MOTION_TEMPLATE_ROOT;
use crate::state::AppState;
use cut_core::{error_codes, Actor, Asset, CutError, OpRecord, VerbResult};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Stdio;
#[cfg(test)]
use std::time::Duration;

pub(crate) const ENV_CANVAS_BIN: &str = "SHELLX_CANVAS_BIN";
const DEFAULT_TEMPLATE_ALIAS: &str = "editable-lower-third";
const MOTION_IMPORT_PLAN_MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct MotionTemplateRequest {
    pub template: String,
    pub params: Map<String, Value>,
    pub policy: MotionTemplatePolicy,
    pub out_dir: PathBuf,
    pub at_ms: u64,
    pub track: String,
    pub duration_ms: Option<u64>,
    pub dry_run_render: bool,
    pub checkpoint: bool,
    pub rationale: Option<String>,
    pub motion_job_id: Option<String>,
    pub caller_scope: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct MotionScriptRequest {
    pub script: Option<Value>,
    pub script_path: Option<PathBuf>,
    pub policy: MotionScriptPolicy,
    pub out_dir: PathBuf,
    pub at_ms: u64,
    pub track: String,
    pub duration_ms: Option<u64>,
    pub dry_run_render: bool,
    pub checkpoint: bool,
    pub rationale: Option<String>,
    pub motion_job_id: Option<String>,
    pub caller_scope: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct MotionImportRequest {
    pub path: PathBuf,
    pub package_dir: Option<PathBuf>,
    pub dry_run: bool,
    pub background: bool,
}

#[derive(Debug, Clone)]
struct MotionImportPlan {
    plan_path: PathBuf,
    plan_hash: String,
    package_dir: Option<PathBuf>,
    package_id: String,
    motion_id: String,
    target_id: String,
    mode: String,
    integration: Value,
    rendered_operations: Vec<MotionRenderedMediaOperation>,
    editable: Option<EditableMotionImportPlan>,
    editable_steps: Vec<Value>,
    warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct MotionRenderedMediaOperation {
    raw: Value,
    rendered_media: Value,
    rendered_path: PathBuf,
    artifact_handle: Option<Value>,
    artifact_proof: Option<Value>,
    rendered_dry_run: bool,
    start_ms: u64,
    duration_ms: u64,
    track: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MotionTemplatePolicy {
    Preview,
    Insert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MotionScriptPolicy {
    Preview,
    Insert,
}

/// Public verb: motion.template_to_cut.
pub(crate) async fn motion_template_to_cut(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let request = parse_motion_template_request(state, args).await?;
    if request.policy == MotionTemplatePolicy::Insert {
        let guard = state.project.read().await;
        guard.as_ref().ok_or_else(no_project)?;
    }
    let connector = run_motion_template_connector(&request).await?;
    match request.policy {
        MotionTemplatePolicy::Preview => {
            Ok(VerbResult::ok(motion_preview_result(&request, connector)))
        }
        MotionTemplatePolicy::Insert => {
            apply_motion_template_insert(state, request, connector, actor).await
        }
    }
}

/// Public verb: motion.script_to_cut.
pub(crate) async fn motion_script_to_cut(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let request = parse_motion_script_request(state, args).await?;
    if request.policy == MotionScriptPolicy::Insert {
        let guard = state.project.read().await;
        guard.as_ref().ok_or_else(no_project)?;
    }
    let script_path = materialize_motion_script(&request).await?;
    let connector = run_motion_script_connector(&request, &script_path).await?;
    match request.policy {
        MotionScriptPolicy::Preview => Ok(VerbResult::ok(motion_script_preview_result(
            &request,
            &script_path,
            connector,
        ))),
        MotionScriptPolicy::Insert => {
            apply_motion_script_insert(state, request, script_path, connector, actor).await
        }
    }
}

/// Public verb: motion.map_import.
pub(crate) async fn motion_map_import(
    _state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    let request = parse_motion_import_request(args, true)?;
    let plan = read_rendered_media_import_plan(&request).await?;
    Ok(VerbResult::ok(motion_import_map_result(&plan)))
}

/// Public verb: motion.apply_import.
pub(crate) async fn motion_apply_import(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    let request = parse_motion_import_request(args, true)?;
    if request.background {
        if request.dry_run {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "background Motion apply cannot be a dry run",
                "use dryRun:true without background, or dryRun:false with background:true",
            ));
        }
        let project_dir = {
            let guard = state.project.read().await;
            guard.as_ref().ok_or_else(no_project)?.dir.clone()
        };
        let job = state.jobs.create("motion_import_apply");
        let job_id = job.job_id.clone();
        let task_job_id = job_id.clone();
        let task_state = state.clone();
        state.jobs.spawn(&job_id, async move {
            task_state.jobs.progress(
                &task_job_id,
                0.05,
                Some("validating attested Motion import plan".into()),
            );
            #[cfg(test)]
            tokio::time::sleep(Duration::from_millis(25)).await;
            let result = match read_rendered_media_import_plan(&request).await {
                Ok(plan) => {
                    task_state.jobs.progress(
                        &task_job_id,
                        0.4,
                        Some("staging verified media and timeline edits".into()),
                    );
                    apply_motion_import_plan(&task_state, &plan, actor, Some(project_dir.as_path()))
                        .await
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(result) => task_state
                    .jobs
                    .finish(&task_job_id, result.result.unwrap_or(Value::Null)),
                Err(error) => task_state.jobs.fail(&task_job_id, error),
            }
        });
        return Ok(VerbResult::ok(json!({
            "ok": true,
            "schema": "shellx-cut/motion-import-apply-job@1",
            "job_id": job_id,
            "cancellable": true,
            "status": "queued",
        })));
    }
    let plan = read_rendered_media_import_plan(&request).await?;
    if request.dry_run {
        return Ok(VerbResult::ok(motion_import_apply_dry_run_result(&plan)));
    }
    apply_motion_import_plan(state, &plan, actor, None).await
}

/// Public verb: motion.link.relink. Repair only the local package binding; the
/// rendered clip remains unchanged until an explicit refresh succeeds.
pub(crate) async fn motion_link_relink(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
        package_dir: String,
        rationale: Option<String>,
    }
    let request: Args = serde_json::from_value(args).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "motion.link.relink arguments are invalid",
            error.to_string(),
        )
    })?;
    let package_dir = PathBuf::from(&request.package_dir)
        .canonicalize()
        .map_err(|error| {
            CutError::new(
                error_codes::NOT_FOUND,
                "Motion package directory was not found",
                format!("{}: {error}", request.package_dir),
            )
        })?;
    let (package_id, motion_id) = motion_package_identity(&package_dir)?;
    let source_revision = motion_package_revision(&package_dir)?;
    let (mut link, expected_revision) = current_motion_link(state, &request.clip).await?;
    if link.get("packageId").and_then(Value::as_str) != Some(package_id.as_str())
        || link.get("motionId").and_then(Value::as_str) != Some(motion_id.as_str())
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "selected Motion package has another identity",
            format!(
                "expected {} / {}, got {} / {}",
                link.get("packageId").and_then(Value::as_str).unwrap_or("?"),
                link.get("motionId").and_then(Value::as_str).unwrap_or("?"),
                package_id,
                motion_id
            ),
        )
        .with_suggested_action("choose the original Motion package for this clip"));
    }
    let object = link.as_object_mut().expect("validated link object");
    object.insert("sourcePath".into(), json!(package_dir));
    object.insert("sourceRevision".into(), json!(source_revision));
    object.insert("sourceRevisionKind".into(), json!("motion-package"));
    object.insert("state".into(), json!("source-dirty"));
    let op = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        ensure_motion_link_unchanged(
            store.log.read_all()?.as_slice(),
            &request.clip,
            &expected_revision,
        )?;
        store.record_motion_link_source_update(
            "motion.link.relink",
            &request.clip,
            link,
            actor,
            request
                .rationale
                .or_else(|| Some("Relink ShellX Motion source".into())),
        )?
    };
    state
        .events
        .publish(crate::events::Event::OpApplied { op: op.clone() });
    Ok(VerbResult::ok_with_ops(
        json!({
            "ok": true,
            "schema": "shellx-cut/motion-link-relink@1",
            "clip": request.clip,
            "packageDir": package_dir,
            "packageId": package_id,
            "motionId": motion_id,
            "sourceRevision": source_revision,
            "state": "source-dirty",
        }),
        vec![op.op_id],
    ))
}

/// Public verb: motion.link.refresh. Render to a new project-owned artifact,
/// verify the Motion receipt against the file, then atomically swap the linked
/// clip in place. Any failure leaves the last good asset and link untouched.
pub(crate) async fn motion_link_refresh(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
        preset: Option<String>,
        job_id: Option<String>,
        rationale: Option<String>,
    }
    let request: Args = serde_json::from_value(args).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "motion.link.refresh arguments are invalid",
            error.to_string(),
        )
    })?;
    let preset = request.preset.as_deref().unwrap_or("mp4-h264");
    if !matches!(preset, "mp4-h264" | "mp4-h265") {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Motion linked-clip preset is unsupported",
            "preset must be mp4-h264 or mp4-h265",
        ));
    }
    let (project_dir, mut link, expected_revision) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let (link, expected) = current_motion_link_from_ops(
            store.log.read_all()?.as_slice(),
            &store.project,
            &request.clip,
        )?;
        (store.dir.clone(), link, expected)
    };
    let resolved_source = crate::motion_edit_return::resolve_latest_source(
        &project_dir,
        &request.clip,
        &safe_fragment(&request.clip),
        &link,
    )?;
    let source_path = resolved_source.package_dir;
    let package_id = resolved_source.package_id;
    let motion_id = resolved_source.motion_id;
    let source_revision = resolved_source.source_revision;
    let canvas_return = resolved_source.canvas_return;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| CutError::new(error_codes::IO, "read system clock", error.to_string()))?
        .as_nanos();
    let render_rel = format!(
        "motion-renders/{}/{}-{preset}/render.mp4",
        safe_fragment(&request.clip),
        stamp
    );
    let output_path = crate::output_paths::fence_project_output_path(
        &project_dir,
        None,
        &render_rel,
        crate::output_paths::OutputPathPolicy::MP4,
    )?;
    let render_dir = output_path
        .parent()
        .expect("fenced render output has a parent")
        .to_path_buf();
    let frames_dir = render_dir.join("frames");
    let spec = build_motion_render_command(
        &source_path,
        &output_path,
        &frames_dir,
        preset,
        request.job_id.as_deref(),
        &project_dir,
    );
    let rendered = run_motion_command_spec(spec, "render linked Motion clip").await?;
    if rendered.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(CutError::new(
            error_codes::SIDECAR,
            "ShellX Motion linked render failed",
            rendered.to_string(),
        ));
    }
    let receipt_package = rendered
        .get("receipt")
        .and_then(|receipt| receipt.get("packageId"))
        .and_then(Value::as_str);
    if receipt_package != Some(package_id.as_str()) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "ShellX Motion render receipt belongs to another package",
            format!(
                "expected {package_id}, got {}",
                receipt_package.unwrap_or("missing")
            ),
        ));
    }
    let expected_sha256 = rendered
        .get("output")
        .and_then(|output| output.get("sha256"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CutError::new(
                error_codes::SIDECAR,
                "ShellX Motion render returned no output digest",
                "receipt.output.sha256 is required",
            )
        })?
        .to_string();
    let expected_byte_length = tokio::fs::metadata(&output_path).await?.len();
    let verified_path = output_path.clone();
    let verified = tokio::task::spawn_blocking(move || {
        verify_attested_media_source(&verified_path, &expected_sha256, expected_byte_length)
    })
    .await
    .map_err(|error| {
        CutError::new(error_codes::IO, "verify Motion rerender", error.to_string())
    })??;
    let revision_after_render = motion_package_revision(&source_path)?;
    if revision_after_render != source_revision {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "Motion package changed during rerender",
            "the candidate render was not attached to the Cut clip",
        )
        .with_suggested_action("refresh again after Motion finishes saving"));
    }
    let last_receipt_id = rendered
        .get("receipt")
        .and_then(|receipt| receipt.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let last_receipt_path = rendered
        .get("receiptPath")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            CutError::new(
                error_codes::SIDECAR,
                "ShellX Motion render returned no receipt path",
                "receiptPath is required for a linked render refresh",
            )
        })?;
    let rendered_duration_ms = rendered
        .get("output")
        .and_then(|output| output.get("durationMs"))
        .and_then(Value::as_u64)
        .or_else(|| motion_source_duration_ms(&source_path));
    let previous_render = link.get("render").cloned().unwrap_or(Value::Null);
    let fallback_path = previous_render
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| link.get("fallbackPath").and_then(Value::as_str))
        .map(str::to_string);
    let object = link.as_object_mut().expect("validated link object");
    object.insert("sourcePath".into(), json!(source_path));
    object.insert("sourceRevision".into(), json!(source_revision));
    object.insert("sourceRevisionKind".into(), json!("motion-package"));
    object.insert("state".into(), json!("linked-current"));
    object.insert(
        "render".into(),
        json!({
            "path": verified.0,
            "sha256": verified.1.strip_prefix("sha256:").unwrap_or(&verified.1),
            "byteLength": expected_byte_length,
            "artifactHandleId": null,
            "preset": preset,
        }),
    );
    object.insert("fallbackPath".into(), json!(fallback_path));
    object.insert("previousRender".into(), previous_render);
    object.insert("lastReceiptId".into(), json!(last_receipt_id));
    object.insert("lastReceiptPath".into(), json!(last_receipt_path.clone()));
    let asset = Asset {
        path: verified.0.display().to_string(),
        hash: verified.1.clone(),
        probe: rendered_duration_ms.map(|duration_ms| {
            json!({
                "kind": "video",
                "duration_ms": duration_ms,
            })
        }),
        transcript: None,
        perception: None,
        proxy: None,
        filmstrip: None,
    };
    let refreshed = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        if store.dir != project_dir {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "project changed while Motion rendered",
                "the candidate render was not attached",
            ));
        }
        ensure_motion_link_unchanged(
            store.log.read_all()?.as_slice(),
            &request.clip,
            &expected_revision,
        )?;
        store.apply_motion_link_refresh(
            &request.clip,
            asset,
            link,
            actor,
            request
                .rationale
                .or_else(|| Some("Refresh linked ShellX Motion clip".into())),
        )?
    };
    state.events.publish(crate::events::Event::OpApplied {
        op: refreshed.op.clone(),
    });
    let job_id = spawn_plain_import_chain(
        state.clone(),
        refreshed.asset_id.clone(),
        verified.0.clone(),
        verified.1.clone(),
        false,
    );
    let motion_job_id = rendered
        .get("jobId")
        .cloned()
        .or_else(|| request.job_id.clone().map(Value::String))
        .unwrap_or(Value::Null);
    Ok(VerbResult::ok_with_ops(
        json!({
            "ok": true,
            "schema": "shellx-cut/motion-link-refresh@1",
            "clip": request.clip,
            "asset": refreshed.asset_id,
            "job_id": job_id,
            "motion_job_id": motion_job_id,
            "packageId": package_id,
            "motionId": motion_id,
            "sourceRevision": source_revision,
            "render": {
                "path": verified.0,
                "sha256": verified.1.strip_prefix("sha256:").unwrap_or(&verified.1),
                "byteLength": expected_byte_length,
                "preset": preset,
            },
            "lastReceiptId": last_receipt_id,
            "receiptPath": last_receipt_path,
            "canvasReturn": canvas_return.as_ref().map(|candidate| candidate.public_evidence()),
            "state": "linked-current",
            "restore_hint": "project.undo restores the previous linked render and clip asset",
        }),
        vec![refreshed.op.op_id],
    ))
}

/// Public verb: motion.link.edit. Launch Canvas against the verified linked
/// package; Canvas owns the path-safe SDK intake and rich editor session.
pub(crate) async fn motion_link_edit(
    state: &AppState,
    args: Value,
    _actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        clip: String,
    }
    let request: Args = serde_json::from_value(args).map_err(|error| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "motion.link.edit arguments are invalid",
            error.to_string(),
        )
    })?;
    let (project_dir, link) = {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        let (link, _) = current_motion_link_from_ops(
            store.log.read_all()?.as_slice(),
            &store.project,
            &request.clip,
        )?;
        (store.dir.clone(), link)
    };
    let source_path = link
        .get("sourcePath")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CutError::new(
                error_codes::NOT_FOUND,
                "linked Motion source is not available",
                "this clip has no local package path",
            )
            .with_suggested_action("relink the Motion source before opening it")
        })?;
    let source_path = PathBuf::from(source_path).canonicalize().map_err(|error| {
        CutError::new(
            error_codes::NOT_FOUND,
            "linked Motion source is missing",
            error.to_string(),
        )
        .with_suggested_action("relink the clip to its original Motion package")
    })?;
    let (package_id, motion_id) = motion_package_identity(&source_path)?;
    if link.get("packageId").and_then(Value::as_str) != Some(package_id.as_str())
        || link.get("motionId").and_then(Value::as_str) != Some(motion_id.as_str())
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "linked Motion source identity changed",
            "Cut refused to open a different package under this clip's identity",
        ));
    }
    let source_revision = motion_package_revision(&source_path)?;
    let program = resolve_canvas_program().ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            "ShellX Canvas executable is not available",
            format!("set {ENV_CANVAS_BIN} or install shellx-canvas on PATH"),
        )
    })?;
    let return_request = crate::motion_edit_return::create_request(
        &project_dir,
        &request.clip,
        &safe_fragment(&request.clip),
        &package_id,
        &motion_id,
        &source_revision,
    )?;
    let child = tokio::process::Command::new(&program)
        .arg("--motion-package")
        .arg(&source_path)
        .arg("--motion-cut-return-request")
        .arg(&return_request)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false)
        .spawn()
        .map_err(|error| {
            CutError::new(
                error_codes::SIDECAR,
                "ShellX Canvas could not be launched",
                format!("{program}: {error}"),
            )
        })?;
    Ok(VerbResult::ok(json!({
        "ok": true,
        "schema": "shellx-cut/motion-link-edit@1",
        "clip": request.clip,
        "packageId": package_id,
        "motionId": motion_id,
        "launched": true,
        "pid": child.id(),
        "returnChannel": { "state": "pending", "pathPrivate": true },
        "localOnly": true,
        "remotePublish": false,
    })))
}

pub(crate) fn canvas_available() -> bool {
    resolve_canvas_program().is_some()
}

fn resolve_canvas_program() -> Option<String> {
    if let Ok(program) = std::env::var(ENV_CANVAS_BIN) {
        let value = program.trim();
        if !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control) {
            return Some(value.to_string());
        }
    }
    let names: &[&str] = if cfg!(windows) {
        &["shellx-canvas.exe", "ShellX Canvas.exe"]
    } else {
        &["shellx-canvas", "shellx-canvas-app"]
    };
    if let Some(found) = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
            .find(|path| path.is_file())
    }) {
        return Some(found.display().to_string());
    }
    #[cfg(target_os = "macos")]
    {
        for path in [
            PathBuf::from("/Applications/ShellX Canvas.app/Contents/MacOS/shellx-canvas"),
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join("Applications/ShellX Canvas.app/Contents/MacOS/shellx-canvas"),
        ] {
            if path.is_file() {
                return Some(path.display().to_string());
            }
        }
    }
    None
}

pub(crate) async fn parse_motion_template_request(
    state: &AppState,
    args: Value,
) -> Result<MotionTemplateRequest, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        #[serde(default)]
        template: Option<String>,
        #[serde(default)]
        params: Map<String, Value>,
        #[serde(default)]
        policy: Option<String>,
        #[serde(default)]
        out_dir: Option<String>,
        #[serde(default)]
        at_ms: Option<u64>,
        #[serde(default)]
        track: Option<String>,
        #[serde(default)]
        duration_ms: Option<u64>,
        #[serde(default)]
        dry_run_render: Option<bool>,
        #[serde(default)]
        checkpoint: Option<bool>,
        #[serde(default)]
        rationale: Option<String>,
        #[serde(default)]
        job_id: Option<String>,
    }

    let a: Args = serde_json::from_value(args).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "motion.template_to_cut args did not match schema",
            e.to_string(),
        )
    })?;
    let policy = match a.policy.as_deref().unwrap_or("insert") {
        "preview" => MotionTemplatePolicy::Preview,
        "insert" => MotionTemplatePolicy::Insert,
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("invalid motion.template_to_cut policy '{other}'"),
                "allowed policy values: preview, insert",
            ));
        }
    };
    let template = a
        .template
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_TEMPLATE_ALIAS.to_string());
    let out_dir = match a.out_dir {
        Some(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => default_motion_out_dir(state, &template, policy).await,
    };
    let caller_scope = motion_caller_scope(state, &out_dir).await;
    let dry_run_render = match policy {
        MotionTemplatePolicy::Preview => true,
        MotionTemplatePolicy::Insert => a.dry_run_render.unwrap_or(false),
    };
    Ok(MotionTemplateRequest {
        template,
        params: a.params,
        policy,
        out_dir,
        at_ms: a.at_ms.unwrap_or(0),
        track: a.track.unwrap_or_else(|| "v1".to_string()),
        duration_ms: a.duration_ms,
        dry_run_render,
        checkpoint: a.checkpoint.unwrap_or(true),
        rationale: a.rationale,
        motion_job_id: a.job_id,
        caller_scope,
    })
}

pub(crate) async fn parse_motion_script_request(
    state: &AppState,
    args: Value,
) -> Result<MotionScriptRequest, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        #[serde(default)]
        script: Option<Value>,
        #[serde(default)]
        script_path: Option<String>,
        #[serde(default)]
        policy: Option<String>,
        #[serde(default)]
        out_dir: Option<String>,
        #[serde(default)]
        at_ms: Option<u64>,
        #[serde(default)]
        track: Option<String>,
        #[serde(default)]
        duration_ms: Option<u64>,
        #[serde(default)]
        dry_run_render: Option<bool>,
        #[serde(default)]
        checkpoint: Option<bool>,
        #[serde(default)]
        rationale: Option<String>,
        #[serde(default)]
        job_id: Option<String>,
    }

    let a: Args = serde_json::from_value(args).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "motion.script_to_cut args did not match schema",
            e.to_string(),
        )
    })?;
    let script_path = a
        .script_path
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from);
    if a.script.is_some() == script_path.is_some() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "motion.script_to_cut requires exactly one of script or script_path",
            "pass an inline shellx-motion/scripted-video@1 object, or a path to one",
        ));
    }
    let policy = match a.policy.as_deref().unwrap_or("insert") {
        "preview" => MotionScriptPolicy::Preview,
        "insert" => MotionScriptPolicy::Insert,
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("invalid motion.script_to_cut policy '{other}'"),
                "allowed policy values: preview, insert",
            ));
        }
    };
    let default_leaf = a
        .script
        .as_ref()
        .and_then(|script| script.get("id"))
        .and_then(|id| id.as_str())
        .map(safe_fragment)
        .unwrap_or_else(|| "scripted_video".to_string());
    let out_dir = match a.out_dir {
        Some(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => default_motion_script_out_dir(state, &default_leaf, policy).await,
    };
    let caller_scope = motion_caller_scope(state, &out_dir).await;
    let dry_run_render = match policy {
        MotionScriptPolicy::Preview => true,
        MotionScriptPolicy::Insert => a.dry_run_render.unwrap_or(false),
    };
    Ok(MotionScriptRequest {
        script: a.script,
        script_path,
        policy,
        out_dir,
        at_ms: a.at_ms.unwrap_or(0),
        track: a.track.unwrap_or_else(|| "v1".to_string()),
        duration_ms: a.duration_ms,
        dry_run_render,
        checkpoint: a.checkpoint.unwrap_or(true),
        rationale: a.rationale,
        motion_job_id: a.job_id,
        caller_scope,
    })
}

pub(crate) fn parse_motion_import_request(
    args: Value,
    default_dry_run: bool,
) -> Result<MotionImportRequest, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        path: String,
        #[serde(default, rename = "packageDir")]
        package_dir: Option<String>,
        #[serde(default, rename = "dryRun")]
        dry_run: Option<bool>,
        #[serde(default)]
        background: Option<bool>,
    }

    let a: Args = serde_json::from_value(args).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "motion import-plan args did not match schema",
            e.to_string(),
        )
    })?;
    if a.path.trim().is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "motion import-plan path is required",
            "pass path to a shellx-motion/cut-import-plan@1 JSON file",
        ));
    }
    Ok(MotionImportRequest {
        path: PathBuf::from(a.path),
        package_dir: a
            .package_dir
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from),
        dry_run: a.dry_run.unwrap_or(default_dry_run),
        background: a.background.unwrap_or(false),
    })
}

async fn default_motion_out_dir(
    state: &AppState,
    template: &str,
    policy: MotionTemplatePolicy,
) -> PathBuf {
    let safe = safe_fragment(template);
    let leaf = match policy {
        MotionTemplatePolicy::Preview => "preview",
        MotionTemplatePolicy::Insert => "insert",
    };
    let project_dir = {
        let guard = state.project.read().await;
        guard.as_ref().map(|store| store.dir.clone())
    };
    project_dir
        .unwrap_or_else(|| std::env::temp_dir().join("shellx-cut"))
        .join("motion")
        .join(leaf)
        .join(safe)
}

async fn default_motion_script_out_dir(
    state: &AppState,
    script_id: &str,
    policy: MotionScriptPolicy,
) -> PathBuf {
    let leaf = match policy {
        MotionScriptPolicy::Preview => "preview",
        MotionScriptPolicy::Insert => "insert",
    };
    let project_dir = {
        let guard = state.project.read().await;
        guard.as_ref().map(|store| store.dir.clone())
    };
    project_dir
        .unwrap_or_else(|| std::env::temp_dir().join("shellx-cut"))
        .join("motion")
        .join("script-to-cut")
        .join(leaf)
        .join(safe_fragment(script_id))
}

async fn motion_caller_scope(state: &AppState, fallback: &Path) -> PathBuf {
    let guard = state.project.read().await;
    guard
        .as_ref()
        .map(|store| store.dir.clone())
        .unwrap_or_else(|| fallback.to_path_buf())
}

async fn materialize_motion_script(request: &MotionScriptRequest) -> Result<PathBuf, CutError> {
    if let Some(script_path) = &request.script_path {
        return Ok(script_path.clone());
    }
    let script = request.script.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "motion.script_to_cut requires script JSON",
            "inline script was missing after argument validation",
        )
    })?;
    let script_path = request.out_dir.join("scripted-video.json");
    if let Some(parent) = script_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            CutError::new(
                error_codes::IO,
                "create Motion scripted-video output dir",
                e.to_string(),
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(script).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "motion.script_to_cut script could not be serialized",
            e.to_string(),
        )
    })?;
    tokio::fs::write(&script_path, [bytes, b"\n".to_vec()].concat())
        .await
        .map_err(|e| {
            CutError::new(
                error_codes::IO,
                "write Motion scripted-video JSON",
                e.to_string(),
            )
        })?;
    Ok(script_path)
}

pub(crate) async fn run_motion_template_connector(
    request: &MotionTemplateRequest,
) -> Result<Value, CutError> {
    let spec = build_motion_template_command(request)?;
    run_motion_command_spec(spec, "render a Motion template for Cut").await
}

pub(crate) async fn run_motion_script_connector(
    request: &MotionScriptRequest,
    script_path: &Path,
) -> Result<Value, CutError> {
    let spec = build_motion_script_command(request, script_path)?;
    run_motion_command_spec(spec, "render a Motion script for Cut").await
}

pub(crate) fn build_motion_template_command(
    request: &MotionTemplateRequest,
) -> Result<MotionCommandSpec, CutError> {
    let package_root = resolve_motion_template_package(&request.template)?;
    let mut connector_args = vec![
        "connector".to_string(),
        "template-to-cut".to_string(),
        package_root.display().to_string(),
        "--out".to_string(),
        request.out_dir.display().to_string(),
        "--cut-import-mode".to_string(),
        "rendered_media".to_string(),
        "--start-ms".to_string(),
        request.at_ms.to_string(),
        "--track".to_string(),
        request.track.clone(),
    ];
    if let Some(duration_ms) = request.duration_ms {
        connector_args.push("--duration-ms".to_string());
        connector_args.push(duration_ms.to_string());
    }
    if request.dry_run_render {
        connector_args.push("--dry-run-render".to_string());
    }
    for (key, value) in &request.params {
        connector_args.push("--set".to_string());
        connector_args.push(format!("{key}={}", template_value_arg(value)));
    }
    if let Some(job_id) = &request.motion_job_id {
        connector_args.push("--job-id".to_string());
        connector_args.push(job_id.clone());
    }

    Ok(build_motion_cli_command(
        connector_args,
        &request.caller_scope,
    ))
}

pub(crate) fn build_motion_script_command(
    request: &MotionScriptRequest,
    script_path: &Path,
) -> Result<MotionCommandSpec, CutError> {
    let mut connector_args = vec![
        "connector".to_string(),
        "script-to-cut".to_string(),
        script_path.display().to_string(),
        "--out".to_string(),
        request.out_dir.display().to_string(),
        "--cut-import-mode".to_string(),
        "rendered_media".to_string(),
        "--start-ms".to_string(),
        request.at_ms.to_string(),
        "--track".to_string(),
        request.track.clone(),
    ];
    if let Some(duration_ms) = request.duration_ms {
        connector_args.push("--duration-ms".to_string());
        connector_args.push(duration_ms.to_string());
    }
    if request.dry_run_render {
        connector_args.push("--dry-run-render".to_string());
    }
    if let Some(job_id) = &request.motion_job_id {
        connector_args.push("--job-id".to_string());
        connector_args.push(job_id.clone());
    }

    Ok(build_motion_cli_command(
        connector_args,
        &request.caller_scope,
    ))
}

fn latest_motion_link(ops: &[OpRecord], clip_id: &str) -> Option<Value> {
    ops.iter()
        .filter(|op| {
            matches!(
                op.verb.as_str(),
                "motion.apply_import"
                    | "motion.link.update"
                    | "motion.link.refresh"
                    | "motion.link.relink"
                    | "motion.link.tracking.request"
                    | "motion.link.tracking.apply"
                    | "motion.link.tracking.detach"
            )
        })
        .flat_map(|op| op.effects.iter())
        .filter_map(|effect| effect.detail.get("motion_links").and_then(Value::as_array))
        .flatten()
        .rfind(|link| {
            link.get("schema").and_then(Value::as_str) == Some("shellx-cut/motion-link@1")
                && link.get("clipId").and_then(Value::as_str) == Some(clip_id)
        })
        .cloned()
}

fn motion_link_generation(link: &Value) -> Result<String, CutError> {
    let bytes = serde_json::to_vec(link)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub(crate) fn current_motion_link_from_ops(
    ops: &[OpRecord],
    project: &cut_core::Project,
    clip_id: &str,
) -> Result<(Value, String), CutError> {
    if project.find_clip(clip_id).is_none() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            format!("clip '{clip_id}' is not on the timeline"),
            "use project.state to select a live linked Motion clip",
        ));
    }
    let link = latest_motion_link(ops, clip_id).ok_or_else(|| {
        CutError::new(
            error_codes::NOT_FOUND,
            format!("clip '{clip_id}' has no ShellX Motion link"),
            "refresh and relink are available only for Motion-rendered clips",
        )
    })?;
    let generation = motion_link_generation(&link)?;
    Ok((link, generation))
}

pub(crate) async fn current_motion_link(
    state: &AppState,
    clip_id: &str,
) -> Result<(Value, String), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    current_motion_link_from_ops(store.log.read_all()?.as_slice(), &store.project, clip_id)
}

pub(crate) fn ensure_motion_link_unchanged(
    ops: &[OpRecord],
    clip_id: &str,
    expected_generation: &str,
) -> Result<(), CutError> {
    let current = latest_motion_link(ops, clip_id).ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "Motion link disappeared while the operation was running",
            clip_id,
        )
    })?;
    if motion_link_generation(&current)? != expected_generation {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "Motion link changed while the operation was running",
            "the completed candidate was not attached",
        )
        .with_suggested_action("refresh the project state and try again"));
    }
    Ok(())
}

fn build_motion_render_command(
    package_dir: &Path,
    output_path: &Path,
    frames_dir: &Path,
    preset: &str,
    motion_job_id: Option<&str>,
    caller_scope: &Path,
) -> MotionCommandSpec {
    let mut connector_args = vec![
        "render".to_string(),
        package_dir.display().to_string(),
        "--lane".to_string(),
        "ffmpeg".to_string(),
        "--out".to_string(),
        output_path.display().to_string(),
        "--preset".to_string(),
        preset.to_string(),
        "--frames-dir".to_string(),
        frames_dir.display().to_string(),
    ];
    if let Some(job_id) = motion_job_id {
        connector_args.push("--job-id".to_string());
        connector_args.push(job_id.to_string());
    }
    build_motion_cli_command(connector_args, caller_scope)
}

fn motion_preview_result(request: &MotionTemplateRequest, connector: Value) -> Value {
    let motion_job_id = connector
        .get("jobId")
        .cloned()
        .or_else(|| request.motion_job_id.clone().map(Value::String))
        .unwrap_or(Value::Null);
    json!({
        "policy": "preview",
        "template": request.template,
        "params": request.params,
        "motion_job_id": motion_job_id,
        "connector": connector,
        "preview": connector.get("preview").cloned().unwrap_or(Value::Null),
        "render": connector.get("render").cloned().unwrap_or(Value::Null),
        "artifacts": connector.get("artifacts").cloned().unwrap_or_else(|| json!([])),
        "receiptPath": connector.get("receiptPath").cloned().unwrap_or(Value::Null),
        "warnings": motion_warnings(&connector),
    })
}

fn motion_script_preview_result(
    request: &MotionScriptRequest,
    script_path: &Path,
    connector: Value,
) -> Value {
    let motion_job_id = connector
        .get("jobId")
        .cloned()
        .or_else(|| request.motion_job_id.clone().map(Value::String))
        .unwrap_or(Value::Null);
    json!({
        "policy": "preview",
        "script": request.script.clone().unwrap_or(Value::Null),
        "scriptPath": script_path,
        "motion_job_id": motion_job_id,
        "connector": connector,
        "preview": connector.get("preview").cloned().unwrap_or(Value::Null),
        "render": connector.get("render").cloned().unwrap_or(Value::Null),
        "artifacts": connector.get("artifacts").cloned().unwrap_or_else(|| json!([])),
        "receiptPath": connector.get("receiptPath").cloned().unwrap_or(Value::Null),
        "warnings": motion_warnings(&connector),
    })
}

fn motion_apply_import_args(path: &str, package_dir: Option<&str>) -> Value {
    let mut args = json!({
        "path": path,
        "dryRun": false,
    });
    if let Some(package_dir) = package_dir {
        args["packageDir"] = json!(package_dir);
    }
    args
}

async fn apply_motion_template_insert(
    state: &AppState,
    request: MotionTemplateRequest,
    connector: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    if request.dry_run_render {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "motion.template_to_cut insert requires a real render",
            "dry_run_render produced no MP4 to import",
        ));
    }
    {
        let guard = state.project.read().await;
        guard.as_ref().ok_or_else(no_project)?;
    }
    let render_output = connector_contract::render_output(&connector)?;
    if !Path::new(render_output).is_file() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "ShellX Motion rendered media was not found",
            render_output,
        ));
    }
    let cut_plan_path = connector_contract::cut_plan_path(&connector)?;
    let package_dir = connector_contract::package_dir(&connector);
    let rationale = request
        .rationale
        .clone()
        .unwrap_or_else(|| format!("motion.template_to_cut {}", request.template));

    let mut op_ids: Vec<String> = Vec::new();
    let checkpoint = if request.checkpoint {
        let cp = dispatch_send(
            state,
            "project.checkpoint",
            json!({
                "name": format!("motion-{}-start", safe_fragment(&request.template)),
                "rationale": rationale,
            }),
            actor.clone(),
        )
        .await;
        if !cp.ok {
            return Ok(cp);
        }
        op_ids.extend(cp.op_ids.clone().unwrap_or_default());
        cp.result
            .as_ref()
            .and_then(|r| r.get("checkpoint"))
            .cloned()
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let mut applied = dispatch_send(
        state,
        "motion.apply_import",
        motion_apply_import_args(cut_plan_path, package_dir),
        actor,
    )
    .await;
    if !applied.ok {
        op_ids.extend(applied.op_ids.clone().unwrap_or_default());
        applied.op_ids = (!op_ids.is_empty()).then_some(op_ids);
        return Ok(applied);
    }
    op_ids.extend(applied.op_ids.clone().unwrap_or_default());
    let atomic = applied.result.clone().unwrap_or(Value::Null);
    let effective_checkpoint = if checkpoint.is_null() {
        atomic.get("checkpoint").cloned().unwrap_or(Value::Null)
    } else {
        checkpoint
    };
    let restore_hint = effective_checkpoint
        .get("id")
        .and_then(Value::as_str)
        .map(|id| format!("project.revert{{to:\"{id}\"}} undoes this Motion template insert"))
        .or_else(|| {
            atomic
                .get("restore_hint")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "project.undo reverts the atomic Motion template insert".to_string());
    let warnings = motion_apply_warnings(&connector, &atomic);
    let render = connector.get("render").cloned().unwrap_or(Value::Null);
    let artifacts = connector
        .get("artifacts")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let receipt_path = connector.get("receiptPath").cloned().unwrap_or(Value::Null);
    let motion_job_id = connector
        .get("jobId")
        .cloned()
        .or_else(|| request.motion_job_id.clone().map(Value::String))
        .unwrap_or(Value::Null);
    Ok(VerbResult::ok_with_ops(
        json!({
            "policy": "insert",
            "status": atomic.get("status").cloned().unwrap_or_else(|| json!("passed")),
            "template": request.template,
            "params": request.params,
            "motion_job_id": motion_job_id,
            "connector": connector,
            "checkpoint": effective_checkpoint,
            "import": atomic.get("import").cloned().unwrap_or(Value::Null),
            "insert": atomic.get("insert").cloned().unwrap_or(Value::Null),
            "op_ids": op_ids.clone(),
            "clips": atomic.get("clips").cloned().unwrap_or_else(|| json!([])),
            "assets": atomic.get("assets").cloned().unwrap_or_else(|| json!([])),
            "idempotencyKey": atomic.get("idempotencyKey").cloned().unwrap_or(Value::Null),
            "alreadyApplied": atomic.get("alreadyApplied").cloned().unwrap_or(json!(false)),
            "render": render,
            "artifacts": artifacts,
            "receiptPath": receipt_path,
            "warnings": warnings,
            "restore_hint": restore_hint,
        }),
        op_ids,
    ))
}

async fn apply_motion_script_insert(
    state: &AppState,
    request: MotionScriptRequest,
    script_path: PathBuf,
    connector: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    if request.dry_run_render {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "motion.script_to_cut insert requires a real render",
            "dry_run_render produced no MP4 to import",
        ));
    }
    {
        let guard = state.project.read().await;
        guard.as_ref().ok_or_else(no_project)?;
    }
    let render_output = connector_contract::render_output(&connector)?;
    if !Path::new(render_output).is_file() {
        return Err(CutError::new(
            error_codes::NOT_FOUND,
            "ShellX Motion rendered media was not found",
            render_output,
        ));
    }
    let cut_plan_path = connector_contract::cut_plan_path(&connector)?;
    let package_dir = connector_contract::package_dir(&connector);
    let script_id = request
        .script
        .as_ref()
        .and_then(|script| script.get("id"))
        .and_then(|id| id.as_str())
        .unwrap_or("scripted-video");
    let rationale = request
        .rationale
        .clone()
        .unwrap_or_else(|| format!("motion.script_to_cut {}", safe_fragment(script_id)));

    let mut op_ids: Vec<String> = Vec::new();
    let checkpoint = if request.checkpoint {
        let cp = dispatch_send(
            state,
            "project.checkpoint",
            json!({
                "name": format!("motion-script-{}-start", safe_fragment(script_id)),
                "rationale": rationale,
            }),
            actor.clone(),
        )
        .await;
        if !cp.ok {
            return Ok(cp);
        }
        op_ids.extend(cp.op_ids.clone().unwrap_or_default());
        cp.result
            .as_ref()
            .and_then(|r| r.get("checkpoint"))
            .cloned()
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let mut applied = dispatch_send(
        state,
        "motion.apply_import",
        motion_apply_import_args(cut_plan_path, package_dir),
        actor,
    )
    .await;
    if !applied.ok {
        op_ids.extend(applied.op_ids.clone().unwrap_or_default());
        applied.op_ids = (!op_ids.is_empty()).then_some(op_ids);
        return Ok(applied);
    }
    op_ids.extend(applied.op_ids.clone().unwrap_or_default());
    let atomic = applied.result.clone().unwrap_or(Value::Null);
    let effective_checkpoint = if checkpoint.is_null() {
        atomic.get("checkpoint").cloned().unwrap_or(Value::Null)
    } else {
        checkpoint
    };
    let restore_hint = effective_checkpoint
        .get("id")
        .and_then(Value::as_str)
        .map(|id| format!("project.revert{{to:\"{id}\"}} undoes this Motion scripted-video insert"))
        .or_else(|| {
            atomic
                .get("restore_hint")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            "project.undo reverts the atomic Motion scripted-video insert".to_string()
        });
    let warnings = motion_apply_warnings(&connector, &atomic);
    let render = connector.get("render").cloned().unwrap_or(Value::Null);
    let artifacts = connector
        .get("artifacts")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let receipt_path = connector.get("receiptPath").cloned().unwrap_or(Value::Null);
    let motion_job_id = connector
        .get("jobId")
        .cloned()
        .or_else(|| request.motion_job_id.clone().map(Value::String))
        .unwrap_or(Value::Null);
    Ok(VerbResult::ok_with_ops(
        json!({
            "policy": "insert",
            "status": atomic.get("status").cloned().unwrap_or_else(|| json!("passed")),
            "script": request.script.clone().unwrap_or(Value::Null),
            "scriptPath": script_path,
            "motion_job_id": motion_job_id,
            "connector": connector,
            "checkpoint": effective_checkpoint,
            "import": atomic.get("import").cloned().unwrap_or(Value::Null),
            "insert": atomic.get("insert").cloned().unwrap_or(Value::Null),
            "op_ids": op_ids.clone(),
            "clips": atomic.get("clips").cloned().unwrap_or_else(|| json!([])),
            "assets": atomic.get("assets").cloned().unwrap_or_else(|| json!([])),
            "idempotencyKey": atomic.get("idempotencyKey").cloned().unwrap_or(Value::Null),
            "alreadyApplied": atomic.get("alreadyApplied").cloned().unwrap_or(json!(false)),
            "render": render,
            "artifacts": artifacts,
            "receiptPath": receipt_path,
            "warnings": warnings,
            "restore_hint": restore_hint,
        }),
        op_ids,
    ))
}

fn motion_apply_warnings(connector: &Value, applied: &Value) -> Vec<String> {
    let mut warnings = motion_warnings(connector);
    for warning in applied
        .get("warnings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if !warnings.iter().any(|existing| existing == warning) {
            warnings.push(warning.to_string());
        }
    }
    warnings
}

async fn read_rendered_media_import_plan(
    request: &MotionImportRequest,
) -> Result<MotionImportPlan, CutError> {
    let plan_path = tokio::fs::canonicalize(&request.path)
        .await
        .map_err(|error| {
            CutError::new(
                error_codes::NOT_FOUND,
                "Motion Cut import plan was not found",
                format!("{}: {error}", request.path.display()),
            )
        })?;
    let before = tokio::fs::metadata(&plan_path).await.map_err(|error| {
        CutError::new(
            error_codes::IO,
            "inspect Motion Cut import plan",
            error.to_string(),
        )
    })?;
    if !before.is_file() || before.len() > MOTION_IMPORT_PLAN_MAX_BYTES {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Motion Cut import plan is not a bounded regular file",
            format!(
                "{} bytes; limit is {}",
                before.len(),
                MOTION_IMPORT_PLAN_MAX_BYTES
            ),
        ));
    }
    let bytes = tokio::fs::read(&plan_path).await.map_err(|e| {
        CutError::new(
            error_codes::IO,
            "read Motion Cut import plan",
            format!("{}: {e}", plan_path.display()),
        )
    })?;
    let after = tokio::fs::metadata(&plan_path).await.map_err(|error| {
        CutError::new(
            error_codes::IO,
            "reinspect Motion Cut import plan",
            error.to_string(),
        )
    })?;
    if bytes.len() as u64 != before.len()
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
    {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "Motion Cut import plan changed while it was read",
            plan_path.display().to_string(),
        ));
    }
    let plan: Value = serde_json::from_slice(&bytes).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "Motion Cut import plan is invalid JSON",
            e.to_string(),
        )
    })?;
    let plan_hash = format!("{:x}", Sha256::digest(&bytes));
    require_plan_string(&plan, "schema", "shellx-motion/cut-import-plan@1")?;
    // Negotiate the producer envelope before reading package identity,
    // operations, artifact handles, or any connector-provided path.
    let integration = verify_motion_import_integration(&plan, bytes.len() as u64)?;
    if plan.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Motion Cut import plan is not ok",
            plan.get("error")
                .cloned()
                .unwrap_or_else(|| json!(plan))
                .to_string(),
        ));
    }
    let package_id = required_str_field(&plan, "packageId")?;
    let motion_id = required_str_field(&plan, "motionId")?;
    let target_id = required_str_field(&plan, "targetId")?;
    if target_id != "shellx-cut" {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Motion Cut import plan targets another product",
            format!("targetId was '{target_id}', expected 'shellx-cut'"),
        ));
    }
    let mode = required_str_field(&plan, "mode")?;
    if mode != "rendered_media" && mode != "editable_lowering" {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Motion Cut import plan mode is not supported by Cut apply yet",
            format!("mode '{mode}' is not implemented; use editable_lowering or rendered_media"),
        )
        .with_suggested_action(
            "Render through ShellX Motion and pass a rendered_media cut-import-plan",
        ));
    }
    let operation_values = plan
        .get("operations")
        .and_then(|v| v.as_array())
        .filter(|ops| !ops.is_empty())
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "Motion Cut import plan has no operations",
                "expected at least one cut.media.import_rendered operation",
            )
        })?;
    let (rendered_operations, editable, editable_steps) = if mode == "rendered_media" {
        let plan_receipt = plan.get("receipt").ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "Motion Cut import plan has no receipt",
                "rendered_media plans require a shellx-motion/receipt@1 commitment",
            )
        })?;
        let mut operations = Vec::with_capacity(operation_values.len());
        for operation in operation_values {
            operations.push(
                parse_rendered_media_operation(
                    &plan_path,
                    request.package_dir.as_ref(),
                    &plan,
                    operation,
                    plan_receipt,
                    &package_id,
                    &motion_id,
                )
                .await?,
            );
        }
        (operations, None, Vec::new())
    } else {
        let editable = parse_editable_motion_import_plan(&plan, &package_id, &motion_id)?;
        let steps = editable_planned_steps(&editable)?;
        (Vec::new(), Some(editable), steps)
    };

    Ok(MotionImportPlan {
        plan_path,
        plan_hash,
        package_dir: request.package_dir.clone(),
        package_id,
        motion_id,
        target_id,
        mode,
        integration,
        rendered_operations,
        editable,
        editable_steps,
        warnings: motion_plan_warnings(&plan),
    })
}

async fn parse_rendered_media_operation(
    plan_path: &Path,
    package_dir: Option<&PathBuf>,
    plan: &Value,
    operation: &Value,
    plan_receipt: &Value,
    package_id: &str,
    motion_id: &str,
) -> Result<MotionRenderedMediaOperation, CutError> {
    require_exact_plan_fields(
        operation,
        "Motion rendered-media operation",
        &[
            "verb",
            "source",
            "startMs",
            "durationMs",
            "media",
            "renderedMedia",
        ],
        &["track"],
    )?;
    require_plan_string(operation, "verb", "cut.media.import_rendered")?;
    let source = operation.get("source").ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "Motion rendered-media operation has no source",
            "expected packageId, motionId, and render mode",
        )
    })?;
    require_exact_plan_fields(
        source,
        "Motion rendered-media source",
        &["packageId", "motionId", "render"],
        &[],
    )?;
    require_plan_string(source, "packageId", package_id)?;
    require_plan_string(source, "motionId", motion_id)?;
    let media = operation.get("media").ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "Motion rendered-media operation has no media metadata",
            "expected positive width, height, and fps",
        )
    })?;
    require_exact_plan_fields(
        media,
        "Motion rendered-media metadata",
        &["width", "height", "fps"],
        &[],
    )?;
    for field in ["width", "height", "fps"] {
        if !media
            .get(field)
            .and_then(Value::as_f64)
            .is_some_and(|value| value.is_finite() && value > 0.0)
        {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "Motion rendered-media metadata is invalid",
                format!("{field} must be a positive finite number"),
            ));
        }
    }
    let rendered_media = operation.get("renderedMedia").cloned().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "Motion rendered-media operation has no renderedMedia",
            "cut.media.import_rendered requires a dry-run plannedPath or attested handle",
        )
    })?;
    let rendered_dry_run = rendered_media
        .get("dryRun")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "Motion renderedMedia.dryRun is invalid",
                "expected a boolean",
            )
        })?;
    require_exact_plan_fields(
        &rendered_media,
        "Motion renderedMedia",
        if rendered_dry_run {
            &["dryRun", "plannedPath", "receiptPath"]
        } else {
            &["dryRun", "handle"]
        },
        &[],
    )?;
    require_plan_string(
        source,
        "render",
        if rendered_dry_run {
            "dry_run"
        } else {
            "artifact"
        },
    )?;
    let (rendered_path, artifact_handle, artifact_proof) = if rendered_dry_run {
        let planned_path = required_str_field(&rendered_media, "plannedPath")?;
        (
            resolve_motion_import_path(&planned_path, plan_path, package_dir),
            None,
            None,
        )
    } else {
        let verified = verify_motion_artifact_reference(
            plan_path,
            package_dir.map(PathBuf::as_path),
            plan,
            operation,
            &rendered_media,
            plan_receipt,
            package_id,
            motion_id,
        )
        .await?;
        (verified.path, Some(verified.handle), Some(verified.proof))
    };
    let duration_ms = operation
        .get("durationMs")
        .and_then(|v| v.as_u64())
        .filter(|v| *v > 0)
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "Motion rendered-media operation has invalid durationMs",
                "durationMs must be a positive integer",
            )
        })?;
    let start_ms = operation
        .get("startMs")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "Motion rendered-media operation has invalid startMs",
                "startMs must be a non-negative integer",
            )
        })?;
    let track = operation
        .get("track")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("v1")
        .to_string();
    Ok(MotionRenderedMediaOperation {
        raw: operation.clone(),
        rendered_media,
        rendered_path,
        artifact_handle,
        artifact_proof,
        rendered_dry_run,
        start_ms,
        duration_ms,
        track,
    })
}

fn require_exact_plan_fields(
    value: &Value,
    label: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<(), CutError> {
    let object = value.as_object().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            format!("{label} is invalid"),
            "expected an object",
        )
    })?;
    if let Some(field) = required.iter().find(|field| !object.contains_key(**field)) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{label} is incomplete"),
            format!("missing required field {field}"),
        ));
    }
    if let Some(field) = object
        .keys()
        .find(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("{label} contains an unsupported field"),
            field.clone(),
        ));
    }
    Ok(())
}

fn resolve_motion_import_path(
    raw: &str,
    plan_path: &Path,
    package_dir: Option<&PathBuf>,
) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return path;
    }
    if let Some(package_dir) = package_dir {
        return package_dir.join(path);
    }
    plan_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path)
}

fn require_plan_string(value: &Value, field: &str, expected: &str) -> Result<(), CutError> {
    let actual = required_str_field(value, field)?;
    if actual != expected {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("Motion Cut import plan field '{field}' is unsupported"),
            format!("expected '{expected}', got '{actual}'"),
        ));
    }
    Ok(())
}

fn required_str_field(value: &Value, field: &str) -> Result<String, CutError> {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                format!("Motion Cut import plan is missing '{field}'"),
                "expected a non-empty string",
            )
        })
}

fn verify_motion_import_integration(plan: &Value, plan_bytes: u64) -> Result<Value, CutError> {
    let Some(envelope) = plan.get("integration") else {
        return Ok(json!({
            "schema": "shellx-motion/integration-compatibility-adapter@1",
            "ok": true,
            "adapter": "shellx-motion/cut-import-plan@1-without-envelope",
            "payloadSchema": "shellx-motion/cut-import-plan@1",
        }));
    };
    let envelope = exact_object(
        envelope,
        &["schema", "producer", "binding"],
        "Motion integration envelope",
    )?;
    require_json_string(
        envelope,
        "schema",
        "shellx-motion/integration-envelope@1",
        "Motion integration envelope",
    )?;
    let producer = exact_object(
        envelope.get("producer").unwrap_or(&Value::Null),
        &[
            "schema", "host", "protocol", "schemas", "modes", "presets", "features", "limits",
        ],
        "Motion integration producer capabilities",
    )?;
    require_json_string(
        producer,
        "schema",
        "shellx-motion/integration-capabilities@1",
        "Motion integration producer capabilities",
    )?;
    require_json_string(
        producer,
        "host",
        "shellx-motion",
        "Motion integration producer capabilities",
    )?;
    let protocol = exact_object(
        producer.get("protocol").unwrap_or(&Value::Null),
        &["min", "max", "preferred"],
        "Motion integration protocol range",
    )?;
    let min = required_positive_integer(protocol, "min", "Motion integration protocol range")?;
    let max = required_positive_integer(protocol, "max", "Motion integration protocol range")?;
    let preferred =
        required_positive_integer(protocol, "preferred", "Motion integration protocol range")?;
    if min > max || preferred < min || preferred > max || min > 1 || max < 1 {
        return Err(integration_error(
            "Motion and Cut integration protocol ranges do not overlap",
            format!("Motion advertised {min}-{max} (preferred {preferred}); Cut supports 1"),
        ));
    }
    let modes = required_unique_strings(producer, "modes", "Motion integration capabilities")?;
    if !modes.iter().any(|value| value == "cut.import.plan") {
        return Err(integration_error(
            "Motion integration has no shared Cut import mode",
            "producer capabilities are missing cut.import.plan",
        ));
    }
    let features =
        required_unique_strings(producer, "features", "Motion integration capabilities")?;
    required_unique_strings(producer, "presets", "Motion integration capabilities")?;
    if !features.iter().any(|value| value == "artifact.attestation") {
        return Err(integration_error(
            "Motion integration is missing artifact attestation",
            "producer capabilities are missing artifact.attestation",
        ));
    }
    let schemas = producer
        .get("schemas")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            integration_error(
                "Motion integration schemas are invalid",
                "producer schemas must be an object",
            )
        })?;
    for key in schemas.keys() {
        required_unique_strings(schemas, key, "Motion integration schemas")?;
    }
    let cut_schemas = schemas
        .get("cut")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            integration_error(
                "Motion integration has no Cut payload schemas",
                "producer schemas.cut must be an array",
            )
        })?;
    if !cut_schemas
        .iter()
        .any(|value| value.as_str() == Some("shellx-motion/cut-import-plan@1"))
    {
        return Err(integration_error(
            "Motion integration does not support this Cut plan schema",
            "producer schemas.cut is missing shellx-motion/cut-import-plan@1",
        ));
    }
    let limits = exact_object(
        producer.get("limits").unwrap_or(&Value::Null),
        &["maxPlanBytes", "maxArtifactBytes", "maxOperations"],
        "Motion integration limits",
    )?;
    let max_plan_bytes =
        required_positive_integer(limits, "maxPlanBytes", "Motion integration limits")?;
    required_positive_integer(limits, "maxArtifactBytes", "Motion integration limits")?;
    let max_operations =
        required_positive_integer(limits, "maxOperations", "Motion integration limits")?;
    let negotiated_plan_bytes = max_plan_bytes.min(MOTION_IMPORT_PLAN_MAX_BYTES);
    if plan_bytes > negotiated_plan_bytes {
        return Err(integration_error(
            "Motion Cut import plan exceeds the negotiated byte limit",
            format!("{plan_bytes} bytes; negotiated limit is {negotiated_plan_bytes}"),
        ));
    }
    let operation_count = plan
        .get("operations")
        .and_then(Value::as_array)
        .map_or(0_u64, |operations| operations.len() as u64);
    let negotiated_operations = max_operations.min(10_000);
    if operation_count > negotiated_operations {
        return Err(integration_error(
            "Motion Cut import plan exceeds the negotiated operation limit",
            format!("{operation_count} operations; negotiated limit is {negotiated_operations}"),
        ));
    }

    let binding = exact_object(
        envelope.get("binding").unwrap_or(&Value::Null),
        &[
            "schema",
            "protocol",
            "producer",
            "consumer",
            "mode",
            "payloadSchema",
            "requiredFeatures",
        ],
        "Motion integration binding",
    )?;
    require_json_string(
        binding,
        "schema",
        "shellx-motion/integration-binding@1",
        "Motion integration binding",
    )?;
    require_json_string(
        binding,
        "producer",
        "shellx-motion",
        "Motion integration binding",
    )?;
    require_json_string(
        binding,
        "consumer",
        "shellx-cut",
        "Motion integration binding",
    )?;
    require_json_string(
        binding,
        "mode",
        "cut.import.plan",
        "Motion integration binding",
    )?;
    require_json_string(
        binding,
        "payloadSchema",
        "shellx-motion/cut-import-plan@1",
        "Motion integration binding",
    )?;
    if required_positive_integer(binding, "protocol", "Motion integration binding")? != 1 {
        return Err(integration_error(
            "Motion integration binding protocol is unsupported",
            "Cut supports protocol 1",
        ));
    }
    let required_features =
        required_unique_strings(binding, "requiredFeatures", "Motion integration binding")?;
    for feature in &required_features {
        if feature != "artifact.attestation" {
            return Err(integration_error(
                "Motion integration requires an unsupported feature",
                feature.clone(),
            ));
        }
    }
    if !required_features
        .iter()
        .any(|value| value == "artifact.attestation")
    {
        return Err(integration_error(
            "Motion integration binding does not require artifact attestation",
            "canonical Cut handoffs must require artifact.attestation",
        ));
    }

    Ok(json!({
        "schema": "shellx-motion/integration-negotiation@1",
        "ok": true,
        "localHost": "shellx-motion",
        "remoteHost": "shellx-cut",
        "selectedProtocol": 1,
        "modes": ["cut.import.plan"],
        "features": ["artifact.attestation"],
        "payloadSchema": "shellx-motion/cut-import-plan@1",
        "missingRequiredModes": [],
    }))
}

fn exact_object<'a>(
    value: &'a Value,
    allowed: &[&str],
    label: &str,
) -> Result<&'a Map<String, Value>, CutError> {
    let object = value
        .as_object()
        .ok_or_else(|| integration_error(format!("{label} is invalid"), "expected an object"))?;
    if let Some(field) = object
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(integration_error(
            format!("{label} contains an unknown field"),
            field.clone(),
        ));
    }
    Ok(object)
}

fn require_json_string(
    object: &Map<String, Value>,
    field: &str,
    expected: &str,
    label: &str,
) -> Result<(), CutError> {
    let actual = object.get(field).and_then(Value::as_str).unwrap_or("");
    if actual != expected {
        return Err(integration_error(
            format!("{label} field '{field}' is unsupported"),
            format!("expected '{expected}', got '{actual}'"),
        ));
    }
    Ok(())
}

fn required_positive_integer(
    object: &Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<u64, CutError> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            integration_error(
                format!("{label} field '{field}' is invalid"),
                "expected a positive integer",
            )
        })
}

fn required_unique_strings(
    object: &Map<String, Value>,
    field: &str,
    label: &str,
) -> Result<Vec<String>, CutError> {
    let values = object.get(field).and_then(Value::as_array).ok_or_else(|| {
        integration_error(
            format!("{label} field '{field}' is invalid"),
            "expected an array",
        )
    })?;
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let value = value
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                integration_error(
                    format!("{label} field '{field}' is invalid"),
                    "expected non-empty strings",
                )
            })?
            .to_string();
        if result.contains(&value) {
            return Err(integration_error(
                format!("{label} field '{field}' has duplicates"),
                value,
            ));
        }
        result.push(value);
    }
    Ok(result)
}

fn integration_error(message: impl Into<String>, details: impl Into<String>) -> CutError {
    CutError::new(error_codes::INVALID_ARGS, message, details)
        .with_suggested_action("regenerate the import plan with a compatible ShellX Motion build")
}

fn motion_plan_warnings(plan: &Value) -> Vec<String> {
    let mut warnings: Vec<String> = Vec::new();
    for value in plan
        .get("warnings")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(warning) = value.as_str() {
            if !warnings.iter().any(|existing| existing == warning) {
                warnings.push(warning.to_string());
            }
        }
    }
    for value in plan
        .get("receipt")
        .and_then(|v| v.get("warnings"))
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(warning) = value.as_str() {
            if !warnings.iter().any(|existing| existing == warning) {
                warnings.push(warning.to_string());
            }
        }
    }
    for value in plan
        .get("unsupported")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(reason) = value.get("reason").and_then(|v| v.as_str()) {
            if !warnings.iter().any(|existing| existing == reason) {
                warnings.push(reason.to_string());
            }
        }
    }
    warnings
}

fn motion_import_map_result(plan: &MotionImportPlan) -> Value {
    let raw_operations = motion_import_raw_operations(plan);
    json!({
        "ok": true,
        "schema": "shellx-cut/motion-import-map@1",
        "planPath": plan.plan_path,
        "packageDir": plan.package_dir,
        "packageId": plan.package_id,
        "motionId": plan.motion_id,
        "targetId": plan.target_id,
        "mode": plan.mode,
        "integration": plan.integration,
        "operationCount": raw_operations.len(),
        "operations": raw_operations,
        "renderedMedia": plan.rendered_operations.first().map(|op| op.rendered_media.clone()).unwrap_or(Value::Null),
        "artifactHandles": plan.rendered_operations.iter().filter_map(|op| op.artifact_handle.clone()).collect::<Vec<_>>(),
        "lineageProofs": motion_import_lineage_proofs(plan),
        "planned": planned_cut_steps(plan),
        "warnings": plan.warnings,
    })
}

fn motion_import_apply_dry_run_result(plan: &MotionImportPlan) -> Value {
    json!({
        "ok": true,
        "schema": "shellx-cut/motion-import-apply@1",
        "dryRun": true,
        "wouldMutate": false,
        "planPath": plan.plan_path,
        "packageDir": plan.package_dir,
        "packageId": plan.package_id,
        "motionId": plan.motion_id,
        "targetId": plan.target_id,
        "mode": plan.mode,
        "integration": plan.integration,
        "operationCount": motion_import_operation_count(plan),
        "lineageProofs": motion_import_lineage_proofs(plan),
        "planned": planned_cut_steps(plan),
        "warnings": plan.warnings,
    })
}

fn motion_import_lineage_proofs(plan: &MotionImportPlan) -> Vec<Value> {
    plan.rendered_operations
        .iter()
        .filter_map(|operation| operation.artifact_proof.clone())
        .collect()
}

fn motion_import_raw_operations(plan: &MotionImportPlan) -> Vec<Value> {
    if let Some(editable) = &plan.editable {
        editable
            .operations
            .iter()
            .map(|operation| operation.raw.clone())
            .collect()
    } else {
        plan.rendered_operations
            .iter()
            .map(|operation| operation.raw.clone())
            .collect()
    }
}

fn motion_import_operation_count(plan: &MotionImportPlan) -> usize {
    plan.rendered_operations.len()
        + plan
            .editable
            .as_ref()
            .map_or(0, |editable| editable.operations.len())
}

fn planned_cut_steps(plan: &MotionImportPlan) -> Vec<Value> {
    if plan.editable.is_some() {
        return plan.editable_steps.clone();
    }
    let mut planned = Vec::new();
    for op in &plan.rendered_operations {
        planned.push(json!({
            "verb": "media.import",
            "args": {
                "path": op.rendered_path.display().to_string(),
                "proxy": false,
                "rationale": "ShellX Motion rendered media import",
            }
        }));
        planned.push(json!({
            "verb": "edit.insert",
            "args": {
                "asset": "<media.import.asset_id>",
                "track": op.track,
                "at_ms": op.start_ms,
                "src_range_ms": [0, op.duration_ms],
                "rationale": "ShellX Motion rendered media timeline insert",
            }
        }));
    }
    planned
}

async fn apply_motion_import_plan(
    state: &AppState,
    plan: &MotionImportPlan,
    actor: Actor,
    expected_project_dir: Option<&Path>,
) -> Result<VerbResult, CutError> {
    {
        let guard = state.project.read().await;
        let store = guard.as_ref().ok_or_else(no_project)?;
        if expected_project_dir.is_some_and(|expected| store.dir != expected) {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "project changed while Motion import was queued",
                format!("the active project is now '{}'", store.dir.display()),
            )
            .with_suggested_action("submit the import again for the currently open project"));
        }
    }
    if let Some(editable) = &plan.editable {
        let applied = apply_editable_motion_import(
            state,
            editable,
            &plan.plan_hash,
            &plan.package_id,
            &plan.motion_id,
            actor,
        )
        .await?;
        let clips = applied
            .bindings
            .iter()
            .filter_map(|binding| binding.get("clipId").cloned())
            .collect::<Vec<_>>();
        let assets = applied
            .bindings
            .iter()
            .filter_map(|binding| binding.get("assetId").cloned())
            .filter(|asset| !asset.is_null())
            .collect::<Vec<_>>();
        let op_ids = applied.op_ids.clone();
        let committed_ops = op_ids.len();
        return Ok(VerbResult::ok_with_ops(
            json!({
                "ok": true,
                "status": if applied.already_applied { "already_applied" } else { "passed" },
                "schema": "shellx-cut/motion-import-apply@1",
                "dryRun": false,
                "wouldMutate": true,
                "planPath": plan.plan_path,
                "packageDir": plan.package_dir,
                "packageId": plan.package_id,
                "motionId": plan.motion_id,
                "targetId": plan.target_id,
                "mode": plan.mode,
                "integration": plan.integration,
                "idempotencyKey": plan.plan_hash,
                "alreadyApplied": applied.already_applied,
                "reimported": applied.reimported,
                "checkpoint": applied.checkpoint,
                "lineageProofs": motion_import_lineage_proofs(plan),
                "planned": planned_cut_steps(plan),
                "bindings": applied.bindings,
                "mappingOpId": applied.mapping_op_id,
                "op_ids": op_ids,
                "clips": clips,
                "assets": assets,
                "warnings": plan.warnings,
                "rollback": { "required": false, "status": "not_needed", "committedOps": committed_ops },
                "restore_hint": "project.undo removes the grouped native Motion import; the returned checkpoint can restore it explicitly",
            }),
            op_ids,
        ));
    }
    for op in &plan.rendered_operations {
        if op.rendered_dry_run {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "motion.apply_import real apply requires rendered media",
                format!("{} was marked dryRun:true", op.rendered_path.display()),
            ));
        }
        if !op.rendered_path.is_file() {
            return Err(CutError::new(
                error_codes::NOT_FOUND,
                "ShellX Motion rendered media was not found",
                op.rendered_path.display().to_string(),
            ));
        }
    }

    let mut staged_assets = Vec::with_capacity(plan.rendered_operations.len());
    let mut staged_sources = Vec::with_capacity(plan.rendered_operations.len());
    let mut inserts = Vec::with_capacity(plan.rendered_operations.len());
    let mut motion_links = Vec::with_capacity(plan.rendered_operations.len());
    for op in &plan.rendered_operations {
        let handle = op.artifact_handle.as_ref().ok_or_else(|| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "Motion real import has no verified artifact handle",
                op.rendered_path.display().to_string(),
            )
        })?;
        let expected_sha256 = handle
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "Motion handle has no SHA-256",
                    handle.to_string(),
                )
            })?
            .to_string();
        let expected_byte_length = handle
            .get("byteLength")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                CutError::new(
                    error_codes::INVALID_ARGS,
                    "Motion handle has no byteLength",
                    handle.to_string(),
                )
            })?;
        let source_path = op.rendered_path.clone();
        let verified = tokio::task::spawn_blocking(move || {
            verify_attested_media_source(&source_path, &expected_sha256, expected_byte_length)
        })
        .await
        .map_err(|error| {
            CutError::new(
                error_codes::IO,
                "verify Motion media before atomic apply",
                error.to_string(),
            )
        })??;
        staged_assets.push(Asset {
            path: verified.0.display().to_string(),
            hash: verified.1.clone(),
            probe: None,
            transcript: None,
            perception: None,
            proxy: None,
            filmstrip: None,
        });
        staged_sources.push(verified);
        let last_receipt_id =
            handle
                .get("receipts")
                .and_then(Value::as_array)
                .and_then(|receipts| {
                    receipts
                        .iter()
                        .rev()
                        .find_map(|receipt| receipt.get("id").and_then(Value::as_str))
                });
        motion_links.push(json!({
            "schema": "shellx-cut/motion-link@1",
            "motionSourceId": format!("{}:{}", plan.package_id, plan.motion_id),
            "packageId": plan.package_id,
            "motionId": plan.motion_id,
            "sourceRevision": plan.plan_hash,
            "sourceRevisionKind": "cut-import-plan",
            "sourcePath": plan.package_dir,
            "planPath": plan.plan_path,
            "mode": "rendered_media",
            "state": "linked-current",
            "render": {
                "path": op.rendered_path,
                "sha256": handle.get("sha256").cloned().unwrap_or(Value::Null),
                "byteLength": handle.get("byteLength").cloned().unwrap_or(Value::Null),
                "artifactHandleId": handle.get("id").cloned().unwrap_or(Value::Null),
            },
            "fallbackPath": op.rendered_path,
            "lastReceiptId": last_receipt_id,
            "originAttestation": op.artifact_proof.clone().unwrap_or(Value::Null),
            "editableInCut": ["placement", "trim", "opacity", "transitions"],
            "opaqueInCut": ["shader", "scene3d", "particles", "motionBlur", "film"],
        }));
        inserts.push(json!({
            "track": op.track,
            "at_ms": op.start_ms,
            "src_range_ms": [0, op.duration_ms],
            "ripple": false,
        }));
    }

    let atomic = {
        let mut guard = state.project.write().await;
        let store = guard.as_mut().ok_or_else(no_project)?;
        if expected_project_dir.is_some_and(|expected| store.dir != expected) {
            return Err(CutError::new(
                error_codes::CONFLICT,
                "project changed before Motion import could commit",
                format!("the active project is now '{}'", store.dir.display()),
            )
            .with_suggested_action("submit the import again for the currently open project"));
        }
        store.apply_atomic_media_insert_plan_with_links(
            &plan.plan_hash,
            staged_assets,
            inserts,
            Some(motion_links),
            actor,
            Some("Attested ShellX Motion import plan".into()),
        )?
    };
    if !atomic.already_applied {
        state.events.publish(crate::events::Event::OpApplied {
            op: atomic.op.clone(),
        });
    }
    let mut import_results = Vec::with_capacity(atomic.asset_ids.len());
    let mut insert_results = Vec::with_capacity(atomic.clip_ids.len());
    for (index, asset_id) in atomic.asset_ids.iter().enumerate() {
        let job_id = if atomic.already_applied {
            None
        } else {
            let (path, hash) = staged_sources[index].clone();
            Some(spawn_plain_import_chain(
                state.clone(),
                asset_id.clone(),
                path,
                hash,
                false,
            ))
        };
        import_results.push(json!({
            "asset_id": asset_id,
            "job_id": job_id,
            "attested": true,
        }));
        insert_results.push(json!({
            "clip_id": atomic.clip_ids[index],
            "asset_id": asset_id,
        }));
    }
    let op_ids = vec![atomic.op.op_id.clone()];
    Ok(VerbResult::ok_with_ops(
        json!({
            "ok": true,
            "status": if atomic.already_applied { "already_applied" } else { "passed" },
            "schema": "shellx-cut/motion-import-apply@1",
            "dryRun": false,
            "wouldMutate": true,
            "planPath": plan.plan_path,
            "packageDir": plan.package_dir,
            "packageId": plan.package_id,
            "motionId": plan.motion_id,
            "targetId": plan.target_id,
            "mode": plan.mode,
            "integration": plan.integration,
            "idempotencyKey": plan.plan_hash,
            "alreadyApplied": atomic.already_applied,
            "checkpoint": atomic.checkpoint,
            "lineageProofs": motion_import_lineage_proofs(plan),
            "planned": planned_cut_steps(plan),
            "import": import_results.first().cloned().unwrap_or(Value::Null),
            "insert": insert_results.first().cloned().unwrap_or(Value::Null),
            "imports": import_results,
            "inserts": insert_results,
            "op_ids": op_ids,
            "clips": atomic.clip_ids,
            "assets": atomic.asset_ids,
            "warnings": plan.warnings,
            "rollback": {
                "required": false,
                "status": "not_needed",
                "committedOps": 1,
            },
            "restore_hint": format!("project.undo removes the whole plan; project.revert{{to:\"{}\"}} restores its pre-plan checkpoint", atomic.checkpoint.id),
        }),
        op_ids,
    ))
}

fn motion_warnings(connector: &Value) -> Vec<String> {
    connector
        .get("warnings")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

fn template_value_arg(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

pub(crate) fn safe_fragment(value: &str) -> String {
    let mut out: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        "motion".to_string()
    } else {
        out.chars().take(96).collect()
    }
}

#[cfg(test)]
#[path = "motion_bridge/tests.rs"]
mod tests;
