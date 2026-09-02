import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { EventEmitter } from "node:events";
import { existsSync, mkdirSync, readFileSync, rmSync, rmdirSync, symlinkSync, writeFileSync } from "node:fs";
import test from "node:test";
import { dirname, join, resolve } from "node:path";

import { assertHqCandidateChainDir, assertHqCandidateListenAddr, parseHqCandidateChainArgs, resolveHqCandidateChainDir } from "../lib/hq-candidate-chain-contracts.mjs";
import { assertUnsignedHqCandidateSignature, candidateAttestationSha256, createHqCandidateAttestation, createHqCandidateReceipt, runHqCandidateChain, stopOwnedHqCandidate } from "../lib/hq-candidate-chain.mjs";
import { createHqCandidateBuildFailureReceipt } from "../lib/hq-candidate-chain-evidence.mjs";
import { assertWindowsOsProof, probeHqCandidateOs } from "../lib/hq-candidate-chain-os-proof.mjs";
import { assertNoReparseAncestors, createSafeDirectories, safeRegularArtifact, writeSafeJson } from "../lib/hq-candidate-chain-security.mjs";
import { assertNoAmbientToolDrift, governedHqCargoLayout, hqCandidateBuildSpec, isolatedHqCargoEnvironment, prepareGovernedHqCargoHome, rustupHomeForPinnedCargo } from "../lib/hq-candidate-chain-toolchain.mjs";
import { HQ_XWIN_CACHE_INVENTORY_SCHEMA, assertHqXwinCacheInventory, cleanupGovernedHqDerivedBuildDirectory, cleanupGovernedHqXwinCache, governedHqXwinCacheLayout, persistHqXwinCacheInventory, prepareGovernedHqXwinCache } from "../lib/hq-candidate-chain-xwin-cache.mjs";
import { validateHqCandidateChainEvidence } from "../lib/hq-media-identity.mjs";
import { HQ_MEDIA_GATE_SCHEMA, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA, classifyHqRenderOutcome } from "../lib/hq-render-outcome.mjs";

const root = resolve(".scratch", "public-tests", "hq-candidate-chain-third-review");
rmSync(root, { recursive: true, force: true });
mkdirSync(root, { recursive: true, mode: 0o700 });
process.on("exit", () => rmSync(root, { recursive: true, force: true }));
let serial = 0;

function write(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, typeof value === "string" ? value : `${JSON.stringify(value, null, 2)}\n`);
  return safeRegularArtifact(path, "HQ test evidence");
}

function ref(artifact, schema = null) { return { path: artifact.path, sha256: artifact.sha256, bytes: artifact.bytes, ...(schema ? { schema } : {}) }; }

