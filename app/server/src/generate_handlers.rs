//! generate_handlers.rs - verb handlers for the native Generate workspace.
//!
//! Keep dispatcher glue thin: domain catalog/IR lives in generate.rs, while
//! public verb handlers live in this owning module.

use crate::dispatch::{
    build_shape_spec, build_title_spec, configured_adapter_python, dispatch_send, no_project,
    ShapeArgs, TitleArgs, ENV_ADAPTER_PYTHON,
};
use crate::generate;
use crate::state::AppState;
use cut_core::{error_codes, Actor, CutError, VerbResult};
use serde_json::{json, Map, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

fn parse_args<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, CutError> {
    serde_json::from_value(args).map_err(|e| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "verb args did not match the schema",
            e.to_string(),
        )
        .with_suggested_action("GET /api/verbs shows the args schema for every verb")
    })
}

fn generate_not_found(id: &str) -> CutError {
    CutError::new(
        error_codes::NOT_FOUND,
        format!("no generate template '{id}'"),
        "template ids come from generate.list",
    )
}

fn validate_generate_filter(field: &str, value: &str, allowed: &[&str]) -> Result<(), CutError> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("invalid {field} '{value}'"),
            format!("allowed {field} values: {}", allowed.join(", ")),
        ))
    }
}

/// One catalog-summary JSON per template. `motion_ok` is the ONE-per-call
/// `motion_bridge::motion_available()` probe: motion-kind templates render
/// through the separate ShellX Motion CLI, so on a machine without it they
/// carry `available:false` — the UI can grey them out and the prompt/
/// storyboard planner adapters are instructed to never choose them (a plan
/// that cannot preview/insert is a confusing failure, not honesty).
fn generate_summary_json(t: &generate::GenerateTemplate, motion_ok: bool) -> Value {
    json!({
        "id": t.id,
        "source": t.source,
        "kind": t.kind,
        "title": t.title,
        "summary": t.summary,
        "tags": t.tags,
        "params": t.params,
        "capabilities": t.capabilities,
        "available": t.kind != "motion" || motion_ok,
    })
}

fn generate_template_json(t: &generate::GenerateTemplate) -> Value {
    json!({
        "id": t.id,
        "source": t.source,
        "kind": t.kind,
        "title": t.title,
        "summary": t.summary,
        "tags": t.tags,
        "params": t.params,
        "defaults": t.defaults,
        "capabilities": t.capabilities,
        "lowering": t.lowering,
        "verification": t.verification,
    })
}

pub(crate) fn validate_preview_dimension(
    label: &str,
    value: u32,
    max: u32,
) -> Result<u32, CutError> {
    if (64..=max).contains(&value) {
        Ok(value)
    } else {
        Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("generate.preview {label} must be between 64 and {max}"),
            format!("got {value}"),
        ))
    }
}

pub(crate) fn generate_safe_id_fragment(id: &str) -> String {
    let mut safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    while safe.contains("__") {
        safe = safe.replace("__", "_");
    }
    safe = safe.trim_matches('_').to_string();
    if safe.is_empty() {
        safe = "template".to_string();
    }
    safe.truncate(96);
    safe
}

pub(crate) fn generate_preview_id(
    id: &str,
    args: &Value,
    width: u32,
    height: u32,
    frame_ms: u64,
) -> String {
    let safe = generate_safe_id_fragment(id);
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    args.to_string().hash(&mut hasher);
    width.hash(&mut hasher);
    height.hash(&mut hasher);
    frame_ms.hash(&mut hasher);
    format!("generate_preview_{safe}_{:016x}", hasher.finish())
}

pub(crate) fn generate_instance_id(id: &str, lowering_args: &Value, op_ids: &[String]) -> String {
    let safe = generate_safe_id_fragment(id);
    let mut hasher = DefaultHasher::new();
    id.hash(&mut hasher);
    lowering_args.to_string().hash(&mut hasher);
    op_ids.hash(&mut hasher);
    format!("gen_{safe}_{:016x}", hasher.finish())
}

fn push_unique_string(out: &mut Vec<String>, value: Option<&Value>) {
    match value {
        Some(Value::String(s)) if !s.is_empty() && !out.contains(s) => out.push(s.clone()),
        Some(Value::Array(values)) => {
            for value in values {
                push_unique_string(out, Some(value));
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_generated_refs(result: &Value) -> (Vec<String>, Vec<String>) {
    let mut clips = Vec::new();
    let mut assets = Vec::new();
    for key in ["clip_id", "clip", "clips"] {
        push_unique_string(&mut clips, result.get(key));
    }
    for key in ["asset_id", "asset", "assets"] {
        push_unique_string(&mut assets, result.get(key));
    }
    if let Some(placed) = result.get("placed").and_then(|v| v.as_array()) {
        for item in placed {
            push_unique_string(&mut clips, item.get("clip_id"));
            push_unique_string(&mut assets, item.get("asset_id"));
        }
    }
    (clips, assets)
}

/// generate.list{} - discovery. Pure read (no project, no op, no checkpoint).
pub(crate) async fn generate_list(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize, Default)]
    struct Args {
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        query: Option<String>,
    }

    let a: Args = parse_args(args)?;
    let kind = a.kind.as_deref().unwrap_or("all");
    let source = a.source.as_deref().unwrap_or("all");
    validate_generate_filter(
        "kind",
        kind,
        &[
            "title", "caption", "shape", "motion", "social", "batch", "all",
        ],
    )?;
    validate_generate_filter("source", source, &["builtin", "project", "user", "all"])?;
    let query = a.query.unwrap_or_default().trim().to_lowercase();
    let motion_ok = crate::motion_bridge::motion_available();
    let templates: Vec<Value> = generate::registry()
        .templates
        .iter()
        .filter(|t| kind == "all" || t.kind == kind)
        .filter(|t| source == "all" || t.source == source)
        .filter(|t| {
            query.is_empty()
                || t.id.to_lowercase().contains(&query)
                || t.title.to_lowercase().contains(&query)
                || t.summary.to_lowercase().contains(&query)
                || t.tags.iter().any(|tag| tag.to_lowercase().contains(&query))
        })
        .map(|t| generate_summary_json(t, motion_ok))
        .collect();
    Ok(VerbResult::ok(json!({ "templates": templates })))
}

/// generate.describe{id} - full manifest for inspection before preview/insert.
/// Pure read; `NOT_FOUND` for an unknown id.
pub(crate) async fn generate_describe(args: Value) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
    }
    let a: Args = parse_args(args)?;
    let t = generate::registry()
        .get(&a.id)
        .ok_or_else(|| generate_not_found(&a.id))?;
    Ok(VerbResult::ok(generate_template_json(t)))
}

fn schema_accepts_arg(state: &AppState, verb: &str, key: &str) -> bool {
    state
        .registry
        .get(verb)
        .and_then(|spec| spec.args.get("properties"))
        .and_then(|props| props.as_object())
        .map(|props| props.contains_key(key))
        .unwrap_or(false)
}

fn insert_lowering_arg_if_accepted(
    state: &AppState,
    verb: &str,
    args: &mut Value,
    key: &str,
    value: Option<Value>,
) {
    if value.is_none() || !schema_accepts_arg(state, verb, key) {
        return;
    }
    if let Some(obj) = args.as_object_mut() {
        obj.insert(key.to_string(), value.unwrap());
    }
}

fn is_motion_generate_lowering(verb: &str) -> bool {
    matches!(verb, "motion.template_to_cut" | "motion.script_to_cut")
}

