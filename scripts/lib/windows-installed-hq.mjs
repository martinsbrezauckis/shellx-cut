import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { openSync, readFileSync } from "node:fs";
import { hostname } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { verifyAgentDocsApi } from "./agent-docs.mjs";
import { runBoundHqWorkload } from "./hq-candidate-chain.mjs";
import { assertHqCandidateListenAddr } from "./hq-candidate-chain-contracts.mjs";
import { assertHqReceiptArtifact, compactSource, evidenceReference, stageBinding, stopOwnedHqCandidate } from "./hq-candidate-chain-evidence.mjs";
import { assertWindowsOsProof, probeHqCandidateOs } from "./hq-candidate-chain-os-proof.mjs";
import { assertPathInside, createSafeDirectories, safeRegularArtifact, writeSafeJson, writeSafeText } from "./hq-candidate-chain-security.mjs";
import { daemonIdentity, oneLine, sourceIdentity } from "./hq-media-identity.mjs";
import { endpointScope, selectHqMediaProfile } from "./hq-media-contracts.mjs";
import { HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA, assertHqRenderVerdicts } from "./hq-render-outcome.mjs";
import { inspectWindowsInstalledCut } from "./windows-qualification-runner-utils.mjs";
import {
  WINDOWS_INSTALLED_HQ_ATTESTATION_SCHEMA,
  WINDOWS_INSTALLED_HQ_LIFECYCLE_SCHEMA,
  WINDOWS_INSTALLED_HQ_SCHEMA,
  loadWindowsInstalledHqPrerequisites,
  windowsInstalledHqRunDir,
} from "./windows-installed-hq-contracts.mjs";

const START_TIMEOUT_MS = 60_000;

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));
const writeJson = (path, value, label) => writeSafeJson(path, value, label);

async function getJson(url, timeoutMs) {
  const response = await fetch(url, { signal: AbortSignal.timeout(timeoutMs) });
  const text = await response.text();
  let value;
  try { value = JSON.parse(text); } catch { throw new Error(`${url} returned non-JSON HTTP ${response.status}: ${text.slice(0, 400)}`); }
  if (!response.ok) throw new Error(`${url} returned HTTP ${response.status}: ${text.slice(0, 400)}`);
  return value;
}

async function waitForAgent({ endpoint, child, get = getJson }) {
  const deadline = Date.now() + START_TIMEOUT_MS;
  let lastError;
  while (Date.now() < deadline) {
    if (child.exitCode !== null && child.exitCode !== undefined) throw new Error(`installed cutd exited before readiness: code=${child.exitCode}`);
    try { return await get(`${endpoint.url}/api/agent`, 5_000); } catch (error) { lastError = error; await sleep(250); }
  }
  throw new Error(`installed cutd did not reach /api/agent: ${oneLine(lastError?.message || lastError)}`);
}

function enrichInstalledArtifact(identity) {
  return Object.fromEntries(Object.entries(identity).map(([name, item]) => {
    const artifact = safeRegularArtifact(item.path, `installed ${name}`);
    if (artifact.sha256 !== item.sha256) throw new Error(`installed ${name} changed during identity inspection`);
    return [name, { ...item, bytes: artifact.bytes }];
  }));
}

function boundJson(reference, label, runDir) {
  const artifact = safeRegularArtifact(assertPathInside(reference?.path || "", runDir, label), label);
  if (artifact.sha256 !== reference.sha256 || artifact.bytes !== reference.bytes) throw new Error(`${label} changed before final binding`);
  let value;
  try { value = JSON.parse(readFileSync(artifact.path, "utf8")); } catch (error) { throw new Error(`${label} is invalid JSON: ${error.message}`); }
  return { artifact, value };
}

function sameJson(left, right) { return JSON.stringify(left) === JSON.stringify(right); }