function validEvidence({ cleanupMethod = "child.kill" } = {}) {
  const candidateRoot = join(root, ".scratch", "hq-candidate-chain");
  const runDir = join(candidateRoot, "hq-4k-uhd-60", `run-${serial += 1}`);
  mkdirSync(join(runDir, "stage"), { recursive: true });
  const cutd = write(join(runDir, "stage", "cutd.exe"), "candidate-cutd");
  const buildLog = write(join(runDir, "build.log"), "cargo-xwin build\n");
  const pinnedCargoXwin = write(join(runDir, "pinned", "cargo-xwin.exe"), "cargo-xwin binary\n");
  const cargoTarget = join(runDir, "cargo-target");
  const cargoHome = join(runDir, "cargo-home");
  mkdirSync(cargoTarget, { recursive: true });
  mkdirSync(cargoHome, { recursive: true });
  const cargoConfig = write(join(runDir, "build-evidence", "cargo-config.toml"), "[net]\n");
  const xwinLayout = governedHqXwinCacheLayout(runDir);
  prepareGovernedHqXwinCache(xwinLayout, runDir);
  mkdirSync(join(xwinLayout.path, "xwin", "sdk"), { recursive: true });
  writeFileSync(join(xwinLayout.path, "xwin", "sdk", "headers.txt"), "SDK cache payload\n");
  const xwinInventory = persistHqXwinCacheInventory(xwinLayout, runDir);
  const xwinCache = { schema: xwinLayout.schema, path: xwinLayout.path, inventory: ref(xwinInventory.artifact, HQ_XWIN_CACHE_INVENTORY_SCHEMA) };
  const script = write(join(runDir, "os-proof.ps1"), "proof script\n");
  const makeProof = (action, phase, vacant) => ({
    schema: "shellx-cut/hq-candidate-os-proof@1", action, phase, vacant, pid: vacant ? 0 : 451,
    listener: { address: "127.0.0.1", port: 6219, owners: vacant ? [] : [451], count: vacant ? 0 : 1, literalCount: vacant ? 0 : 1 },
    expectedExecutable: vacant ? "" : cutd.path, expectedSha256: vacant ? "" : cutd.sha256,
    process: vacant ? null : { executable: cutd.path, sha256: cutd.sha256 },
  });
  const proofValues = {
    preBuildVacancy: makeProof("vacant", "pre-build", true), postBuildVacancy: makeProof("vacant", "post-build", true),
    preGateOwnership: makeProof("owned", "pre-gate", false), postGateOwnership: makeProof("owned", "post-gate", false), postCleanupVacancy: makeProof("vacant", "post-cleanup", true),
  };
  if (cleanupMethod === "taskkill") proofValues.preTaskkillOwnership = makeProof("owned", "pre-taskkill", false);
  const proofArtifacts = Object.fromEntries(Object.entries(proofValues).map(([name, proof]) => [name, write(join(runDir, `os-proof-${name}.json`), { script, proof })]));
  const source = { gitCommit: "a".repeat(40), gitTree: "b".repeat(40), version: "0.6.109", contentManifest: { schema: "shellx-cut/source-content-manifest@1", files: 1, bytes: 1, sha256: "c".repeat(64) } };
  const buildArgs = ["build", "--package", "server"];
  const build = { command: pinnedCargoXwin.path, args: buildArgs, log: buildLog, toolchain: { cargoXwin: { executable: pinnedCargoXwin.path, artifact: pinnedCargoXwin } }, candidateCommand: { command: pinnedCargoXwin.path, args: buildArgs, cargoHomeConfig: cargoConfig, cargoHomeConfigAfterBuild: cargoConfig, cargoLayout: { cargoHome }, xwinCache: xwinLayout, environment: { XWIN_CACHE_DIR: xwinLayout.path, CARGO_TARGET_DIR: cargoTarget } }, xwinCache };
  const candidate = { path: cutd.path, stagedPath: cutd.path, sha256: cutd.sha256, bytes: cutd.bytes };
  const runtime = { runtimeExecutable: cutd.path, executableSha256: cutd.sha256, version: source.version };
  const hostBinding = { checks: { host: true } };
  const attestation = createHqCandidateAttestation({ source, build, candidate, runtime, hostBinding, preGateOwnership: { script, proof: proofValues.preGateOwnership } });
  const attestationArtifact = write(join(runDir, "candidate-attestation.json"), attestation);
  const receiptDir = join(runDir, "hq-media-gate");
  mkdirSync(join(receiptDir, "project"), { recursive: true });
  const input = write(join(runDir, "fixture.mp4"), "real-fixture-placeholder");
  const output = write(join(receiptDir, "project", "out.mp4"), "render-output");
  const inputProbe = write(join(receiptDir, "ffprobe-input.json"), { streams: [] });
  const outputProbe = write(join(receiptDir, "ffprobe-output.json"), { streams: [] });
  const projectState = write(join(receiptDir, "project-state-before-render.json"), { tracks: [] });
  const renderReceiptValue = { render_id: "render_001", output_path: output.path, output_hash: `sha256:${output.sha256}`, checks: [
    { name: "cut_on_word", pass: true, details: {}, evidence: {} },
    { name: "lufs", pass: false, details: {}, evidence: {} },
    { name: "caption_presence", pass: false, details: {}, evidence: {} },
    { name: "silence_at_edges", pass: false, details: {}, evidence: {} },
    { name: "black_or_frozen_frames", pass: true, details: {}, evidence: {} },
    { name: "duration_matches_edl", pass: true, details: {}, evidence: {} },
    { name: "uniform_border", pass: true, details: {}, evidence: {} },
    { name: "footage_profile", pass: true, details: { active_profile: "talking_head", selection: "explicit" }, evidence: {} },
  ], pass: false };
  const renderReceiptArtifact = write(join(receiptDir, "project", "receipts", "render_001.json"), renderReceiptValue);
  const renderJob = { job_id: "job_004", kind: "render", state: "done", completion: "success", outcome: "succeeded", outcome_reason: "completed", progress: 1, persistence_error: null, result: { render_id: "render_001", pass: false, path: output.path, receipt: renderReceiptArtifact.path, verified: true, verification_status: "complete" } };
  const renderOutcome = classifyHqRenderOutcome(renderJob, { jobs: [structuredClone(renderJob)], persistence_notices: [] }, { renderReceipt: renderReceiptValue, receiptArtifact: renderReceiptArtifact, outputArtifact: output });
  const editorialQc = renderOutcome.editorialQc;
  const checks = { actualRenderFinal: true, renderVerification: true, renderPersistence: true, renderTechnicalQc: true, outputProfile: true, hqGeometryRender: true };
  const hqGate = { schema: HQ_MEDIA_GATE_SCHEMA, receiptDir, pass: false, execution: { state: "completed-awaiting-owned-cleanup", terminal: false }, checks, render: { job: renderJob, jobsList: { matchingJob: structuredClone(renderJob), matchingJobCount: 1, persistenceNotices: [] }, outcome: renderOutcome }, verdicts: { hqGeometryRender: { pass: true, gating: true, classification: "pass" }, editorialQc }, artifacts: { input, output }, evidence: { ffprobeInput: ref(inputProbe), ffprobeOutput: ref(outputProbe), projectStateBeforeRender: ref(projectState), renderReceipt: ref(renderReceiptArtifact, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA) }, candidateSource: { hqCandidate: { sameRunOnly: true, backingAttestation: ref(attestationArtifact, attestation.schema) } } };
  const hqArtifact = write(join(receiptDir, "hq-media-gate-receipt.json"), hqGate);
  const xwinCacheCleanup = cleanupGovernedHqXwinCache(xwinLayout, runDir);
  const cargoTargetCleanup = cleanupGovernedHqDerivedBuildDirectory("cargoTarget", cargoTarget, runDir);
  const cargoHomeCleanup = cleanupGovernedHqDerivedBuildDirectory("cargoHome", cargoHome, runDir);
  const lifecycle = { schema: "shellx-cut/hq-candidate-chain-lifecycle@1", runDir, childPid: 451, candidate: { sha256: cutd.sha256, path: cutd.path }, hqGate: ref(hqArtifact, hqGate.schema), cleanup: { status: "stopped", method: cleanupMethod }, xwinCache: { identity: xwinCache, cleanup: xwinCacheCleanup }, derivedBuildState: { cargoTarget: { path: cargoTarget, cleanup: cargoTargetCleanup }, cargoHome: { path: cargoHome, cleanup: cargoHomeCleanup } }, osProofs: Object.fromEntries(Object.entries(proofArtifacts).map(([name, artifact]) => [name, ref(artifact, "shellx-cut/hq-candidate-os-proof@1")])), postCleanupVacancy: { script, proof: proofValues.postCleanupVacancy } };
  const lifecycleArtifact = write(join(runDir, "lifecycle-receipt.json"), lifecycle);
  const receipt = createHqCandidateReceipt({ attestation, attestationArtifact, hqGate, hqGateReceipt: hqArtifact, osProofArtifacts: proofArtifacts, lifecycle, lifecycleArtifact, runDir, candidateRoot });
  return { receipt, candidateRoot, runDir, attestation, attestationArtifact, hqGate, hqArtifact, renderReceiptArtifact, lifecycle, lifecycleArtifact, proofArtifacts, xwinLayout, xwinInventory };
}