/// generate.preview{id, params?, width?, height?, frame_ms?} — render one
/// non-mutating PNG evidence frame for title/shape-backed Generate templates.
/// No project is required; when a project is open the preview is served through
/// the existing `/frames/{file}` route.
pub(crate) async fn generate_preview(
    state: &AppState,
    args: Value,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
        #[serde(default)]
        params: Map<String, Value>,
        width: Option<u32>,
        height: Option<u32>,
        frame_ms: Option<u64>,
    }

    let a: Args = parse_args(args)?;
    if a.width.is_some() != a.height.is_some() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "generate.preview width and height must be supplied together",
            "pass both width and height, or omit both to use project/default geometry",
        ));
    }

    let project_info = {
        let guard = state.project.read().await;
        guard.as_ref().map(|store| {
            let s = &store.project.settings;
            (store.dir.clone(), s.width, s.height, s.fps)
        })
    };
    let (width, height, fps) = match (a.width, a.height) {
        (Some(w), Some(h)) => (
            validate_preview_dimension("width", w, 7_680)?,
            validate_preview_dimension("height", h, 4_320)?,
            project_info
                .as_ref()
                .map(|(_, _, _, fps)| *fps)
                .unwrap_or(30.0)
                .max(1.0),
        ),
        _ => project_info
            .as_ref()
            .map(|(_, w, h, fps)| ((*w).max(64), (*h).max(64), (*fps).max(1.0)))
            .unwrap_or((1_280, 720, 30.0)),
    };

    let template = generate::registry()
        .get(&a.id)
        .ok_or_else(|| generate_not_found(&a.id))?;
    let resolved = generate::resolve_params(template, &a.params)?;
    let duration_ms = generate::resolve_duration_ms(template, &resolved);
    let frame_ms = a
        .frame_ms
        .unwrap_or(duration_ms.saturating_mul(6) / 10)
        .min(duration_ms.saturating_sub(1));
    let lowering_args = generate::interpolate_args(template, &resolved, [0, duration_ms])?;

    if is_motion_generate_lowering(&template.lowering.verb) {
        return generate_motion_preview(
            state,
            project_info.as_ref().map(|(dir, _, _, _)| dir.as_path()),
            template,
            resolved,
            lowering_args,
            width,
            height,
            frame_ms,
        )
        .await;
    }

    let spec = match template.lowering.verb.as_str() {
        "title.add" => {
            let title_args: TitleArgs =
                serde_json::from_value(lowering_args.clone()).map_err(|e| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        "resolved generate title args did not parse",
                        e.to_string(),
                    )
                })?;
            build_title_spec(&title_args, width, height, fps)?
        }
        "edit.add_shape" => {
            let shape_args: ShapeArgs =
                serde_json::from_value(lowering_args.clone()).map_err(|e| {
                    CutError::new(
                        error_codes::INVALID_ARGS,
                        "resolved generate shape args did not parse",
                        e.to_string(),
                    )
                })?;
            build_shape_spec(&shape_args, width, height, fps)?
        }
        other => {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!(
                    "generate.preview does not support template '{}'",
                    template.id
                ),
                format!(
                    "template lowers to {other}; preview supports title.add and edit.add_shape only"
                ),
            ))
        }
    };

    let frame_idx = ((frame_ms as f64 / 1_000.0) * fps).round().max(0.0) as u32;
    let preview_id = generate_preview_id(&template.id, &lowering_args, width, height, frame_ms);
    let file_name = format!("{preview_id}.png");
    let (out_dir, url) = if let Some((project_dir, _, _, _)) = project_info {
        (
            project_dir.join("frames"),
            Some(format!("/frames/{file_name}")),
        )
    } else {
        (
            std::env::temp_dir()
                .join("shellx-cut")
                .join("generate-preview"),
            None,
        )
    };
    std::fs::create_dir_all(&out_dir).map_err(|e| {
        CutError::new(
            error_codes::IO,
            "create generate preview output dir",
            e.to_string(),
        )
    })?;
    let path = out_dir.join(&file_name);
    let spec_for_render = spec.clone();
    let path_for_render = path.clone();
    tokio::task::spawn_blocking(move || {
        cut_media::title::render_frame_png(&spec_for_render, frame_idx, &path_for_render)
    })
    .await
    .map_err(|e| {
        CutError::new(
            error_codes::FFMPEG,
            "generate preview render task panicked",
            e.to_string(),
        )
    })?
    .map_err(|e| {
        CutError::new(
            error_codes::FFMPEG,
            "generate preview frame render failed",
            e.to_string(),
        )
    })?;

    let mut result = json!({
        "id": template.id,
        "preview_id": preview_id,
        "path": path,
        "mime": "image/png",
        "width": width,
        "height": height,
        "frame_ms": frame_ms,
        "params": resolved,
        "lowering": {
            "verb": template.lowering.verb,
            "args": lowering_args,
        },
        "supported": true,
        "warnings": [],
    });
    if let Some(url) = url {
        result["url"] = json!(url);
    }
    Ok(VerbResult::ok(result))
}

async fn generate_motion_preview(
    state: &AppState,
    project_dir: Option<&Path>,
    template: &generate::GenerateTemplate,
    resolved: std::collections::BTreeMap<String, Value>,
    mut lowering_args: Value,
    width: u32,
    height: u32,
    frame_ms: u64,
) -> Result<VerbResult, CutError> {
    if !lowering_args.is_object() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "generate template '{}' lowered to non-object args",
                template.id
            ),
            "Motion Generate templates require object args",
        ));
    }
    let preview_id = generate_preview_id(&template.id, &lowering_args, width, height, frame_ms);
    let out_dir = project_dir
        .map(|dir| {
            dir.join("motion")
                .join("generate-preview")
                .join(&preview_id)
        })
        .unwrap_or_else(|| {
            std::env::temp_dir()
                .join("shellx-cut")
                .join("motion")
                .join("generate-preview")
                .join(&preview_id)
        });
    if let Some(obj) = lowering_args.as_object_mut() {
        obj.insert("policy".to_string(), json!("preview"));
        obj.insert("out_dir".to_string(), json!(out_dir));
    }
    let motion = dispatch_send(
        state,
        template.lowering.verb.as_str(),
        lowering_args.clone(),
        Actor::system(),
    )
    .await;
    if !motion.ok {
        if let Some(err) = motion.error {
            return Err(err);
        }
        return Ok(motion);
    }
    let motion_result = motion.result.clone().unwrap_or(Value::Null);
    let source_path = motion_result
        .get("preview")
        .and_then(|v| v.get("outputPath"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            motion_result
                .get("connector")
                .and_then(|v| v.get("preview"))
                .and_then(|v| v.get("outputPath"))
                .and_then(|v| v.as_str())
        })
        .ok_or_else(|| {
            CutError::new(
                error_codes::SIDECAR,
                "Motion preview returned no PNG path",
                "preview.outputPath is required for generate.preview",
            )
        })?;
    let file_name = format!("{preview_id}.png");
    let (path, url) = if let Some(project_dir) = project_dir {
        let frames_dir = project_dir.join("frames");
        std::fs::create_dir_all(&frames_dir).map_err(|e| {
            CutError::new(
                error_codes::IO,
                "create Motion generate preview frames dir",
                e.to_string(),
            )
        })?;
        let target = frames_dir.join(&file_name);
        if Path::new(source_path) != target {
            std::fs::copy(source_path, &target).map_err(|e| {
                CutError::new(
                    error_codes::IO,
                    "copy Motion preview into served frames dir",
                    e.to_string(),
                )
            })?;
        }
        (target, Some(format!("/frames/{file_name}")))
    } else {
        (PathBuf::from(source_path), None)
    };
    let warnings = motion_result
        .get("warnings")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    let mut result = json!({
        "id": template.id,
        "preview_id": preview_id,
        "path": path,
        "mime": "image/png",
        "width": width,
        "height": height,
        "frame_ms": frame_ms,
        "params": resolved,
        "lowering": {
            "verb": template.lowering.verb,
            "args": lowering_args,
        },
        "supported": true,
        "warnings": warnings,
        "motion": motion_result,
    });
    if let Some(url) = url {
        result["url"] = json!(url);
    }
    Ok(VerbResult::ok(result))
}

