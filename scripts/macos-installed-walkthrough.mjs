#!/usr/bin/env node
// macOS installed-app walkthrough for ShellX Cut. Tauri's official tauri-driver
// WebDriver path has NO WKWebView backend on macOS, so we cannot drive the
// WebView DOM there. Instead we validate the SAME agent-first surface the
// WebDriver smoke checks on Windows/Linux, at the engine layer: launch the
// packaged .app, wait for the cutd ENGINE it spawns on 127.0.0.1:6161, drive the
// verb API directly over loopback (the dispatch surface the whole product rests
// on), and capture a window screenshot for visual evidence.
//
// MUST run on macOS (uses `open`, `osascript`, `screencapture`). The DOM-surface
// coverage (every data-cut-panel mounts) is the Windows/Linux WebDriver smoke's
// job — this is the macOS engine + visual gate.

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { artifactInfo } from "./lib/ignored-test-rig.mjs";
import { buildNativeIntegrityEvidence, macIntegrityCommands } from "./lib/native-artifact-integrity.mjs";
import { buildInstalledWalkthroughReceipt, collectInstalledRuntimeEvidence } from "./lib/installed-walkthrough-receipt.mjs";
import { sourceContentManifest } from "./lib/source-content-manifest.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(__dirname, "..");

function readJsonFile(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (err) {
    throw new Error(`Failed to parse ${path}: ${err?.message || String(err)}`);
  }
}

// Derive the expected verb count from the schema (single source of truth) so this
// installed-walkthrough can't drift behind the registry (regression: fixed-count drift).
const EXPECTED_VERBS = readJsonFile(join(REPO_ROOT, "schema/verbs.json")).verbs.length;
const EXPECTED_VERSION = readJsonFile(join(REPO_ROOT, "app/desktop/src-tauri/tauri.conf.json")).version;
const ENGINE = "http://127.0.0.1:6161";
const DEFAULT_APP = join(
  REPO_ROOT,
  "app",
  "desktop",
  "src-tauri",
  "target",
  "release",
  "bundle",
  "macos",
  "ShellX Cut.app",
);

let pass = 0;
let fail = 0;

function arg(name, fallback = "") {
  const i = process.argv.indexOf(name);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

function check(name, cond, detail = "") {
  if (cond) {
    pass += 1;
    console.log(`    PASS  ${name}${detail ? "  " + detail : ""}`);
  } else {
    fail += 1;
    console.log(`    FAIL  ${name}${detail ? "  " + detail : ""}`);
  }
  return !!cond;
}

function launchArgsForApp(appPath) {
  if (appPath.endsWith(".app") || appPath.includes("/")) {
    return ["-n", appPath];
  }
  return ["-n", "-a", appPath];
}

function windowBounds(windowInfo) {
  const bounds = windowInfo?.kCGWindowBounds || windowInfo?.bounds || {};
  return {
    width: Number(bounds.Width ?? bounds.width ?? 0),
    height: Number(bounds.Height ?? bounds.height ?? 0),
    x: Number(bounds.X ?? bounds.x ?? 0),
    y: Number(bounds.Y ?? bounds.y ?? 0),
  };
}

function isShellxWindow(windowInfo) {
  const owner = String(windowInfo?.kCGWindowOwnerName ?? windowInfo?.ownerName ?? "").toLowerCase();
  const isShellx = owner.includes("shellx cut") || owner.includes("shellx_cut") || owner.includes("shellx-cut");
  const onscreen = Number(windowInfo?.kCGWindowIsOnscreen ?? windowInfo?.isOnscreen ?? 0) === 1;
  const bounds = windowBounds(windowInfo);
  return isShellx && onscreen && bounds.width >= 400 && bounds.height >= 300;
}

export function summarizeWindowEvidence({ screenshotOk, screenshotError = "", windows = [] } = {}) {
  const shellxWindow = windows.find(isShellxWindow);
  if (screenshotOk && shellxWindow) {
    return { ok: true, mode: "screenshot", detail: "" };
  }

  if (shellxWindow) {
    const bounds = windowBounds(shellxWindow);
    const prefix = screenshotError ? `screenshot unavailable (${screenshotError}); ` : "";
    return {
      ok: true,
      mode: "coregraphics",
      detail: `${prefix}onscreen ShellX Cut window ${bounds.width}x${bounds.height} at ${bounds.x},${bounds.y}`,
    };
  }

  return {
    ok: false,
    mode: "none",
    detail: screenshotError
      ? `screenshot unavailable (${screenshotError}); no onscreen ShellX Cut window metadata`
      : "no screenshot or onscreen ShellX Cut window metadata",
  };
}

function collectShellxWindows() {
  const swift = `
import Foundation
import CoreGraphics

let options = CGWindowListOption(arrayLiteral: .optionOnScreenOnly, .excludeDesktopElements)
let raw = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] ?? []
var result: [[String: Any]] = []

for item in raw {
  let owner = item[kCGWindowOwnerName as String] as? String ?? ""
  if owner.lowercased().contains("shellx") {
    let bounds = item[kCGWindowBounds as String] as? [String: Any] ?? [:]
    result.append([
      "kCGWindowOwnerName": owner,
      "kCGWindowOwnerPID": item[kCGWindowOwnerPID as String] as? Int ?? 0,
      "kCGWindowNumber": item[kCGWindowNumber as String] as? Int ?? 0,
      "kCGWindowIsOnscreen": item[kCGWindowIsOnscreen as String] as? Int ?? 0,
      "kCGWindowLayer": item[kCGWindowLayer as String] as? Int ?? 0,
      "kCGWindowBounds": [
        "Width": bounds["Width"] as? Double ?? 0,
        "Height": bounds["Height"] as? Double ?? 0,
        "X": bounds["X"] as? Double ?? 0,
        "Y": bounds["Y"] as? Double ?? 0
      ]
    ])
  }
}

let data = try JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
print(String(data: data, encoding: .utf8)!)
`;
  const run = spawnSync("swift", ["-e", swift], { encoding: "utf8" });
  if (run.status !== 0) {
    return {
      windows: [],
      error: run.stderr?.trim() || run.stdout?.trim() || `swift exited ${run.status}`,
    };
  }
  try {
    return { windows: JSON.parse(run.stdout || "[]"), error: "" };
  } catch (err) {
    return {
      windows: [],
      error: `could not parse CoreGraphics window metadata: ${err?.message || String(err)}`,
      raw: run.stdout,
    };
  }
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

async function waitForEngine(timeoutMs = 45000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const r = await fetch(`${ENGINE}/api/verbs`, { signal: AbortSignal.timeout(3000), headers: { connection: "close" } });
      if (r.ok) return await r.json();
    } catch {
      /* not up yet */
    }
    await sleep(500);
  }
  return null;
}