test("candidate build explicitly confines every cargo-xwin cache input", async () => {
  const runDir = join(root, `xwin-build-${serial += 1}`);
  const home = join(runDir, "tool-user");
  const cargoBin = join(home, ".cargo", "bin");
  mkdirSync(cargoBin, { recursive: true });
  mkdirSync(join(home, ".rustup"), { recursive: true });
  const makeTool = (name) => {
    const path = join(cargoBin, name);
    writeFileSync(path, `${name}\n`);
    return { executable: path, artifact: safeRegularArtifact(path, `test ${name}`) };
  };
  const toolchain = { cargo: makeTool("cargo.exe"), rustc: makeTool("rustc.exe"), cargoXwin: makeTool("cargo-xwin.exe"), linker: makeTool("lld-link.exe") };
  const spec = hqCandidateBuildSpec(runDir, toolchain, { SystemRoot: "C:\\Windows" });
  assert.equal(spec.command, toolchain.cargoXwin.executable);
  assert.equal(spec.args[0], "build");
  assert.equal(spec.args.includes("xwin"), false);
  const ambientCargoXwin = join(runDir, "ambient-shadow", "cargo-xwin.exe");
  mkdirSync(join(runDir, "ambient-shadow"), { recursive: true });
  writeFileSync(ambientCargoXwin, "ambient shadow\n");
  assert.notEqual(spec.command, ambientCargoXwin);
  // The runner supplies this command directly to its build child. An ambient
  // cargo-xwin elsewhere is deliberately not represented in PATH or args.
  assert.equal(spec.environment.PATH.includes(join(runDir, "ambient-shadow")), false);
  assert.equal(spec.environment.XWIN_CACHE_DIR, join(runDir, "xwin-cache"));
  assert.notEqual(spec.environment.XWIN_CACHE_DIR, join(process.cwd(), "cargo-xwin"));
  prepareGovernedHqXwinCache(spec.xwinCache, runDir);
  assert.throws(() => prepareGovernedHqXwinCache(spec.xwinCache, runDir), /creation raced/);
  for (const environment of [{ XWIN_CACHE_DIR: "D:\\ambient" }, { XWIN_CROSS_COMPILER: "clang" }, { XWIN_MSVC_SYSROOT_DOWNLOAD_URL: "https://example.invalid" }, { GITHUB_TOKEN: "not-forwarded" }]) {
    assert.throws(() => assertNoAmbientToolDrift(environment), /inherited (cargo-xwin|Cargo\/Rust\/toolchain)/);
  }
});

test("forbidden cargo-xwin environment fails before candidate probes or tool resolution", async () => {
  let probes = 0, tools = 0;
  await assert.rejects(
    runHqCandidateChain({ profile: "hq-4k-uhd-60", fixture: "fixture.mp4", hostBinding: "binding.json" }, {
      hostPlatform: "win32",
      environment: { XWIN_CACHE_DIR: "D:\\ambient" },
      osProof: () => { probes += 1; throw new Error("must not probe"); },
      pinToolchain: () => { tools += 1; throw new Error("must not resolve tools"); },
    }),
    /inherited cargo-xwin environment/,
  );
  assert.equal(probes, 0);
  assert.equal(tools, 0);
});

