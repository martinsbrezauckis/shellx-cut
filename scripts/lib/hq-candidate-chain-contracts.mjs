import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { isAbsolute, relative, resolve } from "node:path";

import { DEFAULT_HQ_REPO_ROOT } from "./hq-media-contracts.mjs";

export const HQ_CANDIDATE_CHAIN_DEFAULT_ADDR = "127.0.0.1:6219";
export const HQ_CANDIDATE_CHAIN_DEFAULT_TIMEOUT_MS = 7_200_000;

function requireValue(argv, index, name) {
  const value = argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function positiveInt(value, name) {
  const number = Number(value);
  if (!Number.isInteger(number) || number <= 0) throw new Error(`${name} must be a positive integer`);
  return number;
}

function pathInside(path, parent) {
  const relation = relative(resolve(parent), resolve(path));
  return relation !== "" && !relation.startsWith("..") && !isAbsolute(relation);
}

function samePath(left, right) {
  const normalizedLeft = resolve(left || "");
  const normalizedRight = resolve(right || "");
  return process.platform === "win32" ? normalizedLeft.toLowerCase() === normalizedRight.toLowerCase() : normalizedLeft === normalizedRight;
}

export function assertDirectPinnedCargoXwinBuild(build) {
  const pinned = build?.toolchain?.cargoXwin;
  const command = build?.candidateCommand;
  if (!pinned?.executable || !pinned?.artifact?.path || !samePath(pinned.executable, pinned.artifact.path) || !samePath(build?.command, pinned.executable) || !samePath(command?.command, pinned.executable) || !Array.isArray(build?.args) || !Array.isArray(command?.args) || build.args[0] !== "build" || command.args[0] !== "build" || build.args.includes("xwin") || command.args.includes("xwin") || JSON.stringify(build.args) !== JSON.stringify(command.args)) {
    throw new Error("HQ candidate build must execute the sealed pinned cargo-xwin executable directly");
  }
  return pinned;
}

export function parseHqCandidateChainArgs(argv = process.argv.slice(2)) {
  const options = { profile: "", fixture: "", hostBinding: "", addr: HQ_CANDIDATE_CHAIN_DEFAULT_ADDR, timeoutMs: HQ_CANDIDATE_CHAIN_DEFAULT_TIMEOUT_MS, help: false };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--profile") { options.profile = requireValue(argv, index, arg); index += 1; }
    else if (arg === "--fixture") { options.fixture = requireValue(argv, index, arg); index += 1; }
    else if (arg === "--host-binding") { options.hostBinding = requireValue(argv, index, arg); index += 1; }
    else if (arg === "--addr") { options.addr = requireValue(argv, index, arg); index += 1; }
    else if (arg === "--timeout-ms") { options.timeoutMs = positiveInt(requireValue(argv, index, arg), arg); index += 1; }
    else if (arg === "--help" || arg === "-h") options.help = true;
    else throw new Error(`unknown option ${arg}`);
  }
  return options;
}

export function resolveHqCandidateChainDir({ repoRoot = DEFAULT_HQ_REPO_ROOT, profile, runId = randomUUID() } = {}) {
  return resolve(repoRoot, ".scratch", "hq-candidate-chain", String(profile || "hq-media").replace(/[^a-z0-9-]+/gi, "-"), runId);
}

export function assertHqCandidateChainDir(path, repoRoot) {
  const root = resolve(repoRoot, ".scratch", "hq-candidate-chain");
  if (!pathInside(path, root)) throw new Error("HQ candidate-chain evidence must be a new directory inside this checkout's .scratch/hq-candidate-chain root");
  if (existsSync(path)) throw new Error(`HQ candidate-chain evidence directory already exists: ${path}`);
  return resolve(path);
}

export function assertHqCandidateListenAddr(addr) {
  const match = /^(127\.0\.0\.1|\[::1\]):([1-9]\d{0,4})$/.exec(String(addr || ""));
  if (!match || Number(match[2]) > 65_535) throw new Error("HQ candidate chain --addr must be a literal loopback HOST:PORT for the daemon it owns");
  return addr;
}

export function hqCandidateChainUsage() {
  return [
    "Usage: node scripts/hq-candidate-chain.mjs --profile hq-4k-uhd-60|hq-8k-uhd-60 --fixture <real-media-path> --host-binding <private-binding.json> [options]",
    "",
    "Internal component diagnostic only: builds an unsigned cutd.exe into this checkout's .scratch, launches one loopback daemon, and records one HQ render.",
    "This is not the release path. Release qualification must first build and install the complete Windows package with scripts/windows-installed-full-coverage.mjs, verify its installed shell/cutd identities and installed WebView2 matrix, then run scripts/windows-installed-hq.mjs against those receipts. This component chain is never signed-final, installed-app, native-coherence, or UI-matrix evidence.",
    "",
    "Options:",
    "  --addr HOST:PORT       literal loopback cutd endpoint (default 127.0.0.1:6219)",
    "  --timeout-ms MS        HQ media request/job deadline (default 7200000)",
  ].join("\n");
}
