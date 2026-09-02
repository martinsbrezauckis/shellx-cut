#!/usr/bin/env node
import { launchInstalledCutWithCdp, normalizeCdpPort } from "../lib/windows-cdp-launch.mjs";
import { verifyAgentDocsApi } from "../lib/agent-docs.mjs";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { waitForConnectedUi } from "../lib/installed-runtime-evidence.mjs";
import {
  beginInstalledStartupReadiness,
} from "../lib/installed-startup-readiness.mjs";
import {
  getJson,
  waitForInstalledCdpPage,
  waitForInstalledDomRoot,
  waitForTestOwnedSlowFfmpegMarker,
} from "../lib/installed-cdp-readiness.mjs";

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const EXPECTED_VERSION = JSON.parse(
  readFileSync(join(REPO_ROOT, "app", "desktop", "src-tauri", "tauri.conf.json"), "utf8"),
).version;

function arg(name, fallback = "") {
  const index = process.argv.indexOf(name);
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback;
}

function optionalPositiveInteger(name) {
  const index = process.argv.indexOf(name);
  if (index < 0) return undefined;
  const value = process.argv[index + 1];
  if (!/^[1-9]\d*$/.test(value || "")) throw new Error(`${name} must be a positive integer`);
  return Number(value);
}

function hasFlag(name) {
  return process.argv.includes(name);
}

function usage() {
  console.log(`Usage: node scripts/windows/launch-installed-cdp.mjs [--install-dir <dir>] [--cdp-port 9223] [--engine http://127.0.0.1:6161] [--keep-existing] [--with-generate-fixtures] [--no-launch]
  [--startup-out <private-receipt.json> --startup-t0-epoch-ms <epoch-ms>
   --startup-listener-budget-ms <ms> --startup-api-budget-ms <ms>
   --startup-ui-client-budget-ms <ms> --startup-dom-budget-ms <ms>
   --startup-slow-ffmpeg-marker <test-owned-marker.json>]

Launches the installed Windows ShellX Cut app with WebView2 CDP enabled.
Use this before scripts/windows/cdp-*.mjs verifiers instead of passing
--remote-debugging-port directly to shellx-cut.exe. Startup timing is
measurement-only unless all four caller-supplied budget fields are present.`);
}

function generateFixtureEnv() {
  return {
    CUTD_GENERATE_PROMPT_ADAPTER: join(REPO_ROOT, "ui", "tests", "fixtures", "generate-prompt-adapter.py"),
    CUTD_GENERATE_STORYBOARD_ADAPTER: join(REPO_ROOT, "ui", "tests", "fixtures", "generate-storyboard-adapter.py"),
  };
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
    await new Promise((resolveSleep) => setTimeout(resolveSleep, 250));
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
  const startupOut = arg("--startup-out", "");
  const startupT0 = optionalPositiveInteger("--startup-t0-epoch-ms");
  const slowFfmpegMarker = arg("--startup-slow-ffmpeg-marker", "");
  const startupBudgets = {
    listener: optionalPositiveInteger("--startup-listener-budget-ms"),
    api: optionalPositiveInteger("--startup-api-budget-ms"),
    uiClient: optionalPositiveInteger("--startup-ui-client-budget-ms"),
    domRoot: optionalPositiveInteger("--startup-dom-budget-ms"),
  };
  if (hasFlag("--no-launch") && startupOut && startupT0 === undefined) {
    throw new Error("--no-launch --startup-out requires --startup-t0-epoch-ms recorded before the external shell spawn");
  }
  const startup = startupOut ? beginInstalledStartupReadiness({
    surface: "windows-installed",
    t0EpochMs: startupT0 || Date.now(),
    budgets: startupBudgets,
  }) : null;

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

  startup?.mark("shellSpawned", {
    evidence: { launcher: hasFlag("--no-launch") ? "external-prelaunched" : "launch-installed-cdp" },
  });

  const cdp = await waitForInstalledCdpPage({ cdpBase, timeoutMs });
  const page = cdp.page;
  startup?.mark("listener", {
    evidence: { endpoint: `${cdpBase}/json/list`, pageUrl: page.url, waitMs: cdp.attemptsAtMs },
  });
  const verbs = await waitForEngine(engineBase, timeoutMs);
  startup?.mark("api", { evidence: { endpoint: `${engineBase}/api/verbs`, verbs } });
  const connectedUi = startup ? await waitForConnectedUi(engineBase, { timeoutMs }) : null;
  startup?.mark("uiClient", {
    evidence: { clients: connectedUi?.state.result.ui_clients, waitMs: connectedUi?.elapsedMs },
  });
  const domRoot = startup ? await waitForInstalledDomRoot({ page, timeoutMs }) : null;
  startup?.mark("domRoot", { evidence: domRoot || {} });
  if (startup && slowFfmpegMarker) {
    const marker = await waitForTestOwnedSlowFfmpegMarker({
      markerPath: slowFfmpegMarker,
      timeoutMs,
    });
    startup.mark("slowFfmpeg", marker);
  }
  const agentDocs = await verifyAgentDocsApi({
    engineBase,
    sourceRoot: REPO_ROOT,
    expectedVersion: EXPECTED_VERSION,
    timeoutMs,
  });
  if (!agentDocs.ok) {
    throw new Error(`Installed agent-doc verification failed: ${agentDocs.failures.join("; ")}`);
  }
  if (startup) {
    const receipt = startup.build();
    writeFileSync(startupOut, `${JSON.stringify(receipt, null, 2)}\n`, { encoding: "utf8", flag: "wx" });
    console.log(`STARTUP_READINESS ${startupOut} budget=${receipt.budget.status}`);
  }
  console.log(`CDP_READY ${cdpBase} page=${page.url}`);
  console.log(`CUTD_READY ${engineBase} verbs=${verbs}`);
  console.log(`AGENT_DOCS_READY ${engineBase} files=${agentDocs.served} version=${agentDocs.version}`);
}

main().catch((error) => {
  console.error(error?.stack || error?.message || String(error));
  process.exit(1);
});