test("candidate-chain refuses a cache replacement after inventory snapshot and cannot write final evidence", async () => {
  const runId = `mutated-cache-${serial += 1}`;
  const repoRoot = resolve(".");
  const runDir = join(repoRoot, ".scratch", "hq-candidate-chain", "hq-4k-uhd-60", runId);
  const toolRoot = join(root, `runner-tools-${serial += 1}`, "user");
  const cargoBin = join(toolRoot, ".cargo", "bin");
  mkdirSync(cargoBin, { recursive: true });
  mkdirSync(join(toolRoot, ".rustup"), { recursive: true });
  const makeTool = (name) => {
    const path = join(cargoBin, name);
    writeFileSync(path, `${name}\n`);
    return { executable: path, artifact: safeRegularArtifact(path, `runner ${name}`) };
  };
  const toolchain = { cargo: makeTool("cargo.exe"), rustc: makeTool("rustc.exe"), cargoXwin: makeTool("cargo-xwin.exe"), linker: makeTool("lld-link.exe") };
  const proofScript = write(join(root, `runner-proof-${serial += 1}.ps1`), "proof\n");
  const stagedCutd = join(runDir, "stage", "cutd.exe");
  const sha256 = () => safeRegularArtifact(stagedCutd, "runner staged cutd").sha256;
  class Child extends EventEmitter { constructor() { super(); this.pid = 451; this.exitCode = null; } }
  try {
    await assert.rejects(
      runHqCandidateChain({ profile: "hq-4k-uhd-60", fixture: "fixture.mp4", hostBinding: "binding.json", timeoutMs: 10 }, {
        repoRoot,
        hostPlatform: "win32",
        actualHostname: "test-host",
        runId,
        environment: { SystemRoot: "C:\\Windows" },
        readSourceIdentity: () => ({ gitDirty: false, gitCommit: "a".repeat(40), gitTree: "b".repeat(40), version: "0.6.110", contentManifest: { schema: "shellx-cut/source-content-manifest@1", files: 1, bytes: 1, sha256: "c".repeat(64) } }),
        stageAgentDocsFn: (_root, stageRoot) => {
          const docsRoot = join(stageRoot, "agent-docs");
          mkdirSync(docsRoot, { recursive: true });
          return docsRoot;
        },
        pinToolchain: () => toolchain,
        stageHostBinding: (_path, stageRoot) => {
          const artifact = write(join(stageRoot, "hq-host-binding.json"), "binding\n");
          return { supplied: artifact, staged: artifact, validation: { checks: { testBinding: true } } };
        },
        osProof: ({ action, phase, pid = 0, stagedCutd: expected = "", sha256: expectedSha = "" }) => ({
          script: proofScript,
          proof: action === "vacant"
            ? { schema: "shellx-cut/hq-candidate-os-proof@1", action, phase, vacant: true, pid: 0, listener: { address: "127.0.0.1", port: 6219, owners: [], count: 0, literalCount: 0 } }
            : { schema: "shellx-cut/hq-candidate-os-proof@1", action, phase, vacant: false, pid, listener: { address: "127.0.0.1", port: 6219, owners: [pid], count: 1, literalCount: 1 }, expectedExecutable: expected, expectedSha256: expectedSha, process: { executable: expected, sha256: expectedSha } },
        }),
        runBuild: async (command, args, { env, logPath }) => {
          assert.equal(command, toolchain.cargoXwin.executable);
          assert.equal(args[0], "build");
          mkdirSync(join(env.CARGO_TARGET_DIR, "x86_64-pc-windows-msvc", "release"), { recursive: true });
          writeFileSync(join(env.CARGO_TARGET_DIR, "x86_64-pc-windows-msvc", "release", "cutd.exe"), "candidate cutd\n");
          mkdirSync(join(env.XWIN_CACHE_DIR, "xwin"), { recursive: true });
          writeFileSync(join(env.XWIN_CACHE_DIR, "xwin", "sdk.txt"), "original cache\n");
          writeFileSync(logPath, "direct cargo-xwin build\n");
          return { command, args, cwd: repoRoot, environment: env, log: safeRegularArtifact(logPath, "runner build log") };
        },
        unsignedCandidate: (_path, hash) => ({ status: "NotSigned", sha256: hash }),
        startDaemon: () => new Child(),
        get: async () => ({ schema: "shellx-cut/agent-docs/2", product: "ShellX Cut", version: "0.6.110", runtime: { executable: stagedCutd, addr: "127.0.0.1:6219" } }),
        verifyAgentDocs: async () => ({ ok: true, failures: [] }),
        runWorkload: async ({ runDir: activeRunDir }) => {
          writeFileSync(join(activeRunDir, "xwin-cache", "xwin", "sdk.txt"), "replaced after inventory\n");
          const receipt = { schema: HQ_MEDIA_GATE_SCHEMA, pass: false, checks: { synthetic: true } };
          const artifact = write(join(activeRunDir, "hq-media-gate-receipt.json"), receipt);
          return { receipt, receiptPath: artifact.path, artifact };
        },
        stopDaemon: async () => ({ status: "stopped", pid: 451, method: "synthetic", error: null }),
      }),
      /cargo-xwin cache inventory changed before cleanup/,
    );
    assert.equal(existsSync(join(runDir, "hq-candidate-chain-receipt.json")), false);
    const lifecycle = JSON.parse(readFileSync(join(runDir, "lifecycle-receipt.json"), "utf8"));
    assert.equal(lifecycle.cleanup.status, "failed");
    assert.equal(lifecycle.xwinCache.cleanup.status, "retained");
    assert.equal(lifecycle.xwinCache.cleanup.attempted, false);
    assert.equal(existsSync(join(runDir, "xwin-cache")), true);
    assert.equal(existsSync(join(runDir, "cargo-target")), true);
    assert.equal(existsSync(join(runDir, "cargo-home")), true);
    assert.match(sha256(), /^[a-f0-9]{64}$/);
  } finally {
    rmSync(runDir, { recursive: true, force: true });
  }
});

