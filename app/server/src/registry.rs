//! registry.rs — schema/verbs.json loader. The server loads this public verb
//! contract and generates MCP tools from it, keeping REST and MCP aligned.
//!
//! Role: embed the verb registry at compile time (single source of truth
//! lives at <repo>/schema/verbs.json), parse once, expose lookups used by
//! REST dispatch (verb exists?) and MCP tools/list (name+schema generation).
//! Dependencies: serde_json. Primary callers: dispatch.rs, mcp.rs, http.rs.

#[path = "verb_contract.rs"]
pub(crate) mod verb_contract;

use crate::registry::verb_contract::DispatchTarget;
use crate::schema_validation::CompiledVerbSchemas;
use cut_core::{
    contract_for_verb, AgentChatCapability, AsyncJobType, CutError, Idempotency, MutationClass,
    ProjectState, Replayability, SideEffects, UiExposure, VerbFacet, VerbRisk,
};
use serde::Deserialize;
use std::sync::{Arc, OnceLock};

/// The embedded registry source — compiled in so cutd has no runtime file
/// dependency on the repo layout (it may run installed/standalone).
pub const VERBS_JSON: &str = include_str!("../../../schema/verbs.json");

/// One verb entry from schema/verbs.json (subset of fields we consume; the
/// args schema stays raw JSON because we forward it verbatim to MCP clients).
// Domain and result are contract surfaces consumed by the server and parity gate.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct VerbSpec {
    /// Fully-qualified name, e.g. "transcript.cut_words".
    pub name: String,
    pub domain: String,
    pub description: String,
    /// JSON Schema for the verb args (forwarded as MCP inputSchema).
    pub args: serde_json::Value,
    /// Human-readable result description.
    pub result: String,
    /// Operational facts generated from the schema and consumed by core replay,
    /// API/docs discovery, and the exhaustive dispatcher target.
    /// Every public verb must declare all fields; there are no fallback
    /// semantics for a newly added verb.
    pub behavior: VerbBehavior,
    /// Optional machine-readable JSON Schema for the successful result. Present
    /// on the high-use verbs the autopilot /
    /// social pipeline consume (render.final, verify.checks, project.diff,
    /// transcript.assemble/search, comment.draft/apply); points into
    /// schema/receipts.schema.json for the typed sub-contracts. Forwarded to
    /// MCP clients as `outputSchema` when present.
    #[serde(default)]
    pub result_schema: Option<serde_json::Value>,
}

/// Per-verb behavior contract carried in schema/verbs.json. `DispatchTarget`
/// is generated from the schema and the dispatcher exhaustively matches it, so
/// the public name-to-handler mapping cannot silently drift.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerbBehavior {
    pub mutation_class: MutationClass,
    pub side_effects: SideEffects,
    pub dispatch: DispatchTarget,
    pub project_state: ProjectState,
    pub idempotency: Idempotency,
    pub replayability: Replayability,
    pub async_job: AsyncJobType,
    pub ui_exposure: UiExposure,
    pub agent_chat: AgentChatCapability,
    pub risk: VerbRisk,
    pub facets: Vec<VerbFacet>,
}

#[derive(Debug, Deserialize)]
struct RawVerbRegistry {
    schema: String,
    mutation_controls: serde_json::Map<String, serde_json::Value>,
    verbs: Vec<VerbSpec>,
}

/// Parsed registry plus one compiled validator per public verb.
// The schema tag is asserted in tests and used by the parity gate.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct VerbRegistry {
    pub schema: String,
    pub verbs: Vec<VerbSpec>,
    validators: CompiledVerbSchemas,
    public_json: serde_json::Value,
}

impl VerbRegistry {
    /// Parse and compile the embedded registry. Panics on malformed JSON or an
    /// invalid input schema: both are build-artifact corruption, not caller
    /// errors, and must fail before the first verb can run.
    pub fn load() -> Self {
        Self::try_load().expect("schema/verbs.json and every verb input schema must compile")
    }

    pub fn try_load() -> Result<Self, String> {
        Self::try_load_source(VERBS_JSON)
    }

