import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { existsSync, openSync, readFileSync } from "node:fs";
import { hostname } from "node:os";
import { basename, join, resolve } from "node:path";

import { verifyAgentDocsApi } from "./agent-docs.mjs";
import { DEFAULT_HQ_REPO_ROOT, assertProfileProjectFormat, assertRenderRequestSchema, buildRenderRequest, endpointScope, firstVideoTrackId, inspectImportedVideoPlacement, selectHqMediaProfile } from "./hq-media-contracts.mjs";
import { HQ_CANDIDATE_CHAIN_DEFAULT_ADDR as DEFAULT_ADDR, assertHqCandidateChainDir, assertHqCandidateListenAddr, resolveHqCandidateChainDir } from "./hq-candidate-chain-contracts.mjs";
import { ATTESTATION_SCHEMA, LIFECYCLE_SCHEMA, assertHqReceiptArtifact, candidateAttestationSha256, compactSource, createHqCandidateAttestation, createHqCandidateBuildFailureReceipt, createHqCandidateReceipt, evidenceReference, stageAgentDocs, stageBinding, stopOwnedHqCandidate, unsignedHqCandidateSignature } from "./hq-candidate-chain-evidence.mjs";
import { assertCandidateAttestationMatches, assertInstalledHqAttestationMatches, daemonIdentity, hostIdentity, oneLine, readHqHostBinding, sourceIdentity, toolIdentity } from "./hq-media-identity.mjs";
import { assertHqHost, hqHostBindingValidation } from "./hq-media-host-binding.mjs";
import { runFfprobe, summarizeFfprobe, validateInputProbe, validateOutputProbe } from "./hq-media-probe.mjs";
import { HQ_MEDIA_GATE_SCHEMA, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA, classifyHqRenderOutcome } from "./hq-render-outcome.mjs";
import { assertNoReparseAncestors, assertPathInside, copySafeRegularFile, createSafeDirectories, safeRegularArtifact, writeSafeJson, writeSafeText } from "./hq-candidate-chain-security.mjs";
import { assertNoAmbientToolDrift, assertNoAmbientXwinOverrides, assertPinnedToolchain, hqCandidateBuildSpec, pinHqToolchain, prepareGovernedHqCargoHome, runLogged } from "./hq-candidate-chain-toolchain.mjs";
import { HQ_XWIN_CACHE_INVENTORY_SCHEMA, assertHqXwinCacheInventory, cleanupGovernedHqDerivedBuildDirectory, cleanupGovernedHqXwinCache, prepareGovernedHqXwinCache, persistHqXwinCacheInventory } from "./hq-candidate-chain-xwin-cache.mjs";
import { probeHqCandidateOs } from "./hq-candidate-chain-os-proof.mjs";

const START_TIMEOUT_MS = 60_000;

function writeJson(path, value, label = "HQ candidate chain receipt") { return writeSafeJson(path, value, label); }
function sleep(ms) { return new Promise((done) => setTimeout(done, ms)); }

async function getJson(url, timeoutMs) {
  const response = await fetch(url, { signal: AbortSignal.timeout(timeoutMs) });
  const text = await response.text();
  let body;
  try { body = JSON.parse(text); } catch { throw new Error(`${url} returned non-JSON HTTP ${response.status}: ${text.slice(0, 400)}`); }
  if (!response.ok) throw new Error(`${url} returned HTTP ${response.status}: ${text.slice(0, 400)}`);
  return body;
}

