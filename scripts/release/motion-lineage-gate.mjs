#!/usr/bin/env node
/**
 * Cross-repo producer/consumer gate for current Motion SDK -> Cut lineage.
 *
 * The gate launches no browser. It uses a deterministic frame seam, a real
 * probeable media fixture, an isolated Cut server, Debug API, and MCP proxy.
 */
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import readline from "node:readline";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const CUTD = join(ROOT, "app", "target", "debug", "cutd");
const APP = join(ROOT, "app");
const PRODUCER = join(ROOT, "scripts", "release", "motion-lineage-producer.mts");
const SAMPLE_MEDIA = join(ROOT, "app", "server", "assets", "first-edit-sample.mp4");

function option(name) {
  const index = process.argv.indexOf(name);
  const value = index >= 0 ? process.argv[index + 1] : undefined;
  if (!value || value.startsWith("--")) throw new Error(`Missing ${name}.`);
  return value;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: "utf8", ...options });
  if (result.status !== 0) {
    throw new Error(`${command} failed (${result.status}):\n${result.stderr || result.stdout}`);
  }
  return result.stdout.trim();
}

function envelopeResult(envelope, label) {
  assert.equal(envelope?.ok, true, `${label}: ${JSON.stringify(envelope?.error ?? envelope)}`);
  return envelope.result;
}

function verifiedProof(value, label) {
  assert.equal(value?.ok, true, `${label} result: ${JSON.stringify(value)}`);
  const proof = value?.lineageProofs?.[0];
  assert.equal(proof?.schema, "shellx-cut/motion-import-attestation@1", `${label} proof schema`);
  assert.equal(proof?.status, "verified", `${label} proof status`);
  assert.equal(proof?.packageLineage?.schema, "shellx-motion/package-render-lineage@1", `${label} package lineage`);
  assert.equal(proof?.connectorReceipt, null, `${label} current SDK connector receipt`);
  assert.equal(proof?.renderReceipt?.status, "passed", `${label} render receipt`);
  assert.equal(proof?.cutPlanReceipt?.status, "passed", `${label} Cut-plan receipt`);
  return proof;
}

function originAttestations(value, found = []) {
  if (!value || typeof value !== "object") return found;
  if (value.originAttestation?.schema === "shellx-cut/motion-import-attestation@1") {
    found.push(value.originAttestation);
  }
  for (const child of Array.isArray(value) ? value : Object.values(value)) originAttestations(child, found);
  return found;
}

async function freePort() {
  const server = createServer();
  await new Promise((resolveReady, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolveReady);
  });
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : 0;
  await new Promise((resolveClosed, reject) => server.close((error) => error ? reject(error) : resolveClosed()));
  if (!port) throw new Error("Could not allocate an isolated Cut port.");
  return port;
}

async function waitForServer(url, child) {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error(`cutd exited before readiness (${child.exitCode}).`);
    try {
      const response = await fetch(`${url}/api/verb/system.doctor`, {
        method: "POST",
        headers: { "content-type": "application/json", "x-cut-actor": "agent:test:motion-lineage-gate" },
        body: "{}",
      });
      if (response.ok) return;
    } catch {}
    await new Promise((resolveWait) => setTimeout(resolveWait, 75));
  }
  throw new Error("cutd did not become ready within 15 seconds.");
}

async function stopChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode !== null) return;
  child.kill(signal);
  const exited = await Promise.race([
    new Promise((resolveExit) => child.once("exit", () => resolveExit(true))),
    new Promise((resolveWait) => setTimeout(() => resolveWait(false), 2_000)),
  ]);
  if (!exited && child.exitCode === null) {
    child.kill("SIGKILL");
    await new Promise((resolveExit) => child.once("exit", resolveExit));
  }
}

class McpClient {
  constructor(child) {
    this.child = child;
    this.nextId = 1;
    this.pending = new Map();
    this.stderr = "";
    child.stderr.on("data", (chunk) => { this.stderr += chunk; });
    this.lines = readline.createInterface({ input: child.stdout });
    this.lines.on("line", (line) => {
      let message;
      try { message = JSON.parse(line); } catch { return; }
      const pending = this.pending.get(message.id);
      if (pending) {
        this.pending.delete(message.id);
        pending.resolve(message);
      }
    });
  }

  request(method, params) {
    const id = this.nextId++;
    return new Promise((resolveRequest, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`MCP ${method} timed out: ${this.stderr}`));
      }, 15_000);
      this.pending.set(id, { resolve: (message) => { clearTimeout(timer); resolveRequest(message); } });
      this.child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  }

  async close() {
    this.lines.close();
    this.child.stdin.end();
    await stopChild(this.child);
  }
}

