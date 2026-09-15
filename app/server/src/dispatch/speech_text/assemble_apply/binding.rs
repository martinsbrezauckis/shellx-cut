//! Project identity, revision, transcript, and reviewed-plan binding checks.

use super::super::*;
use sha2::{Digest, Sha256};

const PLAN_BINDING_SCHEMA: &str = "shellx-cut/assemble-plan-binding/1";
pub(crate) const ASSEMBLE_CAPTION_TRACK_ID: &str = "asmcap1";

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::dispatch::speech_text) struct PlanBinding {
    pub schema: String,
    pub project_identity: Value,
    pub project_revision: String,
    pub verb: String,
    pub asset: String,
    pub selected_ranges: Vec<[usize; 2]>,
    pub transcript_sha256: String,
    pub materialization: Materialization,
}

#[derive(Clone, Debug)]
pub(in crate::dispatch::speech_text) struct PlanContext {
    project_identity: Value,
    project_revision: String,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(in crate::dispatch::speech_text) enum Materialization {
    Reel,
    Shorts {
        aspect: String,
        /// The proposal carries the computed crop, including `None` when the
        /// source cannot be materialized. Apply only accepts `Some`.
        crop: Option<[f64; 4]>,
    },
}

impl Materialization {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Reel => "reel",
            Self::Shorts { .. } => "shorts",
        }
    }
}

/// Bind a plan to the open project's path-free identity and durable revision.
pub(in crate::dispatch::speech_text) async fn capture_plan_context(
    state: &AppState,
) -> Result<PlanContext, CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    plan_context_for_store(store)
}

/// A planner can spend time reading transcript/perception facts. Refuse to
/// return a proposal if a project edit landed while those facts were collected.
pub(in crate::dispatch::speech_text) async fn ensure_plan_context_current(
    state: &AppState,
    context: &PlanContext,
) -> Result<(), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    ensure_plan_context(store, context)
}

pub(super) async fn ensure_apply_binding_current(
    state: &AppState,
    binding: &PlanBinding,
) -> Result<(), CutError> {
    let guard = state.project.read().await;
    let store = guard.as_ref().ok_or_else(no_project)?;
    ensure_binding(store, binding)
}

pub(in crate::dispatch::speech_text) fn bind_plan(
    context: PlanContext,
    verb: &str,
    asset: &str,
    selected_ranges: Vec<[usize; 2]>,
    transcript_sha256: String,
    materialization: Materialization,
) -> PlanBinding {
    PlanBinding {
        schema: PLAN_BINDING_SCHEMA.into(),
        project_identity: context.project_identity,
        project_revision: context.project_revision,
        verb: verb.into(),
        asset: asset.into(),
        selected_ranges,
        transcript_sha256,
        materialization,
    }
}

pub(in crate::dispatch::speech_text) fn binding_value(
    binding: &PlanBinding,
) -> Result<Value, CutError> {
    Ok(serde_json::to_value(binding)?)
}

/// Hash the canonical word spans used for planning and lowering. Receipt
/// provenance such as the model or language cannot change this binding.
pub(in crate::dispatch::speech_text) fn transcript_content_sha256(
    transcript: &cut_perception::Transcript,
) -> Result<String, CutError> {
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&transcript.words)?)
    ))
}

pub(super) fn ensure_transcript_matches(
    binding: &PlanBinding,
    transcript_sha256: &str,
) -> Result<(), CutError> {
    if binding.transcript_sha256 == transcript_sha256 {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        "the reviewed Assemble plan's transcript changed",
        "review the plan again before applying the current transcript",
    ))
}
fn plan_context_for_store(store: &ProjectStore) -> Result<PlanContext, CutError> {
    let project_revision = store.log.current_revision()?.ok_or_else(|| {
        CutError::new(
            error_codes::CONFLICT,
            "open project has no durable revision",
            "save or reopen the project before planning an Assemble edit",
        )
    })?;
    Ok(PlanContext {
        project_identity: path_free_project_identity(store)?,
        project_revision,
    })
}

fn ensure_plan_context(store: &ProjectStore, context: &PlanContext) -> Result<(), CutError> {
    let current = plan_context_for_store(store)?;
    if current.project_identity != context.project_identity {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the reviewed Assemble plan belongs to a different project",
            "open the planned project or review a new plan for this project",
        ));
    }
    if current.project_revision != context.project_revision {
        return Err(CutError::new(
            error_codes::CONFLICT,
            format!(
                "the reviewed Assemble plan is stale (planned {}, current {})",
                context.project_revision, current.project_revision
            ),
            "review the plan again after the timeline changed",
        ));
    }
    Ok(())
}

pub(super) fn ensure_binding(store: &ProjectStore, binding: &PlanBinding) -> Result<(), CutError> {
    if binding.schema != PLAN_BINDING_SCHEMA {
        return Err(CutError::new(
            error_codes::INVALID_ARGS,
            "Assemble apply needs a valid plan binding",
            "review the plan again, then submit its unmodified plan_binding",
        ));
    }
    ensure_plan_context(
        store,
        &PlanContext {
            project_identity: binding.project_identity.clone(),
            project_revision: binding.project_revision.clone(),
        },
    )
}

pub(super) fn ensure_plan_matches(
    binding: &PlanBinding,
    verb: &str,
    asset: &str,
    selected_ranges: &[[usize; 2]],
    materialization: &Materialization,
) -> Result<(), CutError> {
    if binding.verb == verb
        && binding.asset == asset
        && binding.selected_ranges == selected_ranges
        && binding.materialization == *materialization
    {
        return Ok(());
    }
    Err(CutError::new(
        error_codes::CONFLICT,
        "the apply request does not match the reviewed Assemble plan",
        "resend the unchanged planned request and its returned plan_binding",
    ))
}

pub(super) fn require_apply_request(actor: &Actor, binding: &PlanBinding) -> Result<(), CutError> {
    let request = actor.request.as_ref().ok_or_else(|| {
        CutError::new(
            error_codes::INVALID_ARGS,
            "applying an Assemble plan requires request_id and expected_revision",
            "submit the returned plan_binding.project_revision with a unique request_id",
        )
    })?;
    if request.expected_revision.as_deref() != Some(binding.project_revision.as_str()) {
        return Err(CutError::new(
            error_codes::CONFLICT,
            "the apply request revision does not match the reviewed Assemble plan",
            "use the returned plan_binding.project_revision as expected_revision",
        ));
    }
    Ok(())
}

pub(in crate::dispatch::speech_text) fn aspect_parts(aspect: &str) -> Option<[u32; 2]> {
    match aspect {
        "9:16" => Some([9, 16]),
        "1:1" => Some([1, 1]),
        "4:5" => Some([4, 5]),
        "16:9" => Some([16, 9]),
        _ => None,
    }
}

pub(in crate::dispatch::speech_text) fn aspect_label(width: u32, height: u32) -> String {
    let divisor = gcd(width, height).max(1);
    format!("{}:{}", width / divisor, height / divisor)
}

fn gcd(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}
