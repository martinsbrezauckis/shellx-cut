import { randomUUID } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { isAbsolute, relative, resolve } from "node:path";

import { safeRegularArtifact } from "./hq-candidate-chain-security.mjs";

export const WINDOWS_INSTALLED_HQ_SCHEMA = "shellx-cut/windows-installed-hq@1";
export const WINDOWS_INSTALLED_HQ_ATTESTATION_SCHEMA = "shellx-cut/windows-installed-hq-attestation@1";
export const WINDOWS_INSTALLED_HQ_LIFECYCLE_SCHEMA = "shellx-cut/windows-installed-hq-lifecycle@1";
export const WINDOWS_INSTALLED_HQ_DEFAULT_ADDR = "127.0.0.1:6219";
export const WINDOWS_INSTALLED_HQ_DEFAULT_TIMEOUT_MS = 7_200_000;

const SHA256 = /^[a-f0-9]{64}$/;
const COMMIT = /^[a-f0-9]{40}$/;

function requiredValue(argv, index, name) {
  const value = argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function positiveInt(value, name) {
  const number = Number(value);
  if (!Number.isInteger(number) || number <= 0) throw new Error(`${name} must be a positive integer`);
  return number;
}

export function parseWindowsInstalledHqArgs(argv = process.argv.slice(2)) {
  const options = { sourceReceipt: "", fullCoverageReceipt: "", fixture4k: "", fixture8k: "", hostBinding: "", out: "", addr: WINDOWS_INSTALLED_HQ_DEFAULT_ADDR, timeoutMs: WINDOWS_INSTALLED_HQ_DEFAULT_TIMEOUT_MS, help: false };
  const fields = { "--source-receipt": "sourceReceipt", "--full-coverage-receipt": "fullCoverageReceipt", "--fixture-4k": "fixture4k", "--fixture-8k": "fixture8k", "--host-binding": "hostBinding", "--out": "out", "--addr": "addr" };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (fields[arg]) { options[fields[arg]] = requiredValue(argv, index, arg); index += 1; }
    else if (arg === "--timeout-ms") { options.timeoutMs = positiveInt(requiredValue(argv, index, arg), arg); index += 1; }
    else if (arg === "--help" || arg === "-h") options.help = true;
    else throw new Error(`unknown option ${arg}`);
  }
  if (!options.help) {
    for (const [name, field] of [["--source-receipt", "sourceReceipt"], ["--full-coverage-receipt", "fullCoverageReceipt"], ["--fixture-4k", "fixture4k"], ["--fixture-8k", "fixture8k"], ["--host-binding", "hostBinding"]]) {
      if (!options[field]) throw new Error(`${name} is required`);
    }
  }
  return options;
}

function readReceipt(path, label) {
  const artifact = safeRegularArtifact(resolve(path), label);
  let value;
  try { value = JSON.parse(readFileSync(artifact.path, "utf8")); } catch (error) { throw new Error(`${label} is not valid JSON: ${error.message}`); }
  return { artifact, value };
}

function same(value, expected, label) {
  if (value !== expected) throw new Error(`${label} differs from the installed candidate`);
}

