#!/usr/bin/env node
// mcp-probe.mjs — minimal MCP stdio probe (public verb contract coverage: MCP exposes 100%
// of schema/verbs.json, dots→underscores per the REST-to-MCP tool-name mapping contract).
//
// Role: spawn `cutd mcp`, speak the two-line JSON-RPC handshake the server
// implements (app/server/src/mcp.rs: newline-delimited JSON-RPC 2.0 —
// initialize, then tools/list), print {"tools":[names…]} on stdout, exit 0.
// Any failure → diagnostic on stderr, exit 1. cutd's own logs go to ITS
// stderr (passed through) so a hung probe is debuggable.
//
// Usage: node scripts/mcp-probe.mjs
// Callers: scripts/coverage-audit.sh (asserts every verb appears as a tool).
import { spawn } from "node:child_process";
import { existsSync, statSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { safeJsonParse } from "./lib/safe-data.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CUTD_REL = "./app/target/debug/cutd";
const CUTD_PATH = resolve(root, CUTD_REL);

function fail(message) {
  process.stderr.write(`mcp-probe: ${message}\n`);
  process.exit(1);
}

if (process.argv.length > 2) {
  fail("custom cutd paths are not accepted; run cargo build -p server and use the repo debug binary");
}
if (!existsSync(CUTD_PATH) || !statSync(CUTD_PATH).isFile()) {
  fail("repo debug cutd binary does not exist; build first with cargo build -p server");
}
const child = spawn("./app/target/debug/cutd", ["mcp"], { cwd: root, stdio: ["pipe", "pipe", "inherit"] });
child.on("error", (e) => { fail(`cannot spawn repo debug cutd: ${e.message}`); });

// 15s hard timeout: a silent hang is itself a coverage failure.
const timer = setTimeout(() => { process.stderr.write("mcp-probe: timeout — no tools/list reply in 15s (is `cutd mcp` reading stdin?)\n"); child.kill(); process.exit(1); }, 15000);

// The 2-line probe: initialize (id 1) → tools/list (id 2). The server replies
// per line; we only need the id-2 result.
child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "mcp-probe", version: "1" } } }) + "\n");
child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} }) + "\n");

let buf = "";
child.stdout.on("data", (chunk) => {
  buf += chunk;
  for (const line of buf.split("\n").slice(0, -1)) {       // complete lines only
    let msg; try { msg = safeJsonParse(line); } catch { continue; }
    if (msg.id === 2) {
      clearTimeout(timer); child.kill();
      if (!msg.result?.tools) { fail(`tools/list errored: ${JSON.stringify(msg.error ?? msg)}`); }
      process.stdout.write(`${JSON.stringify({ tools: msg.result.tools.map((t) => t.name) })}\n`);
      process.exit(0);
    }
  }
  buf = buf.slice(buf.lastIndexOf("\n") + 1);
});
child.on("exit", (code) => { clearTimeout(timer); fail(`cutd mcp exited (code ${code}) before answering tools/list`); });
