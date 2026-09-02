import { createHash } from "node:crypto";

const DIMS = ["present", "render", "click", "result"];

function dimValue(value) {
  return value === "pass" || value === "fail" || value === "na" ? value : "na";
}

function hasFail(row) {
  return DIMS.some((dim) => dimValue(row?.[dim]) === "fail");
}

function evidence(row) {
  return String(row?.evidence || "");
}

export function classifyFullCoverageRow(row) {
  if (hasFail(row)) return "failure";
  if (dimValue(row?.result) === "pass") return "fully_verified";

  const text = evidence(row);
  if (/interaction-verify|settings-experience-verify|verify-review-handoff|recorder rig gate|staged update rig gate|staged-publication Chromium gate/i.test(text)) return "delegated";
  if (/nothing to verify|no non-curated|drift check skipped|not in effects\.list|no remaining video clip/i.test(text)) {
    return "guard";
  }
  if (/optional multi-agent/i.test(text)) return "optional_agent_skip";
  if (/honest dev skip|FCV_REQUIRE_FULL=1 enforces/i.test(text)) return "dependency_skip";
  return "could_not_verify";
}

function emptyTally() {
  return { pass: 0, fail: 0, na: 0 };
}

function dimensionTallies(rows) {
  const out = {
    present: emptyTally(),
    render: emptyTally(),
    click: emptyTally(),
    result: emptyTally(),
  };
  for (const row of rows) {
    for (const dim of DIMS) {
      out[dim][dimValue(row?.[dim])] += 1;
    }
  }
  return out;
}

function rowKind(row) {
  return row?.rowKind === "ui_action" ? "ui_action" : "support"
}

function normalizeRow(row, { full, strictAllActions, focusedActionRequirements }) {
  let classification = classifyFullCoverageRow(row);
  const kind = rowKind(row);
  const fullyExercised = DIMS.every((dim) => dimValue(row?.[dim]) === "pass");
  const focused = Array.isArray(focusedActionRequirements)
    ? focusedActionRequirements.find((entry) => entry?.actionId === String(row?.actionId || row?.name || ""))
    : null;
  if (strictAllActions && kind === "ui_action" && classification !== "failure" && !fullyExercised) {
    classification = "strict_unverified";
  }
  if (focused && classification !== "failure" && !Object.entries(focused.dimensions || {}).every(([dimension, value]) => row?.[dimension] === value)) {
    classification = "focused_unverified";
  }
  const releaseBlocking =
    classification === "failure" ||
    classification === "strict_unverified" ||
    classification === "focused_unverified" ||
    (full && (classification === "could_not_verify" || classification === "dependency_skip"));
  return {
    actionId: String(row?.actionId || `${row?.surface || ""}::${row?.name || ""}`),
    rowKind: kind,
    surface: String(row?.surface || ""),
    name: String(row?.name || ""),
    present: dimValue(row?.present),
    render: dimValue(row?.render),
    click: dimValue(row?.click),
    result: dimValue(row?.result),
    ok: !releaseBlocking,
    classification,
    evidence: evidence(row),
    shot: row?.shot ? String(row.shot) : "",
  };
}

function actionManifest(rows) {
  const occurrences = rows
    .filter((row) => row.rowKind === "ui_action")
    .map((row) => row.actionId)
    .sort();
  const actionIds = [...new Set(occurrences)];
  const repeated = actionIds.flatMap((id) => {
    const count = occurrences.filter((candidate) => candidate === id).length;
    return count > 1 ? [{ id, count }] : [];
  });
  return {
    algorithm: "sha256",
    sha256: createHash("sha256").update(JSON.stringify(actionIds)).digest("hex"),
    total: actionIds.length,
    occurrences: occurrences.length,
    observed: actionIds,
    repeated,
  };
}

