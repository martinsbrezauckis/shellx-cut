//! recipes.rs — built-in pipeline-manifest registry.
//!
//! Role: embed `schema/recipes.json` at compile time and expose the typed
//! recipe model + the small pure helpers the `recipe.*` verbs need
//! (param resolution, `{{param}}` interpolation, the closed `RecipeFacts`
//! vocabulary for state gates). A recipe is a declarative, named, gated
//! WORKFLOW over the existing verbs — an ordered list of stages, each
//! `{verb, args, optional gate}`, parameterised by named params the stage
//! args interpolate. The RUNNER (the orchestration: checkpoint, dispatch,
//! poll, gate, stop-on-fail) lives in dispatch.rs `recipe_run`; this module is
//! the data + the pure functions, mirroring registry.rs (verbs.json loader).
//!
//! Trust: a `#[cfg(test)]` validation test (`recipes_parse_and_reference_real_verbs`)
//! asserts every referenced verb exists in the verb registry, every gate check
//! is a real receipt check name on a `render.final` stage, every gate fact/op
//! is in the closed vocabulary, and every `{{param}}` is a declared param — so a
//! recipe that names a non-existent verb / un-emitted check can never ship
//! (drift fails the build, the recipe analog of the verb-count tripwire).
//!
//! Dependencies: serde_json (parse), cut-core (Project model + edl duration for
//! facts). Primary callers: dispatch.rs (`recipe.list`/`recipe.describe`/`recipe.run`).

use cut_core::Project;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::OnceLock;

/// The embedded recipe source — compiled in so cutd has no runtime file
/// dependency on the repo layout (it may run installed/standalone). Mirrors
/// `registry.rs:VERBS_JSON`. A later wave can layer a `<project>/recipes/*.json`
/// lookup ON TOP of this embedded set; the loader already takes a `&str`.
pub const RECIPES_JSON: &str = include_str!("../../../schema/recipes.json");

/// Music-bed track id convention (mirrors dispatch.rs `MUSIC_BED_TRACK_ID`) —
/// the `has_music` fact looks for a media clip on this audio track.
const MUSIC_BED_TRACK_ID: &str = "music1";

/// The closed `RecipeFacts` vocabulary — the ONLY facts a `state` gate may name.
/// No arbitrary JSON-path access (auditable; can't drift into a query DSL).
/// Public contract surface, consumed by the load-time validation test.
#[allow(dead_code)]
pub const FACT_NAMES: &[&str] = &[
    "duration_ms",
    "transcript_words",
    "caption_clips",
    "video_clips",
    "audio_clips",
    "marker_count",
    "has_music",
];

/// The allowed `state` predicate operators. `*_start` compare against the
/// run-start baseline snapshot; the rest compare against the predicate `value`.
/// Public contract surface, consumed by the load-time validation test.
#[allow(dead_code)]
pub const STATE_OPS: &[&str] = &["gt", "gte", "lt", "lte", "eq", "lt_start", "gt_start"];

/// Parsed recipe registry (top-level mirrors verbs.json: `{schema, recipes}`).
/// `schema` is asserted by the validation test (drift tripwire).
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct RecipeRegistry {
    pub schema: String,
    pub recipes: Vec<Recipe>,
}

/// One recipe — a named, parameterised, ordered list of gated stages.
#[derive(Debug, Clone, Deserialize)]
pub struct Recipe {
    /// Unique kebab-case id; the `recipe.run` key.
    pub name: String,
    /// UI label.
    pub title: String,
    pub description: String,
    /// Named, typed inputs the stage args interpolate (`{{name}}`).
    #[serde(default)]
    pub params: BTreeMap<String, RecipeParam>,
    pub stages: Vec<RecipeStage>,
}

/// A declared recipe parameter (defaults baked here; runtime params override).
#[derive(Debug, Clone, Deserialize)]
pub struct RecipeParam {
    /// JSON type hint ("string" | "integer" | "number" | "boolean").
    #[serde(rename = "type")]
    pub ty: String,
    /// Must be supplied at `recipe.run` (no default).
    #[serde(default)]
    pub required: bool,
    /// Default value applied when the run does not override it.
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub description: Option<String>,
    /// Optional allowed-values enum (surfaced in recipe.describe).
    #[serde(default, rename = "enum")]
    pub allowed: Option<Vec<Value>>,
}