async function waitForEngineClosed(timeoutMs = 15000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      await fetch(`${ENGINE}/api/verbs`, { signal: AbortSignal.timeout(1000), headers: { connection: "close" } });
    } catch {
      return true;
    }
    await sleep(250);
  }
  return false;
}

async function captureWindowEvidence(outDir, name) {
  const shotPath = join(outDir, `${name}.png`);
  const windowMetadata = collectShellxWindows();
  const shellxWindow = windowMetadata.windows.find(isShellxWindow);
  const windowId = Number(shellxWindow?.kCGWindowNumber || 0);
  const shot = windowId > 0
    ? spawnSync("screencapture", ["-x", "-o", "-l", String(windowId), shotPath], { encoding: "utf8" })
    : { status: -1, stdout: "", stderr: "no onscreen ShellX Cut window id" };
  const screenshotOk = shot.status === 0 && existsSync(shotPath);
  const screenshotError = shot.stderr?.trim() || shot.stdout?.trim()
    || (screenshotOk ? "" : `screencapture exited ${shot.status}`);
  const windowEvidence = summarizeWindowEvidence({
    screenshotOk,
    screenshotError,
    windows: windowMetadata.windows,
  });
  await writeFile(join(outDir, `${name}-metadata.json`), JSON.stringify({
    screenshot: {
      ok: screenshotOk,
      path: shotPath,
      status: shot.status,
      sha256: screenshotOk ? artifactInfo(shotPath).sha256 : null,
    },
    coregraphics: windowMetadata,
    result: windowEvidence,
  }, null, 2));
  return {
    ok: windowEvidence.ok,
    mode: windowEvidence.mode,
    screenshotSha256: screenshotOk ? artifactInfo(shotPath).sha256 : null,
  };
}

