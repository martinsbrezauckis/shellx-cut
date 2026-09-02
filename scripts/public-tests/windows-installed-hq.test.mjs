import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";

import { createWindowsInstalledHqAttestation, createWindowsInstalledHqReceipt } from "../lib/windows-installed-hq.mjs";
import { loadWindowsInstalledHqPrerequisites, parseWindowsInstalledHqArgs, windowsInstalledHqRunDir } from "../lib/windows-installed-hq-contracts.mjs";
import { safeRegularArtifact } from "../lib/hq-candidate-chain-security.mjs";
import { HQ_MEDIA_GATE_SCHEMA, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA, classifyHqRenderOutcome } from "../lib/hq-render-outcome.mjs";

const sha = (char) => char.repeat(64);
const git = (char) => char.repeat(40);
const artifact = (path, value) => {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, typeof value === "string" ? value : JSON.stringify(value));
  return safeRegularArtifact(path, "installed HQ test artifact");
};
const ref = (item, schema = "") => ({ path: item.path, sha256: item.sha256, bytes: item.bytes, ...(schema ? { schema } : {}) });

function fixture({ signedFinal = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), "shellx-cut-installed-hq-"));
  const sourcePath = join(root, "source-receipt.json");
  const coveragePath = join(root, "full-coverage-receipt.json");
  const installed = {
    shell: { path: "C:\\Users\\Test\\AppData\\Local\\ShellX Cut\\shellx-cut.exe", sha256: sha("a"), productVersion: "0.6.110", signatureStatus: signedFinal ? "Valid" : "NotSigned" },
    cutd: { path: "C:\\Users\\Test\\AppData\\Local\\ShellX Cut\\cutd.exe", sha256: sha("b"), productVersion: "0.6.110", signatureStatus: signedFinal ? "Valid" : "NotSigned" },
  };
  const source = {
    schema: "shellx-cut/windows-installed-source@1", head: git("c"), gitTree: git("d"), version: "0.6.110",
    contentManifest: { sha256: sha("e") }, status: [], signedFinal,
    artifactReceipt: signedFinal ? { installer: { signatureStatus: "Valid" }, packaged: { shell: { sha256: installed.shell.sha256 }, cutd: { sha256: installed.cutd.sha256 } } } : null,
    installedArtifact: installed,
  };
  const coverage = {
    schema: "shellx-cut/full-coverage-results@1", ok: true, full: true, strictAllActions: true, surface: "windows-installed",
    source: { gitCommit: source.head, contentManifestSha256: source.contentManifest.sha256 },
    runtime: { installedApp: true, driver: "webview2-cdp", nativeAttached: true, sourceGitCommit: source.head, sourceContentManifestSha256: source.contentManifest.sha256, installedArtifactSha256: installed.shell.sha256 },
    summary: { controls: { total: 1, failures: 0, couldNotVerify: 0, strictUnverified: 0 } },
    results: [{ actionId: "installed-proof", ok: true, present: "pass", render: "pass", click: "pass", result: "pass" }],
  };
  writeFileSync(sourcePath, JSON.stringify(source));
  writeFileSync(coveragePath, JSON.stringify(coverage));
  const liveSource = { gitCommit: source.head, gitTree: source.gitTree, version: source.version, contentManifest: { sha256: source.contentManifest.sha256 }, gitDirty: false };
  return { root, sourcePath, coveragePath, source, coverage, installed, liveSource };
}

test("installed HQ CLI requires explicit build/install evidence and both real profiles", () => {
  const parsed = parseWindowsInstalledHqArgs(["--source-receipt", "source.json", "--full-coverage-receipt", "coverage.json", "--fixture-4k", "4k.mp4", "--fixture-8k", "8k.mp4", "--host-binding", "host.json"]);
  assert.equal(parsed.fixture4k, "4k.mp4");
  assert.equal(parsed.fixture8k, "8k.mp4");
  assert.equal(parsed.timeoutMs, 7_200_000);
  assert.throws(() => parseWindowsInstalledHqArgs(["--build"]), /unknown option --build/);
  assert.throws(() => parseWindowsInstalledHqArgs(["--source-receipt", "source.json"]), /--full-coverage-receipt is required/);
});