test("cargo-xwin inventory rehashes run-owned cache and build failures retain explicit cleanup truth", () => {
  const runDir = join(root, `xwin-inventory-${serial += 1}`);
  mkdirSync(runDir, { recursive: true });
  const layout = governedHqXwinCacheLayout(runDir);
  prepareGovernedHqXwinCache(layout, runDir);
  mkdirSync(join(layout.path, "xwin"), { recursive: true });
  writeFileSync(join(layout.path, "xwin", "sdk.txt"), "first\n");
  const persisted = persistHqXwinCacheInventory(layout, runDir);
  assert.equal(assertHqXwinCacheInventory(persisted.inventory, layout, runDir).cache.contentSha256, persisted.inventory.cache.contentSha256);
  writeFileSync(join(layout.path, "xwin", "sdk.txt"), "changed\n");
  assert.throws(() => assertHqXwinCacheInventory(persisted.inventory, layout, runDir), /no longer matches/);
  assert.throws(() => cleanupGovernedHqXwinCache(layout, runDir, { removeOwned: () => { throw new Error("simulated removal failure"); } }), /simulated removal failure/);
  assert.equal(cleanupGovernedHqXwinCache(layout, runDir).status, "removed");
  assert.equal(existsSync(layout.path), false);
  for (const kind of ["cargoTarget", "cargoHome"]) {
    const path = join(runDir, kind === "cargoTarget" ? "cargo-target" : "cargo-home");
    mkdirSync(path, { recursive: true });
    assert.throws(() => cleanupGovernedHqDerivedBuildDirectory(kind, path, runDir, { removeOwned: () => { throw new Error(`${kind} removal failure`); } }), /removal failure/);
    assert.equal(cleanupGovernedHqDerivedBuildDirectory(kind, path, runDir).status, "removed");
    assert.equal(existsSync(path), false);
  }
  const failureBuildSpec = { xwinCache: layout, cargoLayout: { cargoHome: join(runDir, "cargo-home") }, environment: { XWIN_CACHE_DIR: layout.path, CARGO_TARGET_DIR: join(runDir, "cargo-target") } };
  const failureDerivedBuildState = { cargoTarget: { path: failureBuildSpec.environment.CARGO_TARGET_DIR, cleanup: { attempted: false, status: "absent" } }, cargoHome: { path: failureBuildSpec.cargoLayout.cargoHome, cleanup: { attempted: false, status: "absent" } } };
  const failure = createHqCandidateBuildFailureReceipt({ source: { gitCommit: "a".repeat(40) }, buildSpec: failureBuildSpec, xwinCacheInventory: ref(persisted.artifact, HQ_XWIN_CACHE_INVENTORY_SCHEMA), derivedBuildState: failureDerivedBuildState, error: new Error("build failed") });
  assert.equal(failure.authorization, "none");
  assert.deepEqual(failure.cleanup.xwinCache, { attempted: false, status: "retained", reason: "build failure retains the confined cache for explicit inspection; no automatic cache deletion ran" });
  assert.throws(() => createHqCandidateBuildFailureReceipt({ source: { gitCommit: "a".repeat(40) }, buildSpec: { ...failureBuildSpec, environment: { ...failureBuildSpec.environment, XWIN_CACHE_DIR: "D:\\ambient" } }, derivedBuildState: failureDerivedBuildState, error: new Error("build failed") }), /exact configured run-owned/);
});

test("candidate CLI has no caller-selected evidence root or receipt authorization input", () => {
  assert.deepEqual(parseHqCandidateChainArgs([]), { profile: "", fixture: "", hostBinding: "", addr: "127.0.0.1:6219", timeoutMs: 7_200_000, help: false });
  for (const option of ["--out", "--candidate-receipt", "--daemon", "--allow-non-loopback"]) assert.throws(() => parseHqCandidateChainArgs([option, "x"]), /unknown option/);
  const checkout = resolve("/work/shellx-cut");
  const run = resolveHqCandidateChainDir({ repoRoot: checkout, profile: "hq-4k-uhd-60", runId: "run-001" });
  assert.equal(assertHqCandidateChainDir(run, checkout), run);
  assert.throws(() => assertHqCandidateChainDir(join(checkout, ".scratch", "hq-media-gate", "run"), checkout), /candidate-chain evidence/);
});

test("OS ownership proof requires exactly one literal-loopback listener owned by staged cutd", () => {
  const owned = { schema: "shellx-cut/hq-candidate-os-proof@1", action: "owned", phase: "pre-gate", pid: 451, listener: { address: "127.0.0.1", port: 6219, owners: [451], count: 1, literalCount: 1 }, expectedExecutable: "C:\\CutQ\\cutd.exe", expectedSha256: "d".repeat(64), process: { executable: "C:\\CutQ\\cutd.exe", sha256: "d".repeat(64) } };
  assert.equal(assertWindowsOsProof(owned, { action: "owned", phase: "pre-gate", addr: "127.0.0.1:6219", pid: 451, stagedCutd: owned.process.executable, sha256: owned.process.sha256 }), owned);
  for (const listener of [{ ...owned.listener, count: 2 }, { ...owned.listener, literalCount: 2 }, { ...owned.listener, owners: [999] }]) assert.throws(() => assertWindowsOsProof({ ...owned, listener }, { action: "owned", phase: "pre-gate", addr: "127.0.0.1:6219", pid: 451, stagedCutd: owned.process.executable, sha256: owned.process.sha256 }));
  assert.throws(() => probeHqCandidateOs({ repoRoot: resolve("."), action: "vacant", phase: "pre-build", addr: "127.0.0.1:6219", spawnProcess: () => ({ status: 1, stderr: "denied" }) }), /failed closed/);
});

test("final chain is evidence-only and deeply rehashes persisted chain artifacts", () => {
  const fixture = validEvidence();
  assert.equal(fixture.receipt.authorization, "none");
  assert.equal(validateHqCandidateChainEvidence(fixture.receipt).hqGate.value.pass, false);
  for (const artifact of [fixture.attestationArtifact, fixture.hqArtifact, fixture.renderReceiptArtifact, fixture.lifecycleArtifact, ...Object.values(fixture.proofArtifacts)]) {
    const fresh = validEvidence();
    const target = artifact === fixture.attestationArtifact ? fresh.attestationArtifact : artifact === fixture.hqArtifact ? fresh.hqArtifact : artifact === fixture.renderReceiptArtifact ? fresh.renderReceiptArtifact : artifact === fixture.lifecycleArtifact ? fresh.lifecycleArtifact : fresh.proofArtifacts[Object.entries(fixture.proofArtifacts).find(([, value]) => value === artifact)[0]];
    writeFileSync(target.path, "forged evidence\n");
    assert.throws(() => validateHqCandidateChainEvidence(fresh.receipt), /hash or byte count|not valid JSON/);
  }
  assert.throws(() => validateHqCandidateChainEvidence({ schema: "shellx-cut/windows-installed-source@1" }), /evidence-only/);
  assert.match(candidateAttestationSha256(validEvidence().attestation), /^[a-f0-9]{64}$/);
});

