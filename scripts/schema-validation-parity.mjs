#!/usr/bin/env node
/**
 * Proves that the shared compiled input-schema gate returns the same error
 * contract through REST, the proxy CLI, and MCP tools/call.
 *
 * Direct-dispatch parity is covered in Rust (`dispatch/tests/schema_contract`).
 * This gate uses only invalid, non-mutating calls against a project-free
 * throwaway server.
 */
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { createServer } from "node:net";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import readline from "node:readline";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const APP = join(ROOT, "app");
const CUTD = join(APP, "target", "debug", "cutd");

const CORPUS = [
  { verb: "ui.open", args: {}, path: "/panel", keyword: "required" },
  { verb: "ui.open", args: { panel: "not-a-real-surface" }, path: "/panel", keyword: "enum" },
  { verb: "ui.playhead", args: {}, path: "/at_ms", keyword: "required" },
  { verb: "ui.playhead", args: { at_ms: "100" }, path: "/at_ms", keyword: "type" },
  { verb: "ui.playhead", args: { at_ms: -1 }, path: "/at_ms", keyword: "minimum" },
  {
    verb: "ui.playhead",
    args: { at_ms: Number.MAX_SAFE_INTEGER + 1 },
    path: "/at_ms",
    keyword: "maximum",
  },
  {
    verb: "ui.select",
    args: { clip_ids: ["duplicate", "duplicate"] },
    path: "/clip_ids",
    keyword: "uniqueItems",
  },
  {
    verb: "ui.highlight",
    args: {},
    path: "/",
    keyword: "required",
  },
  {
    verb: "ui.highlight",
    args: { selector: "[data-cut-app-root]", panel: "preview" },
    path: "/",
    keyword: "required",
  },
  {
    verb: "project.create",
    args: { name: "never-created", settings: { width: 1280, height: 720, fps: 30, bogus: true } },
    path: "/settings/bogus",
    keyword: "additionalProperties",
  },
  {
    verb: "edit.speed_ramp",
    args: { clip: "c1", points: [{ at_ms: 0 }] },
    path: "/points/0/factor",
    keyword: "required",
  },
];

function runBuild() {
  const result = spawnSync("cargo", ["build", "-p", "server", "--bin", "cutd"], {
    cwd: APP,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.equal(existsSync(CUTD), true, "debug cutd binary was not built");
}

async function freePort() {
  const server = createServer();
  await new Promise((resolveReady, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolveReady);
  });
  const address = server.address();
  const port = typeof address === "object" && address ? address.port : 0;
  await new Promise((resolveClosed, reject) => {
    server.close((error) => error ? reject(error) : resolveClosed());
  });
  assert.notEqual(port, 0, "could not allocate a throwaway port");
  return port;
}

async function waitForServer(base, child, stderr) {
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`cutd exited before readiness (${child.exitCode}): ${stderr()}`);
    }
    try {
      const response = await fetch(`${base}/api/verbs`, {
        signal: AbortSignal.timeout(1_000),
      });
      if (response.ok) return;
    } catch {}
    await new Promise((resolveWait) => setTimeout(resolveWait, 75));
  }
  throw new Error(`cutd did not become ready: ${stderr()}`);
}

async function stopChild(child) {
  if (!child || child.exitCode !== null) return;
  child.kill("SIGINT");
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
      try {
        message = JSON.parse(line);
      } catch {
        return;
      }
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      pending.resolve(message);
    });
  }

  request(method, params) {
    const id = this.nextId++;
    return new Promise((resolveRequest, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`MCP ${method} timed out: ${this.stderr}`));
      }, 15_000);
      this.pending.set(id, {
        resolve: (message) => {
          clearTimeout(timer);
          resolveRequest(message);
        },
      });
      this.child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  }

  async close() {
    this.lines.close();
    this.child.stdin.end();
    await stopChild(this.child);
  }
}

