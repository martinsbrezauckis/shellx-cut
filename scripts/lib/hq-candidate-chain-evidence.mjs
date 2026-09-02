import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { AGENT_DOC_PATHS } from "./agent-docs.mjs";
import { assertDirectPinnedCargoXwinBuild } from "./hq-candidate-chain-contracts.mjs";
import { HQ_CANDIDATE_CHAIN_RECEIPT_SCHEMA, oneLine, readHqHostBinding } from "./hq-media-identity.mjs";
import { HQ_MEDIA_GATE_SCHEMA, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA, assertHqRenderVerdicts } from "./hq-render-outcome.mjs";
import { assertHqHost, hqHostBindingValidation } from "./hq-media-host-binding.mjs";
import { assertPathInside, copySafeRegularFile, safeRegularArtifact } from "./hq-candidate-chain-security.mjs";
import { HQ_XWIN_CACHE_INVENTORY_SCHEMA, HQ_XWIN_CACHE_SCHEMA } from "./hq-candidate-chain-xwin-cache.mjs";

export const ATTESTATION_SCHEMA = "shellx-cut/hq-candidate-attestation@1";
export const LIFECYCLE_SCHEMA = "shellx-cut/hq-candidate-chain-lifecycle@1";
export const BUILD_FAILURE_SCHEMA = "shellx-cut/hq-candidate-chain-build-failure@1";

