#!/usr/bin/env node
import { launchInstalledCutWithCdp, normalizeCdpPort } from "../lib/windows-cdp-launch.mjs";
import { verifyAgentDocsApi } from "../lib/agent-docs.mjs";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const EXPECTED_VERSION = JSON.parse(
  readFileSync(join(REPO_ROOT, "app", "desktop", "src-tauri", "tauri.conf.json"), "utf8"),
).version;

function arg(name, fallback = "") {
  const index = process.argv.indexOf(name);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

function hasFlag(name) {
  return process.argv.includes(name);
}

function usage() {
  console.log(`Usage: node scripts/windows/launch-installed-cdp.mjs [--install-dir <dir>] [--cdp-port 9223] [--engine http://127.0.0.1:6161] [--keep-existing] [--with-generate-fixtures] [--no-launch]

Launches the installed Windows ShellX Cut app with WebView2 CDP enabled.
Use this before scripts/windows/cdp-*.mjs verifiers instead of passing
--remote-debugging-port directly to shellx-cut.exe.`);
}

function generateFixtureEnv() {
  return {
    CUTD_GENERATE_PROMPT_ADAPTER: join(REPO_ROOT, "ui", "tests", "fixtures", "generate-prompt-adapter.py"),
    CUTD_GENERATE_STORYBOARD_ADAPTER: join(REPO_ROOT, "ui", "tests", "fixtures", "generate-storyboard-adapter.py"),
  };
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function getJson(url) {
  const response = await fetch(url, {
    headers: { connection: "close" },
    signal: AbortSignal.timeout(3000),
  });
  if (!response.ok) throw new Error(`${url} returned ${response.status}`);
  return response.json();
}

async function waitForCdp(cdpBase, timeoutMs) {
  const started = Date.now();
  let last = "";
  while (Date.now() - started < timeoutMs) {
    try {
      const targets = await getJson(`${cdpBase}/json/list`);
      const page = targets.find((target) => target.type === "page" && /127\.0\.0\.1:\d+/.test(target.url || ""));
      if (page) return page;
      last = `no ShellX Cut page in ${targets.length} target(s)`;
    } catch (error) {
      last = error?.message || String(error);
    }
    await sleep(250);
  }
  throw new Error(`Timed out waiting for CDP at ${cdpBase}: ${last}`);
}

async function waitForEngine(engineBase, timeoutMs) {
  const started = Date.now();
  let last = "";
  while (Date.now() - started < timeoutMs) {
    try {
      const registry = await getJson(`${engineBase}/api/verbs`);
      const verbs = Array.isArray(registry) ? registry : registry.verbs || [];
      if (verbs.some((verb) => verb?.name === "project.state" || verb === "project.state")) {
        return verbs.length;
      }
      last = `verb registry missing project.state (${verbs.length} verbs)`;
    } catch (error) {
      last = error?.message || String(error);
    }
    await sleep(250);
  }
  throw new Error(`Timed out waiting for cutd at ${engineBase}: ${last}`);
}

async function main() {
  if (hasFlag("--help") || hasFlag("-h")) return usage();

  const cdpPort = normalizeCdpPort(arg("--cdp-port", "9223"));
  const cdpBase = `http://127.0.0.1:${cdpPort}`;
  const engineBase = arg("--engine", "http://127.0.0.1:6161").replace(/\/$/, "");
  const timeoutMs = Number(arg("--wait-ms", "20000"));
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) throw new Error(`Invalid --wait-ms: ${timeoutMs}`);

  if (!hasFlag("--no-launch")) {
    const launched = launchInstalledCutWithCdp({
      installDir: arg("--install-dir", ""),
      cdpPort,
      stopExisting: !hasFlag("--keep-existing"),
      env: hasFlag("--with-generate-fixtures") ? generateFixtureEnv() : {},
    });
    if (launched.stdout.trim()) process.stdout.write(launched.stdout);
    if (launched.stderr.trim()) process.stderr.write(launched.stderr);
    if (launched.status !== 0) {
      throw new Error(`PowerShell launch failed with status ${launched.status}`);
    }
  }

  const page = await waitForCdp(cdpBase, timeoutMs);
  const verbs = await waitForEngine(engineBase, timeoutMs);
  const agentDocs = await verifyAgentDocsApi({
    engineBase,
    sourceRoot: REPO_ROOT,
    expectedVersion: EXPECTED_VERSION,
    timeoutMs,
  });
  if (!agentDocs.ok) {
    throw new Error(`Installed agent-doc verification failed: ${agentDocs.failures.join("; ")}`);
  }
  console.log(`CDP_READY ${cdpBase} page=${page.url}`);
  console.log(`CUTD_READY ${engineBase} verbs=${verbs}`);
  console.log(`AGENT_DOCS_READY ${engineBase} files=${agentDocs.served} version=${agentDocs.version}`);
}

main().catch((error) => {
  console.error(error?.stack || error?.message || String(error));
  process.exit(1);
});