/// One stage — the unit of composition: a verb + its (templated) args + an
/// optional success gate.
#[derive(Debug, Clone, Deserialize)]
pub struct RecipeStage {
    /// Stable identifier, unique within the recipe (run report + errors).
    pub id: String,
    /// A verb name from verbs.json (load-validated to exist).
    pub verb: String,
    /// Verb args with `{{param}}` placeholders. `default` = `{}`.
    #[serde(default = "empty_object")]
    pub args: Value,
    /// Per-stage why; threaded into the sub-op rationale for verbs that accept it.
    #[serde(default)]
    pub rationale: Option<String>,
    /// Poll `jobs.get` to completion when the verb returns `{job_id}`. `None` =
    /// auto-detect (poll iff a `job_id` came back); `Some(false)` = never poll.
    #[serde(default)]
    pub await_job: Option<bool>,
    /// The success gate (checks + state). Absent = "verb ok is enough".
    #[serde(default)]
    pub gate: Option<Gate>,
}

/// A stage's success gate. Both arms optional; the gate passes iff ALL `checks`
/// pass AND ALL `state` predicates hold (AND semantics; no OR).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Gate {
    /// Receipt-based checks (reuse `verify.checks` names) — legal only on a
    /// `render.final` stage (a receipt exists only after a render).
    #[serde(default)]
    pub checks: Vec<String>,
    /// Cheap, render-free assertions over the closed `RecipeFacts` vocabulary.
    #[serde(default)]
    pub state: Vec<StatePredicate>,
}

/// One render-free assertion: `fact <op> value` (or `<op>` vs the run-start
/// baseline for the `*_start` operators).
#[derive(Debug, Clone, Deserialize)]
pub struct StatePredicate {
    pub fact: String,
    pub op: String,
    #[serde(default)]
    pub value: Option<Value>,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

impl RecipeRegistry {
    /// Look up a recipe by exact name.
    pub fn get(&self, name: &str) -> Option<&Recipe> {
        self.recipes.iter().find(|r| r.name == name)
    }
}

/// Parse the embedded registry once (lazy `OnceLock`; zero AppState surgery —
/// recipes are read rarely). Panics on malformed JSON: a build-artifact
/// corruption, not a runtime condition (the validation test gates it at build).
pub fn registry() -> &'static RecipeRegistry {
    static REG: OnceLock<RecipeRegistry> = OnceLock::new();
    REG.get_or_init(|| serde_json::from_str(RECIPES_JSON).expect("schema/recipes.json must parse"))
}

/// Resolve a recipe's params: `defaults ⊕ overrides`. Returns name→Value for
/// every param that has a value. `Err(param_name)` when a REQUIRED param has
/// neither an override nor a default (the caller maps it to INVALID_ARGS naming
/// the param). Unknown override keys are ignored (lenient merge).
pub fn resolve_params(
    recipe: &Recipe,
    overrides: &Value,
) -> Result<BTreeMap<String, Value>, String> {
    let ov = overrides.as_object();
    let mut out = BTreeMap::new();
    for (name, p) in &recipe.params {
        let v = ov
            .and_then(|m| m.get(name))
            .cloned()
            .or_else(|| p.default.clone());
        match v {
            Some(val) => {
                out.insert(name.clone(), val);
            }
            None if p.required => return Err(name.clone()),
            None => {}
        }
    }
    Ok(out)
}

/// If `s` is EXACTLY `{{name}}` (after trimming inner whitespace), return the
/// param name. Deliberately minimal — no partial/embedded substitution
/// (`{{a}}-{{b}}` is NOT a placeholder), no expressions (forecloses an
/// accidental templating language).
fn placeholder(s: &str) -> Option<&str> {
    let inner = s.strip_prefix("{{")?.strip_suffix("}}")?.trim();
    if inner.is_empty() || inner.contains("{{") || inner.contains("}}") {
        return None;
    }
    Some(inner)
}

