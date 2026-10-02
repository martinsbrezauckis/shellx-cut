//! Project-bound entry points for Motion template and script materialization.

use super::*;
use crate::project_materialization::{ProjectMaterialization, ProjectMaterializationPin};

#[derive(serde::Deserialize)]
struct TemplateInput {
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

impl TemplateInput {
    fn parse(args: Value) -> Result<Self, CutError> {
        let input: Self = serde_json::from_value(args).map_err(|e| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "motion.template_to_cut args did not match schema",
                e.to_string(),
            )
        })?;
        input.policy()?;
        Ok(input)
    }

    fn policy(&self) -> Result<MotionTemplatePolicy, CutError> {
        match self.policy.as_deref().unwrap_or("insert") {
            "preview" => Ok(MotionTemplatePolicy::Preview),
            "insert" => Ok(MotionTemplatePolicy::Insert),
            other => Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("invalid motion.template_to_cut policy '{other}'"),
                "allowed policy values: preview, insert",
            )),
        }
    }

    async fn build(self, state: &AppState) -> Result<MotionTemplateRequest, CutError> {
        let policy = self.policy()?;
        let template = self
            .template
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_TEMPLATE_ALIAS.to_string());
        let out_dir = match self.out_dir.filter(|value| !value.trim().is_empty()) {
            Some(path) => PathBuf::from(path),
            None => default_motion_out_dir(state, &template, policy).await,
        };
        let caller_scope = motion_caller_scope(state, &out_dir).await;
        let dry_run_render = match policy {
            MotionTemplatePolicy::Preview => true,
            MotionTemplatePolicy::Insert => self.dry_run_render.unwrap_or(false),
        };
        Ok(MotionTemplateRequest {
            template,
            params: self.params,
            policy,
            out_dir,
            at_ms: self.at_ms.unwrap_or(0),
            track: self.track.unwrap_or_else(|| "v1".to_string()),
            duration_ms: self.duration_ms,
            dry_run_render,
            checkpoint: self.checkpoint.unwrap_or(true),
            rationale: self.rationale,
            motion_job_id: self.job_id,
            caller_scope,
        })
    }
}

#[derive(serde::Deserialize)]
struct ScriptInput {
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

impl ScriptInput {
    fn parse(args: Value) -> Result<Self, CutError> {
        let input: Self = serde_json::from_value(args).map_err(|e| {
            CutError::new(
                error_codes::INVALID_ARGS,
                "motion.script_to_cut args did not match schema",
                e.to_string(),
            )
        })?;
        let script_path = input
            .script_path
            .as_deref()
            .filter(|value| !value.trim().is_empty());
        if input.script.is_some() == script_path.is_some() {
            return Err(CutError::new(
                error_codes::INVALID_ARGS,
                "motion.script_to_cut requires exactly one of script or script_path",
                "pass an inline shellx-motion/scripted-video@1 object, or a path to one",
            ));
        }
        input.policy()?;
        Ok(input)
    }

    fn policy(&self) -> Result<MotionScriptPolicy, CutError> {
        match self.policy.as_deref().unwrap_or("insert") {
            "preview" => Ok(MotionScriptPolicy::Preview),
            "insert" => Ok(MotionScriptPolicy::Insert),
            other => Err(CutError::new(
                error_codes::INVALID_ARGS,
                format!("invalid motion.script_to_cut policy '{other}'"),
                "allowed policy values: preview, insert",
            )),
        }
    }

