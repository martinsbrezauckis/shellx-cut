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
  if (/interaction-verify|verify-review-handoff|recorder rig gate/i.test(text)) return "delegated";
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

function normalizeRow(row, { full, strictAllActions }) {
  let classification = classifyFullCoverageRow(row);
  const kind = rowKind(row);
  const fullyExercised = DIMS.every((dim) => dimValue(row?.[dim]) === "pass");
  if (strictAllActions && kind === "ui_action" && classification !== "failure" && !fullyExercised) {
    classification = "strict_unverified";
  }
  const releaseBlocking =
    classification === "failure" ||
    classification === "strict_unverified" ||
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

export function buildFullCoverageReceipt(rows, options = {}) {
  const rawRows = Array.isArray(rows) ? rows : [];
  const full = options.full === true;
  const strictAllActions = options.strictAllActions === true;
  const normalized = rawRows.map((row) => normalizeRow(row, { full, strictAllActions }));
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
    ok,
    surface: options.surface || null,
    runtime: options.runtime || null,
    actionManifest: manifest,
    sourceActionManifest: sourceManifest,
    runtimeSourceActionManifest: runtimeSourceManifest,
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