test("legacy @1 candidate evidence remains inspectable without being promoted to @2", () => {
  const fresh = validEvidence();
  const hqGate = structuredClone(fresh.hqGate);
  hqGate.schema = "shellx-cut/hq-media-gate@1";
  hqGate.checks = { actualRenderFinal: true, outputProfile: true };
  delete hqGate.render;
  delete hqGate.verdicts;
  delete hqGate.evidence.renderReceipt;
  const hqArtifact = write(fresh.hqArtifact.path, hqGate);
  const lifecycle = { ...fresh.lifecycle, hqGate: ref(hqArtifact, hqGate.schema) };
  const lifecycleArtifact = write(fresh.lifecycleArtifact.path, lifecycle);
  const legacy = { ...fresh.receipt, schema: "shellx-cut/hq-candidate-chain@1", lifecycle, evidence: { ...fresh.receipt.evidence, hqGateReceipt: ref(hqArtifact, hqGate.schema), lifecycle: ref(lifecycleArtifact, lifecycle.schema) } };
  assert.equal(validateHqCandidateChainEvidence(legacy).hqGate.value.schema, "shellx-cut/hq-media-gate@1");
  assert.throws(() => createHqCandidateReceipt({ ...fresh, hqGate, hqGateReceipt: hqArtifact, lifecycle, lifecycleArtifact }), /versioned provisional HQ receipt/);
});

test("final chain rejects rewritten HQ geometry, editorial, or persistence verdicts", () => {
  const mutateHqEvidence = (mutate) => {
    const fresh = validEvidence();
    const hqGate = JSON.parse(JSON.stringify(fresh.hqGate));
    mutate(hqGate);
    const hqArtifact = write(fresh.hqArtifact.path, hqGate);
    const lifecycle = { ...fresh.lifecycle, hqGate: ref(hqArtifact, hqGate.schema) };
    const lifecycleArtifact = write(fresh.lifecycleArtifact.path, lifecycle);
    return { ...fresh.receipt, lifecycle, evidence: { ...fresh.receipt.evidence, hqGateReceipt: ref(hqArtifact, hqGate.schema), lifecycle: ref(lifecycleArtifact, lifecycle.schema) } };
  };
  assert.throws(() => validateHqCandidateChainEvidence(mutateHqEvidence((hqGate) => { hqGate.verdicts.editorialQc.gating = true; })), /classified editorial checks/);
  assert.throws(() => validateHqCandidateChainEvidence(mutateHqEvidence((hqGate) => { hqGate.render.jobsList.persistenceNotices = [{ path: "jobs/job_004.json" }]; })), /zero persistence notices/);
  assert.throws(() => validateHqCandidateChainEvidence(mutateHqEvidence((hqGate) => { hqGate.verdicts.hqGeometryRender.pass = false; })), /geometry\/render verdict/);
  assert.throws(() => validateHqCandidateChainEvidence(mutateHqEvidence((hqGate) => { hqGate.render.job.state = "failed"; hqGate.render.jobsList.matchingJob.state = "failed"; })), /state=done/);
  assert.throws(() => validateHqCandidateChainEvidence(mutateHqEvidence((hqGate) => { hqGate.render.job.result.verified = false; hqGate.render.jobsList.matchingJob.result.verified = false; })), /result\.verified=true/);
  assert.throws(() => validateHqCandidateChainEvidence(mutateHqEvidence((hqGate) => { hqGate.render.job.persistence_error = "Access is denied"; hqGate.render.jobsList.matchingJob.persistence_error = "Access is denied"; })), /persistence_error/);
});

test("final chain recomputes technical failures from the hash-bound render receipt", () => {
  const mutateReceiptEvidence = (checkName) => {
    const fresh = validEvidence();
    const renderReceipt = JSON.parse(readFileSync(fresh.renderReceiptArtifact.path, "utf8"));
    renderReceipt.checks.find((check) => check.name === checkName).pass = false;
    const renderReceiptArtifact = write(fresh.renderReceiptArtifact.path, renderReceipt);
    const hqGate = structuredClone(fresh.hqGate);
    hqGate.evidence.renderReceipt = ref(renderReceiptArtifact, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA);
    hqGate.render.outcome.renderReceipt.artifact = ref(renderReceiptArtifact);
    const hqArtifact = write(fresh.hqArtifact.path, hqGate);
    const lifecycle = { ...fresh.lifecycle, hqGate: ref(hqArtifact, hqGate.schema) };
    const lifecycleArtifact = write(fresh.lifecycleArtifact.path, lifecycle);
    return { ...fresh.receipt, lifecycle, evidence: { ...fresh.receipt.evidence, hqGateReceipt: ref(hqArtifact, hqGate.schema), lifecycle: ref(lifecycleArtifact, lifecycle.schema) } };
  };
  for (const checkName of ["uniform_border", "black_or_frozen_frames", "duration_matches_edl"]) {
    assert.throws(() => validateHqCandidateChainEvidence(mutateReceiptEvidence(checkName)), /technical or unknown checks/);
  }
});