export function loadWindowsInstalledHqPrerequisites({ sourceReceipt, fullCoverageReceipt, liveSource, installedArtifact }) {
  const source = readReceipt(sourceReceipt, "Windows installed source receipt");
  const coverage = readReceipt(fullCoverageReceipt, "Windows installed full-coverage receipt");
  const value = source.value;
  if (value?.schema !== "shellx-cut/windows-installed-source@1" || !COMMIT.test(value.head || "") || !COMMIT.test(value.gitTree || "") || !SHA256.test(value.contentManifest?.sha256 || "") || !/^\d+\.\d+\.\d+/.test(value.version || "") || !Array.isArray(value.status) || value.status.length !== 0) {
    throw new Error("Windows installed HQ requires a clean, identity-complete installed source receipt");
  }
  const coverageRows = coverage.value?.results;
  const coverageSummary = coverage.value?.summary?.controls;
  if (coverage.value?.schema !== "shellx-cut/full-coverage-results@1" || coverage.value.ok !== true || coverage.value.full !== true || coverage.value.strictAllActions !== true || coverage.value.surface !== "windows-installed" || coverage.value.runtime?.installedApp !== true || coverage.value.runtime?.driver !== "webview2-cdp" || coverage.value.runtime?.nativeAttached !== true || !Array.isArray(coverageRows) || coverageRows.length === 0 || coverageRows.some((row) => row?.ok !== true) || !Number.isSafeInteger(coverageSummary?.total) || coverageSummary.total !== coverageRows.length || coverageSummary.failures !== 0 || coverageSummary.couldNotVerify !== 0 || coverageSummary.strictUnverified !== 0) {
    throw new Error("Windows installed HQ requires a passing strict installed WebView2 full-coverage receipt");
  }
  same(liveSource.gitCommit, value.head, "live source commit");
  same(liveSource.gitTree, value.gitTree, "live source tree");
  same(liveSource.contentManifest?.sha256, value.contentManifest.sha256, "live source content digest");
  same(liveSource.version, value.version, "live source version");
  if (liveSource.gitDirty) throw new Error("Windows installed HQ refuses a dirty source worktree");
  same(coverage.value.source?.gitCommit, value.head, "full-coverage source commit");
  same(coverage.value.source?.contentManifestSha256, value.contentManifest.sha256, "full-coverage source content digest");
  same(coverage.value.runtime?.sourceGitCommit, value.head, "full-coverage runtime commit");
  same(coverage.value.runtime?.sourceContentManifestSha256, value.contentManifest.sha256, "full-coverage runtime content digest");
  same(coverage.value.runtime?.installedArtifactSha256, value.installedArtifact?.shell?.sha256, "full-coverage installed shell digest");
  for (const name of ["shell", "cutd"]) {
    const recorded = value.installedArtifact?.[name];
    const current = installedArtifact?.[name];
    if (!SHA256.test(recorded?.sha256 || "") || !recorded?.path || !current?.path) throw new Error(`installed ${name} identity is incomplete`);
    same(current.path.toLowerCase(), recorded.path.toLowerCase(), `installed ${name} path`);
    same(current.sha256, recorded.sha256, `installed ${name} digest`);
    same(current.productVersion, recorded.productVersion, `installed ${name} version`);
    same(current.signatureStatus, recorded.signatureStatus, `installed ${name} signature status`);
    if (value.signedFinal === true && current.signatureStatus !== "Valid") throw new Error(`signed-final installed ${name} does not have valid Authenticode`);
  }
  if (value.signedFinal === true && (!value.artifactReceipt || value.artifactReceipt.installer?.signatureStatus !== "Valid" || value.artifactReceipt.packaged?.shell?.sha256 !== installedArtifact.shell.sha256 || value.artifactReceipt.packaged?.cutd?.sha256 !== installedArtifact.cutd.sha256)) {
    throw new Error("signed-final installed HQ requires packaged-to-installed artifact coherence");
  }
  return { source, coverage };
}

export function windowsInstalledHqRunDir({ repoRoot, out = "", runId = randomUUID() }) {
  const root = resolve(repoRoot, ".scratch", "windows-installed-hq");
  const path = out ? resolve(repoRoot, out) : resolve(root, runId);
  const relation = relative(root, path);
  if (!relation || relation.startsWith("..") || isAbsolute(relation)) throw new Error("Windows installed HQ output must be a new directory below this checkout's .scratch/windows-installed-hq root");
  if (existsSync(path)) throw new Error(`Windows installed HQ output already exists: ${path}`);
  return path;
}

export function windowsInstalledHqUsage() {
  return [
    "Usage: node scripts/windows-installed-hq.mjs --source-receipt <installed-run/source-receipt.json> --full-coverage-receipt <installed-run/full-coverage-receipt.json> --fixture-4k <real-4k60-video> --fixture-8k <real-8k60-video> --host-binding <private-binding.json> [options]",
    "",
    "Release order is enforced: this runner accepts only an already built and installed candidate with a strict passing installed WebView2 matrix, re-verifies the installed shell/cutd version, path, hash, and signature state, then runs one 4K and one 8K HQ render through that installed cutd.",
    "It never builds, stages, replaces, signs, or installs a binary.",
    "",
    "Options:",
    "  --out PATH            new checkout-owned evidence directory",
    "  --addr HOST:PORT      owned literal-loopback daemon (default 127.0.0.1:6219)",
    "  --timeout-ms MS       per-job deadline (default 7200000)",
  ].join("\n");
}