/// Replace every arg value that is exactly `{{name}}` with the resolved param
/// value (preserving its JSON type — a number param injects a JSON number).
/// Recursive over objects/arrays. A reference to an unresolved param is left
/// verbatim (the downstream verb rejects it) — but the validation test +
/// the "required-or-default" rule guarantee built-ins always resolve.
pub fn interpolate(args: &Value, params: &BTreeMap<String, Value>) -> Value {
    match args {
        Value::String(s) => match placeholder(s).and_then(|n| params.get(n)) {
            Some(v) => v.clone(),
            None => args.clone(),
        },
        Value::Array(a) => Value::Array(a.iter().map(|v| interpolate(v, params)).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| (k.clone(), interpolate(v, params)))
                .collect(),
        ),
        _ => args.clone(),
    }
}

/// Collect every `{{param}}` name referenced anywhere in `args` (validation +
/// auditing). Same exact-match rule as [`interpolate`]. Consumed by the
/// load-time validation test (every referenced param must be declared).
#[allow(dead_code)]
pub fn referenced_params(args: &Value, out: &mut BTreeSet<String>) {
    match args {
        Value::String(s) => {
            if let Some(n) = placeholder(s) {
                out.insert(n.to_string());
            }
        }
        Value::Array(a) => a.iter().for_each(|v| referenced_params(v, out)),
        Value::Object(m) => m.values().for_each(|v| referenced_params(v, out)),
        _ => {}
    }
}

/// The closed-vocabulary derived facts a `state` gate asserts over. Computed
/// from the live project (+ the project dir for the on-disk transcript word
/// count). Integer-valued for uniform comparison (`has_music` → 0/1).
#[derive(Debug, Clone)]
pub struct RecipeFacts {
    pub duration_ms: i64,
    pub transcript_words: i64,
    pub caption_clips: i64,
    pub video_clips: i64,
    pub audio_clips: i64,
    pub marker_count: i64,
    pub has_music: bool,
}

impl RecipeFacts {
    /// Compute the closed-vocab facts. `dir` is the open project's directory —
    /// needed because the transcript word COUNT lives on disk
    /// (`receipts/<asset>.words.json`), not in the `Project` struct.
    ///
    /// `transcript_words` = the LARGEST transcript word count among the project's
    /// assets (a transcribed asset present ⇒ > 0) — the deterministic reading of
    /// "the targeted/most-recent asset transcript" for a `transcript_words gt 0`
    /// gate after a transcribe stage.
    pub fn compute(project: &Project, dir: &Path) -> Self {
        use cut_core::{Clip, TrackKind};
        let duration_ms = cut_core::edl_from_project(project).duration_ms as i64;
        let transcript_words = project
            .assets
            .values()
            .filter_map(|a| a.transcript.as_ref())
            .filter_map(|rel| std::fs::read_to_string(dir.join(rel)).ok())
            .filter_map(|s| serde_json::from_str::<Value>(&s).ok())
            .filter_map(|v| {
                v.get("words")
                    .and_then(|w| w.as_array())
                    .map(|a| a.len() as i64)
            })
            .max()
            .unwrap_or(0);

        let (mut caption_clips, mut video_clips, mut audio_clips, mut has_music) = (0, 0, 0, false);
        for t in &project.tracks {
            let media = t
                .clips
                .iter()
                .filter(|c| matches!(c, Clip::Media(_)))
                .count() as i64;
            match t.kind {
                TrackKind::Caption => {
                    caption_clips += t
                        .clips
                        .iter()
                        .filter(|c| matches!(c, Clip::Caption(_)))
                        .count() as i64
                }
                TrackKind::Video => video_clips += media,
                TrackKind::Audio => {
                    audio_clips += media;
                    if t.id == MUSIC_BED_TRACK_ID && media > 0 {
                        has_music = true;
                    }
                }
            }
        }
        Self {
            duration_ms,
            transcript_words,
            caption_clips,
            video_clips,
            audio_clips,
            marker_count: project.markers.len() as i64,
            has_music,
        }
    }