export function createWindowsInstalledHqAttestation({ source, sourceReceipt, coverageReceipt, installed, hostBinding, runtime, preGateOwnership, signedFinal = false }) {
  const hostChecks = Object.values(hostBinding?.checks || {});
  if (!hostChecks.length || !hostChecks.every(Boolean)) throw new Error("installed HQ attestation requires a verified HQ host binding");
  if (runtime?.executableSha256 !== installed?.cutd?.sha256 || runtime?.runtimeExecutable?.toLowerCase() !== installed?.cutd?.path?.toLowerCase()) throw new Error("installed HQ runtime differs from the verified installed cutd");
  if (preGateOwnership?.proof?.process?.sha256 !== installed.cutd.sha256) throw new Error("installed HQ attestation requires exact listener ownership");
  return {
    schema: WINDOWS_INSTALLED_HQ_ATTESTATION_SCHEMA,
    generatedAt: new Date().toISOString(),
    source: compactSource(source),
    evidence: { sourceReceipt, fullCoverageReceipt: coverageReceipt },
    installed,
    candidate: { cutd: installed.cutd },
    runtime: { agent: runtime },
    hostBinding,
    osProof: { preGateOwnership },
    signedFinal: signedFinal === true,
    mode: { installedCandidate: true, buildPerformed: false, installPerformed: false, signaturesRequired: signedFinal === true },
  };
}

export function createWindowsInstalledHqReceipt({ attestation, attestationArtifact, workloads, lifecycle, lifecycleArtifact, runDir }) {
  const profiles = workloads?.map((item) => item.profile);
  if (attestation?.schema !== WINDOWS_INSTALLED_HQ_ATTESTATION_SCHEMA || !attestationArtifact?.sha256 || !sameJson(profiles, ["hq-4k-uhd-60", "hq-8k-uhd-60"]) || !workloads.every((item) => item.receipt?.checks?.hqGeometryRender === true && item.artifact?.sha256) || lifecycle?.schema !== WINDOWS_INSTALLED_HQ_LIFECYCLE_SCHEMA || !lifecycleArtifact?.sha256 || !["stopped", "already-exited"].includes(lifecycle.cleanup?.status) || lifecycle.postCleanupVacancy?.proof?.vacant !== true || lifecycle.after?.sourceMatches !== true || lifecycle.after?.installedMatches !== true || lifecycle.after?.receiptHashesMatch !== true || !sameJson(lifecycle.after.source, attestation.source) || !sameJson(lifecycle.after.installed, attestation.installed) || !sameJson(lifecycle.installedCutd, attestation.installed.cutd)) {
    throw new Error("final installed HQ receipt requires verified 4K/8K geometry renders and successful owned-daemon cleanup");
  }
  const persistedAttestation = boundJson(attestationArtifact, "installed HQ attestation", runDir);
  const persistedLifecycle = boundJson(lifecycleArtifact, "installed HQ lifecycle", runDir);
  if (!sameJson(persistedAttestation.value, attestation) || !sameJson(persistedLifecycle.value, lifecycle)) throw new Error("installed HQ attestation or lifecycle changed before final binding");
  for (const [label, reference] of [["installed source receipt", attestation.evidence?.sourceReceipt], ["installed full-coverage receipt", attestation.evidence?.fullCoverageReceipt]]) {
    const artifact = safeRegularArtifact(reference?.path || "", label);
    if (artifact.sha256 !== reference.sha256 || artifact.bytes !== reference.bytes) throw new Error(`${label} changed before final binding`);
  }
  const proofContracts = { preLaunchVacancy: ["vacant", "pre-launch"], preGateOwnership: ["owned", "pre-gate"], postGateOwnership: ["owned", "post-gate"], postCleanupVacancy: ["vacant", "post-cleanup"] };
  for (const [name, [action, phase]] of Object.entries(proofContracts)) {
    const item = boundJson(lifecycle.osProofs?.[name], `installed HQ OS proof ${name}`, runDir);
    const proof = item.value?.proof;
    const address = proof?.listener?.address === "::1" ? "[::1]" : proof?.listener?.address;
    assertWindowsOsProof(proof, { action, phase, addr: `${address}:${proof?.listener?.port}`, pid: action === "owned" ? lifecycle.childPid : undefined, stagedCutd: attestation.installed.cutd.path, sha256: attestation.installed.cutd.sha256 });
    if (name === "postCleanupVacancy" && !sameJson(item.value, lifecycle.postCleanupVacancy)) throw new Error("installed HQ cleanup vacancy proof differs from lifecycle");
  }
  for (const item of workloads) {
    const persistedGate = boundJson(item.artifact, `installed HQ ${item.profile} gate receipt`, runDir);
    if (!sameJson(persistedGate.value, item.receipt)) throw new Error(`installed HQ ${item.profile} gate receipt changed before final binding`);
    if (item.receipt.evidence?.renderReceipt?.schema !== HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA) throw new Error(`installed HQ ${item.profile} is missing versioned render receipt evidence`);
    const renderArtifact = safeRegularArtifact(assertPathInside(item.receipt.evidence.renderReceipt.path, runDir, `installed HQ ${item.profile} render receipt`), `installed HQ ${item.profile} render receipt`);
    const outputArtifact = safeRegularArtifact(assertPathInside(item.receipt.artifacts?.output?.path, runDir, `installed HQ ${item.profile} output`), `installed HQ ${item.profile} output`);
    if (renderArtifact.sha256 !== item.receipt.evidence.renderReceipt.sha256 || renderArtifact.bytes !== item.receipt.evidence.renderReceipt.bytes || outputArtifact.sha256 !== item.receipt.artifacts.output.sha256 || outputArtifact.bytes !== item.receipt.artifacts.output.bytes) throw new Error(`installed HQ ${item.profile} evidence changed before final binding`);
    let renderReceipt;
    try { renderReceipt = JSON.parse(readFileSync(renderArtifact.path, "utf8")); } catch (error) { throw new Error(`installed HQ ${item.profile} render receipt is invalid: ${error.message}`); }
    assertHqRenderVerdicts(item.receipt, { renderReceipt, receiptArtifact: renderArtifact, outputArtifact });
  }
  return {
    schema: WINDOWS_INSTALLED_HQ_SCHEMA,
    generatedAt: new Date().toISOString(),
    completion: { state: "cleaned", pass: true, profiles },
    classification: attestation.signedFinal ? "signed-installed-hq" : "unsigned-installed-candidate-hq",
    authorization: "none",
    releaseClaim: "This receipt proves 4K/8K geometry rendering through the already installed candidate after a passing installed WebView2 matrix. Editorial QC remains separately reported. It does not authorize signing, publication, or another render.",
    signedFinal: attestation.signedFinal,
    source: attestation.source,
    installed: attestation.installed,
    runDir,
    evidence: {
      attestation: evidenceReference(attestationArtifact, attestation.schema),
      sourceReceipt: attestation.evidence.sourceReceipt,
      fullCoverageReceipt: attestation.evidence.fullCoverageReceipt,
      profiles: workloads.map((item) => ({ id: item.profile, receipt: evidenceReference(item.artifact, item.receipt.schema) })),
      lifecycle: evidenceReference(lifecycleArtifact, lifecycle.schema),
    },
    verdicts: Object.fromEntries(workloads.map((item) => [item.profile, item.receipt.verdicts])),
    lifecycle,
  };
}