    async fn build(self, state: &AppState) -> Result<MotionScriptRequest, CutError> {
        let policy = self.policy()?;
        let script_path = self
            .script_path
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        let default_leaf = self
            .script
            .as_ref()
            .and_then(|script| script.get("id"))
            .and_then(Value::as_str)
            .map(safe_fragment)
            .unwrap_or_else(|| "scripted_video".to_string());
        let out_dir = match self.out_dir.filter(|value| !value.trim().is_empty()) {
            Some(path) => PathBuf::from(path),
            None => default_motion_script_out_dir(state, &default_leaf, policy).await,
        };
        let caller_scope = motion_caller_scope(state, &out_dir).await;
        let dry_run_render = match policy {
            MotionScriptPolicy::Preview => true,
            MotionScriptPolicy::Insert => self.dry_run_render.unwrap_or(false),
        };
        Ok(MotionScriptRequest {
            script: self.script,
            script_path,
            policy,
            out_dir,
            at_ms: self.at_ms.unwrap_or(0),
            track: self.track.unwrap_or_else(|| "v1".to_string()),
            duration_ms: self.duration_ms,
            dry_run_render,
            checkpoint: self.checkpoint.unwrap_or(true),
            rationale: self.rationale,
            motion_job_id: self.job_id,
            caller_scope,
        })
    }
}

/// Public verb: motion.template_to_cut.
pub(crate) async fn motion_template_to_cut(
    state: &AppState,
    args: Value,
    actor: Actor,
) -> Result<VerbResult, CutError> {
    motion_template_to_cut_materialized(state, args, actor, None).await
}

/// Generate holds an already-verified transition through its own adapter wait.
pub(crate) async fn motion_template_to_cut_under_materialization(
    state: &AppState,
    args: Value,
    actor: Actor,
    materialization: &ProjectMaterialization<'_>,
) -> Result<VerbResult, CutError> {
    motion_template_to_cut_materialized(state, args, actor, Some(materialization)).await
}

async fn motion_template_to_cut_materialized(
    state: &AppState,
    args: Value,
    actor: Actor,
    materialization: Option<&ProjectMaterialization<'_>>,
) -> Result<VerbResult, CutError> {
    // Validation is state-free, so an Insert pins before defaults read a project.
    let input = TemplateInput::parse(args)?;
    let direct_materialization = direct_materialization(
        state,
        input.policy()? == MotionTemplatePolicy::Insert,
        materialization.is_some(),
    )
    .await?;
    let _materialization = materialization.or(direct_materialization.as_ref());
    let request = input.build(state).await?;
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
    motion_script_to_cut_materialized(state, args, actor, None).await
}

/// Generate uses this only while it already owns the project materialization.
pub(crate) async fn motion_script_to_cut_under_materialization(
    state: &AppState,
    args: Value,
    actor: Actor,
    materialization: &ProjectMaterialization<'_>,
) -> Result<VerbResult, CutError> {
    motion_script_to_cut_materialized(state, args, actor, Some(materialization)).await
}

async fn motion_script_to_cut_materialized(
    state: &AppState,
    args: Value,
    actor: Actor,
    materialization: Option<&ProjectMaterialization<'_>>,
) -> Result<VerbResult, CutError> {
    // Validation is state-free, so an Insert pins before defaults read a project.
    let input = ScriptInput::parse(args)?;
    let direct_materialization = direct_materialization(
        state,
        input.policy()? == MotionScriptPolicy::Insert,
        materialization.is_some(),
    )
    .await?;
    let _materialization = materialization.or(direct_materialization.as_ref());
    let request = input.build(state).await?;
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

async fn direct_materialization<'a>(
    state: &'a AppState,
    inserting: bool,
    caller_owns_materialization: bool,
) -> Result<Option<ProjectMaterialization<'a>>, CutError> {
    if !inserting || caller_owns_materialization {
        return Ok(None);
    }
    let materialization = ProjectMaterializationPin::capture(state, "Motion")
        .await?
        .lock_verified(state)
        .await?;
    #[cfg(all(test, unix))]
    wait_for_motion_request_build_gate().await;
    Ok(Some(materialization))
}

#[cfg(all(test, unix))]
#[derive(Clone)]
pub(crate) struct MotionRequestBuildGate {
    pub project_pinned: std::sync::Arc<tokio::sync::Notify>,
    pub continue_after_pin: std::sync::Arc<tokio::sync::Notify>,
}

#[cfg(all(test, unix))]
impl MotionRequestBuildGate {
    pub(crate) fn new() -> Self {
        Self {
            project_pinned: std::sync::Arc::new(tokio::sync::Notify::new()),
            continue_after_pin: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }
}

#[cfg(all(test, unix))]
static MOTION_REQUEST_BUILD_GATE: std::sync::OnceLock<
    std::sync::Mutex<Option<MotionRequestBuildGate>>,
> = std::sync::OnceLock::new();

#[cfg(all(test, unix))]
pub(crate) fn install_motion_request_build_gate(value: Option<MotionRequestBuildGate>) {
    *MOTION_REQUEST_BUILD_GATE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = value;
}

#[cfg(all(test, unix))]
async fn wait_for_motion_request_build_gate() {
    let value = MOTION_REQUEST_BUILD_GATE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    if let Some(value) = value {
        value.project_pinned.notify_one();
        value.continue_after_pin.notified().await;
    }
}