    /// Look up one fact by name (None for an unknown fact — the gate evaluator
    /// turns that into a failing predicate, never a panic).
    pub fn get(&self, fact: &str) -> Option<i64> {
        Some(match fact {
            "duration_ms" => self.duration_ms,
            "transcript_words" => self.transcript_words,
            "caption_clips" => self.caption_clips,
            "video_clips" => self.video_clips,
            "audio_clips" => self.audio_clips,
            "marker_count" => self.marker_count,
            "has_music" => i64::from(self.has_music),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::VerbRegistry;

    /// The valid receipt check names (the cut_core source of truth) — the closed
    /// set a `checks` gate may name (mirrors receipts.schema.json CheckResult.name).
    fn receipt_check_names() -> BTreeSet<&'static str> {
        use cut_core::check_names as cn;
        [
            cn::CUT_ON_WORD,
            cn::CUT_ON_BEAT,
            cn::J_L_CUT,
            cn::BED_DUCK_UNDER_SPEECH,
            cn::CROSSFADE_SMOOTHNESS,
            cn::LUFS,
            cn::CAPTION_PRESENCE,
            cn::BLACK_OR_FROZEN_FRAMES,
            cn::SILENCE_AT_EDGES,
            cn::DURATION_MATCHES_EDL,
            cn::UNIFORM_BORDER,
            cn::FOOTAGE_PROFILE,
        ]
        .into_iter()
        .collect()
    }

    /// The load-time gate — drift fails the build. Asserts: (a) recipes.json
    /// parses; (b) recipe count == 11 (tripwire); (c) every stage verb exists in
    /// the verb registry; (d) every gate check is a real receipt check name AND
    /// its stage's verb == render.final; (e) every gate fact ∈ the closed
    /// RecipeFacts vocab and op ∈ the allowed set (with the value-presence rule);
    /// (f) every `{{param}}` referenced in a stage's args is a declared param.
    /// This is what makes the manifest TRUSTWORTHY — a recipe that names a
    /// non-existent verb or an un-emitted check can never ship.
    #[test]
    fn recipes_parse_and_reference_real_verbs() {
        let reg = registry();
        assert_eq!(reg.schema, "shellx-cut/recipes/1");
        assert_eq!(
            reg.recipes.len(),
            11,
            "recipe count is a contract tripwire: bump README + skill/shellx-cut/reference.md \
             when schema/recipes.json changes"
        );
        let verbs = VerbRegistry::load();
        let checks = receipt_check_names();
        let facts: BTreeSet<&str> = FACT_NAMES.iter().copied().collect();
        let ops: BTreeSet<&str> = STATE_OPS.iter().copied().collect();

        let mut recipe_names = BTreeSet::new();
        for r in &reg.recipes {
            assert!(
                recipe_names.insert(r.name.clone()),
                "duplicate recipe name {}",
                r.name
            );
            // Every param is required OR has a default → a referenced param always
            // resolves at run time (no dangling `{{name}}`).
            for (pn, p) in &r.params {
                assert!(
                    p.required || p.default.is_some(),
                    "{}: param '{}' must be required or carry a default",
                    r.name,
                    pn
                );
            }
            let mut stage_ids = BTreeSet::new();
            for st in &r.stages {
                assert!(
                    stage_ids.insert(st.id.clone()),
                    "{}: duplicate stage id '{}'",
                    r.name,
                    st.id
                );
                // (c) verb exists.
                assert!(
                    verbs.get(&st.verb).is_some(),
                    "{}/{}: references verb '{}' which is not in verbs.json",
                    r.name,
                    st.id,
                    st.verb
                );
                // (f) every referenced param is declared.
                let mut refs = BTreeSet::new();
                referenced_params(&st.args, &mut refs);
                for rp in &refs {
                    assert!(
                        r.params.contains_key(rp),
                        "{}/{}: args reference undeclared param '{{{{{}}}}}'",
                        r.name,
                        st.id,
                        rp
                    );
                }
                if let Some(g) = &st.gate {
                    // (d) checks: a real receipt check, only on a render.final stage.
                    for c in &g.checks {
                        assert!(
                            checks.contains(c.as_str()),
                            "{}/{}: gate check '{}' is not a receipt check name",
                            r.name,
                            st.id,
                            c
                        );
                        assert_eq!(
                            st.verb, "render.final",
                            "{}/{}: a checks-gate is only valid on render.final (a receipt \
                             exists only after a render)",
                            r.name, st.id
                        );
                    }
                    // (e) state: closed fact + allowed op; value present iff not a *_start op.
                    for sp in &g.state {
                        assert!(
                            facts.contains(sp.fact.as_str()),
                            "{}/{}: gate fact '{}' is not in the closed RecipeFacts vocab",
                            r.name,
                            st.id,
                            sp.fact
                        );
                        assert!(
                            ops.contains(sp.op.as_str()),
                            "{}/{}: gate op '{}' is not allowed",
                            r.name,
                            st.id,
                            sp.op
                        );
                        let baseline_op = sp.op.ends_with("_start");
                        assert_eq!(
                            sp.value.is_some(),
                            !baseline_op,
                            "{}/{}: op '{}' must {} a value",
                            r.name,
                            st.id,
                            sp.op,
                            if baseline_op { "omit" } else { "carry" }
                        );
                    }
                }
            }
        }
        // The shipped built-ins are present.
        for n in [
            "first-project",
            "edit-for-clarity",
            "podcast-repurpose",
            "talking-head-cleanup",
            "screen-demo-polish",
            "phone-clip-cleanup",
            "social-short-bundle",
            "area-privacy-mask",
            "add-captions",
            "youtube-export",
            "tiktok-export",
        ] {
            assert!(reg.get(n).is_some(), "missing built-in recipe {n}");
        }
    }

    #[test]
    fn edit_for_clarity_resolves_intensity_and_conservative_retakes() {
        let recipe = registry().get("edit-for-clarity").unwrap();
        let params = resolve_params(
            recipe,
            &serde_json::json!({"asset":"a1","intensity":"jumpy"}),
        )
        .unwrap();
        let resolved: Vec<_> = recipe
            .stages
            .iter()
            .map(|stage| (stage.id.as_str(), interpolate(&stage.args, &params)))
            .collect();

        let retakes = &resolved.iter().find(|(id, _)| *id == "retakes").unwrap().1;
        assert_eq!(retakes["similarity"], serde_json::json!(0.72));
        assert_eq!(retakes["keep"], "last");
        let tighten = &resolved.iter().find(|(id, _)| *id == "tighten").unwrap().1;
        assert_eq!(tighten["asset"], "a1");
        assert_eq!(tighten["aggressiveness"], "jumpy");
    }

    /// Interpolation is exact-match only and type-preserving.
    #[test]
    fn interpolate_is_exact_match_and_type_preserving() {
        let mut p = BTreeMap::new();
        p.insert("asset".to_string(), serde_json::json!("a1"));
        p.insert("target_lufs".to_string(), serde_json::json!(-16));
        let args = serde_json::json!({
            "asset": "{{asset}}",
            "normalize_loudness": "{{target_lufs}}",
            "literal": "{{asset}}-x",   // partial → NOT substituted
            "nested": { "k": "{{asset}}" }
        });
        let out = interpolate(&args, &p);
        assert_eq!(out["asset"], serde_json::json!("a1"));
        // Type preserved: a number param injects a JSON number, not a string.
        assert_eq!(out["normalize_loudness"], serde_json::json!(-16));
        assert!(out["normalize_loudness"].is_i64());
        assert_eq!(out["literal"], serde_json::json!("{{asset}}-x"));
        assert_eq!(out["nested"]["k"], serde_json::json!("a1"));
    }

    /// resolve_params: defaults applied, overrides win, missing required errors.
    #[test]
    fn resolve_params_defaults_overrides_and_required() {
        let r = registry().get("podcast-repurpose").unwrap();
        // Missing required `asset` → Err naming it.
        assert_eq!(
            resolve_params(r, &serde_json::json!({})),
            Err("asset".to_string())
        );
        // Default applied; override wins.
        let p = resolve_params(
            r,
            &serde_json::json!({ "asset": "a1", "aggressiveness": "jumpy" }),
        )
        .unwrap();
        assert_eq!(p.get("asset"), Some(&serde_json::json!("a1")));
        assert_eq!(p.get("aggressiveness"), Some(&serde_json::json!("jumpy")));
        assert_eq!(p.get("target_lufs"), Some(&serde_json::json!(-16)));
    }
}