function checkedError(envelope, fixture, surface) {
  assert.equal(envelope?.ok, false, `${surface} ${fixture.verb}: ${JSON.stringify(envelope)}`);
  const error = envelope?.error;
  assert.equal(error?.code, "invalid_args", `${surface} ${fixture.verb} code`);
  assert.match(error?.message ?? "", new RegExp(fixture.path.replaceAll("/", "\\/")));
  assert.match(error?.message ?? "", new RegExp(fixture.keyword));
  assert.equal(typeof error?.cause, "string", `${surface} ${fixture.verb} cause`);
  assert.match(error?.suggested_action ?? "", /GET \/api\/verbs/);
  return error;
}

async function main() {
  runBuild();
  const port = await freePort();
  const addr = `127.0.0.1:${port}`;
  const base = `http://${addr}`;
  const childEnv = { ...process.env, CUTD_PROXY_ADDR: addr };
  let serverStderr = "";
  const server = spawn(CUTD, ["serve", "--headless", "--addr", addr], {
    cwd: ROOT,
    env: childEnv,
    stdio: ["ignore", "ignore", "pipe"],
  });
  server.stderr.on("data", (chunk) => { serverStderr += chunk; });
  let mcp;
  try {
    await waitForServer(base, server, () => serverStderr);
    const mcpChild = spawn(CUTD, ["mcp"], {
      cwd: ROOT,
      env: childEnv,
      stdio: ["pipe", "pipe", "pipe"],
    });
    mcp = new McpClient(mcpChild);
    const initialized = await mcp.request("initialize", {
      protocolVersion: "2025-06-18",
      capabilities: {},
      clientInfo: { name: "schema-validation-parity", version: "1" },
    });
    assert.equal(initialized.result?.protocolVersion, "2025-06-18");

    const results = [];
    for (const fixture of CORPUS) {
      const response = await fetch(`${base}/api/verb/${fixture.verb}`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          "x-cut-actor": "agent:test:schema-validation-parity",
        },
        body: JSON.stringify(fixture.args),
      });
      assert.equal(response.status, 200, `REST ${fixture.verb} HTTP ${response.status}`);
      const restEnvelope = await response.json();
      const restError = checkedError(restEnvelope, fixture, "REST");

      const cli = spawnSync(
        CUTD,
        ["verb", fixture.verb, JSON.stringify(fixture.args)],
        { cwd: ROOT, env: childEnv, encoding: "utf8" },
      );
      assert.equal(cli.status, 1, `CLI ${fixture.verb}: ${cli.stderr}`);
      const cliEnvelope = JSON.parse(cli.stdout);
      const cliError = checkedError(cliEnvelope, fixture, "CLI");

      const mcpMessage = await mcp.request("tools/call", {
        name: fixture.verb.replaceAll(".", "_"),
        arguments: fixture.args,
      });
      assert.equal(mcpMessage.error, undefined, `MCP RPC ${fixture.verb}`);
      const mcpEnvelope = mcpMessage.result?.structuredContent;
      const mcpError = checkedError(mcpEnvelope, fixture, "MCP");

      assert.deepEqual(cliError, restError, `${fixture.verb}: CLI differs from REST`);
      assert.deepEqual(mcpError, restError, `${fixture.verb}: MCP differs from REST`);
      results.push({ verb: fixture.verb, path: fixture.path, keyword: fixture.keyword });
    }
    process.stdout.write(`${JSON.stringify({
      schema: "shellx-cut/schema-validation-parity@1",
      ok: true,
      corpus: results,
      surfaces: ["direct-dispatch", "REST", "CLI-proxy", "MCP-proxy"],
    }, null, 2)}\n`);
  } finally {
    if (mcp) await mcp.close();
    await stopChild(server);
  }
}

main().catch((error) => {
  process.stderr.write(`schema-validation-parity: ${error.stack ?? error}\n`);
  process.exitCode = 1;
});
