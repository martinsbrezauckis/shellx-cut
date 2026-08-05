//! registry.rs — schema/verbs.json loader (public verb contract: "server loads it; MCP
//! tools are generated from it; the coverage audit asserts REST and MCP both
//! expose 100%").
//!
//! Role: embed the verb registry at compile time (single source of truth
//! lives at <repo>/schema/verbs.json), parse once, expose lookups used by
//! REST dispatch (verb exists?) and MCP tools/list (name+schema generation).
//! Dependencies: serde_json. Primary callers: dispatch.rs, mcp.rs, http.rs.

use crate::schema_validation::CompiledVerbSchemas;
use cut_core::CutError;
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
    /// Optional machine-readable JSON Schema for the successful result. Present
    /// on the high-use verbs the autopilot /
    /// social pipeline consume (render.final, verify.checks, project.diff,
    /// transcript.assemble/search, comment.draft/apply); points into
    /// schema/receipts.schema.json for the typed sub-contracts. Forwarded to
    /// MCP clients as `outputSchema` when present.
    #[serde(default)]
    pub result_schema: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RawVerbRegistry {
    schema: String,
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
}

impl VerbRegistry {
    /// Parse and compile the embedded registry. Panics on malformed JSON or an
    /// invalid input schema: both are build-artifact corruption, not caller
    /// errors, and must fail before the first verb can run.
    pub fn load() -> Self {
        Self::try_load().expect("schema/verbs.json and every verb input schema must compile")
    }

    pub fn try_load() -> Result<Self, String> {
        let raw: RawVerbRegistry = serde_json::from_str(VERBS_JSON)
            .map_err(|error| format!("schema/verbs.json must parse: {error}"))?;
        let validators = CompiledVerbSchemas::compile(&raw.verbs)?;
        Ok(Self {
            schema: raw.schema,
            verbs: raw.verbs,
            validators,
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
            260,
            "verb count is a deliberate-contract tripwire: bump this AND README + \
             skill/shellx-cut/reference.md + the Windows harness when verbs.json changes"
        );
        // Spot-check one verb per domain (including canonical renames, jobs,
        // audio, system, comments, title domains, and the
        // creative/zoom additions, so a dropped domain trips the test).
        for name in [
            "project.create",
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
}