function sourceActionManifest(sourceActionIds, expectedSourceActionIds) {
  const observed = [...new Set((sourceActionIds || []).map(String))].sort();
  const expected = Array.isArray(expectedSourceActionIds)
    ? [...new Set(expectedSourceActionIds.map(String))].sort()
    : null;
  const observedSet = new Set(observed);
  const expectedSet = expected ? new Set(expected) : null;
  const missing = expected ? expected.filter((id) => !observedSet.has(id)) : [];
  const unexpected = expectedSet ? observed.filter((id) => !expectedSet.has(id)) : [];
  return {
    algorithm: "sha256",
    sha256: createHash("sha256").update(JSON.stringify(observed)).digest("hex"),
    total: observed.length,
    observed,
    expectedSha256: expected
      ? createHash("sha256").update(JSON.stringify(expected)).digest("hex")
      : null,
    expectedTotal: expected?.length ?? null,
    missing,
    unexpected,
    matchesExpected: expected != null && missing.length === 0 && unexpected.length === 0,
  };
}

function focusedScenarioReceipt(value) {
  if (value == null) return null;
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("focused full-coverage receipt metadata must be an object");
  }
  const allowed = new Set(["id", "section", "actionIds", "fuzzSeed", "traceSha256", "finalFingerprintSha256"]);
  const unexpected = Object.keys(value).filter((key) => !allowed.has(key));
  if (unexpected.length) throw new Error(`focused full-coverage receipt metadata has unexpected fields: ${unexpected.join(", ")}`);
  const id = String(value.id || "");
  const section = String(value.section || "");
  const actionIds = Array.isArray(value.actionIds) ? value.actionIds.map(String) : [];
  if (!/^[a-z0-9][a-z0-9-]*$/.test(id) || !/^[a-z0-9][a-z0-9-]*$/.test(section)
    || actionIds.length === 0 || new Set(actionIds).size !== actionIds.length || actionIds.some((actionId) => !actionId)) {
    throw new Error("focused full-coverage receipt metadata is incomplete");
  }
  const fuzzSeed = value.fuzzSeed == null ? "" : String(value.fuzzSeed);
  if (fuzzSeed && (fuzzSeed.length > 128 || /[\r\n]/.test(fuzzSeed))) {
    throw new Error("focused full-coverage receipt fuzzSeed must be a single bounded value");
  }
  const traceSha256 = value.traceSha256 == null ? "" : String(value.traceSha256);
  const finalFingerprintSha256 = value.finalFingerprintSha256 == null ? "" : String(value.finalFingerprintSha256);
  if (fuzzSeed && (!/^[a-f0-9]{64}$/.test(traceSha256) || !/^[a-f0-9]{64}$/.test(finalFingerprintSha256))) {
    throw new Error("focused full-coverage receipt fuzz metadata requires exact trace and final fingerprint hashes");
  }
  if (!fuzzSeed && (traceSha256 || finalFingerprintSha256)) {
    throw new Error("focused full-coverage receipt fuzz hashes require the registered fuzz seed");
  }
  return {
    id,
    section,
    actionIds,
    ...(fuzzSeed ? { fuzzSeed } : {}),
    ...(fuzzSeed ? { traceSha256, finalFingerprintSha256 } : {}),
  };
}