test("final chain rejects coherently missing or empty persisted render IDs", () => {
  const mutateRenderId = (renderId) => {
    const fresh = validEvidence();
    const renderReceipt = JSON.parse(readFileSync(fresh.renderReceiptArtifact.path, "utf8"));
    if (renderId === undefined) delete renderReceipt.render_id;
    else renderReceipt.render_id = renderId;
    const renderReceiptArtifact = write(fresh.renderReceiptArtifact.path, renderReceipt);
    const hqGate = structuredClone(fresh.hqGate);
    for (const job of [hqGate.render.job, hqGate.render.jobsList.matchingJob]) {
      if (renderId === undefined) delete job.result.render_id;
      else job.result.render_id = renderId;
    }
    hqGate.evidence.renderReceipt = ref(renderReceiptArtifact, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA);
    hqGate.render.outcome.renderReceipt.artifact = ref(renderReceiptArtifact);
    hqGate.render.outcome.renderReceipt.renderId = renderId;
    const hqArtifact = write(fresh.hqArtifact.path, hqGate);
    const lifecycle = { ...fresh.lifecycle, hqGate: ref(hqArtifact, hqGate.schema) };
    const lifecycleArtifact = write(fresh.lifecycleArtifact.path, lifecycle);
    return { ...fresh.receipt, lifecycle, evidence: { ...fresh.receipt.evidence, hqGateReceipt: ref(hqArtifact, hqGate.schema), lifecycle: ref(lifecycleArtifact, lifecycle.schema) } };
  };
  for (const renderId of [undefined, ""]) assert.throws(() => validateHqCandidateChainEvidence(mutateRenderId(renderId)), /non-empty (job result\.render_id|render receipt render_id)/);
});

test("final chain requires the complete explicit talking-head receipt battery", () => {
  const mutateRenderReceipt = (mutate) => {
    const fresh = validEvidence();
    const renderReceipt = JSON.parse(readFileSync(fresh.renderReceiptArtifact.path, "utf8"));
    mutate(renderReceipt);
    const renderReceiptArtifact = write(fresh.renderReceiptArtifact.path, renderReceipt);
    const hqGate = structuredClone(fresh.hqGate);
    hqGate.evidence.renderReceipt = ref(renderReceiptArtifact, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA);
    hqGate.render.outcome.renderReceipt.artifact = ref(renderReceiptArtifact);
    const hqArtifact = write(fresh.hqArtifact.path, hqGate);
    const lifecycle = { ...fresh.lifecycle, hqGate: ref(hqArtifact, hqGate.schema) };
    const lifecycleArtifact = write(fresh.lifecycleArtifact.path, lifecycle);
    return { ...fresh.receipt, lifecycle, evidence: { ...fresh.receipt.evidence, hqGateReceipt: ref(hqArtifact, hqGate.schema), lifecycle: ref(lifecycleArtifact, lifecycle.schema) } };
  };
  for (const checkName of ["lufs", "caption_presence", "silence_at_edges", "footage_profile"]) {
    assert.throws(() => validateHqCandidateChainEvidence(mutateRenderReceipt((receipt) => { receipt.checks = receipt.checks.filter((check) => check.name !== checkName); })), /missing required strict check/);
  }
  assert.throws(() => validateHqCandidateChainEvidence(mutateRenderReceipt((receipt) => { receipt.checks.find((check) => check.name === "footage_profile").details.active_profile = "silent_screen_demo"; })), /footage profile active_profile/);
  assert.throws(() => validateHqCandidateChainEvidence(mutateRenderReceipt((receipt) => { receipt.checks.find((check) => check.name === "footage_profile").details.selection = "default"; })), /footage profile selection/);
  assert.throws(() => validateHqCandidateChainEvidence(mutateRenderReceipt((receipt) => {
    for (const name of ["lufs", "caption_presence", "silence_at_edges"]) {
      const check = receipt.checks.find((item) => item.name === name);
      check.pass = true;
      check.details = { waived_by_profile: "silent_screen_demo", waiver_reason: "fixture", measured_pass: false };
    }
    receipt.pass = true;
  })), /profile waiver fields/);
});

test("evidence inspection rejects structural source, runtime, host, hash, signed-final, and reuse tampering", () => {
  const receipt = validEvidence().receipt;
  const cases = [
    { ...receipt, source: { ...receipt.source, gitTree: "f".repeat(40) } },
    { ...receipt, runtime: { agent: { ...receipt.runtime.agent, runtimeExecutable: join(root, "other.exe") } } },
    { ...receipt, hostBinding: { ...receipt.hostBinding, checks: { host: false } } },
    { ...receipt, candidate: { cutd: { ...receipt.candidate.cutd, sha256: "f".repeat(64) } } },
    { ...receipt, signedFinal: true },
    { ...receipt, authorization: "hq-media-gate" },
  ];
  for (const tampered of cases) assert.throws(() => validateHqCandidateChainEvidence(tampered));
  const cacheRemains = validEvidence();
  mkdirSync(cacheRemains.xwinLayout.path, { recursive: true });
  assert.throws(() => validateHqCandidateChainEvidence(cacheRemains.receipt), /cache cleanup is incomplete/);
  const cargoTargetRemains = validEvidence();
  mkdirSync(cargoTargetRemains.receipt.build.candidateCommand.environment.CARGO_TARGET_DIR, { recursive: true });
  assert.throws(() => validateHqCandidateChainEvidence(cargoTargetRemains.receipt), /cargoTarget cleanup is incomplete/);
});

test("successful taskkill evidence cross-binds its lifecycle proof, child PID, and candidate", () => {
  const fixture = validEvidence({ cleanupMethod: "taskkill" });
  assert.equal(validateHqCandidateChainEvidence(fixture.receipt).proofs.preTaskkillOwnership.value.proof.phase, "pre-taskkill");
  const mutateLifecycle = (mutate) => {
    const fresh = validEvidence({ cleanupMethod: "taskkill" });
    const lifecycle = structuredClone(fresh.lifecycle);
    mutate(lifecycle, fresh.receipt);
    const artifact = write(fresh.lifecycleArtifact.path, lifecycle);
    return { ...fresh.receipt, lifecycle, evidence: { ...fresh.receipt.evidence, lifecycle: ref(artifact, "shellx-cut/hq-candidate-chain-lifecycle@1") } };
  };
  const proofRef = mutateLifecycle((lifecycle, receipt) => { lifecycle.osProofs.preTaskkillOwnership = receipt.evidence.osProofs.preGateOwnership; });
  const childPid = mutateLifecycle((lifecycle) => { lifecycle.childPid = 999; });
  const candidate = mutateLifecycle((lifecycle) => { lifecycle.candidate.sha256 = "f".repeat(64); });
  assert.throws(() => validateHqCandidateChainEvidence(proofRef), /does not bind OS proof preTaskkillOwnership/);
  assert.throws(() => validateHqCandidateChainEvidence(childPid), /lifecycle child PID or candidate identity/);
  assert.throws(() => validateHqCandidateChainEvidence(candidate), /lifecycle child PID or candidate identity/);
});