async function main() {
  if (process.argv.includes("--help") || process.argv.includes("-h")) {
    console.log("Usage: node scripts/macos-installed-walkthrough.mjs --out <private-evidence-dir> --source-commit <sha> --source-content-manifest <sha256> [--app <ShellX Cut.app>]");
    return;
  }
  if (process.platform !== "darwin") {
    throw new Error("This walkthrough is macOS-only. On Windows/Linux run: node scripts/tauri-webdriver-smoke.mjs");
  }

  const appPath = resolve(arg("--app", DEFAULT_APP));
  const outArg = arg("--out");
  if (!outArg) throw new Error("--out is required so release evidence stays in its governed private run directory");
  if (process.argv.includes("--keep-open")) throw new Error("--keep-open cannot produce the required post-use code-seal proof");
  const outDir = resolve(outArg);
  await mkdir(outDir, { recursive: true });
  if (!existsSync(appPath)) {
    throw new Error(`App bundle not found: ${appPath}\n(build it: cd app/desktop && cargo tauri build, or pass --app)`);
  }
  const source = {
    gitCommit: arg("--source-commit"),
    version: EXPECTED_VERSION,
    contentManifestSha256: arg("--source-content-manifest"),
  };
  if (!/^[a-f0-9]{40}$/.test(source.gitCommit)) throw new Error("--source-commit must be a full Git SHA");
  const content = sourceContentManifest(REPO_ROOT);
  if (content.sha256 !== source.contentManifestSha256) throw new Error("synchronized source content digest mismatch");
  const beforeArtifact = artifactInfo(appPath, { tree: true });
  if (!beforeArtifact.sha256) throw new Error("could not digest the installed macOS app bundle");
  const preCommands = macIntegrityCommands(appPath, "pre");
  for (const command of preCommands) check(command.id, command.status === 0);
  if (preCommands.some((command) => command.status !== 0)) throw new Error("pre-use macOS signing/notarization proof failed");
  if (await waitForEngine(1500)) throw new Error("cutd was already reachable before the installed app launch");
  console.log(`App: ${appPath}`);
  console.log(`Evidence dir: ${outDir}`);

  const launch = spawnSync("open", launchArgsForApp(appPath), { encoding: "utf8" });
  check("packaged .app launched", launch.status === 0, launch.stderr?.trim() || "");
  if (launch.status !== 0) throw new Error("packaged .app launch failed");
  const reg = await waitForEngine();
  const verbCount = reg ? (Array.isArray(reg) ? reg.length : (Array.isArray(reg.verbs) ? reg.verbs.length : 0)) : 0;
  check("cutd engine answers on 127.0.0.1:6161", !!reg);
  check(`verb registry exposes ${EXPECTED_VERBS} verbs`, verbCount === EXPECTED_VERBS, `count=${verbCount}`);
  if (!reg) throw new Error("installed cutd did not start");
  const runtimeEvidence = await collectInstalledRuntimeEvidence({
    engineBase: ENGINE,
    repoRoot: REPO_ROOT,
    surface: "macos-installed",
    source: { ...source, platform: "darwin", arch: process.arch },
    onSurfaceOpened: (panel) => captureWindowEvidence(outDir, panel),
  });
  check("installed docs, Settings, Library, About, Debug API and MCP", runtimeEvidence.status === "pass");
  const quit = spawnSync("osascript", ["-e", 'quit app "ShellX Cut"'], { encoding: "utf8" });
  if (quit.status !== 0 || !(await waitForEngineClosed())) throw new Error("installed macOS app did not quit cleanly");

  const afterArtifact = artifactInfo(appPath, { tree: true });
  const postCommands = macIntegrityCommands(appPath, "post");
  for (const command of postCommands) check(command.id, command.status === 0);
  const integrity = await buildNativeIntegrityEvidence({
    source,
    surface: "macos-installed",
    artifactSha256: beforeArtifact.sha256,
    preUseSha256: beforeArtifact.sha256,
    postUseSha256: afterArtifact.sha256,
    commands: [...preCommands, ...postCommands],
    signed: true,
    notarized: true,
  });
  const receipt = buildInstalledWalkthroughReceipt({
    source,
    surface: "macos-installed",
    artifact: { sha256: beforeArtifact.sha256 },
    runtimeEvidence,
    integrityEvidence: integrity,
  });
  await writeFile(join(outDir, "installed-runtime-receipt.json"), `${JSON.stringify(runtimeEvidence, null, 2)}\n`, { flag: "wx" });
  await writeFile(join(outDir, "installed-artifact-integrity.json"), `${JSON.stringify(integrity, null, 2)}\n`, { flag: "wx" });
  await writeFile(join(outDir, "installed-walkthrough-receipt.json"), `${JSON.stringify(receipt, null, 2)}\n`, { flag: "wx" });

  console.log(`\nmacos-installed-walkthrough: ${pass} passed, ${fail} failed`);
  console.log(`Evidence: ${outDir}`);
  process.exitCode = fail === 0 ? 0 : 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((err) => {
    console.error(err?.stack || err?.message || String(err));
    process.exit(1);
  });
}
