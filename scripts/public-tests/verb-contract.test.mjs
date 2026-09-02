#!/usr/bin/env node
/** Verify schema-generated behavior metadata is total and dispatcher-complete. */
import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { checkVerbAggregate } from "../lib/verb-fragment-contract.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
checkVerbAggregate(ROOT);
const read = (relative) => readFileSync(resolve(ROOT, relative), "utf8");
const schema = JSON.parse(read("schema/verbs.json"));
const core = read("app/core/src/verb_contract.rs");
const targets = read("app/server/src/verb_contract.rs");
const dispatch = read("app/server/src/dispatch.rs");
const variant = (value) => value
  .split(/[^a-zA-Z0-9]+/)
  .filter(Boolean)
  .map((part) => `${part[0].toUpperCase()}${part.slice(1)}`)
  .join("");

const contract = schema.behavior_contract;
assert.ok(contract && typeof contract === "object", "behavior_contract is required");
const behaviorGenerator = spawnSync(process.execPath, ["scripts/generate-verb-contract.mjs", "--check"], {
  cwd: ROOT,
  encoding: "utf8",
});
assert.equal(behaviorGenerator.status, 0, `verb behavior artifacts must be current: ${behaviorGenerator.stderr}`);
for (const key of [
  "mutation_classes", "project_states", "side_effect_flags", "idempotency_modes",
  "replayability_modes", "async_job_types", "ui_exposures", "agent_chat_capabilities",
  "risk_levels", "facets",
]) {
  assert.ok(Array.isArray(contract[key]) && contract[key].length > 0, `${key} must be non-empty`);
}

const names = new Set();
const dispatches = new Set();
for (const verb of schema.verbs) {
  assert.equal(names.has(verb.name), false, `duplicate verb ${verb.name}`);
  names.add(verb.name);
  const behavior = verb.behavior;
  assert.deepEqual(
    Object.keys(behavior).sort(),
    [
      "agent_chat", "async_job", "dispatch", "facets", "idempotency", "mutation_class",
      "project_state", "replayability", "risk", "side_effects", "ui_exposure",
    ],
    `${verb.name}: behavior fields`,
  );
  assert.ok(contract.mutation_classes.includes(behavior.mutation_class), `${verb.name}: mutation class`);
  assert.ok(contract.project_states.includes(behavior.project_state), `${verb.name}: project state`);
  assert.deepEqual(Object.keys(behavior.side_effects).sort(), [...contract.side_effect_flags].sort(), `${verb.name}: effects`);
  assert.ok(Object.values(behavior.side_effects).every((value) => typeof value === "boolean"), `${verb.name}: effect values`);
  assert.ok(contract.idempotency_modes.includes(behavior.idempotency), `${verb.name}: idempotency`);
  assert.ok(contract.replayability_modes.includes(behavior.replayability), `${verb.name}: replayability`);
  assert.ok(contract.async_job_types.includes(behavior.async_job), `${verb.name}: async job type`);
  assert.ok(contract.ui_exposures.includes(behavior.ui_exposure), `${verb.name}: UI exposure`);
  assert.ok(contract.agent_chat_capabilities.includes(behavior.agent_chat), `${verb.name}: agent capability`);
  assert.ok(contract.risk_levels.includes(behavior.risk), `${verb.name}: risk`);
  assert.ok(Array.isArray(behavior.facets), `${verb.name}: facets array`);
  assert.ok(behavior.facets.every((facet) => contract.facets.includes(facet)), `${verb.name}: known facets`);
  assert.equal(new Set(behavior.facets).size, behavior.facets.length, `${verb.name}: unique facets`);
  assert.equal(
    !(behavior.mutation_class === "external_side_effect" && behavior.replayability === "replayable"),
    true,
    `${verb.name}: external side effect cannot be replayable`,
  );
  assert.equal(
    !(behavior.replayability === "replayable" && !["project_metadata", "asset_metadata", "timeline"].includes(behavior.mutation_class)),
    true,
    `${verb.name}: only durable project mutations may be journal-replayable`,
  );
  if (behavior.mutation_class === "read") {
    assert.equal(behavior.replayability, "not_applicable", `${verb.name}: reads cannot be journal-replayed`);
    assert.equal(behavior.async_job, "none", `${verb.name}: reads do not start jobs`);
    assert.notEqual(behavior.idempotency, "request_key", `${verb.name}: reads do not require durable request keys`);
    assert.equal(["reversible", "destructive"].includes(behavior.risk), false, `${verb.name}: reads do not claim mutation risk`);
  }
  if (behavior.agent_chat === "inspect") {
    assert.equal(behavior.mutation_class, "read", `${verb.name}: inspect is read-only`);
  }
  if (behavior.agent_chat === "edit") {
    assert.ok(["project_metadata", "asset_metadata", "timeline"].includes(behavior.mutation_class), `${verb.name}: agent edit is durable`);
    assert.equal(behavior.idempotency, "request_key", `${verb.name}: agent edit is request-keyed`);
    assert.equal(behavior.replayability, "replayable", `${verb.name}: agent edit is replayable`);
    assert.equal(behavior.side_effects.network, false, `${verb.name}: agent edit has no network`);
  }
  assert.equal(
    !(behavior.risk === "destructive" && ["project_metadata", "asset_metadata", "timeline"].includes(behavior.mutation_class) && behavior.idempotency !== "request_key"),
    true,
    `${verb.name}: destructive mutation requires request_key`,
  );
  assert.equal(dispatches.has(behavior.dispatch), false, `${verb.name}: dispatch target must be unique`);
  dispatches.add(behavior.dispatch);
  assert.match(core, new RegExp(`^\\s*"${verb.name.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")}" =>`, "m"), `${verb.name}: core contract`);
  assert.match(targets, new RegExp(`^\\s*${variant(behavior.dispatch)},$`, "m"), `${verb.name}: generated target`);
}
assert.equal(dispatches.size, schema.verbs.length);