test("installed HQ accepts only an exact clean installed WebView2 candidate", () => {
  const item = fixture();
  try {
    const loaded = loadWindowsInstalledHqPrerequisites({ sourceReceipt: item.sourcePath, fullCoverageReceipt: item.coveragePath, liveSource: item.liveSource, installedArtifact: item.installed });
    assert.equal(loaded.source.value.head, item.source.head);
    assert.equal(loaded.coverage.value.runtime.installedApp, true);
    const drift = { ...item.liveSource, gitTree: git("f") };
    assert.throws(() => loadWindowsInstalledHqPrerequisites({ sourceReceipt: item.sourcePath, fullCoverageReceipt: item.coveragePath, liveSource: drift, installedArtifact: item.installed }), /live source tree differs/);
    const wrongCutd = { ...item.installed, cutd: { ...item.installed.cutd, sha256: sha("0") } };
    assert.throws(() => loadWindowsInstalledHqPrerequisites({ sourceReceipt: item.sourcePath, fullCoverageReceipt: item.coveragePath, liveSource: item.liveSource, installedArtifact: wrongCutd }), /installed cutd digest differs/);
  } finally { rmSync(item.root, { recursive: true, force: true }); }
});

test("signed-final installed HQ requires packaged and installed signature coherence", () => {
  const item = fixture({ signedFinal: true });
  try {
    loadWindowsInstalledHqPrerequisites({ sourceReceipt: item.sourcePath, fullCoverageReceipt: item.coveragePath, liveSource: item.liveSource, installedArtifact: item.installed });
    const invalid = { ...item.installed, shell: { ...item.installed.shell, signatureStatus: "NotSigned" } };
    assert.throws(() => loadWindowsInstalledHqPrerequisites({ sourceReceipt: item.sourcePath, fullCoverageReceipt: item.coveragePath, liveSource: item.liveSource, installedArtifact: invalid }), /installed shell signature status differs/);
  } finally { rmSync(item.root, { recursive: true, force: true }); }
});

test("installed HQ attestation binds source receipts, both installed binaries, runtime, and listener", () => {
  const item = fixture();
  try {
    const sourceReceipt = { path: item.sourcePath, sha256: sha("1"), bytes: 1, schema: item.source.schema };
    const coverageReceipt = { path: item.coveragePath, sha256: sha("2"), bytes: 1, schema: item.coverage.schema };
    const hostBinding = { checks: { host: true } };
    const runtime = { runtimeExecutable: item.installed.cutd.path, executableSha256: item.installed.cutd.sha256 };
    const preGateOwnership = { proof: { process: { sha256: item.installed.cutd.sha256 } } };
    const source = { ...item.liveSource, contentManifest: { schema: "shellx-cut/source-content-manifest@1", files: 1, bytes: 1, sha256: item.liveSource.contentManifest.sha256 } };
    const attestation = createWindowsInstalledHqAttestation({ source, sourceReceipt, coverageReceipt, installed: item.installed, hostBinding, runtime, preGateOwnership });
    assert.equal(attestation.mode.buildPerformed, false);
    assert.equal(attestation.mode.installPerformed, false);
    assert.equal(attestation.installed.shell.sha256, item.installed.shell.sha256);
    assert.equal(attestation.evidence.fullCoverageReceipt.sha256, sha("2"));
    assert.throws(() => createWindowsInstalledHqAttestation({ source, sourceReceipt, coverageReceipt, installed: item.installed, hostBinding, runtime: { ...runtime, executableSha256: sha("9") }, preGateOwnership }), /runtime differs/);
  } finally { rmSync(item.root, { recursive: true, force: true }); }
});

