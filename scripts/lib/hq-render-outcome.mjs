import { isDeepStrictEqual } from "node:util";

export const HQ_MEDIA_GATE_SCHEMA = "shellx-cut/hq-media-gate@2";
export const HQ_RENDER_OUTCOME_SCHEMA = "shellx-cut/hq-render-outcome@2";
export const HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA = "shellx-cut/render-receipt-evidence@1";

const NON_GATING_EDITORIAL_FAILURES = new Set(["lufs", "caption_presence", "silence_at_edges"]);
const REQUIRED_TECHNICAL_CHECKS = ["black_or_frozen_frames", "duration_matches_edl", "uniform_border"];
const REQUIRED_STRICT_CHECKS = ["cut_on_word", "lufs", "caption_presence", "black_or_frozen_frames", "silence_at_edges", "duration_matches_edl", "uniform_border", "footage_profile"];

function object(value, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object`);
  return value;
}

function exact(value, expected, label) {
  if (value !== expected) throw new Error(`HQ render requires ${label}=${expected}; received ${String(value)}`);
}

function nonEmptyString(value, label) {
  if (typeof value !== "string" || !value.trim()) throw new Error(`HQ render requires a non-empty ${label}`);
  return value;
}

function artifact(value, label) {
  const item = object(value, label);
  nonEmptyString(item.path, `${label} path`);
  if (!/^[a-f0-9]{64}$/.test(item.sha256 || "") || !Number.isSafeInteger(item.bytes) || item.bytes <= 0) {
    throw new Error(`${label} requires a SHA-256 and positive byte count`);
  }
  return { path: item.path, sha256: item.sha256, bytes: item.bytes };
}

function normalizedPath(value) {
  const stripped = nonEmptyString(value, "artifact path").replace(/^\\\\\?\\/, "").replaceAll("/", "\\");
  return /^[A-Za-z]:\\/.test(stripped) ? stripped.toLowerCase() : stripped;
}

function samePath(left, right) { return normalizedPath(left) === normalizedPath(right); }

function classifyRenderReceipt(job, evidence) {
  const result = object(job.result, "HQ render job result");
  const receipt = object(evidence?.renderReceipt, "HQ render receipt");
  const receiptArtifact = artifact(evidence?.receiptArtifact, "HQ render receipt artifact");
  const outputArtifact = artifact(evidence?.outputArtifact, "HQ render output artifact");
  const jobRenderId = nonEmptyString(result.render_id, "job result.render_id");
  const receiptRenderId = nonEmptyString(receipt.render_id, "render receipt render_id");
  if (!samePath(result.receipt, receiptArtifact.path)) throw new Error("HQ render job does not bind the persisted render receipt path");
  if (!samePath(result.path, outputArtifact.path) || !samePath(receipt.output_path, outputArtifact.path)) throw new Error("HQ render receipt does not bind the rendered output path");
  exact(receipt.output_hash, `sha256:${outputArtifact.sha256}`, "render receipt output hash");
  exact(receiptRenderId, jobRenderId, "render receipt render_id");
  if (!Array.isArray(receipt.checks) || receipt.checks.length === 0) throw new Error("HQ render receipt requires a non-empty checks array");

  const checks = new Map();
  for (const checkValue of receipt.checks) {
    const check = object(checkValue, "HQ render receipt check");
    const name = nonEmptyString(check.name, "render receipt check name");
    if (checks.has(name)) throw new Error(`HQ render receipt contains duplicate check ${name}`);
    if (typeof check.pass !== "boolean") throw new Error(`HQ render receipt check ${name} requires a boolean pass`);
    const details = object(check.details, `HQ render receipt check ${name} details`);
    const checkEvidence = object(check.evidence, `HQ render receipt check ${name} evidence`);
    const waiverFields = [...Object.keys(details), ...Object.keys(checkEvidence)].filter((key) => /waiv|^measured_pass$/i.test(key));
    if (waiverFields.length) throw new Error(`HQ strict talking-head check ${name} contains profile waiver fields: ${[...new Set(waiverFields)].join(", ")}`);
    checks.set(name, check.pass);
  }
  const aggregatePass = [...checks.values()].every(Boolean);
  exact(receipt.pass, aggregatePass, "render receipt aggregate pass");
  exact(result.pass, aggregatePass, "job/render receipt aggregate pass");
  const failedChecks = [...checks].filter(([, pass]) => !pass).map(([name]) => name).sort();
  const gatingFailures = failedChecks.filter((name) => !NON_GATING_EDITORIAL_FAILURES.has(name));
  if (gatingFailures.length) throw new Error(`HQ geometry/render refuses failed technical or unknown checks: ${gatingFailures.join(", ")}`);
  for (const name of REQUIRED_STRICT_CHECKS) {
    if (!checks.has(name)) throw new Error(`HQ render receipt is missing required strict check ${name}`);
  }
  for (const name of REQUIRED_TECHNICAL_CHECKS) exact(checks.get(name), true, `technical receipt check ${name}`);
  const profile = receipt.checks.find((check) => check.name === "footage_profile");
  exact(profile.pass, true, "footage profile check pass");
  exact(profile.details.active_profile, "talking_head", "footage profile active_profile");
  exact(profile.details.selection, "explicit", "footage profile selection");
  return {
    binding: { schema: HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA, artifact: receiptArtifact, output: outputArtifact, renderId: receiptRenderId },
    technicalQc: { pass: true, gating: true, requiredChecks: REQUIRED_TECHNICAL_CHECKS, strictReceiptChecks: REQUIRED_STRICT_CHECKS, failedChecks: [] },
    editorialQc: { evaluated: true, pass: failedChecks.length === 0, gating: false, classification: failedChecks.length ? "fail" : "pass", requiredChecks: [...NON_GATING_EDITORIAL_FAILURES].sort(), failedChecks },
  };
}

export function classifyHqRenderOutcome(jobValue, jobsListValue, receiptEvidence) {
  const job = object(jobValue, "HQ render job");
  const result = object(job.result, "HQ render job result");
  const jobsList = object(jobsListValue, "jobs.list result");
  if (!Array.isArray(jobsList.jobs)) throw new Error("jobs.list result must contain a jobs array");
  if (!Array.isArray(jobsList.persistence_notices)) throw new Error("jobs.list result must contain persistence_notices");

  const jobId = nonEmptyString(job.job_id, "job_id");
  exact(job.kind, "render", "kind");
  exact(job.state, "done", "state");
  exact(job.completion, "success", "completion");
  exact(job.outcome, "succeeded", "outcome");
  exact(job.outcome_reason, "completed", "outcome_reason");
  exact(job.progress, 1, "progress");
  if (job.persistence_error !== null && job.persistence_error !== undefined) throw new Error(`HQ render job has persistence_error: ${String(job.persistence_error)}`);

  const matching = jobsList.jobs.filter((listedJob) => listedJob?.job_id === jobId);
  if (matching.length !== 1) throw new Error(`jobs.list must contain the terminal HQ render job exactly once; found ${matching.length}`);
  if (!isDeepStrictEqual(matching[0], job)) throw new Error("jobs.status and jobs.list disagree for the terminal HQ render job");
  if (jobsList.persistence_notices.length !== 0) throw new Error(`HQ render refuses ${jobsList.persistence_notices.length} persistence notices`);
  exact(result.verified, true, "result.verified");
  exact(result.verification_status, "complete", "result.verification_status");
  nonEmptyString(result.path, "result.path");
  nonEmptyString(result.receipt, "result.receipt");
  if (typeof result.pass !== "boolean") throw new Error("HQ render requires a boolean aggregate QC verdict in result.pass");

  const receipt = classifyRenderReceipt(job, receiptEvidence);
  return {
    schema: HQ_RENDER_OUTCOME_SCHEMA,
    terminal: { pass: true, jobId, state: job.state, completion: job.completion, outcome: job.outcome, outcomeReason: job.outcome_reason, progress: job.progress },
    verification: { pass: true, verified: result.verified, status: result.verification_status },
    persistence: { pass: true, jobError: null, notices: [], statusAndListMatch: true },
    renderReceipt: receipt.binding,
    technicalQc: receipt.technicalQc,
    editorialQc: receipt.editorialQc,
  };
}

export function assertHqRenderVerdicts(receiptValue, receiptEvidence) {
  const receipt = object(receiptValue, "HQ media gate receipt");
  exact(receipt.schema, HQ_MEDIA_GATE_SCHEMA, "HQ media gate schema");
  const render = object(receipt.render, "HQ media render evidence");
  const outcome = object(render.outcome, "HQ render outcome");
  exact(outcome.schema, HQ_RENDER_OUTCOME_SCHEMA, "render outcome schema");
  const listSnapshot = object(render.jobsList, "HQ jobs.list snapshot");
  exact(listSnapshot.matchingJobCount, 1, "jobs.list matching job count");
  if (!isDeepStrictEqual(listSnapshot.matchingJob, render.job)) throw new Error("HQ jobs.list snapshot no longer matches the terminal job");
  if (!Array.isArray(listSnapshot.persistenceNotices) || listSnapshot.persistenceNotices.length !== 0) throw new Error("HQ jobs.list snapshot must retain zero persistence notices");
  const recomputed = classifyHqRenderOutcome(render.job, { jobs: [listSnapshot.matchingJob], persistence_notices: listSnapshot.persistenceNotices }, receiptEvidence);
  if (!isDeepStrictEqual(outcome, recomputed)) throw new Error("stored HQ render outcome does not match raw terminal, persistence, receipt, and output evidence");

  const verdicts = object(receipt.verdicts, "HQ media verdicts");
  if (!isDeepStrictEqual(verdicts.editorialQc, outcome.editorialQc)) throw new Error("HQ media verdicts do not bind the classified editorial checks");
  if (!isDeepStrictEqual(verdicts.hqGeometryRender, { pass: true, gating: true, classification: "pass" })) throw new Error("HQ geometry/render verdict must be an explicit gating pass");
  for (const name of ["actualRenderFinal", "renderVerification", "renderPersistence", "renderTechnicalQc", "outputProfile", "hqGeometryRender"]) exact(receipt.checks?.[name], true, `check ${name}`);
  return true;
}