const dispatchStart = /let mut result(?:: VerbResult)? = match dispatch_target \{/.exec(dispatch);
const start = dispatchStart?.index ?? -1;
const end = dispatch.indexOf("\n    };", start);
assert.ok(start >= 0 && end > start, "dispatcher must match generated targets");
const dispatchMatch = dispatch.slice(start, end);
assert.doesNotMatch(dispatchMatch, /=>\s*VerbResult::err/, "dispatcher must not hide an unmatched target fallback");
for (const target of dispatches) {
  assert.match(dispatchMatch, new RegExp(`DispatchTarget::${variant(target)}\\b`), `${target}: dispatch arm`);
}

// Representative source-backed contracts prevent metadata from being made
// cosmetically pure to satisfy policy rules. These are intentionally handler
// markers rather than a duplicate dispatch implementation: schema remains the
// complete registry, while each row pins a meaningful I/O/ownership boundary.
const behaviorByName = new Map(schema.verbs.map((verb) => [verb.name, verb.behavior]));
const handlerEvidence = [
  ["project.list", "app/server/src/dispatch/project_workspace.rs", "projects_index::list", { mutation_class: "read", idempotency: "natural", replayability: "not_applicable", risk: "none", side_effects: { filesystem: true } }],
  ["jobs.status", "app/server/src/dispatch/jobs_handlers.rs", "state.jobs.get", { mutation_class: "read", idempotency: "natural", replayability: "not_applicable", async_job: "none", risk: "none" }],
  ["media.waveform", "app/media/src/waveform.rs", "Command::new(ffmpeg_bin())", { mutation_class: "read", idempotency: "none", risk: "external", side_effects: { filesystem: true, process: true } }],
  ["media.index_status", "app/server/src/vissearch.rs", "std::fs::read_to_string", { mutation_class: "read", idempotency: "natural", risk: "none", side_effects: { filesystem: true } }],
  ["media.search", "app/server/src/dispatch/edit_tools/assets_plugins.rs", "--embed-text", { mutation_class: "read", idempotency: "none", risk: "external", side_effects: { filesystem: true, process: true } }],
  ["assets.providers", "app/server/src/dispatch/edit_tools/assets_plugins.rs", "crate::providers::provider_info()", { mutation_class: "read", idempotency: "natural", risk: "none", side_effects: { filesystem: false, network: false } }],
  ["assets.search", "app/server/src/dispatch/edit_tools/assets_plugins.rs", "crate::providers::search", { mutation_class: "read", idempotency: "none", risk: "external", side_effects: { filesystem: true, network: true } }],
  ["verify.checks", "app/server/src/dispatch/verify_handlers.rs", "std::fs::read_to_string", { mutation_class: "read", idempotency: "natural", risk: "none", side_effects: { filesystem: true } }],
  ["verify.loudness", "app/media/src/loudness.rs", "Command::new(ffmpeg_bin())", { mutation_class: "read", idempotency: "none", risk: "external", side_effects: { filesystem: true, process: true } }],
  ["verify.scopes", "app/server/src/dispatch/verify_handlers.rs", "std::fs::write(&frame_path", { mutation_class: "external_side_effect", idempotency: "none", risk: "external", side_effects: { filesystem: true, process: true } }],
  ["motion.map_import", "app/server/src/motion_bridge.rs", "tokio::fs::read(&plan_path)", { mutation_class: "read", idempotency: "natural", risk: "none", side_effects: { filesystem: true } }],
  ["motion.link.tracking.verify", "app/server/src/motion_tracking/handlers.rs", "command::verify(", { mutation_class: "read", idempotency: "none", risk: "external", side_effects: { filesystem: true, process: true } }],
  ["screen_record.doctor", "app/server/src/screen_record/doctor_projection.rs", "warm_mic", { mutation_class: "read", idempotency: "none", risk: "external", side_effects: { filesystem: true, process: false, ui: true } }],
  ["screen_record.recovery_status", "app/server/src/screen_record/recovery.rs", "recovery_status_handler", { mutation_class: "read", idempotency: "not_applicable", replayability: "not_applicable", async_job: "none", risk: "none", side_effects: { filesystem: true, process: false, network: false, ui: false } }],
  ["screen_record.status", "app/server/src/screen_record/start_handler.rs", "readiness_status_handler", { mutation_class: "read", idempotency: "not_applicable", replayability: "not_applicable", async_job: "none", risk: "none", side_effects: { filesystem: false, process: false, network: false, ui: false } }],
  ["system.mcp_test", "app/server/src/mcp/self_test.rs", "tokio::process::Command", { mutation_class: "read", idempotency: "none", risk: "external", side_effects: { process: true, network: true } }],
  ["system.doctor", "app/server/src/doctor.rs", "Command::new", { mutation_class: "read", idempotency: "none", risk: "external", side_effects: { filesystem: true, process: true } }],
];
for (const [name, sourcePath, marker, expected] of handlerEvidence) {
  assert.match(read(sourcePath), new RegExp(marker.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")), `${name}: handler evidence ${marker}`);
  const behavior = behaviorByName.get(name);
  assert.ok(behavior, `${name}: schema behavior`);
  for (const [key, value] of Object.entries(expected)) {
    if (key === "side_effects") {
      for (const [flag, expectedValue] of Object.entries(value)) {
        assert.equal(behavior.side_effects[flag], expectedValue, `${name}: ${flag} tracks handler evidence`);
      }
    } else {
      assert.equal(behavior[key], value, `${name}: ${key} tracks handler evidence`);
    }
  }
}

console.log(`PASS verb-contract (${schema.verbs.length} exact classes and dispatch arms)`);