/// generate.insert{id, params?, at_ms?, track?, rationale?} — materialize a
/// Generate template through the normal native verb path. The Generate layer
/// records no custom op; replay remains owned by the checkpoint and lowered
/// native verbs (`title.add`, `edit.add_shape`, `captions.kinetic`).
pub(crate) async fn generate_insert(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        id: String,
        #[serde(default)]
        params: Map<String, Value>,
        #[serde(default)]
        at_ms: Option<u64>,
        #[serde(default)]
        track: Option<String>,
        #[serde(default)]
        rationale: Option<String>,
    }

    let a: Args = parse_args(args)?;
    let template = generate::registry()
        .get(&a.id)
        .ok_or_else(|| generate_not_found(&a.id))?;
    let resolved = generate::resolve_params(template, &a.params)?;
    let at_ms = a.at_ms.unwrap_or(0);
    let duration_ms = generate::resolve_duration_ms(template, &resolved);
    let mut lowering_args = generate::interpolate_args(
        template,
        &resolved,
        [at_ms, at_ms.saturating_add(duration_ms)],
    )?;
    if !lowering_args.is_object() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!(
                "generate template '{}' lowered to non-object args",
                template.id
            ),
            "native verbs require object args",
        ));
    }
    let lowering_verb = template.lowering.verb.as_str();
    if !matches!(
        lowering_verb,
        "title.add"
            | "edit.add_shape"
            | "captions.kinetic"
            | "motion.template_to_cut"
            | "motion.script_to_cut"
    ) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("generate.insert does not support template '{}'", template.id),
            format!(
                "template lowers to {lowering_verb}; Generate insert supports title.add, edit.add_shape, captions.kinetic, motion.template_to_cut, and motion.script_to_cut"
            ),
        ));
    }

    {
        let guard = state.project.read().await;
        guard.as_ref().ok_or_else(no_project)?;
    }

    let rationale = a
        .rationale
        .clone()
        .unwrap_or_else(|| format!("generate.insert {}", template.id));
    insert_lowering_arg_if_accepted(
        state,
        lowering_verb,
        &mut lowering_args,
        "rationale",
        Some(json!(rationale.clone())),
    );
    insert_lowering_arg_if_accepted(
        state,
        lowering_verb,
        &mut lowering_args,
        "track",
        a.track.clone().map(Value::String),
    );
    if is_motion_generate_lowering(lowering_verb) {
        insert_lowering_arg_if_accepted(
            state,
            lowering_verb,
            &mut lowering_args,
            "policy",
            Some(json!("insert")),
        );
        insert_lowering_arg_if_accepted(
            state,
            lowering_verb,
            &mut lowering_args,
            "checkpoint",
            Some(json!(false)),
        );
    }

    let checkpoint_name = format!("generate-{}-start", generate_safe_id_fragment(&template.id));
    let cp = dispatch_send(
        state,
        "project.checkpoint",
        json!({"name": checkpoint_name, "rationale": rationale}),
        actor.clone(),
    )
    .await;
    if !cp.ok {
        return Ok(cp);
    }
    let mut op_ids = cp.op_ids.clone().unwrap_or_default();
    let checkpoint = cp
        .result
        .as_ref()
        .and_then(|r| r.get("checkpoint"))
        .cloned()
        .unwrap_or(Value::Null);
    let checkpoint_id = checkpoint
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let native = dispatch_send(state, lowering_verb, lowering_args.clone(), actor.clone()).await;
    if !native.ok {
        if let Some(err) = native.error {
            let mut err = err;
            if !checkpoint_id.is_empty() && err.suggested_action.is_none() {
                err.suggested_action = Some(format!(
                    "project.revert{{to:\"{checkpoint_id}\"}} removes the checkpoint from this failed generate.insert attempt"
                ));
            }
            return Err(err);
        }
        return Ok(native);
    }
    op_ids.extend(native.op_ids.clone().unwrap_or_default());
    let native_result = native.result.clone().unwrap_or(Value::Null);
    let (clips, assets) = collect_generated_refs(&native_result);
    let instance_id = generate_instance_id(&template.id, &lowering_args, &op_ids);
    let restore_hint = if checkpoint_id.is_empty() {
        "project.undo reverts the generated native op".to_string()
    } else {
        format!("project.revert{{to:\"{checkpoint_id}\"}} undoes this generated insert")
    };

    Ok(VerbResult::ok_with_ops(
        json!({
            "id": template.id,
            "instance_id": instance_id,
            "checkpoint": checkpoint,
            "op_ids": op_ids.clone(),
            "clips": clips,
            "assets": assets,
            "params": resolved,
            "lowering": {
                "verb": template.lowering.verb,
                "args": lowering_args,
            },
            "result": native_result,
            "restore_hint": restore_hint,
        }),
        op_ids,
    ))
}

const GENERATE_PROMPT_ADAPTER_REL: &str = "app/perception/py/generate_prompt_adapter.py";
const GENERATE_PROMPT_TIMEOUT_MS_DEFAULT: u64 = 120_000;

/// Resolved local CLI agent paths ({"claude": path|null, ...}) handed to the
/// Generate planner adapters in their request JSON. Resolution stays in ONE
/// place (gen.rs ladder: PATH first, then known install dirs) so the Python
/// shims never guess platform layouts; a null for every agent lets the shim
/// return an honest not_run.
fn generate_agent_paths() -> Value {
    let mut map = Map::new();
    for name in ["claude", "codex", "grok"] {
        let path = crate::gen::resolve_agent(name)
            .map(|p| Value::String(p.to_string_lossy().into_owned()))
            .unwrap_or(Value::Null);
        map.insert(name.to_string(), path);
    }
    Value::Object(map)
}

fn find_generate_prompt_adapter() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CUTD_GENERATE_PROMPT_ADAPTER") {
        if !p.is_empty() {
            let pb = PathBuf::from(&p);
            return pb.is_file().then_some(pb);
        }
    }
    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        starts.push(exe);
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    for start in starts {
        for dir in start.ancestors().take(6) {
            let cand = dir.join(GENERATE_PROMPT_ADAPTER_REL);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    // Installed layout: the shim ships in the perception sidecar payload
    // (beside instruments.py) because that dir already travels with cutd on
    // every platform — on macOS the bundle resources live under
    // Contents/Resources, which the exe-ancestor walk above cannot reach.
    let (_py, instruments) = cut_perception::sidecar_paths();
    let cand = instruments.parent()?.join("generate_prompt_adapter.py");
    cand.is_file().then_some(cand)
}

async fn run_generate_prompt_adapter(
    adapter: Option<&Path>,
    context: &Value,
    timeout_ms: u64,
) -> Value {
    let Some(adapter) = adapter else {
        return json!({
            "schema": "shellx-cut/generate-plan/1",
            "status": "not_run",
            "reason": format!("generate prompt adapter not found ({GENERATE_PROMPT_ADAPTER_REL} near the cutd binary/cwd; override: CUTD_GENERATE_PROMPT_ADAPTER) - honest not_run, no Generate prompt backend"),
            "plan": Value::Null,
            "warnings": [],
        });
    };

    let Some(python) = configured_adapter_python() else {
        return json!({
            "schema": "shellx-cut/generate-plan/1",
            "status": "not_run",
            "reason": format!("no adapter Python configured (set {ENV_ADAPTER_PYTHON} or install the ShellX Cut perception runtime) - honest not_run, no Generate prompt backend"),
            "plan": Value::Null,
            "warnings": [],
        });
    };

    let input = context.to_string();
    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(adapter)
        .arg("plan")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return json!({
                "schema": "shellx-cut/generate-plan/1",
                "status": "error",
                "reason": format!("generate prompt adapter spawn failed: {e}"),
                "plan": Value::Null,
                "warnings": [],
            })
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        if let Err(e) = stdin.write_all(input.as_bytes()).await {
            let _ = child.kill().await;
            return json!({
                "schema": "shellx-cut/generate-plan/1",
                "status": "error",
                "reason": format!("generate prompt adapter stdin write failed: {e}"),
                "plan": Value::Null,
                "warnings": [],
            });
        }
        if let Err(e) = stdin.shutdown().await {
            let _ = child.kill().await;
            return json!({
                "schema": "shellx-cut/generate-plan/1",
                "status": "error",
                "reason": format!("generate prompt adapter stdin close failed: {e}"),
                "plan": Value::Null,
                "warnings": [],
            });
        }
    } else {
        let _ = child.kill().await;
        return json!({
            "schema": "shellx-cut/generate-plan/1",
            "status": "error",
            "reason": "generate prompt adapter stdin pipe was unavailable",
            "plan": Value::Null,
            "warnings": [],
        });
    }
    let out = match tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return json!({
                "schema": "shellx-cut/generate-plan/1",
                "status": "error",
                "reason": format!("generate prompt adapter io error: {e}"),
                "plan": Value::Null,
                "warnings": [],
            })
        }
        Err(_) => {
            return json!({
                "schema": "shellx-cut/generate-plan/1",
                "status": "error",
                "reason": format!("generate prompt adapter exceeded {timeout_ms}ms timeout"),
                "plan": Value::Null,
                "warnings": [],
            })
        }
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: String = err
            .chars()
            .rev()
            .take(600)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return json!({
            "schema": "shellx-cut/generate-plan/1",
            "status": "error",
            "reason": format!("generate prompt adapter exit {:?}: {tail}", out.status.code()),
            "plan": Value::Null,
            "warnings": [],
        });
    }
    match serde_json::from_slice::<Value>(&out.stdout) {
        Ok(v) => v,
        Err(e) => {
            let s = String::from_utf8_lossy(&out.stdout);
            let head: String = s.chars().take(300).collect();
            json!({
                "schema": "shellx-cut/generate-plan/1",
                "status": "error",
                "reason": format!("generate prompt adapter emitted non-JSON ({e}): {head}"),
                "plan": Value::Null,
                "warnings": [],
            })
        }
    }
}

