import { spawnSync } from "node:child_process";
import { resolve } from "node:path";

import { oneLine } from "./hq-media-identity.mjs";
import { safeRegularArtifact } from "./hq-candidate-chain-security.mjs";

const OS_PROOF_SCHEMA = "shellx-cut/hq-candidate-os-proof@1";

function expectedListener(addr) {
  const match = /^(127\.0\.0\.1|\[::1\]):([1-9]\d{0,4})$/.exec(String(addr || ""));
  if (!match) throw new Error("HQ OS proof requires a literal loopback address");
  return { address: match[1] === "[::1]" ? "::1" : match[1], port: Number(match[2]) };
}

function sameWindowsPath(left, right) {
  return resolve(left).toLowerCase() === resolve(right).toLowerCase();
}

export function assertWindowsOsProof(proof, { action, phase, addr, pid, stagedCutd, sha256 } = {}) {
  const listener = expectedListener(addr);
  if (proof?.schema !== OS_PROOF_SCHEMA || proof?.action !== action || proof?.phase !== phase) throw new Error("HQ OS listener proof has an unexpected schema, action, or phase");
  if (proof?.listener?.address !== listener.address || proof?.listener?.port !== listener.port) throw new Error("HQ OS listener proof did not inspect the literal loopback listener");
  if (action === "vacant") {
    if (proof?.vacant !== true || proof?.listener?.count !== 0 || proof?.listener?.literalCount !== 0 || proof?.listener?.owners?.length !== 0) throw new Error("HQ OS vacancy proof found a listener or is incomplete");
    return proof;
  }
  if (!Number.isInteger(pid) || proof?.pid !== pid || proof?.listener?.count !== 1 || proof?.listener?.literalCount !== 1 || proof?.listener?.owners?.length !== 1 || proof.listener.owners[0] !== pid) {
    throw new Error("HQ OS listener proof does not show the spawned PID owning the literal loopback listener");
  }
  if (!sameWindowsPath(proof?.process?.executable || "", stagedCutd || "") || !sameWindowsPath(proof?.expectedExecutable || "", stagedCutd || "")) {
    throw new Error("HQ OS listener proof executable differs from staged cutd");
  }
  if (proof?.process?.sha256 !== sha256 || proof?.expectedSha256 !== sha256) throw new Error("HQ OS listener proof executable hash differs from staged cutd");
  return proof;
}

export function probeHqCandidateOs({ repoRoot, action, phase, addr, pid = 0, stagedCutd = "", sha256 = "", spawnProcess = spawnSync } = {}) {
  const script = safeRegularArtifact(resolve(repoRoot, "scripts", "windows", "hq-candidate-os-proof.ps1"), "HQ candidate OS proof script");
  const result = spawnProcess("powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", script.path, "-Action", action, "-Phase", phase, "-Addr", addr, "-CandidatePid", String(pid), "-ExpectedExecutable", stagedCutd, "-ExpectedSha256", sha256], { encoding: "utf8", windowsHide: true });
  if (result.status !== 0 || result.error) throw new Error(`HQ OS ${phase} ${action} probe failed closed: ${oneLine(result.stderr || result.stdout || result.error?.message)}`);
  let proof;
  try { proof = JSON.parse(String(result.stdout || "").trim()); } catch (error) { throw new Error(`HQ OS ${phase} ${action} probe returned invalid JSON: ${error.message}`); }
  return { script, proof: assertWindowsOsProof(proof, { action, phase, addr, pid, stagedCutd, sha256 }) };
}

export { OS_PROOF_SCHEMA };