    fn try_load_source(source: &str) -> Result<Self, String> {
        let mut raw: RawVerbRegistry = serde_json::from_str(source)
            .map_err(|error| format!("schema/verbs.json must parse: {error}"))?;
        validate_behavior_contract(&raw.verbs)?;
        for spec in &mut raw.verbs {
            merge_mutation_controls(&mut spec.args, &raw.mutation_controls, &spec.name)?;
        }
        let validators = CompiledVerbSchemas::compile(&raw.verbs)?;
        let mut public_json: serde_json::Value = serde_json::from_str(source)
            .map_err(|error| format!("schema/verbs.json must parse: {error}"))?;
        let public_verbs = public_json
            .get_mut("verbs")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| "schema/verbs.json verbs must be an array".to_string())?;
        for (public, spec) in public_verbs.iter_mut().zip(&raw.verbs) {
            public
                .as_object_mut()
                .ok_or_else(|| format!("verb '{}' must be an object", spec.name))?
                .insert("args".into(), spec.args.clone());
        }
        Ok(Self {
            schema: raw.schema,
            verbs: raw.verbs,
            validators,
            public_json,
        })
    }

    /// Process-wide immutable registry. AppState creation is frequent in tests
    /// and cheap in production; validators compile only on the first call.
    pub fn shared() -> Arc<Self> {
        static REGISTRY: OnceLock<Arc<VerbRegistry>> = OnceLock::new();
        Arc::clone(REGISTRY.get_or_init(|| Arc::new(Self::load())))
    }

    /// Look up a verb by exact name.
    pub fn get(&self, name: &str) -> Option<&VerbSpec> {
        self.verbs.iter().find(|v| v.name == name)
    }

    pub fn validate_args(&self, spec: &VerbSpec, args: &serde_json::Value) -> Result<(), CutError> {
        self.validators.validate(spec, args)
    }

    pub fn public_json(&self) -> &serde_json::Value {
        &self.public_json
    }

    /// MCP tool name for a verb: dots → underscores ("transcript.cut_words"
    /// → "transcript_cut_words"). MCP tool names must match ^[a-zA-Z0-9_-]+$.
    pub fn mcp_tool_name(verb_name: &str) -> String {
        verb_name.replace('.', "_")
    }

    /// Inverse of mcp_tool_name: find the verb a tool name refers to.
    pub fn verb_for_tool<'a>(&'a self, tool_name: &str) -> Option<&'a VerbSpec> {
        self.verbs
            .iter()
            .find(|v| Self::mcp_tool_name(&v.name) == tool_name)
    }
}

fn validate_behavior_contract(verbs: &[VerbSpec]) -> Result<(), String> {
    let mut names = std::collections::BTreeSet::new();
    let mut dispatches = std::collections::BTreeSet::new();
    for spec in verbs {
        if !names.insert(&spec.name) {
            return Err(format!("schema/verbs.json duplicates verb '{}'", spec.name));
        }
        if !dispatches.insert(format!("{:?}", spec.behavior.dispatch)) {
            return Err(format!(
                "schema/verbs.json reuses dispatch target for '{}'",
                spec.name
            ));
        }
        let generated = contract_for_verb(&spec.name).ok_or_else(|| {
            format!(
                "schema/verbs.json verb '{}' has no generated core behavior contract; run node scripts/generate-verb-contract.mjs",
                spec.name
            )
        })?;
        if generated.mutation_class != spec.behavior.mutation_class
            || generated.side_effects != spec.behavior.side_effects
            || generated.project_state != spec.behavior.project_state
            || generated.idempotency != spec.behavior.idempotency
            || generated.replayability != spec.behavior.replayability
            || generated.async_job != spec.behavior.async_job
            || generated.ui_exposure != spec.behavior.ui_exposure
            || generated.agent_chat != spec.behavior.agent_chat
            || generated.risk != spec.behavior.risk
            || generated.facets != spec.behavior.facets.as_slice()
        {
            return Err(format!(
                "schema/verbs.json behavior for '{}' differs from generated core contract; run node scripts/generate-verb-contract.mjs",
                spec.name
            ));
        }
    }
    Ok(())
}