test("signed candidate and failed cleanup cannot create final evidence", () => {
  const fixture = validEvidence();
  const failedLifecycle = { ...fixture.lifecycle, cleanup: { status: "failed", method: "child.kill" }, postCleanupVacancy: { ...fixture.lifecycle.postCleanupVacancy, proof: { ...fixture.lifecycle.postCleanupVacancy.proof, vacant: false } } };
  assert.throws(() => createHqCandidateReceipt({ ...fixture, hqGateReceipt: fixture.hqArtifact, lifecycle: failedLifecycle }), /final HQ candidate receipt refuses cleanup that is not independently proven vacant/);
  const failedCacheLifecycle = { ...fixture.lifecycle, xwinCache: { ...fixture.lifecycle.xwinCache, cleanup: { attempted: true, status: "failed", path: fixture.xwinLayout.path, error: "simulated removal failure" } } };
  assert.throws(() => createHqCandidateReceipt({ ...fixture, hqGateReceipt: fixture.hqArtifact, lifecycle: failedCacheLifecycle }), /confined cargo-xwin cache/);
  assert.deepEqual(assertUnsignedHqCandidateSignature({ status: "NotSigned", sha256: "d".repeat(64) }, "d".repeat(64)), { status: "NotSigned", sha256: "d".repeat(64) });
  assert.throws(() => assertUnsignedHqCandidateSignature({ status: "Valid", sha256: "d".repeat(64) }, "d".repeat(64)), /unsigned/);
});

test("fresh Cargo home ignores a hostile inherited profile in a real Cargo subprocess", () => {
  const runDir = join(root, `cargo-${serial += 1}`);
  mkdirSync(runDir, { recursive: true });
  const hostile = join(runDir, "hostile-profile", ".cargo");
  mkdirSync(hostile, { recursive: true });
  writeFileSync(join(hostile, "config.toml"), '[alias]\nhqhostile = "version"\n');
  const layout = governedHqCargoLayout(runDir);
  prepareGovernedHqCargoHome(layout);
  const cargoPath = String(spawnSync(process.platform === "win32" ? "where.exe" : "which", ["cargo"], { encoding: "utf8" }).stdout).split(/\r?\n/).find(Boolean)?.trim();
  const env = { PATH: process.env.PATH, HOME: join(runDir, "hostile-profile"), USERPROFILE: join(runDir, "hostile-profile"), CARGO_HOME: hostile, RUSTUP_HOME: rustupHomeForPinnedCargo(cargoPath), ...isolatedHqCargoEnvironment(layout) };
  const result = spawnSync("cargo", ["hqhostile"], { cwd: runDir, env, encoding: "utf8" });
  const output = `${result.stdout}\n${result.stderr}`;
  assert.notEqual(result.status, 0);
  assert.match(output, /no such command|unknown command/i);
  assert.throws(() => assertNoAmbientToolDrift({ CARGO_TARGET_DIR: "D:\\other" }), /inherited Cargo/);
  assert.equal(assertNoAmbientToolDrift({ SystemRoot: "C:\\Windows" }), true);
});

test("creation races and reparse swaps fail closed", () => {
  const path = join(root, `race-${serial += 1}`);
  mkdirSync(join(path, "real"), { recursive: true });
  let swapped = false;
  const hostileMkdir = (target, options) => { mkdirSync(target, options); if (!swapped && target.endsWith("target")) { swapped = true; rmdirSync(target); symlinkSync(join(path, "real"), target, "dir"); } };
  assert.throws(() => createSafeDirectories(join(path, "target", "child"), "HQ candidate root", { mkdir: hostileMkdir }), /symlink|junction|reparse/);
  assert.throws(() => assertNoReparseAncestors(join(path, "target", "child"), "HQ candidate root"), /symlink|junction|reparse/);
  const xwinRun = join(path, "xwin-run");
  mkdirSync(xwinRun, { recursive: true });
  const xwinLayout = governedHqXwinCacheLayout(xwinRun);
  symlinkSync(join(path, "real"), xwinLayout.path, "dir");
  assert.throws(() => prepareGovernedHqXwinCache(xwinLayout, xwinRun), /creation raced/);
  assert.throws(() => assertNoReparseAncestors(xwinLayout.path, "HQ candidate cargo-xwin cache"), /symlink|junction|reparse/);
});

test("taskkill fallback rechecks owned listener and reports cleanup failure", async () => {
  class Child extends EventEmitter { constructor() { super(); this.pid = 451; this.exitCode = null; } kill() { return true; } }
  let checked = 0;
  const child = new Child();
  const result = await stopOwnedHqCandidate(child, { timeoutMs: 1, beforeTaskkill: () => { checked += 1; throw new Error("PID reused"); }, forceKill: () => { throw new Error("must not taskkill"); } });
  assert.equal(checked, 1);
  assert.equal(result.method, "taskkill-precheck");
  await assert.rejects(runHqCandidateChain({ profile: "hq-4k-uhd-60", fixture: "x", hostBinding: "x" }, { hostPlatform: "linux" }), /native Windows/);
});