fn generate_prompt_warnings(envelope: &Value) -> Vec<String> {
    envelope
        .get("warnings")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(String::from))
        .collect()
}

fn generate_prompt_result(
    status: &str,
    request: Value,
    backend: Value,
    plan: Value,
    validation: Value,
    reason: Value,
    warnings: Vec<String>,
    next_actions: Vec<&str>,
) -> Value {
    json!({
        "status": status,
        "request": request,
        "backend": backend,
        "plan": plan,
        "validation": validation,
        "preview": Value::Null,
        "insert": Value::Null,
        "reason": reason,
        "warnings": warnings,
        "next_actions": next_actions,
    })
}

fn validate_generate_prompt_plan(plan: &Value, template_hint: Option<&str>) -> Value {
    let mut errors: Vec<String> = Vec::new();
    let Some(obj) = plan.as_object() else {
        return json!({
            "ok": false,
            "template_id": Value::Null,
            "resolved_params": Value::Null,
            "errors": ["plan must be an object"],
        });
    };
    let template_id = obj
        .get("template_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if template_id.is_empty() {
        errors.push("plan.template_id is required".to_string());
    }
    if let Some(hint) = template_hint {
        if !hint.trim().is_empty() && hint != template_id {
            errors.push(format!(
                "plan.template_id '{template_id}' does not match requested template_id '{hint}'"
            ));
        }
    }
    let template = if template_id.is_empty() {
        None
    } else {
        generate::registry().get(template_id)
    };
    if template.is_none() && !template_id.is_empty() {
        errors.push(format!("unknown generate template '{template_id}'"));
    }
    if obj.get("params").is_some_and(|v| !v.is_object()) {
        errors.push("plan.params must be an object".to_string());
    }
    if obj
        .get("at_ms")
        .is_some_and(|v| !v.is_u64() && !v.is_null())
    {
        errors.push("plan.at_ms must be a non-negative integer".to_string());
    }

    let mut resolved_params = Value::Null;
    if let Some(template) = template {
        let empty = Map::new();
        let params = obj
            .get("params")
            .and_then(|v| v.as_object())
            .unwrap_or(&empty);
        match generate::resolve_params(template, params) {
            Ok(resolved) => {
                resolved_params = json!(resolved);
            }
            Err(e) => {
                errors.push(e.message);
            }
        }
    }
    json!({
        "ok": errors.is_empty(),
        "template_id": if template_id.is_empty() { Value::Null } else { json!(template_id) },
        "resolved_params": resolved_params,
        "errors": errors,
    })
}

/// generate.from_prompt{prompt, policy?} — ask the configured Generate prompt
/// adapter for a typed template plan, then validate it against the local
/// catalog. The adapter is optional and honest: no backend means status:not_run,
/// never a fabricated plan.
pub(crate) async fn generate_from_prompt(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        prompt: String,
        #[serde(default)]
        policy: Option<String>,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        template_id: Option<String>,
        #[serde(default)]
        at_ms: Option<u64>,
        #[serde(default)]
        width: Option<u32>,
        #[serde(default)]
        height: Option<u32>,
        #[serde(default)]
        timeout_ms: Option<u64>,
        #[serde(default)]
        context: Option<Value>,
        #[serde(default)]
        rationale: Option<String>,
    }

    let a: Args = parse_args(args)?;
    let prompt = a.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "generate.from_prompt prompt is required",
            "pass a non-empty prompt",
        ));
    }
    if a.width.is_some() != a.height.is_some() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "generate.from_prompt width and height must be supplied together",
            "pass both width and height, or omit both to use project/default geometry",
        ));
    }
    let policy = a.policy.unwrap_or_else(|| "plan".to_string());
    if !["plan", "preview", "insert"].contains(&policy.as_str()) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("invalid generate.from_prompt policy '{policy}'"),
            "allowed policy values: plan, preview, insert",
        ));
    }
    let agent = a.agent.unwrap_or_else(|| "auto".to_string());
    if !["auto", "claude", "codex", "grok"].contains(&agent.as_str()) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("invalid generate.from_prompt agent '{agent}'"),
            "allowed agent values: auto, claude, codex, grok",
        ));
    }

    let project_geometry = {
        let guard = state.project.read().await;
        guard.as_ref().map(|store| {
            (
                store.project.settings.width,
                store.project.settings.height,
                store.project.settings.fps,
            )
        })
    };
    let width = a
        .width
        .or_else(|| project_geometry.as_ref().map(|(w, _, _)| *w))
        .unwrap_or(1_280);
    let height = a
        .height
        .or_else(|| project_geometry.as_ref().map(|(_, h, _)| *h))
        .unwrap_or(720);
    let timeout_ms = a
        .timeout_ms
        .unwrap_or(GENERATE_PROMPT_TIMEOUT_MS_DEFAULT)
        .clamp(1_000, 300_000);
    let motion_ok = crate::motion_bridge::motion_available();
    let request = json!({
        "schema": "shellx-cut/generate-prompt-request/1",
        "prompt": prompt,
        "policy": policy,
        "agent": agent,
        "agents": generate_agent_paths(),
        "timeout_ms": timeout_ms,
        "template_id": a.template_id,
        "at_ms": a.at_ms,
        "geometry": {"width": width, "height": height},
        "templates": generate::registry()
            .templates
            .iter()
            .map(|t| generate_summary_json(t, motion_ok))
            .collect::<Vec<_>>(),
        "context": a.context.unwrap_or_else(|| json!({})),
        "rationale": a.rationale,
    });
    let adapter = find_generate_prompt_adapter();
    let envelope = run_generate_prompt_adapter(adapter.as_deref(), &request, timeout_ms).await;
    let status = envelope
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("error");
    let backend = envelope.get("backend").cloned().unwrap_or(Value::Null);
    let warnings = generate_prompt_warnings(&envelope);
    if status != "completed" {
        return Ok(VerbResult::ok(generate_prompt_result(
            status,
            request,
            backend,
            Value::Null,
            Value::Null,
            envelope.get("reason").cloned().unwrap_or(Value::Null),
            warnings,
            vec![
                "configure CUTD_GENERATE_PROMPT_ADAPTER",
                "retry generate.from_prompt",
            ],
        )));
    }
    if envelope.get("schema").and_then(|v| v.as_str()) != Some("shellx-cut/generate-plan/1") {
        return Ok(VerbResult::ok(generate_prompt_result(
            "error",
            request,
            backend,
            Value::Null,
            Value::Null,
            json!("generate prompt adapter returned an invalid schema"),
            warnings,
            vec![
                "fix the Generate prompt adapter schema",
                "retry generate.from_prompt",
            ],
        )));
    }

    let raw_plan = envelope.get("plan").cloned().unwrap_or(Value::Null);
    let validation = validate_generate_prompt_plan(
        &raw_plan,
        request.get("template_id").and_then(|v| v.as_str()),
    );
    if !validation["ok"].as_bool().unwrap_or(false) {
        return Ok(VerbResult::ok(generate_prompt_result(
            "error",
            request,
            backend,
            raw_plan,
            validation,
            json!("generate prompt plan failed validation"),
            warnings,
            vec![
                "adjust prompt or template hint",
                "retry generate.from_prompt",
            ],
        )));
    }

    let mut plan = raw_plan;
    if let Some(obj) = plan.as_object_mut() {
        obj.insert("params".to_string(), validation["resolved_params"].clone());
        if let Some(at_ms) = request.get("at_ms").and_then(|v| v.as_u64()) {
            obj.insert("at_ms".to_string(), json!(at_ms));
        }
    }

    let template_id = validation["template_id"].as_str().unwrap_or("").to_string();
    let params = validation["resolved_params"].clone();
    let at_ms = request
        .get("at_ms")
        .and_then(|v| v.as_u64())
        .or_else(|| plan.get("at_ms").and_then(|v| v.as_u64()))
        .unwrap_or(0);

    match policy.as_str() {
        "preview" => {
            let preview = dispatch_send(
                state,
                "generate.preview",
                json!({
                    "id": template_id,
                    "params": params,
                    "width": width,
                    "height": height,
                    "frame_ms": at_ms,
                }),
                actor,
            )
            .await;
            if !preview.ok {
                return Ok(VerbResult::ok(generate_prompt_result(
                    "error",
                    request,
                    backend,
                    plan,
                    validation,
                    preview
                        .error
                        .map(|e| json!(e.message))
                        .unwrap_or_else(|| json!("generate.preview failed")),
                    warnings,
                    vec!["adjust prompt or template", "retry policy:preview"],
                )));
            }
            let mut result = generate_prompt_result(
                "completed",
                request,
                backend,
                plan,
                validation,
                Value::Null,
                warnings,
                vec!["review preview", "run policy:insert"],
            );
            result["preview"] = preview.result.unwrap_or(Value::Null);
            return Ok(VerbResult::ok(result));
        }
        "insert" => {
            let rationale = request
                .get("rationale")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(String::from)
                .or_else(|| {
                    plan.get("rationale")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(String::from)
                })
                .unwrap_or_else(|| format!("generate.from_prompt {template_id}"));
            let insert = dispatch_send(
                state,
                "generate.insert",
                json!({
                    "id": template_id,
                    "params": params,
                    "at_ms": at_ms,
                    "rationale": rationale,
                }),
                actor,
            )
            .await;
            if !insert.ok {
                return Ok(VerbResult::ok(generate_prompt_result(
                    "error",
                    request,
                    backend,
                    plan,
                    validation,
                    insert
                        .error
                        .map(|e| json!(e.message))
                        .unwrap_or_else(|| json!("generate.insert failed")),
                    warnings,
                    vec!["open a project or adjust prompt", "retry policy:insert"],
                )));
            }
            let op_ids = insert.op_ids.clone().unwrap_or_default();
            let mut result = generate_prompt_result(
                "completed",
                request,
                backend,
                plan,
                validation,
                Value::Null,
                warnings,
                vec!["review generated timeline instance"],
            );
            result["insert"] = insert.result.unwrap_or(Value::Null);
            return Ok(VerbResult::ok_with_ops(result, op_ids));
        }
        _ => {}
    }

    Ok(VerbResult::ok(generate_prompt_result(
        "completed",
        request,
        backend,
        plan,
        validation,
        Value::Null,
        warnings,
        vec!["review plan", "run policy:preview", "run policy:insert"],
    )))
}