function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`;
  return JSON.stringify(value);
}

export function candidateAttestationSha256(attestation) {
  return createHash("sha256").update(canonical(attestation)).digest("hex");
}

export function compactSource(source) {
  return { gitCommit: source.gitCommit, gitTree: source.gitTree, version: source.version, contentManifest: { schema: source.contentManifest.schema, files: source.contentManifest.files, bytes: source.contentManifest.bytes, sha256: source.contentManifest.sha256 } };
}

export function evidenceReference(artifact, schema = null) {
  return { path: artifact?.path || null, sha256: artifact?.sha256 || null, bytes: artifact?.bytes || null, ...(schema ? { schema } : {}) };
}

export function stageAgentDocs(root, stageRoot) {
  const docsRoot = join(stageRoot, "agent-docs");
  for (const relativePath of AGENT_DOC_PATHS) copySafeRegularFile(join(root, relativePath), join(docsRoot, relativePath), `candidate agent document ${relativePath}`);
  return docsRoot;
}

export function stageBinding(path, stageRoot, actualHostname) {
  safeRegularArtifact(path, "HQ host binding");
  const supplied = readHqHostBinding(path, actualHostname);
  if (!supplied.binding || supplied.error) throw new Error(supplied.error || "HQ host binding is unavailable");
  const validation = hqHostBindingValidation(supplied.binding, actualHostname);
  assertHqHost("win32", supplied.binding, actualHostname);
  const stagedArtifact = copySafeRegularFile(supplied.evidence.path, join(stageRoot, "hq-host-binding.json"), "HQ host binding");
  const staged = readHqHostBinding(stagedArtifact.path, actualHostname);
  if (!staged.binding || staged.error || staged.evidence.sha256 !== supplied.evidence.sha256) throw new Error("staged HQ host binding differs from the explicit private binding");
  return { supplied: supplied.evidence, staged: staged.evidence, validation };
}

export function assertUnsignedHqCandidateSignature(identity, expectedSha256) {
  if (identity?.status !== "NotSigned") throw new Error(`HQ candidate cutd must be unsigned, got Authenticode status ${identity?.status || "missing"}`);
  if (identity.sha256 !== expectedSha256) throw new Error("HQ candidate Authenticode probe hash differs from staged cutd");
  return identity;
}

export function unsignedHqCandidateSignature(path, expectedSha256) {
  const script = '$ErrorActionPreference="Stop";$sig=Get-AuthenticodeSignature -LiteralPath $env:SHELLX_HQ_CANDIDATE_CUTD;[pscustomobject]@{status=[string]$sig.Status;sha256=(Get-FileHash -LiteralPath $env:SHELLX_HQ_CANDIDATE_CUTD -Algorithm SHA256).Hash.ToLower()}|ConvertTo-Json -Compress';
  const result = spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8", windowsHide: true, env: { ...process.env, SHELLX_HQ_CANDIDATE_CUTD: path } });
  if (result.status !== 0) throw new Error(`HQ candidate Authenticode probe failed: ${oneLine(result.stderr || result.stdout || result.error?.message)}`);
  try { return assertUnsignedHqCandidateSignature(JSON.parse(result.stdout), expectedSha256); } catch (error) { throw new Error(`HQ candidate Authenticode evidence is invalid: ${error.message}`); }
}

function awaitExit(child, timeoutMs) {
  if (child.exitCode !== null && child.exitCode !== undefined) return Promise.resolve(true);
  return new Promise((done) => {
    const timer = setTimeout(() => done(false), timeoutMs);
    child.once("exit", () => { clearTimeout(timer); done(true); });
  });
}

export async function stopOwnedHqCandidate(child, { timeoutMs = 15_000, beforeTaskkill = null, forceKill = (pid) => spawnSync("taskkill.exe", ["/PID", String(pid), "/T", "/F"], { encoding: "utf8", windowsHide: true }).status === 0 } = {}) {
  if (!child?.pid) return { status: "not-started", pid: null, method: null, error: null };
  if (child.exitCode !== null && child.exitCode !== undefined) return { status: "already-exited", pid: child.pid, method: null, error: null };
  try { if (!child.kill()) return { status: "failed", pid: child.pid, method: "child.kill", error: "child.kill returned false" }; } catch (error) { return { status: "failed", pid: child.pid, method: "child.kill", error: oneLine(error.message) }; }
  if (await awaitExit(child, timeoutMs)) return { status: "stopped", pid: child.pid, method: "child.kill", error: null };
  try { if (beforeTaskkill) await beforeTaskkill(); } catch (error) { return { status: "failed", pid: child.pid, method: "taskkill-precheck", error: `OS ownership recheck refused taskkill: ${oneLine(error.message)}` }; }
  if (!forceKill(child.pid)) return { status: "failed", pid: child.pid, method: "taskkill", error: "taskkill failed after child.kill timeout" };
  if (await awaitExit(child, timeoutMs)) return { status: "stopped", pid: child.pid, method: "taskkill", error: null };
  return { status: "failed", pid: child.pid, method: "taskkill", error: "process did not exit after taskkill" };
}

export function createHqCandidateAttestation({ source, build, candidate, runtime, hostBinding, preGateOwnership }) {
  if (!source || !build || !candidate || !runtime || !hostBinding || !preGateOwnership) throw new Error("HQ candidate attestation requires source, build, candidate, runtime, host binding, and OS ownership evidence");
  if (candidate.sha256 !== runtime.executableSha256 || candidate.path !== runtime.runtimeExecutable) throw new Error("HQ candidate attestation refuses a runtime that differs from staged cutd");
  const hostChecks = Object.values(hostBinding.checks || {});
  if (!hostChecks.length || !hostChecks.every(Boolean)) throw new Error("HQ candidate attestation refuses an unverified HQ host binding");
  if (preGateOwnership.proof?.vacant || preGateOwnership.proof?.process?.sha256 !== candidate.sha256) throw new Error("HQ candidate attestation requires an OS proof of owned staged cutd listener");
  assertDirectPinnedCargoXwinBuild(build);
  return { schema: ATTESTATION_SCHEMA, generatedAt: new Date().toISOString(), source, build, candidate: { cutd: candidate }, runtime: { agent: runtime }, hostBinding, osProof: { preGateOwnership }, signedFinal: false, mode: { updaterArtifactsDisabled: true, authenticodeSignature: "unsigned", installedShellTouched: false } };
}

export function createHqCandidateBuildFailureReceipt({ source, buildSpec, buildLog = null, xwinCacheInventory = null, inventoryError = null, derivedBuildState, error }) {
  const xwinCache = buildSpec?.xwinCache;
  const cargoTarget = buildSpec?.environment?.CARGO_TARGET_DIR;
  const cargoHome = buildSpec?.cargoLayout?.cargoHome;
  if (!source || !xwinCache || xwinCache.schema !== HQ_XWIN_CACHE_SCHEMA || buildSpec?.environment?.XWIN_CACHE_DIR !== xwinCache.path || !cargoTarget || !cargoHome || derivedBuildState?.cargoTarget?.path !== cargoTarget || derivedBuildState?.cargoHome?.path !== cargoHome || !error) {
    throw new Error("HQ candidate build failure receipt requires the exact configured run-owned cargo-xwin cache and failure");
  }
  if (xwinCacheInventory && xwinCacheInventory.schema !== HQ_XWIN_CACHE_INVENTORY_SCHEMA) throw new Error("HQ candidate build failure receipt refuses an unknown cargo-xwin cache inventory schema");
  return {
    schema: BUILD_FAILURE_SCHEMA,
    completedAt: new Date().toISOString(),
    classification: "candidate-chain-build-failure",
    authorization: "none",
    source,
    build: {
      candidateCommand: buildSpec,
      log: buildLog,
      xwinCache: {
        schema: HQ_XWIN_CACHE_SCHEMA,
        path: xwinCache.path,
        configuredChildPath: buildSpec.environment.XWIN_CACHE_DIR,
        inventory: xwinCacheInventory,
        inventoryError: inventoryError ? oneLine(inventoryError) : null,
      },
    },
    cleanup: {
      daemon: { attempted: false, status: "not-started" },
      xwinCache: { attempted: false, status: "retained", reason: "build failure retains the confined cache for explicit inspection; no automatic cache deletion ran" },
      derivedBuildState,
    },
    error: oneLine(error?.message || error),
  };
}

export function assertHqReceiptArtifact(hqReceipt, artifact) {
  let persisted;
  try { persisted = JSON.parse(readFileSync(artifact.path, "utf8")); } catch (error) { throw new Error(`provisional HQ media gate receipt could not be read back: ${error.message}`); }
  if (canonical(persisted) !== canonical(hqReceipt)) throw new Error("provisional HQ media gate receipt changed before final chain binding");
  return artifact;
}

export function createHqCandidateReceipt({ attestation, attestationArtifact, hqGate, hqGateReceipt, osProofArtifacts, lifecycle, lifecycleArtifact, runDir, candidateRoot }) {
  if (attestation?.schema !== ATTESTATION_SCHEMA || !attestationArtifact?.sha256 || !hqGateReceipt?.sha256 || !lifecycleArtifact?.sha256 || !lifecycle || hqGate?.schema !== HQ_MEDIA_GATE_SCHEMA) throw new Error("final HQ candidate receipt requires persisted attestation, versioned provisional HQ receipt, lifecycle, and cleanup evidence");
  if (hqGate?.pass !== false || hqGate?.execution?.state !== "completed-awaiting-owned-cleanup" || hqGate?.execution?.terminal !== false || !Object.values(hqGate?.checks || {}).every(Boolean)) throw new Error("final HQ candidate receipt refuses a non-provisional or failed HQ result");
  if (hqGate.evidence?.renderReceipt?.schema !== HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA) throw new Error("final HQ candidate receipt requires versioned render receipt evidence");
  const renderReceiptArtifact = safeRegularArtifact(assertPathInside(hqGate.evidence.renderReceipt.path, runDir, "HQ render receipt evidence"), "HQ render receipt evidence");
  const outputArtifact = safeRegularArtifact(assertPathInside(hqGate.artifacts?.output?.path, runDir, "HQ render output evidence"), "HQ render output evidence");
  if (renderReceiptArtifact.sha256 !== hqGate.evidence.renderReceipt.sha256 || renderReceiptArtifact.bytes !== hqGate.evidence.renderReceipt.bytes || outputArtifact.sha256 !== hqGate.artifacts.output.sha256 || outputArtifact.bytes !== hqGate.artifacts.output.bytes) throw new Error("final HQ candidate receipt requires unchanged render receipt and output evidence");
  let renderReceipt;
  try { renderReceipt = JSON.parse(readFileSync(renderReceiptArtifact.path, "utf8")); } catch (error) { throw new Error(`HQ render receipt evidence is not valid JSON: ${error.message}`); }
  assertHqRenderVerdicts(hqGate, { renderReceipt, receiptArtifact: renderReceiptArtifact, outputArtifact });
  if (!['stopped', 'already-exited'].includes(lifecycle.cleanup?.status) || lifecycle.postCleanupVacancy?.proof?.vacant !== true) throw new Error("final HQ candidate receipt refuses cleanup that is not independently proven vacant");
  const xwinCache = attestation.build?.xwinCache;
  if (attestation.build?.candidateCommand?.xwinCache?.schema !== HQ_XWIN_CACHE_SCHEMA || attestation.build.candidateCommand.environment?.XWIN_CACHE_DIR !== attestation.build.candidateCommand.xwinCache.path || xwinCache?.schema !== HQ_XWIN_CACHE_SCHEMA || xwinCache.path !== attestation.build.candidateCommand.xwinCache.path || xwinCache.inventory?.schema !== HQ_XWIN_CACHE_INVENTORY_SCHEMA || !xwinCache.inventory.sha256) {
    throw new Error("final HQ candidate receipt requires a hash-bound confined cargo-xwin cache inventory");
  }
  if (canonical(lifecycle.xwinCache?.identity) !== canonical(xwinCache) || lifecycle.xwinCache?.cleanup?.attempted !== true || lifecycle.xwinCache?.cleanup?.status !== "removed") {
    throw new Error("final HQ candidate receipt requires lifecycle truth for the confined cargo-xwin cache");
  }
  for (const [name, path] of [["cargoTarget", attestation.build.candidateCommand.environment?.CARGO_TARGET_DIR], ["cargoHome", attestation.build.candidateCommand.cargoLayout?.cargoHome]]) {
    const state = lifecycle.derivedBuildState?.[name];
    if (!path || state?.path !== path || state.cleanup?.attempted !== true || state.cleanup?.status !== "removed") throw new Error(`final HQ candidate receipt requires lifecycle truth for ${name} cleanup`);
  }
  const requiredProofs = ["preBuildVacancy", "postBuildVacancy", "preGateOwnership", "postGateOwnership", "postCleanupVacancy"];
  if (lifecycle.cleanup?.method === "taskkill") requiredProofs.push("preTaskkillOwnership");
  if (!requiredProofs.every((name) => osProofArtifacts?.[name]?.sha256)) throw new Error("final HQ candidate receipt requires every persisted OS proof");
  if (!requiredProofs.every((name) => lifecycle.osProofs?.[name]?.path === osProofArtifacts[name].path && lifecycle.osProofs[name].sha256 === osProofArtifacts[name].sha256 && lifecycle.osProofs[name].bytes === osProofArtifacts[name].bytes)) throw new Error("final HQ candidate receipt requires lifecycle references to every persisted OS proof");
  return { schema: HQ_CANDIDATE_CHAIN_RECEIPT_SCHEMA, generatedAt: new Date().toISOString(), completion: { state: "cleaned", pass: true, gate: "hq-media-gate" }, classification: "candidate-chain-evidence-only", authorization: "none", releaseClaim: "This unsigned loopback chain records one HQ render after owned-child cleanup. It is evidence only: never an input that authorizes a later render, and never signed-final, installed-app, native-coherence, or UI-matrix evidence.", signedFinal: false, source: attestation.source, build: attestation.build, candidate: attestation.candidate, runtime: attestation.runtime, hostBinding: attestation.hostBinding, mode: attestation.mode, candidateRoot, runDir, evidence: { candidateAttestation: evidenceReference(attestationArtifact, attestation.schema), hqGateReceipt: evidenceReference(hqGateReceipt, hqGate.schema), lifecycle: evidenceReference(lifecycleArtifact, LIFECYCLE_SCHEMA), osProofs: Object.fromEntries(Object.entries(osProofArtifacts).map(([name, artifact]) => [name, evidenceReference(artifact, "shellx-cut/hq-candidate-os-proof@1")])) }, lifecycle };
}