async function postVerb(ctx, name, args) {
  const response = await fetch(`${ctx.baseUrl}/api/verb/${name}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(args), signal: AbortSignal.timeout(ctx.timeoutMs) });
  const text = await response.text();
  let body;
  try { body = JSON.parse(text); } catch { throw new Error(`${name} returned non-JSON HTTP ${response.status}: ${text.slice(0, 400)}`); }
  if (!response.ok || body.ok !== true) throw new Error(`${name} failed: ${oneLine(JSON.stringify(body.error || body)).slice(0, 800)}`);
  return body.result || {};
}

async function waitJob(ctx, jobId) {
  const deadline = Date.now() + ctx.timeoutMs;
  let last;
  while (Date.now() < deadline) {
    last = await postVerb(ctx, "jobs.status", { job_id: jobId });
    if (last.state === "done") return last;
    if (last.state === "failed") throw new Error(`job ${jobId} failed: ${oneLine(JSON.stringify(last.error || last)).slice(0, 1_200)}`);
    await sleep(1_000);
  }
  throw new Error(`job ${jobId} timed out after ${ctx.timeoutMs}ms: ${oneLine(JSON.stringify(last)).slice(0, 1_200)}`);
}

function projectDurationMs(state, assetId) {
  const clip = (state.tracks || []).flatMap((track) => track.clips || []).find((item) => item.asset === assetId);
  const duration = Number(clip?.src_out_ms) - Number(clip?.src_in_ms || 0);
  if (!Number.isSafeInteger(duration) || duration <= 0) throw new Error("HQ project did not retain a positive base-clip duration after edit.insert");
  return duration;
}

async function waitForAgent({ baseUrl, child, timeoutMs = START_TIMEOUT_MS, get = getJson }) {
  const deadline = Date.now() + timeoutMs;
  let lastError = null;
  while (Date.now() < deadline) {
    if (child.exitCode !== null && child.exitCode !== undefined) throw new Error(`HQ candidate cutd exited before readiness: code=${child.exitCode}`);
    try { return await get(`${baseUrl}/api/agent`, Math.min(timeoutMs, 5_000)); } catch (error) { lastError = error; await sleep(250); }
  }
  throw new Error(`HQ candidate cutd did not reach /api/agent within ${timeoutMs}ms: ${oneLine(lastError?.message || lastError)}`);
}

// The only callers are the active source-owned component and installed HQ
// orchestrators. Both supply a fresh run root and an in-memory attestation;
// neither public CLI can authorize a workload from an old receipt.
export async function runBoundHqWorkload({ root, runDir, profile, fixturePath, hostBindingPath, endpoint, daemonPath, timeoutMs, candidateAttestation, attestationEvidence, bindingKind = "unsigned-candidate", sourceSnapshot = null, receiptSubdir = "hq-media-gate" }) {
  const receiptDir = join(runDir, receiptSubdir);
  assertPathInside(receiptDir, runDir, "HQ media receipt directory");
  createSafeDirectories(receiptDir, "HQ media receipt", { requireNewLeaf: true });
  const receiptPath = join(receiptDir, "hq-media-gate-receipt.json");
  const installed = bindingKind === "installed-candidate";
  if (!installed && bindingKind !== "unsigned-candidate") throw new Error(`unsupported HQ workload binding ${bindingKind}`);
  const receipt = { schema: HQ_MEDIA_GATE_SCHEMA, startedAt: new Date().toISOString(), pass: false, classification: installed ? "installed-candidate-provisional-render" : "candidate-chain-provisional-render", releaseClaim: "This is an incomplete same-run workload record. It never authorizes a later render and becomes evidence only after owned-child cleanup.", receiptDir, profile: { id: profile.id, label: profile.label, input: profile.input, output: profile.output, render: profile.render }, endpoint, source: null, daemon: null, candidateSource: null, hqHostBinding: null, host: null, artifacts: { input: null, output: null }, probes: { input: null, output: null }, checks: {} };
  try {
    const hostBinding = readHqHostBinding(hostBindingPath);
    const hostValidation = hqHostBindingValidation(hostBinding.binding, hostBinding.evidence.actualHostname);
    receipt.hqHostBinding = { ...hostBinding.evidence, ...hostValidation, checks: { ...hostValidation.checks, windowsPlatform: process.platform === "win32" } };
    receipt.checks.hqHostBinding = Object.values(receipt.hqHostBinding.checks).every(Boolean);
    if (!hostBinding.binding || hostBinding.error) throw new Error(hostBinding.error);
    assertHqHost(process.platform, hostBinding.binding, receipt.hqHostBinding.actualHostname);
    receipt.source = sourceSnapshot || sourceIdentity(root);
    if (receipt.source.gitDirty) throw new Error("HQ media qualification refuses a dirty source worktree");
    const fixture = safeRegularArtifact(resolve(fixturePath), "HQ fixture");
    receipt.artifacts.input = fixture;
    const ffprobeTool = toolIdentity("ffprobe");
    receipt.host = hostIdentity(ffprobeTool);
    const inputRaw = runFfprobe(fixture.path, ffprobeTool.executable);
    const inputArtifact = writeJson(join(receiptDir, "ffprobe-input.json"), inputRaw, "HQ input ffprobe receipt");
    receipt.evidence = { ffprobeInput: evidenceReference(inputArtifact) };
    receipt.probes.input = summarizeFfprobe(inputRaw);
    receipt.checks.inputProfile = validateInputProbe(receipt.probes.input, profile);
    const ctx = { baseUrl: endpoint.url, timeoutMs };
    const agent = await getJson(`${ctx.baseUrl}/api/agent`, ctx.timeoutMs);
    receipt.daemon = daemonIdentity(agent, daemonPath);
    receipt.checks.explicitDaemonMatchesRunningDaemon = receipt.daemon.explicitPathMatchesReported;
    if (!receipt.daemon.explicitPathMatchesReported) throw new Error("--daemon must exactly match /api/agent runtime.executable for the running cutd");
    receipt.candidateSource = { schema: candidateAttestation.schema, signedFinal: candidateAttestation.signedFinal, source: { head: candidateAttestation.source.gitCommit, gitTree: candidateAttestation.source.gitTree, contentManifestSha256: candidateAttestation.source.contentManifest.sha256, version: candidateAttestation.source.version }, installedArtifact: installed ? { shellSha256: candidateAttestation.installed.shell.sha256, cutdSha256: candidateAttestation.candidate.cutd.sha256 } : { cutdSha256: candidateAttestation.candidate.cutd.sha256 }, ...(installed ? { installedCandidate: { sourceReceipt: candidateAttestation.evidence.sourceReceipt, fullCoverageReceipt: candidateAttestation.evidence.fullCoverageReceipt } } : { hqCandidate: { sameRunOnly: true, backingAttestation: attestationEvidence } }) };
    receipt.candidateSource.matchChecks = installed
      ? assertInstalledHqAttestationMatches(candidateAttestation, receipt.source, receipt.daemon)
      : assertCandidateAttestationMatches(candidateAttestation, receipt.source, receipt.daemon);
    receipt.checks.candidateSource = Object.values(receipt.candidateSource.matchChecks).every(Boolean);
    receipt.daemon.doctor = await postVerb(ctx, "system.doctor", {});
    receipt.checks.daemonVersionMatchesSource = receipt.daemon.api.version === receipt.source.version;
    if (!receipt.checks.daemonVersionMatchesSource) throw new Error(`daemon version ${receipt.daemon.api.version} does not match source version ${receipt.source.version}`);
    const projectRoot = createSafeDirectories(join(receiptDir, "project"), "HQ media project root", { requireNewLeaf: true });
    const projectDir = join(projectRoot, `${profile.id}-${basename(runDir).slice(0, 8)}.cutproj`);
    assertNoReparseAncestors(projectRoot, "HQ media project root");
    receipt.project = { name: `${profile.id}-${basename(runDir).slice(0, 8)}`, dir: projectDir, outputPath: join(projectDir, "exports", `${profile.id}.mp4`) };
    await postVerb(ctx, "project.create", { name: receipt.project.name, dir: projectDir, settings: { width: profile.output.width, height: profile.output.height, fps: profile.output.fps } });
    createSafeDirectories(projectDir, "HQ media project directory");
    createSafeDirectories(join(projectDir, "exports"), "HQ media render root", { requireNewLeaf: true });
    const createdState = await postVerb(ctx, "project.state", {});
    receipt.checks.projectFormat = assertProfileProjectFormat(createdState, profile);
    receipt.project.videoTrackId = firstVideoTrackId(createdState);
    const imported = await postVerb(ctx, "media.import", { path: fixture.path, proxy: true, rationale: `HQ candidate chain ${profile.id}: explicit real fixture import` });
    if (!imported.asset_id || !imported.job_id) throw new Error("media.import did not return asset_id and job_id");
    receipt.import = { assetId: imported.asset_id, jobId: imported.job_id, job: await waitJob(ctx, imported.job_id) };
    let state = await postVerb(ctx, "project.state", {});
    let placement = inspectImportedVideoPlacement(state, imported.asset_id, receipt.project.videoTrackId);
    if (placement.mode === "insert-required") {
      const inserted = await postVerb(ctx, "edit.insert", { asset: imported.asset_id, track: receipt.project.videoTrackId, at_ms: 0, ripple: false, rationale: `HQ candidate chain ${profile.id}: seed actual final render` });
      state = await postVerb(ctx, "project.state", {});
      placement = inspectImportedVideoPlacement(state, imported.asset_id, receipt.project.videoTrackId);
      if (placement.mode !== "already-placed" || (inserted.clip_id && placement.clipId !== inserted.clip_id)) throw new Error("edit.insert did not leave exactly one expected HQ video clip");
      receipt.import.placement = { kind: "explicit-insert", ...placement };
    } else receipt.import.placement = { kind: "auto-placement", ...placement };
    const stateArtifact = writeJson(join(receiptDir, "project-state-before-render.json"), state, "HQ project state receipt");
    receipt.evidence.projectStateBeforeRender = evidenceReference(stateArtifact);
    receipt.project.expectedDurationMs = projectDurationMs(state, imported.asset_id);
    const renderRequest = buildRenderRequest(profile, receipt.project.outputPath);
    const registry = await getJson(`${ctx.baseUrl}/api/verbs`, ctx.timeoutMs);
    receipt.checks.renderRequestSchema = assertRenderRequestSchema(renderRequest, (registry.verbs || []).find((verb) => verb?.name === "render.final"));
    receipt.render = { request: renderRequest, response: await postVerb(ctx, "render.final", renderRequest) };
    if (!receipt.render.response.job_id) throw new Error("render.final did not return job_id; dry-run is not qualification");
    receipt.render.job = await waitJob(ctx, receipt.render.response.job_id);
    const jobsList = await postVerb(ctx, "jobs.list", {});
    const matchingJobs = Array.isArray(jobsList.jobs) ? jobsList.jobs.filter((job) => job?.job_id === receipt.render.response.job_id) : [];
    receipt.render.jobsList = { matchingJob: matchingJobs.length === 1 ? matchingJobs[0] : null, matchingJobCount: matchingJobs.length, persistenceNotices: jobsList.persistence_notices };
    const actualOutput = String(receipt.render.job?.result?.path || receipt.project.outputPath);
    if (resolve(actualOutput) !== resolve(receipt.project.outputPath)) throw new Error(`render.final wrote an unexpected output path: ${actualOutput}`);
    receipt.artifacts.output = safeRegularArtifact(actualOutput, "HQ render output");
    const renderReceiptPath = assertPathInside(String(receipt.render.job?.result?.receipt || ""), projectDir, "HQ render receipt");
    const renderReceiptArtifact = safeRegularArtifact(renderReceiptPath, "HQ render receipt");
    let renderReceipt;
    try { renderReceipt = JSON.parse(readFileSync(renderReceiptArtifact.path, "utf8")); } catch (error) { throw new Error(`HQ render receipt is not valid JSON: ${error.message}`); }
    receipt.evidence.renderReceipt = evidenceReference(renderReceiptArtifact, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA);
    receipt.render.outcome = classifyHqRenderOutcome(receipt.render.job, jobsList, { renderReceipt, receiptArtifact: renderReceiptArtifact, outputArtifact: receipt.artifacts.output });
    const outputRaw = runFfprobe(actualOutput, ffprobeTool.executable);
    const outputArtifact = writeJson(join(receiptDir, "ffprobe-output.json"), outputRaw, "HQ output ffprobe receipt");
    receipt.evidence.ffprobeOutput = evidenceReference(outputArtifact);
    receipt.probes.output = summarizeFfprobe(outputRaw);
    receipt.checks.actualRenderFinal = receipt.render.outcome.terminal.pass;
    receipt.checks.renderVerification = receipt.render.outcome.verification.pass;
    receipt.checks.renderPersistence = receipt.render.outcome.persistence.pass;
    receipt.checks.renderTechnicalQc = receipt.render.outcome.technicalQc.pass;
    receipt.checks.outputProfile = validateOutputProbe(receipt.probes.output, profile, receipt.project.expectedDurationMs);
    receipt.verdicts = {
      hqGeometryRender: {
        pass: receipt.checks.actualRenderFinal && receipt.checks.renderVerification && receipt.checks.renderPersistence && receipt.checks.renderTechnicalQc && receipt.checks.outputProfile,
        gating: true,
        classification: "pass",
      },
      editorialQc: receipt.render.outcome.editorialQc,
    };
    receipt.checks.hqGeometryRender = receipt.verdicts.hqGeometryRender.pass;
    receipt.completedAt = new Date().toISOString();
    receipt.execution = { state: "completed-awaiting-owned-cleanup", terminal: false, pass: false };
  } catch (error) { receipt.completedAt = new Date().toISOString(); receipt.error = oneLine(error?.stack || error?.message || error); }
  const artifact = writeJson(receiptPath, receipt, "provisional HQ media gate receipt");
  if (receipt.error || !Object.values(receipt.checks).every(Boolean)) throw new Error(`HQ media gate failed; receipt: ${receiptPath}; ${receipt.error || "one or more checks failed"}`);
  return { receipt, receiptPath, artifact };
}

function sourceStillMatches(before, root, readSourceIdentity = sourceIdentity) {
  const after = readSourceIdentity(root);
  if (after.gitDirty || after.gitCommit !== before.gitCommit || after.gitTree !== before.gitTree || after.contentManifest.sha256 !== before.contentManifest.sha256) throw new Error("HQ candidate chain source changed while building; refusing a mixed candidate");
}

export async function runHqCandidateChain(options, { repoRoot = DEFAULT_HQ_REPO_ROOT, hostPlatform = process.platform, actualHostname = hostname(), runId = randomUUID(), environment = process.env, get = getJson, startDaemon = (path, args, config) => spawn(path, args, config), stopDaemon = stopOwnedHqCandidate, osProof = probeHqCandidateOs, pinToolchain = pinHqToolchain, runBuild = runLogged, stageHostBinding = stageBinding, verifyAgentDocs = verifyAgentDocsApi, runWorkload = runBoundHqWorkload, unsignedCandidate = unsignedHqCandidateSignature, readSourceIdentity = sourceIdentity, stageAgentDocsFn = stageAgentDocs } = {}) {
  assertNoAmbientXwinOverrides(environment);
  assertNoAmbientToolDrift(environment);
  if (hostPlatform !== "win32") throw new Error("HQ candidate chain runs only from native Windows on the reserved HQ workstation");
  const profile = selectHqMediaProfile(options.profile);
  if (!options.fixture) throw new Error("--fixture is required; the candidate chain never creates a substitute");
  const listenAddr = assertHqCandidateListenAddr(options.addr || DEFAULT_ADDR);
  const endpoint = endpointScope(listenAddr);
  const root = resolve(repoRoot);
  const candidateRoot = resolve(root, ".scratch", "hq-candidate-chain");
  const runDir = assertHqCandidateChainDir(resolveHqCandidateChainDir({ repoRoot: root, profile: profile.id, runId }), root);
  assertNoReparseAncestors(root, "HQ candidate checkout");
  createSafeDirectories(runDir, "HQ candidate evidence", { requireNewLeaf: true });
  const stageRoot = createSafeDirectories(join(runDir, "stage"), "HQ candidate stage");
  const source = readSourceIdentity(root);
  if (source.gitDirty) throw new Error("HQ candidate chain refuses a dirty source worktree");
  const binding = stageHostBinding(options.hostBinding, stageRoot, actualHostname);
  const osProofs = {}, osProofArtifacts = {};
  const recordProof = (name, action, phase, extra = {}) => {
    const value = osProof({ repoRoot: root, action, phase, addr: listenAddr, ...extra });
    osProofs[name] = value;
    osProofArtifacts[name] = writeJson(join(runDir, `os-proof-${name}.json`), value, `HQ OS proof ${name}`);
    return value;
  };
  recordProof("preBuildVacancy", "vacant", "pre-build");
  const toolchain = pinToolchain();
  const buildSpec = hqCandidateBuildSpec(runDir, toolchain, environment);
  createSafeDirectories(join(runDir, "tmp"), "HQ candidate build temporary directory");
  let build, xwinCacheEvidence = null;
  try {
    prepareGovernedHqXwinCache(buildSpec.xwinCache, runDir);
    const cargoHomeConfigBeforeBuild = prepareGovernedHqCargoHome(buildSpec.cargoLayout);
    build = await runBuild(buildSpec.command, buildSpec.args, { cwd: root, env: buildSpec.environment, logPath: join(runDir, "build.log") });
    const cargoHomeConfigAfterBuild = safeRegularArtifact(buildSpec.cargoLayout.configPath, "HQ candidate Cargo config after build");
    if (cargoHomeConfigAfterBuild.sha256 !== cargoHomeConfigBeforeBuild.sha256 || cargoHomeConfigAfterBuild.bytes !== cargoHomeConfigBeforeBuild.bytes) throw new Error("HQ candidate Cargo config changed while building");
    const buildEvidenceRoot = createSafeDirectories(join(runDir, "build-evidence"), "HQ candidate retained build evidence", { requireNewLeaf: true });
    buildSpec.cargoHomeConfig = copySafeRegularFile(cargoHomeConfigBeforeBuild.path, join(buildEvidenceRoot, "cargo-config-before-build.toml"), "HQ candidate Cargo config before build");
    buildSpec.cargoHomeConfigAfterBuild = copySafeRegularFile(cargoHomeConfigAfterBuild.path, join(buildEvidenceRoot, "cargo-config-after-build.toml"), "HQ candidate Cargo config after build");
    xwinCacheEvidence = persistHqXwinCacheInventory(buildSpec.xwinCache, runDir);
    build.toolchain = toolchain;
    build.xwinCache = { schema: buildSpec.xwinCache.schema, path: buildSpec.xwinCache.path, inventory: evidenceReference(xwinCacheEvidence.artifact, HQ_XWIN_CACHE_INVENTORY_SCHEMA) };
    assertPinnedToolchain(toolchain);
  } catch (error) {
    let inventory = xwinCacheEvidence, inventoryError = null, buildLog = null;
    try {
      if (!inventory && existsSync(buildSpec.xwinCache.path)) inventory = persistHqXwinCacheInventory(buildSpec.xwinCache, runDir);
    } catch (cacheError) { inventoryError = cacheError.message; }
    try {
      const logPath = join(runDir, "build.log");
      if (existsSync(logPath)) buildLog = safeRegularArtifact(logPath, "HQ candidate failed build log");
    } catch (logError) { inventoryError = inventoryError || `build log unavailable: ${logError.message}`; }
    const retainedDerivedBuildState = {
      cargoTarget: { path: buildSpec.environment.CARGO_TARGET_DIR, cleanup: { attempted: false, status: existsSync(buildSpec.environment.CARGO_TARGET_DIR) ? "retained" : "absent", reason: "build failure did not perform derived build-state cleanup" } },
      cargoHome: { path: buildSpec.cargoLayout.cargoHome, cleanup: { attempted: false, status: existsSync(buildSpec.cargoLayout.cargoHome) ? "retained" : "absent", reason: "build failure did not perform derived build-state cleanup" } },
    };
    const failure = createHqCandidateBuildFailureReceipt({
      source: compactSource(source),
      buildSpec,
      buildLog,
      xwinCacheInventory: inventory ? evidenceReference(inventory.artifact, HQ_XWIN_CACHE_INVENTORY_SCHEMA) : null,
      inventoryError,
      derivedBuildState: retainedDerivedBuildState,
      error,
    });
    const failureArtifact = writeJson(join(runDir, "hq-candidate-chain-build-failure.json"), failure, "HQ candidate build failure receipt");
    throw new Error(`HQ candidate build failed; failure receipt: ${failureArtifact.path}; ${oneLine(error.message)}`);
  }
  recordProof("postBuildVacancy", "vacant", "post-build");
  sourceStillMatches(source, root, readSourceIdentity);
  const builtCutd = safeRegularArtifact(buildSpec.output, "HQ candidate cutd build output");
  const stagedCutd = copySafeRegularFile(builtCutd.path, join(stageRoot, "cutd.exe"), "HQ candidate cutd");
  const authenticode = unsignedCandidate(stagedCutd.path, stagedCutd.sha256);
  const agentDocsRoot = stageAgentDocsFn(root, stageRoot);
  const runtimeRoot = createSafeDirectories(join(stageRoot, "runtime"), "HQ candidate runtime");
  createSafeDirectories(join(runtimeRoot, "home"), "HQ candidate runtime home");
  createSafeDirectories(join(runtimeRoot, "projects"), "HQ candidate runtime projects");
  const daemonLog = join(runDir, "cutd.log");
  writeSafeText(daemonLog, "HQ candidate daemon log\n", "HQ candidate daemon log");
  const child = startDaemon(stagedCutd.path, ["serve", "--headless", "--addr", listenAddr], { cwd: stageRoot, windowsHide: true, env: { ...process.env, SHELLX_CUT_AGENT_DOCS_DIR: agentDocsRoot, SHELLX_CUT_HOME: join(runtimeRoot, "home"), SHELLX_CUT_PROJECTS_DIR: join(runtimeRoot, "projects") }, stdio: ["ignore", openSync(daemonLog, "a", 0o600), openSync(daemonLog, "a", 0o600)] });
  let hq = null, attestation = null, attestationArtifact = null, gateError = null;
  try {
    const agent = await waitForAgent({ baseUrl: endpoint.url, child, get });
    const daemon = daemonIdentity(agent, stagedCutd.path);
    if (!daemon.explicitPathMatchesReported || daemon.artifact.sha256 !== stagedCutd.sha256) throw new Error("running /api/agent runtime.executable does not bind to staged HQ candidate cutd");
    recordProof("preGateOwnership", "owned", "pre-gate", { pid: child.pid, stagedCutd: stagedCutd.path, sha256: stagedCutd.sha256 });
    const agentDocs = await verifyAgentDocs({ engineBase: endpoint.url, sourceRoot: root, expectedVersion: source.version, timeoutMs: START_TIMEOUT_MS });
    if (!agentDocs.ok) throw new Error(`staged candidate agent docs differ from source: ${agentDocs.failures.join("; ")}`);
    const hostBinding = { ...binding.staged, validation: binding.validation, checks: { ...binding.validation.checks, stagedCopyMatchesSupplied: binding.supplied.sha256 === binding.staged.sha256 } };
    const runtime = { schema: agent.schema, product: agent.product, version: agent.version, addr: agent.runtime?.addr || null, runtimeExecutable: daemon.reportedPath, executableSha256: daemon.artifact.sha256, agentDocs };
    attestation = createHqCandidateAttestation({ source: compactSource(source), build: { ...build, candidateCommand: buildSpec }, candidate: { path: daemon.reportedPath, stagedPath: stagedCutd.path, sha256: stagedCutd.sha256, bytes: stagedCutd.bytes, authenticode }, runtime, hostBinding, preGateOwnership: osProofs.preGateOwnership });
    attestationArtifact = writeJson(join(runDir, "candidate-attestation.json"), attestation, "HQ candidate attestation");
    hq = await runWorkload({ root, runDir, profile, fixturePath: options.fixture, hostBindingPath: binding.staged.path, endpoint, daemonPath: stagedCutd.path, timeoutMs: options.timeoutMs, candidateAttestation: attestation, attestationEvidence: evidenceReference(attestationArtifact, ATTESTATION_SCHEMA) });
    recordProof("postGateOwnership", "owned", "post-gate", { pid: child.pid, stagedCutd: stagedCutd.path, sha256: stagedCutd.sha256 });
  } catch (error) { gateError = error; }
  const cleanup = await stopDaemon(child, { beforeTaskkill: () => recordProof("preTaskkillOwnership", "owned", "pre-taskkill", { pid: child.pid, stagedCutd: stagedCutd.path, sha256: stagedCutd.sha256 }) });
  if (["stopped", "already-exited"].includes(cleanup.status)) {
    try { recordProof("postCleanupVacancy", "vacant", "post-cleanup"); } catch (error) { cleanup.status = "failed"; cleanup.error = `post-cleanup OS vacancy proof failed: ${oneLine(error.message)}`; }
  }
  const retainedDerivedState = (path, reason) => ({ attempted: false, status: "retained", path, reason });
  let xwinCacheCleanup = retainedDerivedState(build.xwinCache.path, "candidate daemon cleanup did not complete, so the run-owned cargo-xwin cache was retained for diagnosis");
  let cargoTargetCleanup = retainedDerivedState(buildSpec.environment.CARGO_TARGET_DIR, "candidate daemon cleanup did not complete, so the run-owned cargo target was retained for diagnosis");
  let cargoHomeCleanup = retainedDerivedState(buildSpec.cargoLayout.cargoHome, "candidate daemon cleanup did not complete, so the run-owned Cargo home was retained for diagnosis");
  if (["stopped", "already-exited"].includes(cleanup.status)) {
    try { assertHqXwinCacheInventory(xwinCacheEvidence.inventory, buildSpec.xwinCache, runDir); } catch (error) {
      xwinCacheCleanup = { attempted: false, status: "retained", path: buildSpec.xwinCache.path, error: oneLine(error.message) };
      cleanup.status = "failed";
      cleanup.error = `cargo-xwin cache inventory changed before cleanup: ${oneLine(error.message)}`;
    }
    if (["stopped", "already-exited"].includes(cleanup.status)) {
      try { xwinCacheCleanup = cleanupGovernedHqXwinCache(buildSpec.xwinCache, runDir); } catch (error) {
        xwinCacheCleanup = { attempted: true, status: "failed", path: buildSpec.xwinCache.path, error: oneLine(error.message) };
        cleanup.status = "failed";
        cleanup.error = `cargo-xwin cache cleanup failed: ${oneLine(error.message)}`;
      }
    }
    if (["stopped", "already-exited"].includes(cleanup.status)) {
      try { cargoTargetCleanup = cleanupGovernedHqDerivedBuildDirectory("cargoTarget", buildSpec.environment.CARGO_TARGET_DIR, runDir); } catch (error) {
        cargoTargetCleanup = { attempted: true, status: "failed", path: buildSpec.environment.CARGO_TARGET_DIR, error: oneLine(error.message) };
        cleanup.status = "failed";
        cleanup.error = `cargo target cleanup failed: ${oneLine(error.message)}`;
      }
    }
    if (["stopped", "already-exited"].includes(cleanup.status)) {
      try { cargoHomeCleanup = cleanupGovernedHqDerivedBuildDirectory("cargoHome", buildSpec.cargoLayout.cargoHome, runDir); } catch (error) {
        cargoHomeCleanup = { attempted: true, status: "failed", path: buildSpec.cargoLayout.cargoHome, error: oneLine(error.message) };
        cleanup.status = "failed";
        cleanup.error = `Cargo home cleanup failed: ${oneLine(error.message)}`;
      }
    }
  }
  const hqGateReceipt = hq ? assertHqReceiptArtifact(hq.receipt, hq.artifact) : null;
  const lifecycle = { schema: LIFECYCLE_SCHEMA, completedAt: new Date().toISOString(), runDir, childPid: child.pid || null, candidate: attestation ? { sha256: attestation.candidate.cutd.sha256, path: attestation.candidate.cutd.stagedPath } : null, hqGate: hqGateReceipt ? evidenceReference(hqGateReceipt, hq.receipt.schema) : { status: "failed", error: oneLine(gateError?.message || gateError) }, cleanup, xwinCache: { identity: build.xwinCache, cleanup: xwinCacheCleanup }, derivedBuildState: { cargoTarget: { path: buildSpec.environment.CARGO_TARGET_DIR, cleanup: cargoTargetCleanup }, cargoHome: { path: buildSpec.cargoLayout.cargoHome, cleanup: cargoHomeCleanup } }, osProofs: Object.fromEntries(Object.entries(osProofArtifacts).map(([name, artifact]) => [name, evidenceReference(artifact, "shellx-cut/hq-candidate-os-proof@1")])), postCleanupVacancy: osProofs.postCleanupVacancy || null };
  const lifecycleArtifact = writeJson(join(runDir, "lifecycle-receipt.json"), lifecycle, "HQ candidate lifecycle receipt");
  if (gateError || cleanup.status === "failed") {
    if (cleanup.status === "failed") throw new Error(`HQ candidate chain cleanup failed; lifecycle receipt: ${lifecycleArtifact.path}; ${cleanup.error || cleanup.status}`);
    throw gateError;
  }
  const finalReceipt = createHqCandidateReceipt({ attestation, attestationArtifact, hqGate: hq.receipt, hqGateReceipt, osProofArtifacts, lifecycle, lifecycleArtifact, runDir, candidateRoot });
  const candidateReceiptPath = join(runDir, "hq-candidate-chain-receipt.json");
  writeJson(candidateReceiptPath, finalReceipt, "HQ candidate final evidence receipt");
  return { runDir, candidateReceiptPath, lifecyclePath: lifecycleArtifact.path, hqReceiptPath: hq.receiptPath };
}

export { assertHqCandidateChainDir, hqCandidateChainUsage, parseHqCandidateChainArgs, resolveHqCandidateChainDir } from "./hq-candidate-chain-contracts.mjs";
export { assertUnsignedHqCandidateSignature, candidateAttestationSha256, createHqCandidateAttestation, createHqCandidateReceipt, stopOwnedHqCandidate } from "./hq-candidate-chain-evidence.mjs";