const GENERATE_STORYBOARD_ADAPTER_REL: &str = "app/perception/py/generate_storyboard_adapter.py";
const GENERATE_STORYBOARD_TIMEOUT_MS_DEFAULT: u64 = 120_000;
const GENERATE_STORYBOARD_SKILL_PATH: [&str; 2] = [
    "skill/shellx-cut/craft/generate-director-questioning.md",
    "skill/shellx-cut/craft/generate-storyboard-planning.md",
];

fn find_generate_storyboard_adapter() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CUTD_GENERATE_STORYBOARD_ADAPTER") {
        if !p.is_empty() {
            let pb = PathBuf::from(&p);
            return pb.is_file().then_some(pb);
        }
    }
    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        starts.push(exe);
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    for start in starts {
        for dir in start.ancestors().take(6) {
            let cand = dir.join(GENERATE_STORYBOARD_ADAPTER_REL);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    // Installed layout: ships beside instruments.py (see the prompt-adapter
    // finder for why the sidecar payload is the cross-platform carrier).
    let (_py, instruments) = cut_perception::sidecar_paths();
    let cand = instruments.parent()?.join("generate_storyboard_adapter.py");
    cand.is_file().then_some(cand)
}

async fn run_generate_storyboard_adapter(
    adapter: Option<&Path>,
    context: &Value,
    timeout_ms: u64,
) -> Value {
    let Some(adapter) = adapter else {
        return json!({
            "schema": "shellx-cut/generate-storyboard-result/1",
            "status": "not_run",
            "reason": format!("generate storyboard adapter not found ({GENERATE_STORYBOARD_ADAPTER_REL} near the cutd binary/cwd; override: CUTD_GENERATE_STORYBOARD_ADAPTER) - honest not_run, no Generate storyboard backend"),
            "storyboard": Value::Null,
            "questions": [],
            "warnings": [],
        });
    };

    let Some(python) = configured_adapter_python() else {
        return json!({
            "schema": "shellx-cut/generate-storyboard-result/1",
            "status": "not_run",
            "reason": format!("no adapter Python configured (set {ENV_ADAPTER_PYTHON} or install the ShellX Cut perception runtime) - honest not_run, no Generate storyboard backend"),
            "storyboard": Value::Null,
            "questions": [],
            "warnings": [],
        });
    };

    let input = context.to_string();
    let mut cmd = tokio::process::Command::new(python);
    cmd.arg(adapter)
        .arg("plan")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return json!({
                "schema": "shellx-cut/generate-storyboard-result/1",
                "status": "error",
                "reason": format!("generate storyboard adapter spawn failed: {e}"),
                "storyboard": Value::Null,
                "questions": [],
                "warnings": [],
            })
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        if let Err(e) = stdin.write_all(input.as_bytes()).await {
            let _ = child.kill().await;
            return json!({
                "schema": "shellx-cut/generate-storyboard-result/1",
                "status": "error",
                "reason": format!("generate storyboard adapter stdin write failed: {e}"),
                "storyboard": Value::Null,
                "questions": [],
                "warnings": [],
            });
        }
        if let Err(e) = stdin.shutdown().await {
            let _ = child.kill().await;
            return json!({
                "schema": "shellx-cut/generate-storyboard-result/1",
                "status": "error",
                "reason": format!("generate storyboard adapter stdin close failed: {e}"),
                "storyboard": Value::Null,
                "questions": [],
                "warnings": [],
            });
        }
    } else {
        let _ = child.kill().await;
        return json!({
            "schema": "shellx-cut/generate-storyboard-result/1",
            "status": "error",
            "reason": "generate storyboard adapter stdin pipe was unavailable",
            "storyboard": Value::Null,
            "questions": [],
            "warnings": [],
        });
    }
    let out = match tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return json!({
                "schema": "shellx-cut/generate-storyboard-result/1",
                "status": "error",
                "reason": format!("generate storyboard adapter io error: {e}"),
                "storyboard": Value::Null,
                "questions": [],
                "warnings": [],
            })
        }
        Err(_) => {
            return json!({
                "schema": "shellx-cut/generate-storyboard-result/1",
                "status": "error",
                "reason": format!("generate storyboard adapter exceeded {timeout_ms}ms timeout"),
                "storyboard": Value::Null,
                "questions": [],
                "warnings": [],
            })
        }
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: String = err
            .chars()
            .rev()
            .take(600)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        return json!({
            "schema": "shellx-cut/generate-storyboard-result/1",
            "status": "error",
            "reason": format!("generate storyboard adapter exit {:?}: {tail}", out.status.code()),
            "storyboard": Value::Null,
            "questions": [],
            "warnings": [],
        });
    }
    match serde_json::from_slice::<Value>(&out.stdout) {
        Ok(v) => v,
        Err(e) => {
            let s = String::from_utf8_lossy(&out.stdout);
            let head: String = s.chars().take(300).collect();
            json!({
                "schema": "shellx-cut/generate-storyboard-result/1",
                "status": "error",
                "reason": format!("generate storyboard adapter emitted non-JSON ({e}): {head}"),
                "storyboard": Value::Null,
                "questions": [],
                "warnings": [],
            })
        }
    }
}

fn generate_storyboard_string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

fn generate_storyboard_brief_fields(storyboard: &Value) -> (Vec<String>, Vec<String>, Vec<String>) {
    let stated = generate_storyboard_string_array(
        storyboard.get("brief_meta").and_then(|v| v.get("stated")),
    );
    let inferred = generate_storyboard_string_array(
        storyboard.get("brief_meta").and_then(|v| v.get("inferred")),
    );
    let mut missing = generate_storyboard_string_array(
        storyboard.get("brief_meta").and_then(|v| v.get("missing")),
    );
    if missing.is_empty() {
        missing = generate_storyboard_string_array(
            storyboard
                .get("validation")
                .and_then(|v| v.get("missing_inputs")),
        );
    }
    (stated, inferred, missing)
}