export async function runWindowsInstalledHq(options, {
  repoRoot = resolve(fileURLToPath(new URL("../..", import.meta.url))), hostPlatform = process.platform,
  actualHostname = hostname(), runId = randomUUID(), readSource = sourceIdentity,
  inspectInstalled = inspectWindowsInstalledCut, startDaemon = (path, args, config) => spawn(path, args, config),
  stopDaemon = stopOwnedHqCandidate, osProof = probeHqCandidateOs, get = getJson,
  verifyAgentDocs = verifyAgentDocsApi, runWorkload = runBoundHqWorkload,
} = {}) {
  if (hostPlatform !== "win32") throw new Error("Windows installed HQ must run under native Windows Node after installed qualification");
  for (const [name, value] of [["--source-receipt", options.sourceReceipt], ["--full-coverage-receipt", options.fullCoverageReceipt], ["--fixture-4k", options.fixture4k], ["--fixture-8k", options.fixture8k], ["--host-binding", options.hostBinding]]) {
    if (!value) throw new Error(`${name} is required`);
  }
  const root = resolve(repoRoot);
  const source = readSource(root);
  const installed = enrichInstalledArtifact(inspectInstalled({ expectedVersion: source.version, cwd: root }));
  const prerequisites = loadWindowsInstalledHqPrerequisites({ sourceReceipt: options.sourceReceipt, fullCoverageReceipt: options.fullCoverageReceipt, liveSource: source, installedArtifact: installed });
  const runDir = windowsInstalledHqRunDir({ repoRoot: root, out: options.out, runId });
  createSafeDirectories(runDir, "Windows installed HQ evidence", { requireNewLeaf: true });
  const binding = stageBinding(options.hostBinding, runDir, actualHostname);
  const listenAddr = assertHqCandidateListenAddr(options.addr);
  const endpoint = endpointScope(listenAddr);
  const proofs = {}, proofArtifacts = {};
  const recordProof = (name, action, phase, extra = {}) => {
    const value = osProof({ repoRoot: root, action, phase, addr: listenAddr, ...extra });
    proofs[name] = value;
    proofArtifacts[name] = writeJson(join(runDir, `os-proof-${name}.json`), value, `installed HQ OS proof ${name}`);
    return value;
  };
  recordProof("preLaunchVacancy", "vacant", "pre-launch");
  const runtimeRoot = createSafeDirectories(join(runDir, "runtime"), "installed HQ runtime");
  createSafeDirectories(join(runtimeRoot, "home"), "installed HQ runtime home");
  createSafeDirectories(join(runtimeRoot, "projects"), "installed HQ runtime projects");
  const daemonLog = join(runDir, "cutd.log");
  writeSafeText(daemonLog, "Windows installed HQ daemon log\n", "installed HQ daemon log");
  const child = startDaemon(installed.cutd.path, ["serve", "--headless", "--addr", listenAddr], { cwd: runDir, windowsHide: true, env: { ...process.env, SHELLX_CUT_HOME: join(runtimeRoot, "home"), SHELLX_CUT_PROJECTS_DIR: join(runtimeRoot, "projects") }, stdio: ["ignore", openSync(daemonLog, "a", 0o600), openSync(daemonLog, "a", 0o600)] });
  let attestation = null, attestationArtifact = null, workloads = [], gateError = null;
  try {
    const agent = await waitForAgent({ endpoint, child, get });
    const daemon = daemonIdentity(agent, installed.cutd.path);
    if (!daemon.explicitPathMatchesReported || daemon.artifact.sha256 !== installed.cutd.sha256) throw new Error("running daemon differs from the verified installed cutd");
    recordProof("preGateOwnership", "owned", "pre-gate", { pid: child.pid, stagedCutd: installed.cutd.path, sha256: installed.cutd.sha256 });
    const docs = await verifyAgentDocs({ engineBase: endpoint.url, sourceRoot: root, expectedVersion: source.version, timeoutMs: START_TIMEOUT_MS });
    if (!docs.ok) throw new Error(`installed agent docs differ from source: ${docs.failures.join("; ")}`);
    const hostBinding = { ...binding.staged, validation: binding.validation, checks: { ...binding.validation.checks, stagedCopyMatchesSupplied: binding.supplied.sha256 === binding.staged.sha256 } };
    const runtime = { schema: agent.schema, product: agent.product, version: agent.version, addr: agent.runtime?.addr || null, runtimeExecutable: daemon.reportedPath, executableSha256: daemon.artifact.sha256, agentDocs: docs };
    attestation = createWindowsInstalledHqAttestation({ source, sourceReceipt: evidenceReference(prerequisites.source.artifact, prerequisites.source.value.schema), coverageReceipt: evidenceReference(prerequisites.coverage.artifact, prerequisites.coverage.value.schema), installed, hostBinding, runtime, preGateOwnership: proofs.preGateOwnership, signedFinal: prerequisites.source.value.signedFinal });
    attestationArtifact = writeJson(join(runDir, "installed-hq-attestation.json"), attestation, "installed HQ attestation");
    for (const [profileId, fixturePath] of [["hq-4k-uhd-60", options.fixture4k], ["hq-8k-uhd-60", options.fixture8k]]) {
      const profile = selectHqMediaProfile(profileId);
      const result = await runWorkload({ root, runDir, profile, fixturePath, hostBindingPath: binding.staged.path, endpoint, daemonPath: installed.cutd.path, timeoutMs: options.timeoutMs, candidateAttestation: attestation, attestationEvidence: evidenceReference(attestationArtifact, attestation.schema), bindingKind: "installed-candidate", sourceSnapshot: source, receiptSubdir: `hq-media-gate-${profileId}` });
      workloads.push({ profile: profileId, ...result, artifact: assertHqReceiptArtifact(result.receipt, result.artifact) });
    }
    recordProof("postGateOwnership", "owned", "post-gate", { pid: child.pid, stagedCutd: installed.cutd.path, sha256: installed.cutd.sha256 });
  } catch (error) { gateError = error; }
  const cleanup = await stopDaemon(child, { beforeTaskkill: () => recordProof("preTaskkillOwnership", "owned", "pre-taskkill", { pid: child.pid, stagedCutd: installed.cutd.path, sha256: installed.cutd.sha256 }) });
  if (["stopped", "already-exited"].includes(cleanup.status)) {
    try { recordProof("postCleanupVacancy", "vacant", "post-cleanup"); } catch (error) { cleanup.status = "failed"; cleanup.error = oneLine(error.message); }
  }
  let after = { sourceMatches: false, installedMatches: false, receiptHashesMatch: false, source: null, installed: null, error: null };
  if (["stopped", "already-exited"].includes(cleanup.status)) {
    try {
      const afterSource = readSource(root);
      const afterInstalled = enrichInstalledArtifact(inspectInstalled({ expectedVersion: source.version, cwd: root }));
      const afterPrerequisites = loadWindowsInstalledHqPrerequisites({ sourceReceipt: options.sourceReceipt, fullCoverageReceipt: options.fullCoverageReceipt, liveSource: afterSource, installedArtifact: afterInstalled });
      after = {
        sourceMatches: afterSource.gitDirty === false && afterSource.gitCommit === source.gitCommit && afterSource.gitTree === source.gitTree && afterSource.contentManifest?.sha256 === source.contentManifest?.sha256,
        installedMatches: ["shell", "cutd"].every((name) => afterInstalled[name].path.toLowerCase() === installed[name].path.toLowerCase() && afterInstalled[name].sha256 === installed[name].sha256 && afterInstalled[name].bytes === installed[name].bytes && afterInstalled[name].productVersion === installed[name].productVersion && afterInstalled[name].signatureStatus === installed[name].signatureStatus),
        receiptHashesMatch: afterPrerequisites.source.artifact.sha256 === prerequisites.source.artifact.sha256 && afterPrerequisites.coverage.artifact.sha256 === prerequisites.coverage.artifact.sha256,
        source: compactSource(afterSource), installed: afterInstalled, error: null,
      };
      if (!after.sourceMatches || !after.installedMatches || !after.receiptHashesMatch) throw new Error("source, installed binaries, or prerequisite receipts changed during installed HQ");
    } catch (error) { after.error = oneLine(error.message); cleanup.status = "failed"; cleanup.error = `post-run installed identity check failed: ${after.error}`; }
  }
  const lifecycle = { schema: WINDOWS_INSTALLED_HQ_LIFECYCLE_SCHEMA, completedAt: new Date().toISOString(), runDir, childPid: child.pid || null, installedCutd: installed.cutd, cleanup, osProofs: Object.fromEntries(Object.entries(proofArtifacts).map(([name, artifact]) => [name, evidenceReference(artifact, "shellx-cut/hq-candidate-os-proof@1")])), postCleanupVacancy: proofs.postCleanupVacancy || null, after };
  const lifecycleArtifact = writeJson(join(runDir, "installed-hq-lifecycle.json"), lifecycle, "installed HQ lifecycle");
  if (gateError) throw new Error(`Windows installed HQ failed; lifecycle: ${lifecycleArtifact.path}; ${oneLine(gateError.message)}`);
  if (cleanup.status === "failed") throw new Error(`Windows installed HQ cleanup failed; lifecycle: ${lifecycleArtifact.path}; ${cleanup.error || cleanup.status}`);
  const receipt = createWindowsInstalledHqReceipt({ attestation, attestationArtifact, workloads, lifecycle, lifecycleArtifact, runDir });
  const receiptPath = join(runDir, "windows-installed-hq-receipt.json");
  writeJson(receiptPath, receipt, "Windows installed HQ final receipt");
  return { runDir, receiptPath, receipt };
}
