import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { arch, hostname, platform, release } from "node:os";
import { dirname, isAbsolute, relative, resolve } from "node:path";

import { collectSourceIdentity } from "./ignored-test-rig.mjs";
import { assertDirectPinnedCargoXwinBuild } from "./hq-candidate-chain-contracts.mjs";
import { assertNoReparseAncestors, safeRegularArtifact } from "./hq-candidate-chain-security.mjs";
import { HQ_XWIN_CACHE_INVENTORY_SCHEMA, HQ_XWIN_CACHE_SCHEMA, assertGovernedHqDerivedBuildDirectoryRemoved, assertGovernedHqXwinCacheLayout, assertGovernedHqXwinCacheRemoved, assertPersistedHqXwinCacheInventory } from "./hq-candidate-chain-xwin-cache.mjs";
import { sourceContentManifest } from "./source-content-manifest.mjs";
import { HQ_MEDIA_GATE_SCHEMA, HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA, assertHqRenderVerdicts } from "./hq-render-outcome.mjs";

export const HQ_CANDIDATE_CHAIN_RECEIPT_SCHEMA = "shellx-cut/hq-candidate-chain@2";
export const HQ_CANDIDATE_CHAIN_LEGACY_RECEIPT_SCHEMA = "shellx-cut/hq-candidate-chain@1";
export const HQ_CANDIDATE_ATTESTATION_SCHEMA = "shellx-cut/hq-candidate-attestation@1";

export function oneLine(value) {
  return String(value ?? "").replace(/[\r\n\t\0]/g, " ");
}

function commandPath(command) {
  if (isAbsolute(command)) return command;
  const locator = process.platform === "win32" ? "where.exe" : "which";
  const resolved = spawnSync(locator, [command], { encoding: "utf8", windowsHide: true });
  if (resolved.status !== 0) throw new Error(`required command '${command}' is not on PATH`);
  const first = String(resolved.stdout || "").split(/\r?\n/).find(Boolean);
  if (!first) throw new Error(`required command '${command}' did not resolve to an executable path`);
  return first.trim();
}

export function regularArtifact(path, label) {
  return safeRegularArtifact(path, label);
}

export function toolIdentity(command, args = ["-version"]) {
  const executable = commandPath(command);
  const result = spawnSync(executable, args, { encoding: "utf8", windowsHide: true });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed: ${oneLine(result.stderr || result.stdout || result.error?.message)}`);
  }
  const version = String(result.stdout || result.stderr || "").split(/\r?\n/).find(Boolean)?.trim();
  if (!version) throw new Error(`${command} did not report a version`);
  return { command, executable, version, artifact: regularArtifact(executable, `${command} executable`) };
}

export function sourceIdentity(repoRoot) {
  const identity = collectSourceIdentity(repoRoot);
  const tree = spawnSync("git", ["rev-parse", "HEAD^{tree}"], { cwd: repoRoot, encoding: "utf8", windowsHide: true });
  if (tree.status !== 0 || !/^[a-f0-9]{40}$/.test(tree.stdout.trim())) {
    throw new Error(`could not determine source Git tree: ${oneLine(tree.stderr || tree.stdout)}`);
  }
  const manifest = sourceContentManifest(repoRoot);
  return { ...identity, gitTree: tree.stdout.trim(), contentManifest: manifest };
}

function pathsMatch(left, right) {
  const normalizedLeft = resolve(left);
  const normalizedRight = resolve(right);
  return process.platform === "win32"
    ? normalizedLeft.toLowerCase() === normalizedRight.toLowerCase()
    : normalizedLeft === normalizedRight;
}

const CHAIN_PROOF_EXPECTATIONS = {
  preBuildVacancy: { action: "vacant", phase: "pre-build", vacant: true },
  postBuildVacancy: { action: "vacant", phase: "post-build", vacant: true },
  preGateOwnership: { action: "owned", phase: "pre-gate", vacant: false },
  postGateOwnership: { action: "owned", phase: "post-gate", vacant: false },
  postCleanupVacancy: { action: "vacant", phase: "post-cleanup", vacant: true },
};

function pathBelow(path, root) {
  const relation = relative(resolve(root), resolve(path));
  return relation && !relation.startsWith("..") && !isAbsolute(relation);
}

function checkedArtifact(reference, label, { root = null } = {}) {
  if (!reference?.path || !/^[a-f0-9]{64}$/.test(reference.sha256 || "") || !Number.isSafeInteger(reference.bytes) || reference.bytes <= 0) {
    throw new Error(`${label} reference requires a path, SHA-256, and byte count`);
  }
  if (root && !pathBelow(reference.path, root)) throw new Error(`${label} must remain under final chain run directory`);
  const artifact = safeRegularArtifact(resolve(reference.path), label);
  if (artifact.sha256 !== reference.sha256 || artifact.bytes !== reference.bytes) throw new Error(`${label} hash or byte count no longer matches the final chain receipt`);
  return artifact;
}

function referencedJson(reference, label, { root = null } = {}) {
  const artifact = checkedArtifact(reference, label, { root });
  let value;
  try { value = JSON.parse(readFileSync(artifact.path, "utf8")); } catch (error) { throw new Error(`${label} is not valid JSON: ${error.message}`); }
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must contain a JSON object`);
  return { artifact, value };
}