fn generate_storyboard_evidence(
    scene_count: u64,
    duration_ms: u64,
    template_ids: Vec<String>,
    brief_fields: (Vec<String>, Vec<String>, Vec<String>),
) -> Value {
    json!({
        "policy": "plan",
        "mutated": false,
        "skill_path": GENERATE_STORYBOARD_SKILL_PATH,
        "scene_count": scene_count,
        "duration_ms": duration_ms,
        "template_ids": template_ids,
        "brief_fields": {
            "stated": brief_fields.0,
            "inferred": brief_fields.1,
            "missing": brief_fields.2,
        },
    })
}

fn generate_storyboard_empty_evidence() -> Value {
    generate_storyboard_evidence(0, 0, Vec::new(), (Vec::new(), Vec::new(), Vec::new()))
}

fn validate_generate_storyboard(storyboard: &Value) -> (Value, Value) {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut scene_count = 0u64;
    let mut duration_ms = 0u64;
    let mut template_ids: Vec<String> = Vec::new();
    let missing_inputs = generate_storyboard_string_array(
        storyboard
            .get("validation")
            .and_then(|v| v.get("missing_inputs")),
    );
    let brief_fields = generate_storyboard_brief_fields(storyboard);

    let Some(obj) = storyboard.as_object() else {
        let validation = json!({
            "ok": false,
            "result": "fail",
            "errors": ["storyboard must be an object"],
            "warnings": warnings,
            "missing_inputs": missing_inputs,
            "scene_count": scene_count,
            "duration_ms": duration_ms,
            "template_ids": template_ids,
        });
        return (
            validation,
            generate_storyboard_evidence(scene_count, duration_ms, template_ids, brief_fields),
        );
    };

    if obj.get("schema").and_then(|v| v.as_str()) != Some("shellx-cut/generate-storyboard/1") {
        errors.push("storyboard.schema must be shellx-cut/generate-storyboard/1".to_string());
    }

    let mode = obj
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if !["quick_prompt", "director_brief", "script", "existing_media"].contains(&mode) {
        errors.push(
            "storyboard.mode must be quick_prompt, director_brief, script, or existing_media"
                .to_string(),
        );
    }

    let status = obj
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if !["draft", "needs_input", "valid", "previewed", "inserted"].contains(&status) {
        errors.push(
            "storyboard.status must be draft, needs_input, valid, previewed, or inserted"
                .to_string(),
        );
    }

    match obj.get("scenes").and_then(|v| v.as_array()) {
        Some(scenes) => {
            scene_count = scenes.len() as u64;
            if scenes.is_empty() {
                warnings.push("storyboard.scenes is empty".to_string());
            }
            for (idx, scene) in scenes.iter().enumerate() {
                let label = format!("scene[{idx}]");
                let Some(scene_obj) = scene.as_object() else {
                    errors.push(format!("{label} must be an object"));
                    continue;
                };

                let scene_id = scene_obj
                    .get("scene_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if scene_id.is_empty() {
                    errors.push(format!("{label}.scene_id is required"));
                }

                match scene_obj.get("index").and_then(|v| v.as_u64()) {
                    Some(index) if index > 0 => {}
                    _ => errors.push(format!("{label}.index must be a positive integer")),
                }

                let role = scene_obj
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if role.is_empty() {
                    errors.push(format!("{label}.role is required"));
                }

                let source = scene_obj
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if ![
                    "generate_template",
                    "existing_media",
                    "assemble_slot",
                    "generated_asset",
                    "caption",
                    "audio",
                ]
                .contains(&source)
                {
                    errors.push(format!("{label}.source has unsupported value '{source}'"));
                }

                match scene_obj.get("range_ms").and_then(|v| v.as_array()) {
                    Some(range) if range.len() == 2 => {
                        let start = range[0].as_u64();
                        let end = range[1].as_u64();
                        match (start, end) {
                            (Some(start), Some(end)) if end > start => {
                                duration_ms = duration_ms.saturating_add(end - start);
                            }
                            _ => errors.push(format!(
                                "{label}.range_ms must be [start,end] with end > start"
                            )),
                        }
                    }
                    _ => errors.push(format!("{label}.range_ms must be a two-item array")),
                }

                match source {
                    "generate_template" => {
                        let template_id = scene_obj
                            .get("template_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        if template_id.is_empty() {
                            errors.push(format!(
                                "{label}.template_id is required for generate_template scenes"
                            ));
                        } else if generate::registry().get(template_id).is_none() {
                            errors.push(format!("unknown generate template '{template_id}'"));
                        } else if !template_ids.iter().any(|id| id == template_id) {
                            template_ids.push(template_id.to_string());
                        }
                    }
                    "assemble_slot" => {
                        let query = scene_obj
                            .get("query")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        if query.is_empty() {
                            errors.push(format!(
                                "{label}.query is required for assemble_slot scenes"
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        None => errors.push("storyboard.scenes must be an array".to_string()),
    }

    let result = if !errors.is_empty() {
        "fail"
    } else if !warnings.is_empty() || !missing_inputs.is_empty() {
        "warn"
    } else {
        "pass"
    };
    let ok = errors.is_empty();
    let validation = json!({
        "ok": ok,
        "result": result,
        "errors": errors,
        "warnings": warnings,
        "missing_inputs": missing_inputs,
        "scene_count": scene_count,
        "duration_ms": duration_ms,
        "template_ids": template_ids,
    });
    let evidence =
        generate_storyboard_evidence(scene_count, duration_ms, template_ids, brief_fields);
    (validation, evidence)
}

fn generate_storyboard_questions(envelope: &Value) -> (Value, Vec<String>) {
    let mut warnings = Vec::new();
    let mut questions = envelope
        .get("questions")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if questions.len() > 1 {
        warnings.push(
            "adapter returned multiple questions; ShellX Cut surfaces one question at a time"
                .to_string(),
        );
        questions.truncate(1);
    }
    (Value::Array(questions), warnings)
}

fn generate_storyboard_result(
    status: &str,
    request: Value,
    backend: Value,
    storyboard: Value,
    questions: Value,
    validation: Value,
    evidence: Value,
    reason: Value,
    warnings: Vec<String>,
    next_actions: Vec<&str>,
) -> Value {
    json!({
        "status": status,
        "request": request,
        "backend": backend,
        "storyboard": storyboard,
        "questions": questions,
        "validation": validation,
        "evidence": evidence,
        "preview": Value::Null,
        "insert": Value::Null,
        "reason": reason,
        "warnings": warnings,
        "next_actions": next_actions,
    })
}

fn generate_storyboard_next_actions(status: &str) -> Vec<&'static str> {
    match status {
        "not_run" => vec![
            "configure CUTD_GENERATE_STORYBOARD_ADAPTER",
            "retry generate.storyboard",
        ],
        "needs_input" => vec![
            "answer the director question",
            "retry generate.storyboard with answers",
        ],
        "completed" => vec![
            "review storyboard",
            "run policy:preview after generation completes",
        ],
        _ => vec![
            "adjust storyboard input or adapter output",
            "retry generate.storyboard",
        ],
    }
}

fn generate_storyboard_scene_id(scene: &Value, fallback_index: usize) -> String {
    scene
        .get("scene_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("scene-{:03}", fallback_index + 1))
}

fn generate_storyboard_scene_index(scene: &Value, fallback_index: usize) -> u64 {
    scene
        .get("index")
        .and_then(|v| v.as_u64())
        .filter(|i| *i > 0)
        .unwrap_or((fallback_index + 1) as u64)
}

fn generate_storyboard_scene_range(scene: &Value) -> Value {
    scene
        .get("range_ms")
        .cloned()
        .unwrap_or_else(|| json!([0, 0]))
}

fn generate_storyboard_scene_start_ms(scene: &Value) -> u64 {
    scene
        .get("range_ms")
        .and_then(|v| v.as_array())
        .and_then(|range| range.first())
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

fn generate_storyboard_scene_params(scene: &Value) -> Value {
    scene
        .get("params")
        .and_then(|v| v.as_object())
        .cloned()
        .map(Value::Object)
        .unwrap_or_else(|| json!({}))
}

fn generate_storyboard_template_lowering(template_id: &str) -> Option<&'static str> {
    generate::registry()
        .get(template_id)
        .map(|template| template.lowering.verb.as_str())
}

fn generate_storyboard_set_policy_evidence(evidence: &Value, policy: &str, mutated: bool) -> Value {
    let mut evidence = evidence.clone();
    if let Some(obj) = evidence.as_object_mut() {
        obj.insert("policy".to_string(), json!(policy));
        obj.insert("mutated".to_string(), json!(mutated));
    }
    evidence
}

fn generate_storyboard_string_values(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect()
}

struct GenerateStoryboardInsertError {
    reason: String,
    insert: Value,
    op_ids: Vec<String>,
}

impl GenerateStoryboardInsertError {
    fn plain(reason: String) -> Self {
        Self {
            reason,
            insert: Value::Null,
            op_ids: Vec::new(),
        }
    }
}

async fn generate_storyboard_preview_scenes(
    state: &AppState,
    storyboard: &Value,
    actor: Actor,
) -> Result<Value, String> {
    let scenes = storyboard
        .get("scenes")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "storyboard.scenes must be an array before preview".to_string())?;
    let mut preview_scenes: Vec<Value> = Vec::new();

    for (idx, scene) in scenes.iter().enumerate() {
        let scene_id = generate_storyboard_scene_id(scene, idx);
        let source = scene.get("source").and_then(|v| v.as_str()).unwrap_or("");
        if source != "generate_template" {
            return Err(format!(
                "scene {scene_id} source '{source}' is not preview-supported yet"
            ));
        }
        let template_id = scene
            .get("template_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| format!("scene {scene_id} is missing template_id"))?;
        match generate_storyboard_template_lowering(template_id) {
            Some("title.add" | "edit.add_shape") => {}
            Some(verb) if is_motion_generate_lowering(verb) => {}
            Some(other) => {
                return Err(format!(
                    "scene {scene_id} template '{template_id}' lowers to {other}; storyboard preview supports title.add, edit.add_shape, and Motion Generate connector scenes"
                ))
            }
            None => return Err(format!("scene {scene_id} uses unknown template '{template_id}'")),
        }
        let preview = dispatch_send(
            state,
            "generate.preview",
            json!({
                "id": template_id,
                "params": generate_storyboard_scene_params(scene),
                "frame_ms": generate_storyboard_scene_start_ms(scene),
            }),
            actor.clone(),
        )
        .await;
        if !preview.ok {
            return Err(preview
                .error
                .map(|e| format!("scene {scene_id} preview failed: {}", e.message))
                .unwrap_or_else(|| format!("scene {scene_id} preview failed")));
        }
        let out = preview.result.unwrap_or(Value::Null);
        preview_scenes.push(json!({
            "scene_id": scene_id,
            "index": generate_storyboard_scene_index(scene, idx),
            "status": "previewed",
            "source": "generate_template",
            "template_id": template_id,
            "range_ms": generate_storyboard_scene_range(scene),
            "preview_id": out.get("preview_id").cloned().unwrap_or(Value::Null),
            "path": out.get("path").cloned().unwrap_or(Value::Null),
            "url": out.get("url").cloned().unwrap_or(Value::Null),
            "mime": out.get("mime").cloned().unwrap_or(Value::Null),
            "width": out.get("width").cloned().unwrap_or(Value::Null),
            "height": out.get("height").cloned().unwrap_or(Value::Null),
            "frame_ms": out.get("frame_ms").cloned().unwrap_or(Value::Null),
            "params": out.get("params").cloned().unwrap_or(Value::Null),
            "lowering": out.get("lowering").cloned().unwrap_or(Value::Null),
        }));
    }

    Ok(json!({
        "policy": "preview",
        "mutated": false,
        "scenes": preview_scenes,
        "unsupported": [],
        "warnings": [],
    }))
}

async fn generate_storyboard_insert_scenes(
    state: &AppState,
    storyboard: &Value,
    actor: Actor,
    rationale: Option<String>,
) -> Result<(Value, Vec<String>), GenerateStoryboardInsertError> {
    let scenes = storyboard
        .get("scenes")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            GenerateStoryboardInsertError::plain(
                "storyboard.scenes must be an array before insert".to_string(),
            )
        })?;

    for (idx, scene) in scenes.iter().enumerate() {
        let scene_id = generate_storyboard_scene_id(scene, idx);
        let source = scene.get("source").and_then(|v| v.as_str()).unwrap_or("");
        if source != "generate_template" {
            return Err(GenerateStoryboardInsertError::plain(format!(
                "scene {scene_id} source '{source}' is not insert-supported yet"
            )));
        }
        let template_id = scene
            .get("template_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                GenerateStoryboardInsertError::plain(format!(
                    "scene {scene_id} is missing template_id"
                ))
            })?;
        match generate_storyboard_template_lowering(template_id) {
            Some("title.add" | "edit.add_shape") => {}
            Some(verb) if is_motion_generate_lowering(verb) => {}
            Some(other) => {
                return Err(GenerateStoryboardInsertError::plain(format!(
                    "scene {scene_id} template '{template_id}' lowers to {other}; storyboard insert supports title.add, edit.add_shape, and Motion Generate connector scenes"
                )))
            }
            None => {
                return Err(GenerateStoryboardInsertError::plain(format!(
                    "scene {scene_id} uses unknown template '{template_id}'"
                )))
            }
        }
    }

    let storyboard_id = storyboard
        .get("storyboard_id")
        .and_then(|v| v.as_str())
        .unwrap_or("storyboard");
    let mut insert_scenes: Vec<Value> = Vec::new();
    let mut all_op_ids: Vec<String> = Vec::new();
    let mut checkpoints: Vec<String> = Vec::new();
    let mut clips: Vec<String> = Vec::new();
    let mut assets: Vec<String> = Vec::new();

    for (idx, scene) in scenes.iter().enumerate() {
        let scene_id = generate_storyboard_scene_id(scene, idx);
        let template_id = scene
            .get("template_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let scene_rationale = rationale
            .clone()
            .unwrap_or_else(|| format!("generate.storyboard {storyboard_id} scene {scene_id}"));
        let insert = dispatch_send(
            state,
            "generate.insert",
            json!({
                "id": template_id,
                "params": generate_storyboard_scene_params(scene),
                "at_ms": generate_storyboard_scene_start_ms(scene),
                "rationale": scene_rationale,
            }),
            actor.clone(),
        )
        .await;
        if !insert.ok {
            let reason = insert
                .error
                .map(|e| format!("scene {scene_id} insert failed: {}", e.message))
                .unwrap_or_else(|| format!("scene {scene_id} insert failed"));
            let restore_hint = checkpoints
                .first()
                .map(|id| {
                    format!(
                        "project.revert{{to:\"{id}\"}} returns to before this storyboard insert"
                    )
                })
                .unwrap_or_else(|| "project.undo reverts the generated storyboard ops".to_string());
            return Err(GenerateStoryboardInsertError {
                reason: reason.clone(),
                insert: json!({
                    "policy": "insert",
                    "mutated": !all_op_ids.is_empty(),
                    "status": "failed",
                    "failed_scene": scene_id,
                    "failed_index": generate_storyboard_scene_index(scene, idx),
                    "reason": reason,
                    "scenes": insert_scenes,
                    "checkpoints": checkpoints,
                    "op_ids": all_op_ids,
                    "clips": clips,
                    "assets": assets,
                    "restore_hint": restore_hint,
                    "unsupported": [],
                    "warnings": [],
                }),
                op_ids: all_op_ids,
            });
        }
        let out = insert.result.unwrap_or(Value::Null);
        let scene_op_ids = generate_storyboard_string_values(out.get("op_ids"));
        let scene_clips = generate_storyboard_string_values(out.get("clips"));
        let scene_assets = generate_storyboard_string_values(out.get("assets"));
        all_op_ids.extend(scene_op_ids.clone());
        for clip in &scene_clips {
            if !clips.contains(clip) {
                clips.push(clip.clone());
            }
        }
        for asset in &scene_assets {
            if !assets.contains(asset) {
                assets.push(asset.clone());
            }
        }
        let checkpoint = out.get("checkpoint").cloned().unwrap_or(Value::Null);
        if let Some(id) = checkpoint.get("id").and_then(|v| v.as_str()) {
            checkpoints.push(id.to_string());
        }
        insert_scenes.push(json!({
            "scene_id": scene_id,
            "index": generate_storyboard_scene_index(scene, idx),
            "status": "inserted",
            "source": "generate_template",
            "template_id": template_id,
            "range_ms": generate_storyboard_scene_range(scene),
            "checkpoint": checkpoint,
            "op_ids": scene_op_ids,
            "clips": scene_clips,
            "assets": scene_assets,
            "params": out.get("params").cloned().unwrap_or(Value::Null),
            "lowering": out.get("lowering").cloned().unwrap_or(Value::Null),
            "restore_hint": out.get("restore_hint").cloned().unwrap_or(Value::Null),
            "result": out.get("result").cloned().unwrap_or(Value::Null),
        }));
    }

    let restore_hint = checkpoints
        .first()
        .map(|id| format!("project.revert{{to:\"{id}\"}} returns to before this storyboard insert"))
        .unwrap_or_else(|| "project.undo reverts the generated storyboard ops".to_string());
    Ok((
        json!({
            "policy": "insert",
            "mutated": true,
            "scenes": insert_scenes,
            "checkpoints": checkpoints,
            "op_ids": all_op_ids,
            "clips": clips,
            "assets": assets,
            "restore_hint": restore_hint,
            "unsupported": [],
            "warnings": [],
        }),
        all_op_ids,
    ))
}

/// generate.storyboard{input, mode?, policy?} — create a validated multi-scene
/// Generate Storyboard IR. Preview is non-mutating per-scene PNG evidence;
/// insert materializes supported Generate template scenes through native verbs.
pub(crate) async fn generate_storyboard(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    #[derive(serde::Deserialize)]
    struct Args {
        input: String,
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        policy: Option<String>,
        #[serde(default)]
        answers: Option<Value>,
        #[serde(default)]
        context: Option<Value>,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        timeout_ms: Option<u64>,
        #[serde(default)]
        rationale: Option<String>,
    }

    let a: Args = parse_args(args)?;
    let input = a.input.trim().to_string();
    if input.is_empty() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "generate.storyboard input is required",
            "pass a non-empty prompt, brief, script, outline, or media request",
        ));
    }
    let mode = a.mode.unwrap_or_else(|| "auto".to_string());
    if ![
        "auto",
        "quick_prompt",
        "director_brief",
        "script",
        "existing_media",
    ]
    .contains(&mode.as_str())
    {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("invalid generate.storyboard mode '{mode}'"),
            "allowed mode values: auto, quick_prompt, director_brief, script, existing_media",
        ));
    }
    let policy = a.policy.unwrap_or_else(|| "plan".to_string());
    if !["plan", "preview", "insert"].contains(&policy.as_str()) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("invalid generate.storyboard policy '{policy}'"),
            "allowed policy values: plan, preview, insert",
        ));
    }
    let agent = a.agent.unwrap_or_else(|| "auto".to_string());
    if !["auto", "claude", "codex", "grok"].contains(&agent.as_str()) {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            format!("invalid generate.storyboard agent '{agent}'"),
            "allowed agent values: auto, claude, codex, grok",
        ));
    }
    let answers = a.answers.unwrap_or_else(|| json!({}));
    if !answers.is_object() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "generate.storyboard answers must be an object",
            "pass structured director answers keyed by field",
        ));
    }
    let context = a.context.unwrap_or_else(|| json!({}));
    if !context.is_object() {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "generate.storyboard context must be an object",
            "pass selected media, transcript, brand, or platform context as an object",
        ));
    }

    let timeout_ms = a
        .timeout_ms
        .unwrap_or(GENERATE_STORYBOARD_TIMEOUT_MS_DEFAULT)
        .clamp(1_000, 300_000);
    let motion_ok = crate::motion_bridge::motion_available();
    let request = json!({
        "schema": "shellx-cut/generate-storyboard-request/1",
        "input": input,
        "mode": mode,
        "policy": policy,
        "agent": agent,
        "agents": generate_agent_paths(),
        "timeout_ms": timeout_ms,
        "answers": answers,
        "context": context,
        "templates": generate::registry()
            .templates
            .iter()
            .map(|t| generate_summary_json(t, motion_ok))
            .collect::<Vec<_>>(),
        "skill_path": GENERATE_STORYBOARD_SKILL_PATH,
        "rationale": a.rationale,
    });
    let adapter = find_generate_storyboard_adapter();
    let envelope = run_generate_storyboard_adapter(adapter.as_deref(), &request, timeout_ms).await;
    let status = envelope
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("error");
    let backend = envelope.get("backend").cloned().unwrap_or(Value::Null);
    let mut warnings = generate_prompt_warnings(&envelope);
    let (questions, question_warnings) = generate_storyboard_questions(&envelope);
    warnings.extend(question_warnings);

    if status != "completed" && status != "needs_input" {
        return Ok(VerbResult::ok(generate_storyboard_result(
            status,
            request,
            backend,
            Value::Null,
            questions,
            Value::Null,
            generate_storyboard_empty_evidence(),
            envelope.get("reason").cloned().unwrap_or(Value::Null),
            warnings,
            generate_storyboard_next_actions(status),
        )));
    }
    if envelope.get("schema").and_then(|v| v.as_str())
        != Some("shellx-cut/generate-storyboard-result/1")
    {
        return Ok(VerbResult::ok(generate_storyboard_result(
            "error",
            request,
            backend,
            Value::Null,
            questions,
            Value::Null,
            generate_storyboard_empty_evidence(),
            json!("generate storyboard adapter returned an invalid schema"),
            warnings,
            generate_storyboard_next_actions("error"),
        )));
    }

    let storyboard = envelope.get("storyboard").cloned().unwrap_or(Value::Null);
    let (validation, evidence) = validate_generate_storyboard(&storyboard);
    if !validation["ok"].as_bool().unwrap_or(false) {
        return Ok(VerbResult::ok(generate_storyboard_result(
            "error",
            request,
            backend,
            storyboard,
            questions,
            validation,
            evidence,
            json!("generate storyboard failed validation"),
            warnings,
            generate_storyboard_next_actions("error"),
        )));
    }

    let final_status = if status == "needs_input"
        || storyboard.get("status").and_then(|v| v.as_str()) == Some("needs_input")
    {
        "needs_input"
    } else {
        "completed"
    };
    if final_status == "completed" && policy == "preview" {
        let evidence = generate_storyboard_set_policy_evidence(&evidence, "preview", false);
        return match generate_storyboard_preview_scenes(state, &storyboard, actor).await {
            Ok(preview) => {
                let mut result = generate_storyboard_result(
                    final_status,
                    request,
                    backend,
                    storyboard,
                    questions,
                    validation,
                    evidence,
                    Value::Null,
                    warnings,
                    vec!["review storyboard previews", "run policy:insert"],
                );
                result["preview"] = preview;
                Ok(VerbResult::ok(result))
            }
            Err(reason) => Ok(VerbResult::ok(generate_storyboard_result(
                "error",
                request,
                backend,
                storyboard,
                questions,
                validation,
                evidence,
                json!(reason),
                warnings,
                vec!["adjust storyboard scenes", "retry policy:preview"],
            ))),
        };
    }
    if final_status == "completed" && policy == "insert" {
        let evidence = generate_storyboard_set_policy_evidence(&evidence, "insert", true);
        return match generate_storyboard_insert_scenes(state, &storyboard, actor, a.rationale).await
        {
            Ok((insert, op_ids)) => {
                let mut result = generate_storyboard_result(
                    final_status,
                    request,
                    backend,
                    storyboard,
                    questions,
                    validation,
                    evidence,
                    Value::Null,
                    warnings,
                    vec!["review generated storyboard timeline"],
                );
                result["insert"] = insert;
                Ok(VerbResult::ok_with_ops(result, op_ids))
            }
            Err(failure) => {
                let mut result = generate_storyboard_result(
                    "error",
                    request,
                    backend,
                    storyboard,
                    questions,
                    validation,
                    evidence,
                    json!(failure.reason),
                    warnings,
                    vec![
                        "open a project or adjust storyboard scenes",
                        "retry policy:insert",
                    ],
                );
                if !failure.insert.is_null() {
                    result["insert"] = failure.insert;
                }
                if failure.op_ids.is_empty() {
                    Ok(VerbResult::ok(result))
                } else {
                    Ok(VerbResult::ok_with_ops(result, failure.op_ids))
                }
            }
        };
    }
    Ok(VerbResult::ok(generate_storyboard_result(
        final_status,
        request,
        backend,
        storyboard,
        questions,
        validation,
        evidence,
        Value::Null,
        warnings,
        generate_storyboard_next_actions(final_status),
    )))
}