export function controlledCandidateBinding(environment = process.env) {
  const keys = [
    "FCV_CANDIDATE_ID",
    "FCV_SOURCE_GIT_COMMIT",
    "FCV_SOURCE_GIT_TREE",
    "FCV_SOURCE_WORKTREE",
    "FCV_SOURCE_CONTENT_MANIFEST_SHA256",
    "FCV_TEST_CONTROL_MANIFEST_SHA256",
    "FCV_FIXTURE_ID",
    "FCV_FIXTURE_SEED_SHA256",
    "FCV_RUNNER_ID",
  ];
  const values = Object.fromEntries(keys.map((key) => [key, String(environment[key] || "").trim()]));
  const controlledEntryRequested = [
    values.FCV_CANDIDATE_ID,
    values.FCV_RUNNER_ID,
    values.FCV_TEST_CONTROL_MANIFEST_SHA256,
  ].some(Boolean);
  if (!controlledEntryRequested) return null;
  if (keys.some((key) => values[key] === "")) {
    throw new Error("controlled full-coverage candidate binding is incomplete");
  }
  return {
    id: values.FCV_CANDIDATE_ID,
    sourceCommit: values.FCV_SOURCE_GIT_COMMIT,
    sourceTree: values.FCV_SOURCE_GIT_TREE,
    worktree: values.FCV_SOURCE_WORKTREE,
    contentManifestSha256: values.FCV_SOURCE_CONTENT_MANIFEST_SHA256,
    testControlManifestSha256: values.FCV_TEST_CONTROL_MANIFEST_SHA256,
    fixtureId: values.FCV_FIXTURE_ID,
    fixtureSeedSha256: values.FCV_FIXTURE_SEED_SHA256,
    runnerId: values.FCV_RUNNER_ID,
  };
}

export function buildFullCoverageReceipt(rows, options = {}) {
  const rawRows = Array.isArray(rows) ? rows : [];
  const full = options.full === true;
  const strictAllActions = options.strictAllActions === true;
  const normalized = rawRows.map((row) => normalizeRow(row, {
    full,
    strictAllActions,
    focusedActionRequirements: options.focusedActionRequirements,
  }));
  const count = (classification) => normalized.filter((row) => row.classification === classification).length;
  const manifest = actionManifest(normalized);
  const sourceManifest = sourceActionManifest(
    options.sourceActionIds,
    options.expectedSourceActionIds,
  );
  const runtimeSourceManifest = sourceActionManifest(
    options.runtimeSourceActionIds,
    options.expectedRuntimeSourceActionIds,
  );
  const delegatedSourceActionIds = [...new Set((options.delegatedSourceActionIds || []).map(String))].sort();
  const focusedScenario = focusedScenarioReceipt(options.focusedScenario);
  const controls = {
    total: normalized.length,
    uiActions: normalized.filter((row) => row.rowKind === "ui_action").length,
    supportRows: normalized.filter((row) => row.rowKind === "support").length,
    fullyVerified: count("fully_verified"),
    delegated: count("delegated"),
    dependencySkips: count("dependency_skip"),
    optionalAgentSkips: count("optional_agent_skip"),
    guards: count("guard"),
    couldNotVerify: count("could_not_verify"),
    strictUnverified: count("strict_unverified"),
    focusedUnverified: count("focused_unverified"),
    failures: count("failure"),
  };
  const runtimeCoverageRequired = Array.isArray(options.expectedRuntimeSourceActionIds);
  const ok = (!strictAllActions || (
    sourceManifest.matchesExpected
    && (!runtimeCoverageRequired || runtimeSourceManifest.matchesExpected)
  ))
    && normalized.every((row) => row.ok);

  return {
    schema: "shellx-cut/full-coverage-results@1",
    generatedAt: options.generatedAt || new Date().toISOString(),
    full,
    strictAllActions,
    verificationScope: options.verificationScope || null,
    ok,
    surface: options.surface || null,
    runtime: options.runtime || null,
    matrixProfile: options.matrixProfile || null,
    candidate: options.candidate || null,
    actionManifest: manifest,
    sourceActionManifest: sourceManifest,
    runtimeSourceActionManifest: runtimeSourceManifest,
    delegatedSourceActionIds,
    ...(focusedScenario ? { focusedScenario } : {}),
    sectionAttempts: options.sectionAttempts || null,
    summary: {
      dimensions: dimensionTallies(normalized),
      controls,
      coverage: options.coverage || null,
    },
    coverage: options.coverage || null,
    media: options.media || null,
    screenshotsDir: options.screenshotsDir || null,
    results: normalized,
  };
}