function matchingJson(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function validateProof(item, expected, candidate, label) {
  const proof = item.value?.proof;
  const listener = proof?.listener;
  const script = checkedArtifact(item.value?.script, `${label} script`);
  if (proof?.schema !== "shellx-cut/hq-candidate-os-proof@1" || proof.action !== expected.action || proof.phase !== expected.phase || proof.vacant !== expected.vacant || !script.sha256) throw new Error(`${label} has an unexpected contract`);
  if (expected.vacant) {
    if (listener?.count !== 0 || listener?.literalCount !== 0 || listener?.owners?.length !== 0) throw new Error(`${label} is not a complete vacant-listener proof`);
  } else if (listener?.count !== 1 || listener?.literalCount !== 1 || listener?.owners?.length !== 1 || listener.owners[0] !== proof.pid || proof.process?.sha256 !== candidate.sha256 || !pathsMatch(proof.process?.executable || "", candidate.stagedPath || "")) {
    throw new Error(`${label} does not bind exactly one literal-loopback listener to staged cutd`);
  }
  return proof;
}

// Evidence inspection only. This structural validator deliberately has no
// source/daemon arguments and cannot authorize a subsequent HQ workload.
export function validateHqCandidateChainEvidence(receipt) {
  const evidence = receipt?.evidence;
  const runDir = resolve(receipt?.runDir || "");
  const candidateRoot = receipt?.candidateRoot;
  const legacy = receipt?.schema === HQ_CANDIDATE_CHAIN_LEGACY_RECEIPT_SCHEMA;
  if ((!legacy && receipt?.schema !== HQ_CANDIDATE_CHAIN_RECEIPT_SCHEMA) || receipt?.authorization !== "none" || receipt?.signedFinal !== false || receipt?.completion?.state !== "cleaned" || receipt?.completion?.pass !== true || receipt?.mode?.updaterArtifactsDisabled !== true || receipt?.mode?.authenticodeSignature !== "unsigned" || !candidateRoot || !pathBelow(runDir, candidateRoot) || !evidence) throw new Error("final HQ candidate receipt is evidence-only and requires a checkout-owned candidate run directory");
  assertNoReparseAncestors(runDir, "final HQ candidate evidence");
  const attestation = referencedJson(evidence.candidateAttestation, "persisted HQ candidate attestation", { root: runDir });
  if (evidence.candidateAttestation.schema !== "shellx-cut/hq-candidate-attestation@1" || attestation.value.schema !== evidence.candidateAttestation.schema) {
    throw new Error("persisted HQ candidate attestation schema does not match final chain receipt");
  }
  const hqGate = referencedJson(evidence.hqGateReceipt, "provisional HQ media gate receipt", { root: runDir });
  const expectedGateSchema = legacy ? "shellx-cut/hq-media-gate@1" : HQ_MEDIA_GATE_SCHEMA;
  if (evidence.hqGateReceipt.schema !== expectedGateSchema || hqGate.value.schema !== evidence.hqGateReceipt.schema) {
    throw new Error("provisional HQ media gate receipt schema does not match final chain receipt");
  }
  if (!pathBelow(hqGate.value.receiptDir, runDir) || !pathsMatch(dirname(hqGate.artifact.path), hqGate.value.receiptDir) || hqGate.value.pass !== false || hqGate.value.execution?.state !== "completed-awaiting-owned-cleanup" || hqGate.value.execution?.terminal !== false || hqGate.value.error || !Object.values(hqGate.value.checks || {}).every(Boolean)) {
    throw new Error("provisional HQ media gate receipt is not a completed non-terminal result");
  }
  for (const name of ["ffprobeInput", "ffprobeOutput", "projectStateBeforeRender"]) referencedJson(hqGate.value.evidence?.[name], `provisional HQ ${name}`, { root: runDir });
  checkedArtifact(hqGate.value.artifacts?.input, "HQ input fixture");
  const outputArtifact = checkedArtifact(hqGate.value.artifacts?.output, "HQ output artifact", { root: runDir });
  if (!legacy) {
    if (hqGate.value.evidence?.renderReceipt?.schema !== HQ_RENDER_RECEIPT_EVIDENCE_SCHEMA) throw new Error("provisional HQ render receipt evidence has an unexpected schema");
    const renderReceipt = referencedJson(hqGate.value.evidence.renderReceipt, "provisional HQ render receipt", { root: runDir });
    assertHqRenderVerdicts(hqGate.value, { renderReceipt: renderReceipt.value, receiptArtifact: renderReceipt.artifact, outputArtifact });
  }
  const lifecycle = referencedJson(evidence.lifecycle, "persisted HQ lifecycle", { root: runDir });
  if (lifecycle.value?.schema !== "shellx-cut/hq-candidate-chain-lifecycle@1" || !matchingJson(lifecycle.value, receipt.lifecycle) || !pathsMatch(lifecycle.value.runDir || "", runDir) || !matchingJson(lifecycle.value.hqGate, evidence.hqGateReceipt) || !["stopped", "already-exited"].includes(lifecycle.value.cleanup?.status) || lifecycle.value.postCleanupVacancy?.proof?.vacant !== true) throw new Error("persisted HQ lifecycle does not cross-bind successful cleanup");
  const proofs = {};
  for (const [name, expected] of Object.entries(CHAIN_PROOF_EXPECTATIONS)) {
    const item = referencedJson(evidence.osProofs?.[name], `persisted HQ OS proof ${name}`, { root: runDir });
    validateProof(item, expected, receipt?.candidate?.cutd || {}, `persisted HQ OS proof ${name}`);
    if (!matchingJson(lifecycle.value.osProofs?.[name], evidence.osProofs?.[name])) throw new Error(`persisted HQ lifecycle does not bind OS proof ${name}`);
    proofs[name] = item;
  }
  if (lifecycle.value.cleanup?.method === "taskkill") {
    const item = referencedJson(evidence.osProofs?.preTaskkillOwnership, "persisted HQ OS proof preTaskkillOwnership", { root: runDir });
    validateProof(item, { action: "owned", phase: "pre-taskkill", vacant: false }, receipt.candidate.cutd, "persisted HQ OS proof preTaskkillOwnership");
    if (!matchingJson(lifecycle.value.osProofs?.preTaskkillOwnership, evidence.osProofs?.preTaskkillOwnership)) throw new Error("persisted HQ lifecycle does not bind OS proof preTaskkillOwnership");
    proofs.preTaskkillOwnership = item;
  }
  const preGate = proofs.preGateOwnership.value.proof;
  const postGate = proofs.postGateOwnership.value.proof;
  const candidate = receipt?.candidate?.cutd;
  if (preGate.process?.sha256 !== candidate?.sha256 || postGate.process?.sha256 !== candidate?.sha256 || !pathsMatch(preGate.process?.executable || "", candidate?.stagedPath || "") || !pathsMatch(postGate.process?.executable || "", candidate?.stagedPath || "")) {
    throw new Error("persisted HQ OS ownership proofs do not bind the staged cutd");
  }
  if (!Number.isInteger(lifecycle.value.childPid) || lifecycle.value.childPid !== preGate.pid || lifecycle.value.childPid !== postGate.pid || !matchingJson(lifecycle.value.candidate, { sha256: candidate?.sha256, path: candidate?.stagedPath }) || (proofs.preTaskkillOwnership && lifecycle.value.childPid !== proofs.preTaskkillOwnership.value.proof?.pid)) {
    throw new Error("persisted HQ lifecycle child PID or candidate identity does not bind final candidate ownership proofs");
  }
  if (!matchingJson(attestation.value?.osProof?.preGateOwnership?.proof, preGate)) throw new Error("persisted attestation does not bind the persisted pre-gate OS proof");
  const backing = hqGate.value?.candidateSource?.hqCandidate?.backingAttestation;
  if (!backing || backing.path !== evidence.candidateAttestation.path || backing.sha256 !== evidence.candidateAttestation.sha256 || backing.bytes !== evidence.candidateAttestation.bytes) {
    throw new Error("provisional HQ media gate receipt does not bind the persisted candidate attestation");
  }
  if (!matchingJson(lifecycle.value.postCleanupVacancy?.proof, proofs.postCleanupVacancy.value.proof)) throw new Error("final chain lifecycle does not bind the persisted post-cleanup OS proof");
  if (!matchingJson(attestation.value.source, receipt.source) || !matchingJson(attestation.value.build, receipt.build) || !matchingJson(attestation.value.candidate, receipt.candidate) || !matchingJson(attestation.value.runtime, receipt.runtime) || !matchingJson(attestation.value.hostBinding, receipt.hostBinding) || !matchingJson(attestation.value.mode, receipt.mode) || !Object.values(receipt.hostBinding?.checks || {}).length || !Object.values(receipt.hostBinding.checks).every(Boolean)) throw new Error("final chain evidence does not cross-bind its backing attestation");
  checkedArtifact({ path: receipt.candidate?.cutd?.stagedPath, sha256: receipt.candidate?.cutd?.sha256, bytes: receipt.candidate?.cutd?.bytes }, "staged HQ candidate cutd", { root: runDir });
  checkedArtifact(receipt.build?.log, "HQ candidate build log", { root: runDir });
  checkedArtifact(receipt.build?.candidateCommand?.cargoHomeConfig, "HQ governed Cargo config", { root: runDir });
  checkedArtifact(receipt.build?.candidateCommand?.cargoHomeConfigAfterBuild, "HQ governed Cargo config after build", { root: runDir });
  assertDirectPinnedCargoXwinBuild(receipt.build);
  const xwinLayout = receipt.build?.candidateCommand?.xwinCache;
  const xwinCache = receipt.build?.xwinCache;
  if (xwinLayout?.schema !== HQ_XWIN_CACHE_SCHEMA || receipt.build?.candidateCommand?.environment?.XWIN_CACHE_DIR !== xwinLayout.path || xwinCache?.schema !== HQ_XWIN_CACHE_SCHEMA || !pathsMatch(xwinCache.path || "", xwinLayout.path || "") || xwinCache.inventory?.schema !== HQ_XWIN_CACHE_INVENTORY_SCHEMA || !matchingJson(lifecycle.value?.xwinCache?.identity, xwinCache)) {
    throw new Error("final chain evidence does not bind the confined cargo-xwin cache lifecycle");
  }
  assertGovernedHqXwinCacheLayout(xwinLayout, runDir);
  const xwinInventory = referencedJson(xwinCache.inventory, "persisted HQ cargo-xwin cache inventory", { root: runDir });
  if (xwinInventory.value?.schema !== HQ_XWIN_CACHE_INVENTORY_SCHEMA || xwinCache.inventory.schema !== xwinInventory.value.schema) throw new Error("persisted HQ cargo-xwin cache inventory schema does not match final chain receipt");
  assertPersistedHqXwinCacheInventory(xwinInventory.value, xwinLayout, runDir);
  assertGovernedHqXwinCacheRemoved(lifecycle.value.xwinCache?.cleanup, xwinLayout, runDir);
  for (const [name, path] of [["cargoTarget", receipt.build?.candidateCommand?.environment?.CARGO_TARGET_DIR], ["cargoHome", receipt.build?.candidateCommand?.cargoLayout?.cargoHome]]) {
    const state = lifecycle.value?.derivedBuildState?.[name];
    if (!pathsMatch(state?.path || "", path || "")) throw new Error(`final chain evidence does not bind ${name} cleanup to the candidate build`);
    assertGovernedHqDerivedBuildDirectoryRemoved(name, state.cleanup, path, runDir);
  }
  for (const [name, tool] of Object.entries(receipt.build?.toolchain || {})) checkedArtifact(tool.artifact, `HQ pinned ${name} tool`);
  if (receipt.hostBinding?.path) checkedArtifact(receipt.hostBinding, "staged HQ host binding", { root: runDir });
  return { attestation, hqGate, lifecycle, proofs };
}

export function daemonIdentity(agent, explicitPath) {
  if (agent?.schema !== "shellx-cut/agent-docs/2" || agent?.product !== "ShellX Cut" || !agent?.version) {
    throw new Error("/api/agent did not return a ShellX Cut daemon identity");
  }
  const reportedPath = String(agent?.runtime?.executable || "");
  if (!reportedPath) throw new Error("/api/agent did not report the exact running cutd executable");
  const artifact = regularArtifact(reportedPath, "running cutd daemon executable");
  const explicitPathMatchesReported = !explicitPath || pathsMatch(explicitPath, reportedPath);
  return {
    api: { schema: agent.schema, product: agent.product, version: agent.version, addr: agent?.runtime?.addr || null },
    reportedPath,
    selectedPath: reportedPath,
    explicitPath: explicitPath || null,
    explicitPathMatchesReported,
    artifact,
  };
}


export function candidateAttestationMatchChecks(attestation, source, daemon) {
  return {
    schemaMatches: attestation?.schema === HQ_CANDIDATE_ATTESTATION_SCHEMA,
    unsignedCandidate: attestation?.signedFinal === false,
    headMatchesSource: attestation?.source?.gitCommit === source?.gitCommit,
    gitTreeMatchesSource: attestation?.source?.gitTree === source?.gitTree,
    contentManifestMatchesSource: attestation?.source?.contentManifest?.sha256 === source?.contentManifest?.sha256,
    versionMatchesSource: attestation?.source?.version === source?.version,
    stagedCutdMatchesRunningDaemon: attestation?.candidate?.cutd?.sha256 === daemon?.artifact?.sha256,
    explicitDaemonMatchesRunningDaemon: daemon?.explicitPathMatchesReported === true,
    runtimeExecutableMatchesRunningDaemon: pathsMatch(attestation?.runtime?.agent?.runtimeExecutable || "", daemon?.reportedPath || ""),
    runtimeHashMatchesRunningDaemon: attestation?.runtime?.agent?.executableSha256 === daemon?.artifact?.sha256,
    updaterArtifactsDisabled: attestation?.mode?.updaterArtifactsDisabled === true,
    authenticodeUnsigned: attestation?.mode?.authenticodeSignature === "unsigned",
    recordedHostBindingPassed: Object.values(attestation?.hostBinding?.checks || {}).length > 0
      && Object.values(attestation.hostBinding.checks).every(Boolean),
    osOwnedListenerBeforeGate: attestation?.osProof?.preGateOwnership?.proof?.vacant !== true
      && attestation?.osProof?.preGateOwnership?.proof?.process?.sha256 === daemon?.artifact?.sha256,
  };
}

export function installedHqAttestationMatchChecks(attestation, source, daemon) {
  return {
    schemaMatches: attestation?.schema === "shellx-cut/windows-installed-hq-attestation@1",
    installedCandidate: attestation?.mode?.installedCandidate === true,
    headMatchesSource: attestation?.source?.gitCommit === source?.gitCommit,
    gitTreeMatchesSource: attestation?.source?.gitTree === source?.gitTree,
    contentManifestMatchesSource: attestation?.source?.contentManifest?.sha256 === source?.contentManifest?.sha256,
    versionMatchesSource: attestation?.source?.version === source?.version,
    installedCutdMatchesRunningDaemon: attestation?.candidate?.cutd?.sha256 === daemon?.artifact?.sha256,
    installedCutdPathMatchesRunningDaemon: pathsMatch(attestation?.candidate?.cutd?.path || "", daemon?.reportedPath || ""),
    explicitDaemonMatchesRunningDaemon: daemon?.explicitPathMatchesReported === true,
    sourceReceiptBound: /^[a-f0-9]{64}$/.test(attestation?.evidence?.sourceReceipt?.sha256 || ""),
    fullCoverageReceiptBound: /^[a-f0-9]{64}$/.test(attestation?.evidence?.fullCoverageReceipt?.sha256 || ""),
    installedShellBound: /^[a-f0-9]{64}$/.test(attestation?.installed?.shell?.sha256 || ""),
  };
}

export function assertInstalledHqAttestationMatches(attestation, source, daemon) {
  const checks = installedHqAttestationMatchChecks(attestation, source, daemon);
  const failed = Object.entries(checks).filter(([, passed]) => !passed).map(([name]) => name);
  if (failed.length) throw new Error(`installed HQ attestation does not bind the running installed candidate: ${failed.join(", ")}`);
  return checks;
}

export function assertCandidateAttestationMatches(attestation, source, daemon) {
  const checks = candidateAttestationMatchChecks(attestation, source, daemon);
  const failed = Object.entries(checks).filter(([, passed]) => !passed).map(([name]) => name);
  if (failed.length) throw new Error(`candidate attestation does not bind this exact source and running cutd: ${failed.join(", ")}`);
  return checks;
}

export function readHqHostBinding(path, actualHostname = hostname()) {
  const evidence = {
    path: null,
    sha256: null,
    schema: null,
    actualHostname,
  };
  if (!path) return { evidence, binding: null, error: "--host-binding is required; no generic Windows host may run the HQ media gate" };

  let bindingArtifact;
  try {
    bindingArtifact = regularArtifact(resolve(path), "HQ host binding");
  } catch (error) {
    return { evidence, binding: null, error: error.message };
  }
  evidence.path = bindingArtifact.path;
  evidence.sha256 = bindingArtifact.sha256;

  let binding;
  try {
    binding = JSON.parse(readFileSync(bindingArtifact.path, "utf8"));
  } catch (error) {
    return { evidence, binding: null, error: `HQ host binding is not valid JSON: ${error.message}` };
  }
  if (!binding || typeof binding !== "object" || Array.isArray(binding)) {
    return { evidence, binding: null, error: "HQ host binding must contain a JSON object" };
  }
  evidence.schema = typeof binding.schema === "string" ? binding.schema : null;
  return { evidence, binding, error: null };
}

export function hostIdentity(ffprobeTool) {
  const node = process.execPath;
  return {
    platform: platform(),
    release: release(),
    arch: arch(),
    hostname: hostname(),
    node: { version: process.version, executable: node, artifact: regularArtifact(node, "Node executable") },
    ffprobe: ffprobeTool,
  };
}