fn merge_mutation_controls(
    args: &mut serde_json::Value,
    controls: &serde_json::Map<String, serde_json::Value>,
    verb: &str,
) -> Result<(), String> {
    let args = args
        .as_object_mut()
        .ok_or_else(|| format!("verb '{verb}' args must be an object schema"))?;
    let properties = args
        .entry("properties")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| format!("verb '{verb}' args.properties must be an object"))?;
    for (name, schema) in controls {
        if properties.insert(name.clone(), schema.clone()).is_some() {
            return Err(format!(
                "verb '{verb}' duplicates shared mutation control '{name}'"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "registry/behavior_contract_tests.rs"]
mod behavior_contract_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// The embedded registry must expose every schema entry. The count and
    /// representative domain checks below are deliberate contract tripwires;
    /// feature behavior belongs in schema/verbs.json and docs/public/FEATURES.md.
    #[test]
    fn registry_parses_with_all_verbs() {
        let reg = VerbRegistry::load();
        assert_eq!(reg.schema, "shellx-cut/verbs/1");
        assert_eq!(
            reg.verbs.len(),
            262,
            "verb count is a deliberate-contract tripwire: bump this AND README + \
             skill/shellx-cut/reference.md when verbs.json changes"
        );
        // Spot-check one verb per domain (including canonical renames, jobs,
        // audio, system, comments, title domains, and the
        // creative/zoom additions, so a dropped domain trips the test).
        for name in [
            "project.create",
            "project.health",
            "project.rename",
            "project.format",
            "project.color",
            "project.brand",
            "project.sequence_list",
            "project.sequence_index",
            "project.sequence_create",
            "project.sequence_switch",
            "project.sequence_rename",
            "project.sequence_delete",
            "project.undo",
            "project.redo",
            "project.list",
            "project.forget",
            "media.import",
            "media.filmstrip",
            "jobs.status",
            "jobs.list",
            "jobs.cancel",
            "edit.split",
            "edit.duplicate",
            "edit.nest",
            "edit.replace",
            "edit.fit_to_fill",
            "edit.cut_to_beat",
            "edit.detach_audio",
            "edit.split_edit",
            "edit.adjustment",
            "edit.add_marker",
            "edit.add_track",
            "edit.remove_track",
            "edit.blend",
            "edit.track_visible",
            "edit.track_lock",
            "edit.mute",
            "edit.solo",
            "edit.duck",
            "edit.transform",
            "edit.crop",
            "edit.fade",
            "edit.crossfade",
            "edit.move_marker",
            "edit.grade",
            "edit.grade_stack",
            "edit.grade_window",
            "grade.save",
            "grade.apply",
            "grade.list",
            "edit.color_space",
            "edit.color_match",
            "edit.auto_balance",
            "edit.matte",
            "edit.add_shape",
            "shape.update",
            "edit.speed",
            "edit.speed_ramp",
            "edit.reverse",
            "edit.stabilize",
            "edit.freeze",
            "edit.animate",
            "edit.keyframe",
            "edit.auto_zoom",
            "edit.multicam_switch",
            "edit.eq",
            "edit.slide",
            "audio.cleanup_voice",
            "audio.add_music",
            "audio.dub",
            "transcript.cut_words",
            "transcript.chapters",
            "transcript.remove_retakes",
            "transcript.translate",
            "captions.generate",
            "captions.translate",
            "captions.add_text",
            "captions.import",
            "captions.set_style",
            "captions.set_range",
            "captions.set_text",
            "captions.kinetic",
            "title.add",
            "title.update",
            "title.templates",
            "assets.providers",
            "assets.search",
            "assets.fetch",
            "agent.chat",
            "plugins.list",
            "plugins.enable",
            "plugins.call",
            "media.index_status",
            "media.index",
            "media.search",
            "effects.list",
            "render.final",
            "render.reframe",
            "render.direct",
            "render.qc",
            "verify.checks",
            "export.xml",
            "import.otio",
            "export.srt",
            "export.frame",
            "export.audio",
            "export.publish",
            "export.gif",
            "export.range",
            "comment.add",
            "comment.apply",
            "ui.screenshot",
            "ui.highlight",
            "debug.screenshot",
            "system.doctor",
            "system.fetch_tool",
            "system.setup_perception",
            "system.setup_matte",
            "clip.candidates",
            "render.bundle",
            "render.queue",
            "autopilot.run",
            "generate.list",
            "generate.describe",
            "generate.preview",
            "generate.insert",
            "generate.from_prompt",
            "generate.storyboard",
            "motion.template_to_cut",
            "motion.script_to_cut",
            "motion.job.get",
            "motion.job.list",
            "motion.map_import",
            "motion.apply_import",
            "motion.link.refresh",
            "motion.link.relink",
            "motion.link.edit",
            "motion.link.tracking.inventory",
            "motion.link.tracking.request",
            "motion.link.tracking.inspect",
            "motion.link.tracking.apply",
            "motion.link.tracking.verify",
            "motion.link.tracking.detach",
            "recipe.list",
            "recipe.describe",
            "recipe.run",
            "assemble.broll",
            "assemble.shorts",
            "screen_record.polish",
            "library.list",
            "library.add",
            "library.add_to_project",
            "library.folder_add",
        ] {
            assert!(reg.get(name).is_some(), "missing verb {name}");
        }
        // The canonical export drops captions.export_srt; the background-job
        // contract removes media.status.
        assert!(reg.get("captions.export_srt").is_none());
        assert!(reg.get("media.status").is_none());
        // Under the two-segment verb-name contract, ordinary verbs are exactly
        // domain.verb. The established
        // Motion connectors keep explicit motion.link.* and motion.job.*
        // namespaces so linked-asset lifecycle and upstream job observation
        // cannot be confused with native Cut edits or jobs.*.
        for v in &reg.verbs {
            let segments: Vec<_> = v.name.split('.').collect();
            assert_eq!(segments.first().copied(), Some(v.domain.as_str()));
            if segments.len() != 2 {
                assert!(
                    segments.len() >= 3
                        && matches!(segments[..2], ["motion", "link"] | ["motion", "job"]),
                    "{} must be domain.verb or an established motion.link.* / motion.job.* connector verb",
                    v.name
                );
            }
        }
        // MCP names are bijective.
        let v = reg.get("transcript.cut_words").unwrap();
        assert_eq!(VerbRegistry::mcp_tool_name(&v.name), "transcript_cut_words");
        assert_eq!(
            reg.verb_for_tool("transcript_cut_words").unwrap().name,
            v.name
        );
    }

    /// Agent-receipt contract: the high-use verbs the autopilot / social
    /// pipeline consume declare a machine-readable result_schema, and each is a
    /// JSON object (a schema fragment or a $ref into receipts.schema.json).
    #[test]
    fn high_use_verbs_declare_result_schema() {
        let reg = VerbRegistry::load();
        for name in [
            "render.final",
            "verify.checks",
            "project.diff",
            "transcript.assemble",
            "transcript.search",
            "comment.draft",
            "comment.apply",
        ] {
            let v = reg
                .get(name)
                .unwrap_or_else(|| unreachable!("missing verb {name}"));
            let rs = v
                .result_schema
                .as_ref()
                .unwrap_or_else(|| unreachable!("{name} must declare result_schema"));
            assert!(rs.is_object(), "{name} result_schema must be a JSON object");
            // Either an inline object schema (has "type") or a typed $ref.
            assert!(
                rs.get("type").is_some() || rs.get("$ref").is_some(),
                "{name} result_schema must be a schema (type) or a $ref"
            );
        }
        // Verbs without a declared contract parse fine (result_schema = None).
        assert!(reg.get("project.state").unwrap().result_schema.is_none());
    }

    #[test]
    fn every_live_input_schema_advertises_shared_mutation_controls() {
        let registry = VerbRegistry::load();
        for verb in &registry.verbs {
            let properties = verb.args["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{} has no args properties", verb.name));
            assert!(properties.contains_key("request_id"), "{}", verb.name);
            assert!(
                properties.contains_key("expected_revision"),
                "{}",
                verb.name
            );
        }
        let public = registry.public_json();
        assert_eq!(public["mutation_controls"]["request_id"]["minLength"], 8);
    }

    #[test]
    fn every_verb_has_one_generated_behavior_and_dispatch_target() {
        let registry = VerbRegistry::load();
        let mut classes = std::collections::BTreeSet::new();
        let mut dispatches = std::collections::BTreeSet::new();
        for verb in &registry.verbs {
            let generated = contract_for_verb(&verb.name)
                .unwrap_or_else(|| panic!("{} is missing generated core metadata", verb.name));
            assert_eq!(
                verb.behavior.mutation_class, generated.mutation_class,
                "{}",
                verb.name
            );
            assert_eq!(
                verb.behavior.side_effects, generated.side_effects,
                "{}",
                verb.name
            );
            assert_eq!(
                verb.behavior.project_state, generated.project_state,
                "{}",
                verb.name
            );
            classes.insert(verb.behavior.mutation_class as u8);
            assert!(
                dispatches.insert(format!("{:?}", verb.behavior.dispatch)),
                "{} shares a dispatch target; each schema verb must have one dispatch arm",
                verb.name
            );
        }
        assert_eq!(dispatches.len(), registry.verbs.len());
        assert_eq!(
            classes.len(),
            6,
            "all mutation classes must remain represented"
        );
    }
}
