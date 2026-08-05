#!/usr/bin/env node
// Windows/Linux installed-app smoke through Tauri's official tauri-driver
// WebDriver bridge. macOS is covered by scripts/macos-installed-walkthrough.mjs
// because Tauri's official
// tauri-driver path has no WKWebView backend on macOS.
//
// What it proves: the NATIVE WebView (wry/WebKitGTK on Linux, WebView2 on Windows)
// reaches the ShellX Cut UI, the embedded cutd ENGINE answers on 127.0.0.1:6161,
// the full verb API is reachable FROM the WebView (agent-first surface), and every
// UI surface (data-cut-panel) mounts + is drivable. This is the cross-platform
// installed-desktop gate.
//
// PRECONDITION: a cutd must be answering on 127.0.0.1:6161 (the desktop app reuses
// it; else it spawns the bundled engine — but for a Linux dev binary the simplest
// path is to `cutd serve` first so the UI is served). The script reports clearly
// if the engine link never comes up.
//
// Prereqs:  cargo install tauri-driver --locked
//   Linux:  WebKitWebDriver on PATH (apt: webkit2gtk-driver) + a display (WSLg/X)
//   Windows: matching msedgedriver.exe on PATH, or pass --native-driver <path>

import { spawn, spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { base64ToBuffer } from "./lib/safe-data.mjs";
import { verifyAgentDocsApi } from "./lib/agent-docs.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(__dirname, "..");

function readJsonFile(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (err) {
    throw new Error(`Failed to parse ${path}: ${err?.message || String(err)}`);
  }
}

// Derive the expected verb count from the schema so this smoke cannot drift
// behind the live registry when verbs are added or removed.
const EXPECTED_VERBS = readJsonFile(join(REPO_ROOT, "schema/verbs.json")).verbs.length;
const EXPECTED_VERSION = readJsonFile(join(REPO_ROOT, "app/desktop/src-tauri/tauri.conf.json")).version;

// Every UI surface that carries a data-cut-panel marker. Some are always mounted
// (topbar/timeline/preview/statusbar); the rest are reachable by activating a
// left tab or a drawer — the smoke activates each and confirms it mounts.
const ALWAYS_PANELS = ["topbar", "timeline", "preview", "statusbar"];
const LEFT_TAB_PANELS = [
  { tab: "transcript", panel: "transcript" },
  { tab: "assets", panel: "assets" },
  { tab: "projects", panel: "projects" },
  { tab: "library", panel: "library" },
];

let pass = 0;
let fail = 0;

function usage() {
  console.log(`Usage: node scripts/tauri-webdriver-smoke.mjs [--app <built-app-executable>] [--driver <tauri-driver>] [--port <port>] [--native-driver <path>] [--out <dir>] [--keep-driver]

Runs only on Windows/Linux. macOS → scripts/macos-installed-walkthrough.mjs.
A cutd must answer on 127.0.0.1:6161 (start 'cutd serve' first, or let the app spawn its bundled engine).`);
}

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

function defaultAppPath() {
  const base = join(REPO_ROOT, "app", "desktop", "src-tauri", "target", "release");
  return process.platform === "win32" ? join(base, "shellx-cut.exe") : join(base, "shellx-cut");
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

async function wd(base, method, path, body) {
  // tauri-driver's hyper server does not like Node/undici's pooled keep-alive
  // connections (they surface as "connection closed before message completed"
  // and stall the poll). Force a fresh connection per request + a hard per-request
  // timeout so a stuck socket can't eat the whole wait budget.
  const headers = { connection: "close" };
  if (body !== undefined) headers["content-type"] = "application/json";
  const res = await fetch(new URL(path, base), {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    keepalive: false,
    signal: AbortSignal.timeout(20000),
  });
  const text = await res.text();
  let json = {};
  if (text.trim()) {
    try {
      json = JSON.parse(text);
    } catch {
      json = { value: text };
    }
  }
  if (!res.ok) {
    const msg = typeof json?.value === "object" ? json.value.message : text;
    throw new Error(`WebDriver ${method} ${path} failed (${res.status}): ${msg || res.statusText}`);
  }
  return json;
}

async function waitForDriver(base, failed, timeoutMs = 30000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (failed()) throw failed();
    try {
      await wd(base, "GET", "status");
      return;
    } catch {
      await sleep(250);
    }
  }
  throw new Error(`Timed out waiting for tauri-driver at ${base}`);
}

async function exec(session, script, args = []) {
  const res = await wd(session.base, "POST", `session/${session.id}/execute/sync`, { script, args });
  return res.value;
}

async function execAsync(session, body, args = []) {
  const script = `
    const done = arguments[arguments.length - 1];
    Promise.resolve((async () => {
      ${body}
    })()).then(
      (value) => done(value),
      (err) => done({ __smokeError: String(err && err.stack || err) })
    );
  `;
  const res = await wd(session.base, "POST", `session/${session.id}/execute/async`, { script, args });
  if (res.value?.__smokeError) throw new Error(res.value.__smokeError);
  return res.value;
}

async function waitFor(session, body, timeoutMs = 30000) {
  const start = Date.now();
  let last = null;
  while (Date.now() - start < timeoutMs) {
    try {
      last = await execAsync(session, body);
      if (last) return last;
    } catch (err) {
      last = err.message;
    }
    await sleep(300);
  }
  throw new Error(`Timed out waiting for condition: ${String(last)}`);
}

async function screenshot(session, path) {
  const res = await wd(session.base, "GET", `session/${session.id}/screenshot`);
  const bytes = base64ToBuffer(res.value || "");
  await writeFile(path, bytes);
  return bytes.length;
}

async function newSession(base, appPath) {
  const res = await wd(base, "POST", "session", {
    capabilities: { alwaysMatch: { browserName: "wry", "tauri:options": { application: appPath } } },
  });
  const value = res.value || res;
  const id = value.sessionId || res.sessionId;
  if (!id) throw new Error(`tauri-driver did not return a session id: ${JSON.stringify(res)}`);
  return { base, id };
}

function toWindowsPath(path) {
  const r = spawnSync("wslpath", ["-w", path], { encoding: "utf8" });
  if (r.status !== 0) throw new Error(`wslpath failed for ${path}: ${(r.stderr || r.stdout).trim()}`);
  return r.stdout.trim();
}

async function main() {
  if (process.argv.includes("--help") || process.argv.includes("-h")) return usage();
  if (process.platform === "darwin") {
    throw new Error(
      "Tauri's tauri-driver WebDriver path supports Windows/Linux only (no WKWebView backend). " +
        "On macOS run: node scripts/macos-installed-walkthrough.mjs",
    );
  }

  const appPath = resolve(arg("--app", defaultAppPath()));
  const driverPath = arg("--driver", "tauri-driver");
  const port = Number(arg("--port", "4444"));
  const nativeDriver = arg("--native-driver", "");
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const outDir = resolve(arg("--out", join(homedir(), ".shellx-scratch", "shellx-cut", `tauri-webdriver-${stamp}`)));
  await mkdir(outDir, { recursive: true });

  if (!Number.isFinite(port) || port <= 0) throw new Error(`Invalid --port: ${port}`);
  if (!existsSync(appPath)) throw new Error(`Built app executable not found: ${appPath}\n(build it: cd app/desktop/src-tauri && cargo build --release)`);

  // On WSL driving a Windows .exe driver, paths must be Windows-shaped.
  const winInterop = process.platform !== "win32" && /\.exe$/i.test(driverPath);
  const driverAppPath = winInterop ? toWindowsPath(appPath) : appPath;
  const driverNativePath = nativeDriver && winInterop ? toWindowsPath(resolve(nativeDriver)) : nativeDriver;
  // tauri-driver 2.x defaults --native-port to 4445; pin it to port+1000 so the
  // underlying WebDriver never collides with the intermediary --port (the
  // "FATAL: Unable to listen … port" + EADDRNOTAVAIL failure mode).
  const nativePort = port + 1000;
  const driverArgs = ["--port", String(port), "--native-port", String(nativePort)];
  if (driverNativePath) driverArgs.push("--native-driver", driverNativePath);

  console.log(`WebDriver app: ${appPath}`);
  if (driverAppPath !== appPath) console.log(`WebDriver app (driver path): ${driverAppPath}`);
  console.log(`Evidence dir: ${outDir}`);
  console.log(`tauri-driver: ${driverPath} ${driverArgs.join(" ")}`);

  let driverFailure = null;
  let driverStopping = false;
  const driver = spawn(driverPath, driverArgs, { cwd: REPO_ROOT, stdio: ["ignore", "pipe", "pipe"] });
  let driverLog = "";
  driver.stdout.on("data", (c) => {
    driverLog += c.toString();
    if (process.env.VERBOSE) process.stdout.write(c);
  });
  driver.stderr.on("data", (c) => {
    driverLog += c.toString();
    if (process.env.VERBOSE) process.stderr.write(c);
  });
  driver.on("error", (err) => {
    driverFailure = new Error(`Could not start tauri-driver (${driverPath}): ${err.message}`);
  });
  driver.on("exit", (code, signal) => {
    if (!driverStopping && code !== 0) driverFailure = new Error(`tauri-driver exited early: code=${code} signal=${signal || "none"}`);
  });

  const base = `http://127.0.0.1:${port}/`;
  let session = null;
  try {
    await waitForDriver(base, () => driverFailure);
    session = await newSession(base, driverAppPath);

    // 1. The native WebView reached the ShellX Cut UI shell (topbar mounts).
    await waitFor(session, `return !!document.querySelector('[data-cut-panel="topbar"]');`);
    check("native WebView reached the ShellX Cut UI", true);

    // 2. The embedded cutd engine link is live — the statusbar reflects the REAL
    //    UI↔cutd WS connection (agent-first single state holder). "open" = connected.
    const conn = await waitFor(session, `
      const el = document.querySelector('[data-cut-panel="statusbar"] [data-cut-connection]');
      const v = el && el.getAttribute('data-cut-connection');
      return v === 'open' ? { connected: true } : null;
    `);
    check("statusbar shows cutd engine connected", conn?.connected === true);

    // 3. The full verb API is reachable FROM the WebView (same origin as the UI's
    //    own fetches → this is the agent-first surface the whole product rests on).
    const verbs = await execAsync(session, `
      const r = await fetch('/api/verbs');
      const j = await r.json();
      const list = Array.isArray(j) ? j : (j.verbs || []);
      return { count: list.length, hasState: JSON.stringify(list).includes('project.state') };
    `);
    check(`verb registry reachable from WebView (${EXPECTED_VERBS} verbs)`, verbs.count === EXPECTED_VERBS && verbs.hasState, `count=${verbs.count}`);

    const agentDocs = await verifyAgentDocsApi({
      engineBase: "http://127.0.0.1:6161",
      sourceRoot: REPO_ROOT,
      expectedVersion: EXPECTED_VERSION,
    });
    check(
      `installed agent docs are exact (${agentDocs.checked} files)`,
      agentDocs.ok,
      agentDocs.ok ? `version=${agentDocs.version}` : agentDocs.failures.join("; "),
    );

    // 4. Drive verbs through the engine FROM the WebView — proves the DISPATCH
    //    surface (not just the registry) works end-to-end in the packaged app.
    //    A fresh smoke has no project OPEN, so we drive project-INDEPENDENT verbs
    //    that should succeed, plus confirm a project-scoped verb DISPATCHES with a
    //    proper actionable envelope (ok:false + a no_project/empty error — NOT a
    //    404/unknown-verb), which is what an agent relies on.
    const drive = await execAsync(session, `
      const post = (n, a) => fetch('/api/verb/' + n, { method:'POST', headers:{'content-type':'application/json'}, body: JSON.stringify(a||{}) }).then(r => r.json());
      const doctor = await post('system.doctor', {});
      const list = await post('project.list', {});
      const pub = await post('export.publish', { platform: 'tiktok', dry_run: true });
      return {
        doctorOk: doctor.ok === true && Array.isArray(doctor.result?.cards),
        cards: doctor.result?.cards?.length || 0,
        listOk: list.ok === true,
        // export.publish is WIRED (today's verb): either it ran (ok) or it
        // returned a structured engine error (has .error.code) — both prove the
        // verb dispatched. A 404/"unknown verb" would have neither.
        publishWired: pub.ok === true || (pub.ok === false && !!pub.error?.code),
        publishCode: pub.ok === false ? pub.error?.code : "ok",
      };
    `);
    check("system.doctor drivable from WebView", drive.doctorOk, `cards=${drive.cards}`);
    check("project.list drivable from WebView", drive.listOk);
    check("export.publish wired + dispatches via WebView", drive.publishWired, `→ ${drive.publishCode}`);

    // 5. Always-mounted surfaces are present.
    const present = await exec(session, `
      return ${JSON.stringify(ALWAYS_PANELS)}.map((p) => ({ p, ok: !!document.querySelector('[data-cut-panel="' + p + '"]') }));
    `);
    const missing = present.filter((x) => !x.ok).map((x) => x.p);
    check("core surfaces mounted (topbar/timeline/preview/statusbar)", missing.length === 0, missing.length ? `missing=${missing.join(",")}` : "");

    // 6. Each left-tab surface mounts when its tab is activated (the "all surfaces"
    //    coverage the release gate is about).
    for (const { tab, panel } of LEFT_TAB_PANELS) {
      const r = await execAsync(session, `
        const btn = document.querySelector('[data-cut-left-tab="${tab}"]');
        if (!btn) return { ok: false, why: 'no tab button' };
        btn.click();
        for (let i = 0; i < 40; i++) {
          if (document.querySelector('[data-cut-panel="${panel}"]')) return { ok: true };
          await new Promise((r) => setTimeout(r, 50));
        }
        return { ok: false, why: 'panel did not mount' };
      `);
      check(`left-tab surface '${panel}' mounts`, r.ok === true, r.ok ? "" : r.why);
    }

    // 7. Primary action controls present (Render + Export entry points).
    const controls = await exec(session, `
      const render = !!document.querySelector('[data-cut-render-btn]');
      const buttons = [...document.querySelectorAll('[data-cut-panel="topbar"] button')].map((b) => (b.textContent || '').trim());
      return { render, hasExport: buttons.some((t) => /export/i.test(t)) };
    `);
    check("topbar Render control present", controls.render);
    check("topbar Export menu present", controls.hasExport);

    // 8. Visual evidence.
    const bytes = await screenshot(session, join(outDir, "tauri-webview.png"));
    check("WebDriver screenshot captured the native WebView", bytes > 1000, `bytes=${bytes}`);
  } finally {
    if (session?.id) await wd(session.base, "DELETE", `session/${session.id}`).catch(() => {});
    await writeFile(join(outDir, "tauri-driver.log"), driverLog);
    if (!process.argv.includes("--keep-driver")) {
      driverStopping = true;
      driver.kill();
    }
  }

  console.log(`\ntauri-webdriver-smoke: ${pass} passed, ${fail} failed`);
  console.log(`Evidence: ${outDir}`);
  process.exitCode = fail === 0 ? 0 : 1;
}

main().catch((err) => {
  console.error(err?.stack || err?.message || String(err));
  process.exit(1);
});