const motionRoot = resolve(option("--motion-root"));
const expectedMotionCommit = option("--motion-commit");
assert.match(expectedMotionCommit, /^[a-f0-9]{40}$/, "--motion-commit must be a full commit hash");
assert.equal(run("git", ["rev-parse", "HEAD"], { cwd: motionRoot }), expectedMotionCommit, "Motion source commit drifted");
assert.equal(run("git", ["status", "--porcelain"], { cwd: motionRoot }), "", "Motion source worktree must be clean");
assert.equal(run("git", ["rev-parse", "--show-toplevel"], { cwd: ROOT }), ROOT, "run from the Cut worktree");
run("cargo", ["build", "-p", "server", "--bin", "cutd"], { cwd: APP });
run(CUTD, ["--version"], { cwd: ROOT });

const scratch = mkdtempSync(join(tmpdir(), "shellx-cut-motion-lineage-gate-"));
let cutd;
let mcp;
try {
  const artifactRoot = join(scratch, "motion-artifact");
  const producerStdout = run("pnpm", ["exec", "tsx", PRODUCER, "--motion-root", motionRoot, "--artifact-root", artifactRoot, "--sample-media", SAMPLE_MEDIA], { cwd: motionRoot });
  const producer = JSON.parse(producerStdout.split("\n").at(-1));
  assert.equal(producer.schema, "shellx-motion/cut-lineage-gate-producer@1");

  const port = await freePort();
  const addr = `127.0.0.1:${port}`;
  const url = `http://${addr}`;
  const isolatedHome = join(scratch, "home");
  const childEnv = { ...process.env, HOME: isolatedHome, XDG_DATA_HOME: join(isolatedHome, ".local", "share"), CUTD_PROXY_ADDR: addr };
  cutd = spawn(CUTD, ["serve", "--addr", addr, "--headless"], { cwd: ROOT, env: childEnv, stdio: ["ignore", "ignore", "pipe"] });
  let serverStderr = "";
  cutd.stderr.on("data", (chunk) => { serverStderr += chunk; });
  await waitForServer(url, cutd).catch((error) => { throw new Error(`${error.message}\n${serverStderr}`); });

  const verb = async (name, args) => {
    const response = await fetch(`${url}/api/verb/${name}`, {
      method: "POST",
      headers: { "content-type": "application/json", "x-cut-actor": "agent:test:motion-lineage-gate" },
      body: JSON.stringify(args),
    });
    assert.equal(response.ok, true, `${name} HTTP ${response.status}`);
    return response.json();
  };

  const mapped = envelopeResult(await verb("motion.map_import", { path: producer.planPath }), "Debug API map");
  const mapProof = verifiedProof(mapped, "Debug API map");
  assert.deepEqual(mapProof.packageLineage, producer.packageLineage, "producer and consumer lineage differ");
  assert.equal(mapProof.artifactHandleId, producer.artifactHandleId, "producer and consumer handle identity differ");

  envelopeResult(await verb("project.create", { name: "motion-lineage-gate", dir: join(scratch, "motion-lineage-gate.cutproj") }), "project.create");
  const mcpChild = spawn(CUTD, ["mcp"], { cwd: ROOT, env: childEnv, stdio: ["pipe", "pipe", "pipe"] });
  mcp = new McpClient(mcpChild);
  const initialized = await mcp.request("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "motion-lineage-gate", version: "1" } });
  assert.equal(initialized.result?.protocolVersion, "2025-06-18", "MCP initialization failed");
  const appliedMessage = await mcp.request("tools/call", { name: "motion_apply_import", arguments: { path: producer.planPath, dryRun: false } });
  assert.equal(appliedMessage.error, undefined, `MCP apply RPC error: ${JSON.stringify(appliedMessage.error)}`);
  const applied = envelopeResult(appliedMessage.result?.structuredContent, "MCP apply");
  const applyProof = verifiedProof(applied, "MCP apply");
  assert.deepEqual(applyProof, mapProof, "Debug API map and MCP apply returned different proof");

  const state = envelopeResult(await verb("project.state", {}), "project.state");
  const origins = originAttestations(state);
  assert.ok(origins.length > 0, "project state did not persist originAttestation");
  assert.ok(origins.some((proof) => JSON.stringify(proof) === JSON.stringify(mapProof)), "persisted originAttestation differs from verified proof");
  const proofText = JSON.stringify(mapProof);
  for (const localPath of [scratch, ROOT, motionRoot]) {
    assert.equal(proofText.includes(localPath), false, "lineage proof leaked a local path");
  }

  process.stdout.write(`${JSON.stringify({
    ok: true,
    schema: "shellx-cut/motion-lineage-cross-repo-gate@1",
    motionCommit: expectedMotionCommit,
    producerSchema: producer.schema,
    debugApiMap: "verified",
    mcpApply: "verified",
    persistedOriginAttestation: true,
    artifactHandleId: mapProof.artifactHandleId,
    operationHash: mapProof.artifactOperationHash,
  })}\n`);
} finally {
  if (mcp) await mcp.close();
  await stopChild(cutd, "SIGINT");
  rmSync(scratch, { recursive: true, force: true });
}
