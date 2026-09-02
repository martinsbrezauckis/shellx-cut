import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { assertProfileProjectFormat, assertRenderRequestSchema, buildRenderRequest, endpointScope, firstVideoTrackId, HQ_MEDIA_PROFILES, inspectImportedVideoPlacement, parseHqMediaGateArgs, selectHqMediaProfile } from "../lib/hq-media-contracts.mjs";
import { assertHqHost, canonicalJsonSha256, HQ_ATLAS_HQ_SURFACE_SHA256, hqHostBindingValidation } from "../lib/hq-media-host-binding.mjs";
import { summarizeFfprobe, validateInputProbe, validateOutputProbe } from "../lib/hq-media-probe.mjs";
import { runHqMediaGate } from "../lib/hq-media-gate.mjs";
import { HQ_MEDIA_GATE_SCHEMA, classifyHqRenderOutcome } from "../lib/hq-render-outcome.mjs";

const profile4k = selectHqMediaProfile("hq-4k-uhd-60");
const profile8k = selectHqMediaProfile("hq-8k-uhd-60");
const renderFinalVerb = JSON.parse(readFileSync(resolve("schema/verbs.json"), "utf8")).verbs.find((verb) => verb.name === "render.final");
const probe = ({ width, height, fps = "60/1", duration = "12.000", codec = "h264" }) => ({ format: { duration }, streams: [{ codec_type: "video", codec_name: codec, width, height, avg_frame_rate: fps, r_frame_rate: fps }] });
const completedRenderJob = ({ editorialPass = false, ...overrides } = {}) => ({
  job_id: "job_004",
  kind: "render",
  state: "done",
  completion: "success",
  outcome: "succeeded",
  outcome_reason: "completed",
  progress: 1,
  persistence_error: null,
  result: {
    render_id: "render_001",
    pass: editorialPass,
    path: "C:\\CutQ\\output.mp4",
    receipt: "C:\\CutQ\\render_001.json",
    verified: true,
    verification_status: "complete",
  },
  ...overrides,
});
const listed = (job, persistenceNotices = []) => ({ jobs: [structuredClone(job)], persistence_notices: persistenceNotices });
const receiptCheck = (name, pass = true, details = {}) => ({ name, pass, details, evidence: {} });
const completedRenderEvidence = ({ failed = [], extraChecks = [] } = {}) => {
  const checks = ["cut_on_word", "lufs", "caption_presence", "silence_at_edges", "black_or_frozen_frames", "duration_matches_edl", "uniform_border"].map((name) => receiptCheck(name, !failed.includes(name)));
  checks.push(receiptCheck("footage_profile", true, { active_profile: "talking_head", selection: "explicit" }));
  checks.push(...extraChecks);
  const pass = checks.every((check) => check.pass);
  return {
    renderReceipt: { render_id: "render_001", output_path: "C:\\CutQ\\output.mp4", output_hash: `sha256:${"b".repeat(64)}`, checks, pass },
    receiptArtifact: { path: "C:\\CutQ\\render_001.json", sha256: "a".repeat(64), bytes: 1_024 },
    outputArtifact: { path: "C:\\CutQ\\output.mp4", sha256: "b".repeat(64), bytes: 4_096 },
  };
};

test("HQ profiles remain exact 4K/8K 60 contracts", () => {
  assert.deepEqual(Object.keys(HQ_MEDIA_PROFILES), ["hq-4k-uhd-60", "hq-8k-uhd-60"]);
  assert.deepEqual(profile4k.output, { width: 3840, height: 2160, fps: 60, codecs: ["h264"] });
  assert.deepEqual(profile8k.output, { width: 7680, height: 4320, fps: 60, codecs: ["h264"] });
  assert.equal(profile4k.render.profile, "talking_head");
  assert.equal(profile8k.render.profile, "talking_head");
  assert.equal(validateInputProbe(summarizeFfprobe(probe({ width: 3840, height: 2160 })), profile4k), true);
  assert.throws(() => validateInputProbe(summarizeFfprobe(probe({ width: 1920, height: 1080 })), profile4k), /3840x2160/);
  assert.equal(validateOutputProbe(summarizeFfprobe(probe({ width: 7680, height: 4320, duration: "12.100" })), profile8k, 12_000), true);
  assert.throws(() => validateOutputProbe(summarizeFfprobe(probe({ width: 3840, height: 2160 })), profile8k, 12_000), /7680x4320/);
});