test("final installed HQ refuses missing profile evidence or unfinished cleanup", () => {
  const item = fixture();
  try {
    const input = { attestation: { schema: "shellx-cut/windows-installed-hq-attestation@1" }, attestationArtifact: {}, workloads: [], lifecycle: { schema: "shellx-cut/windows-installed-hq-lifecycle@1", cleanup: { status: "failed" } }, lifecycleArtifact: {}, runDir: item.root };
    assert.throws(() => createWindowsInstalledHqReceipt(input), /requires verified 4K\/8K/);
  } finally { rmSync(item.root, { recursive: true, force: true }); }
});

test("final installed HQ rehashes both raw render verdicts, prerequisite receipts, lifecycle, and OS proofs", () => {
  const item = fixture();
  const runDir = join(item.root, ".scratch", "windows-installed-hq", "run-final");
  mkdirSync(runDir, { recursive: true });
  try {
    const sourceArtifact = safeRegularArtifact(item.sourcePath, "source receipt");
    const coverageArtifact = safeRegularArtifact(item.coveragePath, "coverage receipt");
    const source = { gitCommit: item.source.head, gitTree: item.source.gitTree, version: item.source.version, contentManifest: { schema: "shellx-cut/source-content-manifest@1", files: 1, bytes: 1, sha256: item.source.contentManifest.sha256 } };
    const attestation = { schema: "shellx-cut/windows-installed-hq-attestation@1", signedFinal: false, source, installed: item.installed, candidate: { cutd: item.installed.cutd }, evidence: { sourceReceipt: ref(sourceArtifact, item.source.schema), fullCoverageReceipt: ref(coverageArtifact, item.coverage.schema) }, mode: { installedCandidate: true } };
    const attestationArtifact = artifact(join(runDir, "attestation.json"), attestation);
    const workloads = ["hq-4k-uhd-60", "hq-8k-uhd-60"].map((profile, index) => {
      const dir = join(runDir, profile);
      const output = artifact(join(dir, "output.mp4"), `output-${profile}`);
      const checks = [
        ["cut_on_word", true, {}], ["lufs", false, {}], ["caption_presence", false, {}], ["silence_at_edges", false, {}],
        ["black_or_frozen_frames", true, {}], ["duration_matches_edl", true, {}], ["uniform_border", true, {}],
        ["footage_profile", true, { active_profile: "talking_head", selection: "explicit" }],
      ].map(([name, pass, details]) => ({ name, pass, details, evidence: {} }));
      const renderId = `render_00${index + 1}`;
      const renderReceiptValue = { render_id: renderId, output_path: output.path, output_hash: `sha256:${output.sha256}`, checks, pass: false };
      const renderReceiptArtifact = artifact(join(dir, `${renderId}.json`), renderReceiptValue);
      const job = { job_id: `job_00${index + 1}`, kind: "render", state: "done", completion: "success", outcome: "succeeded", outcome_reason: "completed", progress: 1, persistence_error: null, result: { render_id: renderId, pass: false, path: output.path, receipt: renderReceiptArtifact.path, verified: true, verification_status: "complete" } };
      const outcome = classifyHqRenderOutcome(job, { jobs: [structuredClone(job)], persistence_notices: [] }, { renderReceipt: renderReceiptValue, receiptArtifact: renderReceiptArtifact, outputArtifact: output });
      const receipt = { schema: HQ_MEDIA_GATE_SCHEMA, checks: { actualRenderFinal: true, renderVerification: true, renderPersistence: true, renderTechnicalQc: true, outputProfile: true, hqGeometryRender: true }, render: { job, jobsList: { matchingJob: structuredClone(job), matchingJobCount: 1, persistenceNotices: [] }, outcome }, verdicts: { hqGeometryRender: { pass: true, gating: true, classification: "pass" }, editorialQc: outcome.editorialQc }, artifacts: { output }, evidence: { renderReceipt: ref(renderReceiptArtifact, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA) } };
      return { profile, receipt, artifact: artifact(join(dir, "gate.json"), receipt) };
    });
    const pid = 451;
    const proof = (action, phase, vacant) => ({ schema: "shellx-cut/hq-candidate-os-proof@1", action, phase, vacant, pid: vacant ? 0 : pid, listener: { address: "127.0.0.1", port: 6219, owners: vacant ? [] : [pid], count: vacant ? 0 : 1, literalCount: vacant ? 0 : 1 }, expectedExecutable: vacant ? "" : item.installed.cutd.path, expectedSha256: vacant ? "" : item.installed.cutd.sha256, process: vacant ? null : { executable: item.installed.cutd.path, sha256: item.installed.cutd.sha256 } });
    const proofValues = { preLaunchVacancy: proof("vacant", "pre-launch", true), preGateOwnership: proof("owned", "pre-gate", false), postGateOwnership: proof("owned", "post-gate", false), postCleanupVacancy: proof("vacant", "post-cleanup", true) };
    const proofArtifacts = Object.fromEntries(Object.entries(proofValues).map(([name, value]) => [name, artifact(join(runDir, `${name}.json`), { script: { path: "proof.ps1", sha256: sha("9"), bytes: 1 }, proof: value })]));
    const lifecycle = { schema: "shellx-cut/windows-installed-hq-lifecycle@1", runDir, childPid: pid, installedCutd: item.installed.cutd, cleanup: { status: "stopped" }, osProofs: Object.fromEntries(Object.entries(proofArtifacts).map(([name, value]) => [name, ref(value, "shellx-cut/hq-candidate-os-proof@1")])), postCleanupVacancy: { script: { path: "proof.ps1", sha256: sha("9"), bytes: 1 }, proof: proofValues.postCleanupVacancy }, after: { sourceMatches: true, installedMatches: true, receiptHashesMatch: true, source, installed: item.installed, error: null } };
    const lifecycleArtifact = artifact(join(runDir, "lifecycle.json"), lifecycle);
    const receipt = createWindowsInstalledHqReceipt({ attestation, attestationArtifact, workloads, lifecycle, lifecycleArtifact, runDir });
    assert.equal(receipt.completion.pass, true);
    assert.deepEqual(receipt.completion.profiles, ["hq-4k-uhd-60", "hq-8k-uhd-60"]);
    const drifted = structuredClone(workloads);
    drifted[1].receipt.render.job.state = "failed";
    assert.throws(() => createWindowsInstalledHqReceipt({ attestation, attestationArtifact, workloads: drifted, lifecycle, lifecycleArtifact, runDir }), /(gate receipt changed|state=done)/);
  } finally { rmSync(item.root, { recursive: true, force: true }); }
});

test("installed HQ output is always a fresh checkout-owned evidence directory", () => {
  const item = fixture();
  try {
    const path = windowsInstalledHqRunDir({ repoRoot: item.root, runId: "run-001" });
    assert.equal(path, join(item.root, ".scratch", "windows-installed-hq", "run-001"));
    assert.throws(() => windowsInstalledHqRunDir({ repoRoot: item.root, out: join(item.root, "outside") }), /below this checkout/);
  } finally { rmSync(item.root, { recursive: true, force: true }); }
});

test("official installed HQ path never exposes build or install options", () => {
  const [cli, runner, legacy] = ["../windows-installed-hq.mjs", "../lib/windows-installed-hq.mjs", "../lib/hq-candidate-chain-contracts.mjs"].map((path) => readFileSync(new URL(path, import.meta.url), "utf8"));
  assert.doesNotMatch(cli, /cargo|build-windows|install-cut-current/);
  assert.doesNotMatch(runner, /cargo-xwin|hqCandidateBuildSpec|build-windows|install-cut-current/);
  assert.match(legacy, /Internal component diagnostic only/);
  assert.match(legacy, /windows-installed-full-coverage[.]mjs/);
  assert.match(legacy, /windows-installed-hq[.]mjs/);
});
