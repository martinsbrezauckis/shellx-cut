#!/usr/bin/env node
/**
 * Generate the Rust consumers of schema/verbs.json behavior metadata.
 *
 * The schema is authoritative.  This script deliberately has no behavior
 * tables of its own: it validates and renders the complete per-verb contract
 * supplied by schema/verbs.json.  `--check` makes generated-artifact drift a
 * normal test failure rather than a runtime surprise.
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CHECK = process.argv.slice(2).join(" ") === "--check";
const schemaPath = resolve(ROOT, "schema/verbs.json");
const corePath = resolve(ROOT, "app/core/src/verb_contract.rs");
const serverPath = resolve(ROOT, "app/server/src/verb_contract.rs");
const uiPath = resolve(ROOT, "ui/src/lib/generatedVerbBehavior.ts");
const jsonPath = resolve(ROOT, "schema/generated/verb-behavior.json");

const semanticKinds = {
  idempotency_modes: ["idempotency", "Idempotency"],
  replayability_modes: ["replayability", "Replayability"],
  async_job_types: ["async_job", "AsyncJobType"],
  ui_exposures: ["ui_exposure", "UiExposure"],
  agent_chat_capabilities: ["agent_chat", "AgentChatCapability"],
  risk_levels: ["risk", "VerbRisk"],
  facets: ["facets", "VerbFacet"],
};

const rustVariant = (value) => {
  const result = value
    .split(/[^a-zA-Z0-9]+/)
    .filter(Boolean)
    .map((part) => `${part[0].toUpperCase()}${part.slice(1)}`)
    .join("");
  if (!/^[A-Z][A-Za-z0-9]*$/.test(result)) {
    throw new Error(`cannot turn '${value}' into a Rust enum variant`);
  }
  return result;
};

const rustString = (value) => JSON.stringify(value);

function requireArray(value, label) {
  if (!Array.isArray(value) || value.length === 0 || value.some((item) => typeof item !== "string" || !item)) {
    throw new Error(`${label} must be a non-empty string array`);
  }
  if (new Set(value).size !== value.length) {
    throw new Error(`${label} contains duplicates`);
  }
  return value;
}

function requireBehavior(schema) {
  const contract = schema.behavior_contract;
  if (!contract || typeof contract !== "object" || Array.isArray(contract)) {
    throw new Error("schema/verbs.json behavior_contract must be an object");
  }
  const mutationClasses = requireArray(contract.mutation_classes, "behavior_contract.mutation_classes");
  const projectStates = requireArray(contract.project_states, "behavior_contract.project_states");
  const sideEffectFlags = requireArray(contract.side_effect_flags, "behavior_contract.side_effect_flags");
  const semantics = Object.fromEntries(
    Object.entries(semanticKinds).map(([contractKey]) => [
      contractKey,
      requireArray(contract[contractKey], `behavior_contract.${contractKey}`),
    ]),
  );
  if (!mutationClasses.includes("timeline")) {
    throw new Error("behavior_contract.mutation_classes must include 'timeline'");
  }
  if (!projectStates.includes("required")) {
    throw new Error("behavior_contract.project_states must include 'required'");
  }
  const names = new Set();
  const dispatches = new Set();
  for (const verb of schema.verbs) {
    if (!verb || typeof verb !== "object" || typeof verb.name !== "string" || !verb.name) {
      throw new Error("every schema verb needs a non-empty name");
    }
    if (names.has(verb.name)) throw new Error(`duplicate verb '${verb.name}'`);
    names.add(verb.name);
    const behavior = verb.behavior;
    if (!behavior || typeof behavior !== "object" || Array.isArray(behavior)) {
      throw new Error(`${verb.name}: behavior must be an object`);
    }
    const allowed = new Set([
      "mutation_class", "side_effects", "dispatch", "project_state",
      "idempotency", "replayability", "async_job", "ui_exposure", "agent_chat", "risk", "facets",
    ]);
    for (const key of Object.keys(behavior)) {
      if (!allowed.has(key)) throw new Error(`${verb.name}: unknown behavior field '${key}'`);
    }
    if (!mutationClasses.includes(behavior.mutation_class)) {
      throw new Error(`${verb.name}: invalid mutation_class '${behavior.mutation_class}'`);
    }
    if (!projectStates.includes(behavior.project_state)) {
      throw new Error(`${verb.name}: invalid project_state '${behavior.project_state}'`);
    }
    for (const [contractKey, [field]] of Object.entries(semanticKinds)) {
      if (field !== "facets" && !semantics[contractKey].includes(behavior[field])) {
        throw new Error(`${verb.name}: invalid ${field} '${behavior[field]}'`);
      }
    }
    if (!Array.isArray(behavior.facets) || behavior.facets.some((facet) => !semantics.facets.includes(facet)) || new Set(behavior.facets).size !== behavior.facets.length) {
      throw new Error(`${verb.name}: facets must be a unique subset of behavior_contract.facets`);
    }
    if (typeof behavior.dispatch !== "string" || !/^[a-z][a-z0-9_]*$/.test(behavior.dispatch)) {
      throw new Error(`${verb.name}: dispatch must be a lower_snake_case handler id`);
    }
    if (dispatches.has(behavior.dispatch)) {
      throw new Error(`${verb.name}: dispatch '${behavior.dispatch}' is shared; each public verb needs one dispatch arm`);
    }
    dispatches.add(behavior.dispatch);
    if (!behavior.side_effects || typeof behavior.side_effects !== "object" || Array.isArray(behavior.side_effects)) {
      throw new Error(`${verb.name}: side_effects must be an object`);
    }
    const sideEffectKeys = Object.keys(behavior.side_effects).sort();
    if (sideEffectKeys.join(",") !== [...sideEffectFlags].sort().join(",")) {
      throw new Error(`${verb.name}: side_effects must declare exactly ${sideEffectFlags.join(", ")}`);
    }
    for (const flag of sideEffectFlags) {
      if (typeof behavior.side_effects[flag] !== "boolean") {
        throw new Error(`${verb.name}: side_effects.${flag} must be boolean`);
      }
    }
    const resultMentionsJobId = /job_id/.test(verb.result) || JSON.stringify(verb.result_schema ?? {}).includes("job_id");
    // A job *starter* owns the returned job handle; a status/list verb can also
    // describe a job record without starting one.  Do not infer ownership from
    // the result shape in the reverse direction (jobs.status is the important
    // counterexample).  A declared starter must still return a job id.
    if (behavior.async_job !== "none" && !resultMentionsJobId) {
      throw new Error(`${verb.name}: async_job '${behavior.async_job}' must return a job_id contract`);
    }
    if (behavior.mutation_class === "external_side_effect" && behavior.replayability === "replayable") {
      throw new Error(`${verb.name}: external side effect cannot be replayable`);
    }
    if (behavior.replayability === "replayable" && !["project_metadata", "asset_metadata", "timeline"].includes(behavior.mutation_class)) {
      throw new Error(`${verb.name}: only durable project mutations may be journal-replayable`);
    }
    if (behavior.mutation_class === "read") {
      if (behavior.replayability !== "not_applicable") {
        throw new Error(`${verb.name}: read-only verbs cannot be journal-replayable`);
      }
      if (behavior.async_job !== "none") {
        throw new Error(`${verb.name}: read-only verbs cannot claim to start a background job`);
      }
      if (behavior.idempotency === "request_key") {
        throw new Error(`${verb.name}: read-only verbs cannot require a durable request key`);
      }
    }
    if (behavior.risk === "destructive" && ["project_metadata", "asset_metadata", "timeline"].includes(behavior.mutation_class) && behavior.idempotency !== "request_key") {
      throw new Error(`${verb.name}: destructive mutation requires request_key idempotency`);
    }
    if (["reversible", "destructive"].includes(behavior.risk) && behavior.mutation_class === "read") {
      throw new Error(`${verb.name}: read-only verbs cannot claim mutation risk '${behavior.risk}'`);
    }
    if (behavior.agent_chat === "inspect" && behavior.mutation_class !== "read") {
      throw new Error(`${verb.name}: inspect agent capability must be read-only`);
    }
    if (behavior.agent_chat === "edit") {
      if (!["project_metadata", "asset_metadata", "timeline"].includes(behavior.mutation_class)
        || behavior.idempotency !== "request_key"
        || behavior.replayability !== "replayable"
        || behavior.side_effects.network) {
        throw new Error(`${verb.name}: agent edit must be a request-keyed replayable project mutation without network access`);
      }
    }
    // `agent_chat` is a separate, broker-enforced capability. A safe Cut verb
    // may still use a bounded project asset or local helper internally (for
    // example, colour analysis invokes ffmpeg against registered clip paths).
    // Do not falsify those engine interactions merely to make a provider
    // containment policy look simpler.
  }
  return { mutationClasses, projectStates, sideEffectFlags, semantics };
}

function generateCore(schema, kinds) {
  const enumBlock = (name, values) => `#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]\n#[serde(rename_all = "snake_case")]\npub enum ${name} {\n${values.map((value) => `    ${rustVariant(value)},`).join("\n")}\n}`;
  const contracts = schema.verbs
    .map((verb) => {
      const b = verb.behavior;
      const flags = kinds.sideEffectFlags.map((flag) => `${flag}: ${b.side_effects[flag]}`).join(", ");
      return `        ${rustString(verb.name)} => Some(VerbContract { mutation_class: MutationClass::${rustVariant(b.mutation_class)}, side_effects: SideEffects { ${flags} }, project_state: ProjectState::${rustVariant(b.project_state)} }),`;
    })
    .join("\n");
  return `// @generated by scripts/generate-verb-contract.mjs; DO NOT EDIT.\n//! Schema-derived verb semantics for core replay and history.\n\n${enumBlock("MutationClass", kinds.mutationClasses)}\n\nimpl MutationClass {\n    pub const fn mutates_timeline(self) -> bool {\n        matches!(self, Self::Timeline)\n    }\n}\n\n${enumBlock("ProjectState", kinds.projectStates)}\n\n#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]\n#[serde(deny_unknown_fields)]\npub struct SideEffects {\n${kinds.sideEffectFlags.map((flag) => `    pub ${flag}: bool,`).join("\n")}\n}\n\n#[derive(Debug, Clone, Copy, PartialEq, Eq)]\npub struct VerbContract {\n    pub mutation_class: MutationClass,\n    pub side_effects: SideEffects,\n    pub project_state: ProjectState,\n}\n\n/// Generated total mapping for the public registry. Unknown verbs intentionally\n/// return None so journal replay and undo fail closed instead of guessing.\n#[rustfmt::skip]\npub fn contract_for_verb(name: &str) -> Option<VerbContract> {\n    match name {\n${contracts}\n        _ => None,\n    }\n}\n`;
}

function generateServer(schema) {
  const targets = schema.verbs.map((verb) => `    ${rustVariant(verb.behavior.dispatch)},`).join("\n");
  return `// @generated by scripts/generate-verb-contract.mjs; DO NOT EDIT.\n//! Schema-derived dispatch targets. The dispatcher must exhaustively match this enum.\n\n#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]\n#[serde(rename_all = "snake_case")]\npub enum DispatchTarget {\n${targets}\n}\n`;
}

function generateExpandedCore(schema, kinds) {
  const enumBlock = (name, values) => `#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]\n#[serde(rename_all = "snake_case")]\npub enum ${name} {\n${values.map((value) => `    ${rustVariant(value)},`).join("\n")}\n}`;
  const semanticEnums = Object.entries(semanticKinds)
    .map(([contractKey, [, enumName]]) => enumBlock(enumName, kinds.semantics[contractKey]))
    .join("\n\n");
  let source = generateCore(schema, kinds);
  source = source.replace(
    `${enumBlock("ProjectState", kinds.projectStates)}\n\n`,
    `${enumBlock("ProjectState", kinds.projectStates)}\n\n${semanticEnums}\n\n`,
  );
  source = source.replace(
    "    pub project_state: ProjectState,\n}",
    "    pub project_state: ProjectState,\n    pub idempotency: Idempotency,\n    pub replayability: Replayability,\n    pub async_job: AsyncJobType,\n    pub ui_exposure: UiExposure,\n    pub agent_chat: AgentChatCapability,\n    pub risk: VerbRisk,\n    pub facets: &'static [VerbFacet],\n}",
  );
  for (const verb of schema.verbs) {
    const b = verb.behavior;
    const flags = kinds.sideEffectFlags.map((flag) => `${flag}: ${b.side_effects[flag]}`).join(", ");
    const original = `        ${rustString(verb.name)} => Some(VerbContract { mutation_class: MutationClass::${rustVariant(b.mutation_class)}, side_effects: SideEffects { ${flags} }, project_state: ProjectState::${rustVariant(b.project_state)} }),`;
    const facets = b.facets.map((facet) => `VerbFacet::${rustVariant(facet)}`).join(", ");
    const expanded = `        ${rustString(verb.name)} => Some(VerbContract { mutation_class: MutationClass::${rustVariant(b.mutation_class)}, side_effects: SideEffects { ${flags} }, project_state: ProjectState::${rustVariant(b.project_state)}, idempotency: Idempotency::${rustVariant(b.idempotency)}, replayability: Replayability::${rustVariant(b.replayability)}, async_job: AsyncJobType::${rustVariant(b.async_job)}, ui_exposure: UiExposure::${rustVariant(b.ui_exposure)}, agent_chat: AgentChatCapability::${rustVariant(b.agent_chat)}, risk: VerbRisk::${rustVariant(b.risk)}, facets: &[${facets}] }),`;
    if (!source.includes(original)) throw new Error(`${verb.name}: core generator lost its base contract line`);
    source = source.replace(original, expanded);
  }
  return source;
}

function generateUi(schema, kinds) {
  const union = (values) => values.map((value) => JSON.stringify(value)).join(" | ");
  const behavior = Object.fromEntries(schema.verbs.map((verb) => [verb.name, verb.behavior]));
  return `// @generated by scripts/generate-verb-contract.mjs; DO NOT EDIT.\nexport type VerbBehavior = { mutation_class: ${union(kinds.mutationClasses)}; project_state: ${union(kinds.projectStates)}; side_effects: { ${kinds.sideEffectFlags.map((flag) => `${flag}: boolean`).join("; ")} }; dispatch: string; idempotency: ${union(kinds.semantics.idempotency_modes)}; replayability: ${union(kinds.semantics.replayability_modes)}; async_job: ${union(kinds.semantics.async_job_types)}; ui_exposure: ${union(kinds.semantics.ui_exposures)}; agent_chat: ${union(kinds.semantics.agent_chat_capabilities)}; risk: ${union(kinds.semantics.risk_levels)}; facets: Array<${union(kinds.semantics.facets)}> }\nexport const VERB_BEHAVIOR: Record<string, VerbBehavior> = ${JSON.stringify(behavior, null, 2)}\nexport const VERB_NAMES = Object.freeze(Object.keys(VERB_BEHAVIOR))\n`;
}

function generateJson(schema) {
  const verbs = schema.verbs.map(({ name, behavior }) => ({ name, behavior }));
  return `${JSON.stringify({ schema: "shellx-cut/verb-behavior/2", behavior_contract: schema.behavior_contract, verbs }, null, 2)}\n`;
}

function writeOrCheck(path, expected) {
  const current = existsSync(path) ? readFileSync(path, "utf8") : "";
  if (CHECK) {
    if (current !== expected) throw new Error(`${path} is stale; run node scripts/generate-verb-contract.mjs`);
    return;
  }
  if (current !== expected) {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, expected);
  }
}

try {
  const schema = JSON.parse(readFileSync(schemaPath, "utf8"));
  if (!Array.isArray(schema.verbs) || schema.verbs.length === 0) {
    throw new Error("schema/verbs.json verbs must be a non-empty array");
  }
  const kinds = requireBehavior(schema);
  writeOrCheck(corePath, generateExpandedCore(schema, kinds));
  writeOrCheck(serverPath, generateServer(schema));
  writeOrCheck(uiPath, generateUi(schema, kinds));
  writeOrCheck(jsonPath, generateJson(schema));
  process.stdout.write(`verb contract ${CHECK ? "is current" : "generated"} (${schema.verbs.length} verbs)\n`);
} catch (error) {
  process.stderr.write(`generate-verb-contract: ${error.message}\n`);
  process.exitCode = 1;
}