test("HQ terminal render admission records editorial QC separately", () => {
  const editorialFailure = completedRenderJob();
  const failedEditorial = classifyHqRenderOutcome(editorialFailure, listed(editorialFailure), completedRenderEvidence({ failed: ["lufs", "caption_presence", "silence_at_edges"] }));
  assert.equal(failedEditorial.terminal.pass, true);
  assert.equal(failedEditorial.verification.pass, true);
  assert.equal(failedEditorial.persistence.pass, true);
  assert.deepEqual(failedEditorial.editorialQc, {
    evaluated: true,
    pass: false,
    gating: false,
    classification: "fail",
    requiredChecks: ["caption_presence", "lufs", "silence_at_edges"],
    failedChecks: ["caption_presence", "lufs", "silence_at_edges"],
  });

  const editorialPass = completedRenderJob({ editorialPass: true });
  const passedEditorial = classifyHqRenderOutcome(editorialPass, listed(editorialPass), completedRenderEvidence());
  assert.equal(passedEditorial.terminal.pass, true);
  assert.equal(passedEditorial.editorialQc.pass, true);
  assert.equal(passedEditorial.editorialQc.gating, false);
});

test("HQ render admission fails closed on terminal, verification, or persistence drift", () => {
  const job = completedRenderJob();
  const evidence = completedRenderEvidence({ failed: ["lufs", "caption_presence", "silence_at_edges"] });
  assert.throws(() => classifyHqRenderOutcome({ ...job, completion: "done_with_warnings" }, listed({ ...job, completion: "done_with_warnings" }), evidence), /completion=success/);
  assert.throws(() => classifyHqRenderOutcome({ ...job, persistence_error: "Access is denied" }, listed({ ...job, persistence_error: "Access is denied" }), evidence), /persistence_error/);
  assert.throws(() => classifyHqRenderOutcome(job, listed(job, [{ job_id: job.job_id, error: "recovered corrupt record" }]), evidence), /persistence notices/);
  assert.throws(() => classifyHqRenderOutcome({ ...job, result: { ...job.result, verified: false } }, listed({ ...job, result: { ...job.result, verified: false } }), evidence), /verified/);
  assert.throws(() => classifyHqRenderOutcome(job, { jobs: [], persistence_notices: [] }, evidence), /exactly once/);
  assert.throws(() => classifyHqRenderOutcome({ ...job, result: { ...job.result, pass: undefined } }, listed({ ...job, result: { ...job.result, pass: undefined } }), evidence), /aggregate QC verdict/);
});

test("HQ receipt classifier permits only named editorial failures", () => {
  for (const technical of ["uniform_border", "black_or_frozen_frames", "duration_matches_edl", "future_unknown_check"]) {
    const job = completedRenderJob();
    const evidence = completedRenderEvidence({ failed: technical === "future_unknown_check" ? [] : [technical], extraChecks: technical === "future_unknown_check" ? [receiptCheck(technical, false)] : [] });
    assert.throws(() => classifyHqRenderOutcome(job, listed(job), evidence), /technical or unknown checks/);
  }
  const job = completedRenderJob({ editorialPass: true });
  const drifted = completedRenderEvidence();
  drifted.renderReceipt.output_hash = `sha256:${"c".repeat(64)}`;
  assert.throws(() => classifyHqRenderOutcome(job, listed(job), drifted), /output hash/);
  for (const renderId of [undefined, ""]) {
    const missingJobId = completedRenderJob({ editorialPass: true });
    missingJobId.result.render_id = renderId;
    const missingReceiptId = completedRenderEvidence();
    missingReceiptId.renderReceipt.render_id = renderId;
    assert.throws(() => classifyHqRenderOutcome(missingJobId, listed(missingJobId), missingReceiptId), /non-empty (job result\.render_id|render receipt render_id)/);
  }
  for (const required of ["lufs", "caption_presence", "silence_at_edges", "footage_profile"]) {
    const missing = completedRenderEvidence();
    missing.renderReceipt.checks = missing.renderReceipt.checks.filter((check) => check.name !== required);
    assert.throws(() => classifyHqRenderOutcome(job, listed(job), missing), /missing required strict check/);
  }
  for (const [active_profile, selection] of [["silent_screen_demo", "explicit"], ["talking_head", "default"]]) {
    const wrongProfile = completedRenderEvidence();
    const marker = wrongProfile.renderReceipt.checks.find((check) => check.name === "footage_profile");
    marker.details = { active_profile, selection };
    assert.throws(() => classifyHqRenderOutcome(job, listed(job), wrongProfile), /footage profile (active_profile|selection)/);
  }
  const waived = completedRenderEvidence();
  for (const name of ["lufs", "caption_presence", "silence_at_edges"]) {
    waived.renderReceipt.checks.find((check) => check.name === name).details = { waived_by_profile: "silent_screen_demo", waiver_reason: "fixture", measured_pass: false };
  }
  assert.throws(() => classifyHqRenderOutcome(job, listed(job), waived), /profile waiver fields/);
});

test("candidate chain consumes the shared terminal and editorial classifier", () => {
  const candidateChain = readFileSync(resolve("scripts/lib/hq-candidate-chain.mjs"), "utf8");
  assert.match(candidateChain, /postVerb\(ctx, "jobs\.list", \{\}\)/);
  assert.match(candidateChain, /classifyHqRenderOutcome\(receipt\.render\.job, jobsList, \{/);
  assert.match(candidateChain, /safeRegularArtifact\(renderReceiptPath, "HQ render receipt"\)/);
  assert.match(candidateChain, /editorialQc: receipt\.render\.outcome\.editorialQc/);
  assert.match(candidateChain, /hqGeometryRender: \{/);
  assert.equal(HQ_MEDIA_GATE_SCHEMA, "shellx-cut/hq-media-gate@2");
});

test("public media CLI accepts no receipt or provisional candidate context", async () => {
  assert.deepEqual(parseHqMediaGateArgs([]), { profile: "", fixture: "", hostBinding: "", addr: "127.0.0.1:6219", daemon: "", out: "", timeoutMs: 7_200_000, allowNonLoopback: false, help: false });
  assert.throws(() => parseHqMediaGateArgs(["--candidate-receipt", "previous.json"]), /unknown option/);
  await assert.rejects(runHqMediaGate({ profile: "hq-4k-uhd-60" }), /owned by hq-candidate-chain/);
  const publicGate = readFileSync(resolve("scripts/lib/hq-media-gate.mjs"), "utf8");
  assert.doesNotMatch(publicGate, /createOwnedHqCandidateGateRunner|assertCandidateReceiptMatches|candidateReceipt/);
});

test("endpoint and render contracts remain fail-closed", () => {
  assert.equal(endpointScope("127.0.0.1:6219").scope, "literal-loopback");
  assert.throws(() => endpointScope("localhost:6219"), /non-loopback/);
  const request = buildRenderRequest(profile8k, "C:\\CutQ\\out.mp4");
  assert.equal(assertRenderRequestSchema(request, renderFinalVerb), true);
  assert.throws(() => assertRenderRequestSchema({ ...request, impossible_argument: true }, renderFinalVerb), /schema-unknown/);
  assert.equal(assertProfileProjectFormat({ settings: { width: 3840, height: 2160, fps: 60 } }, profile4k), true);
  assert.equal(firstVideoTrackId({ tracks: [{ id: "a", kind: "audio" }, { id: "v", kind: "video" }] }), "v");
  assert.equal(inspectImportedVideoPlacement({ assets: { a: { probe: { duration_ms: 12_000 } }, }, tracks: [{ id: "v", kind: "video", clips: [] }] }, "a", "v").mode, "insert-required");
});

test("private Windows-HQ binding rejects a generic or GROK assertion", () => {
  const projection = { schema: "release-studio.project-atlas@1", project: "shellx-cut", surface: { id: "windows-hq", platform: "Windows HQ reference", role: "deliberate HQ 4K/8K GPU matrix only", transport: "WSL to native PowerShell", shell: "PowerShell", target: "hq-workstation-example\\Operator" } };
  const pin = canonicalJsonSha256(projection);
  const binding = { schema: "shellx-cut/hq-host-binding@2", atlasProjection: projection };
  assert.equal(assertHqHost("win32", binding, "HQ-WORKSTATION-EXAMPLE", { pinnedProjectionSha256: pin }).checks.actualHostnameMatchesPinnedTarget, true);
  assert.match(HQ_ATLAS_HQ_SURFACE_SHA256, /^[a-f0-9]{64}$/);
  assert.throws(() => assertHqHost("win32", binding, "grok-windows", { pinnedProjectionSha256: pin }), /actualHostnameMatchesPinnedTarget/);
  const drift = structuredClone(projection); drift.surface.target = "grok-windows\\Operator";
  assert.equal(hqHostBindingValidation({ schema: "shellx-cut/hq-host-binding@2", atlasProjection: drift }, "grok-windows", { pinnedProjectionSha256: pin }).checks.canonicalProjectionMatchesPinnedIdentity, false);
});
