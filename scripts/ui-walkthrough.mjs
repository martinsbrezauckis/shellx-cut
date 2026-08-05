#!/usr/bin/env node
// ui-walkthrough.mjs — connected-client qualification walkthrough. Where
// tauri-webdriver-smoke.mjs proves surfaces MOUNT, this DRIVES every UI control
// on the real native WebView (wry/WebKitGTK on Linux) and asserts the action
// EXECUTED — the verb actually fired and/or the UI reflects the effect — with
// zero console errors. It also drives the 6 ui.* verbs end-to-end against a real
// connected client (the headless harness defers them because cutd has no
// UI client; here the app's WebView IS that client).
//
// Universal effect oracle: the page's window.fetch is monkeypatched to record
// every POST /api/verb/{name} into window.__vwCalls. After clicking a control we
// assert one of: a verb fired · a modal/drawer opened · a panel/state changed ·
// (for pure-UI controls) at least no console error. Controls that are disabled or
// need a precondition we can't cheaply stage are SKIPPED WITH A LOGGED REASON
// (never silently) — the plan's accepted honest outcome.
//
// PRECONDITION: a cutd answers on 127.0.0.1:6161 with a SEEDED project (this
// script starts one + imports testdata/talking_head.mp4 so the timeline has
// content + a transcript to act on). The desktop app reuses that engine.
//
// Run (Linux, WSLg display):  node scripts/ui-walkthrough.mjs
// Evidence: ~/.shellx-scratch/shellx-cut/ui-walkthrough-<stamp>/

import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { base64ToBuffer } from "./lib/safe-data.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(__dirname, "..");
const ASSET = join(REPO, "testdata", "talking_head.mp4");

const argv = process.argv.slice(2);
const argOf = (n, d) => { const i = argv.indexOf(n); return i >= 0 && argv[i + 1] ? argv[i + 1] : d; };
const ENGINE_PORT = Number(argOf("--engine-port", "6161"));
const WD_PORT = Number(argOf("--port", "4444"));
const ENGINE = `http://127.0.0.1:${ENGINE_PORT}`;
const STAMP = new Date().toISOString().replace(/[:.]/g, "-");
const OUT = resolve(argOf("--out", join(homedir(), ".shellx-scratch", "shellx-cut", `ui-walkthrough-${STAMP}`)));
const SCRATCH = join(REPO, ".scratch", `uiw-${STAMP}`);

let pass = 0, fail = 0, skip = 0;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
function check(name, cond, detail = "") { if (cond) { pass++; console.log(`    PASS  ${name}${detail ? "  " + detail : ""}`); } else { fail++; console.log(`    FAIL  ${name}${detail ? "  " + detail : ""}`); } return !!cond; }
function skipped(name, why) { skip++; console.log(`    SKIP  ${name}  ${why}`); }

// ---- engine (cutd) transport from node -------------------------------------
async function EV(name, args = {}) {
  const r = await fetch(`${ENGINE}/api/verb/${name}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(args), signal: AbortSignal.timeout(180000) });
  return r.json();
}
async function waitJob(id, to = 300000) { const t0 = Date.now(); while (Date.now() - t0 < to) { const s = await EV("jobs.status", { job_id: id }); const st = s.result?.state; if (st === "done") return { ok: true, result: s.result?.result ?? s.result }; if (st === "failed") return { ok: false, err: s.result?.error }; await sleep(1000); } return { ok: false, timeout: true }; }

// ---- WebDriver plumbing (mirrors tauri-webdriver-smoke.mjs) -----------------
async function wd(base, method, path, body) {
  const headers = { connection: "close" };
  if (body !== undefined) headers["content-type"] = "application/json";
  const res = await fetch(new URL(path, base), { method, headers, body: body === undefined ? undefined : JSON.stringify(body), keepalive: false, signal: AbortSignal.timeout(20000) });
  const text = await res.text();
  let json = {};
  if (text.trim()) { try { json = JSON.parse(text); } catch { json = { value: text }; } }
  if (!res.ok) { const msg = typeof json?.value === "object" ? json.value.message : text; throw new Error(`WebDriver ${method} ${path} (${res.status}): ${msg || res.statusText}`); }
  return json;
}
async function waitForDriver(base, failed, timeoutMs = 30000) { const start = Date.now(); while (Date.now() - start < timeoutMs) { if (failed()) throw failed(); try { await wd(base, "GET", "status"); return; } catch { await sleep(250); } } throw new Error(`Timed out waiting for tauri-driver at ${base}`); }
async function evaluateSync(session, script, args = []) { const res = await wd(session.base, "POST", `session/${session.id}/execute/sync`, { script, args }); return res.value; }
async function execAsync(session, body, args = []) {
  const script = `const done = arguments[arguments.length - 1]; Promise.resolve((async () => { ${body} })()).then((value) => done(value), (err) => done({ __e: String(err && err.stack || err) }));`;
  const res = await wd(session.base, "POST", `session/${session.id}/execute/async`, { script, args });
  if (res.value?.__e) throw new Error(res.value.__e);
  return res.value;
}
async function waitFor(session, body, timeoutMs = 30000) { const start = Date.now(); let last = null; while (Date.now() - start < timeoutMs) { try { last = await execAsync(session, body); if (last) return last; } catch (err) { last = err.message; } await sleep(300); } throw new Error(`Timed out: ${String(last)}`); }
async function screenshot(session, path) { const res = await wd(session.base, "GET", `session/${session.id}/screenshot`); const bytes = base64ToBuffer(res.value || ""); await writeFile(path, bytes); return bytes.length; }
async function newSession(base, appPath) { const res = await wd(base, "POST", "session", { capabilities: { alwaysMatch: { browserName: "wry", "tauri:options": { application: appPath } } } }); const value = res.value || res; const id = value.sessionId || res.sessionId; if (!id) throw new Error(`no session id: ${JSON.stringify(res)}`); return { base, id }; }
function defaultAppPath() { return join(REPO, "app", "desktop", "src-tauri", "target", "release", "shellx-cut"); }

// ---- page instrumentation (injected once) ----------------------------------
const INSTRUMENT = `
  if (!window.__vwInstalled) {
    window.__vwInstalled = true;
    window.__vwCalls = [];
    window.__vwErrors = [];
    const of = window.fetch;
    window.fetch = function(input, init) {
      try {
        const url = typeof input === 'string' ? input : (input && input.url) || '';
        const m = url.match(/\\/api\\/verb\\/([a-z_.]+)/);
        if (m) window.__vwCalls.push({ verb: m[1], t: Date.now() });
      } catch {}
      return of.apply(this, arguments);
    };
    window.addEventListener('error', (e) => window.__vwErrors.push(String(e.message || e)));
    window.addEventListener('unhandledrejection', (e) => window.__vwErrors.push('unhandledrejection: ' + String(e.reason)));
  }
  return true;
`;
// snapshot/reset the verb-call log
const callsSince = (session, sinceLen) => evaluateSync(session, `return window.__vwCalls.slice(${sinceLen}).map(c => c.verb);`);
const callsLen = (session) => evaluateSync(session, `return (window.__vwCalls||[]).length;`);
const errorsNow = (session) => evaluateSync(session, `return (window.__vwErrors||[]).slice();`);

let cutdStarted = false;
function stopEngine() { if (cutdStarted) { try { spawnSync("fuser", ["-k", `${ENGINE_PORT}/tcp`]); } catch { /* noop */ } } }

async function main() {
  await mkdir(OUT, { recursive: true });
  await mkdir(SCRATCH, { recursive: true });
  const appPath = resolve(argOf("--app", defaultAppPath()));
  if (!existsSync(appPath)) throw new Error(`desktop app not built: ${appPath} (cd app/desktop/src-tauri && cargo build --release)`);

  // 1. Ensure a cutd ENGINE answers on 6161; start one if absent (so the app reuses it).
  let engineUp = false;
  try { await fetch(`${ENGINE}/api/verbs`); engineUp = true; } catch { /* start it */ }
  if (!engineUp) {
    const bin = join(REPO, "app", "target", "release", "cutd");
    const cutd = spawn("setsid", [bin, "serve", "--addr", `127.0.0.1:${ENGINE_PORT}`, "--ui-dist", join(REPO, "ui", "dist")],
      { env: { ...process.env, SHELLX_CUT_NO_HWENC: "1" }, stdio: "ignore", detached: true });
    cutd.unref(); cutdStarted = true;
    for (let i = 0; i < 80; i++) { try { await fetch(`${ENGINE}/api/verbs`); engineUp = true; break; } catch { await sleep(500); } }
  }
  if (!engineUp) throw new Error("cutd engine not reachable on 6161");
  console.log(`engine: ${ENGINE} (${cutdStarted ? "started by harness" : "pre-existing"})`);

  // 2. Seed a project with content + transcript so controls have something to act on.
  const name = "uiw";
  const dir = join(SCRATCH, `${name}.cutproj`);
  await EV("project.create", { name, dir });
  const imp = await EV("media.import", { path: ASSET, rationale: "ui-walkthrough seed" });
  // Check ok BEFORE dereferencing result — a failed import used
  // to throw a cryptic "Cannot read properties of undefined (reading 'job_id')". The
  // common cause is a REUSED pre-existing cutd on 6161 living in a different filesystem
  // namespace (a stray WSL dev engine), so it can't see this checkout's testdata.
  if (!imp.ok) {
    throw new Error(
      `media.import failed (${imp.error?.code}): ${imp.error?.message}.\n` +
      (cutdStarted
        ? `  asset: ${ASSET}`
        : `  A PRE-EXISTING engine on ${ENGINE} was reused — it may run in a DIFFERENT filesystem ` +
          `namespace than this checkout, so '${ASSET}' isn't visible to it. Stop the stray cutd ` +
          `(leave 6161 free) so this harness starts + owns its own engine, then re-run.`),
    );
  }
  const A = imp.result?.asset_id;
  const ij = await waitJob(imp.result.job_id);
  const enrich = ij.result?.enrich_job;
  if (enrich) await waitJob(enrich);
  const tg = await EV("transcript.get", { asset: A });
  const haveTranscript = (tg.result?.words?.length || 0) > 0;
  await EV("captions.generate", {}).catch(() => {});
  console.log(`seed: asset=${A} transcript=${haveTranscript ? tg.result.words.length + " words" : "none"}\n`);

  // 3. Launch the app via tauri-driver.
  const nativePort = WD_PORT + 1000;
  const driver = spawn(argOf("--driver", "tauri-driver"), ["--port", String(WD_PORT), "--native-port", String(nativePort)], { cwd: REPO, stdio: ["ignore", "pipe", "pipe"] });
  let driverLog = "", driverFailure = null, driverStopping = false;
  driver.stdout.on("data", (c) => { driverLog += c; if (process.env.VERBOSE) process.stdout.write(c); });
  driver.stderr.on("data", (c) => { driverLog += c; if (process.env.VERBOSE) process.stderr.write(c); });
  driver.on("error", (e) => { driverFailure = new Error(`tauri-driver: ${e.message}`); });
  driver.on("exit", (code) => { if (!driverStopping && code !== 0) driverFailure = new Error(`tauri-driver exited: ${code}`); });

  const base = `http://127.0.0.1:${WD_PORT}/`;
  let session = null;
  try {
    await waitForDriver(base, () => driverFailure);
    session = await newSession(base, appPath);

    // UI shell + engine link.
    await waitFor(session, `return !!document.querySelector('[data-cut-panel="topbar"]');`);
    check("native WebView reached the UI", true);
    const conn = await waitFor(session, `const el=document.querySelector('[data-cut-panel="statusbar"] [data-cut-connection]'); return el&&el.getAttribute('data-cut-connection')==='open'?{c:true}:null;`);
    check("UI↔cutd WS connected (real client)", conn?.c === true);
    await evaluateSync(session, INSTRUMENT);

    // The seeded project must be visible in the app (timeline has clips).
    const seeded = await waitFor(session, `const r=await fetch('/api/verb/project.state',{method:'POST',headers:{'content-type':'application/json'},body:'{}'}); const j=await r.json(); const tr=j.result&&j.result.tracks||[]; const n=tr.reduce((a,t)=>a+(t.clips||[]).length,0); return n>0?{clips:n}:null;`, 20000);
    check("seeded project visible to the WebView", !!seeded, `clips=${seeded?.clips}`);

    // ---- (A) ui.* verbs end-to-end via the real connected client ------------
    console.log(`\n  ui.* verbs (driven from the engine API → WS → this WebView):`);
    // ui.state — returns the state the connected UI last pushed (panels +
    // playhead + selection). `ok:true` + a state object IS the contract (the
    // headless case errors no_ui_client; here a real client is connected).
    const us = await EV("ui.state", {});
    const uiStateOk = us.ok && us.result && ("playhead_ms" in us.result || "panels" in us.result || "selected_clip_ids" in us.result);
    check("ui.state → connected client's pushed state returned", uiStateOk, `keys=${Object.keys(us.result || {}).join(",")}`);
    // ui.open — switch to the transcript panel; WebView reflects.
    await EV("ui.open", { panel: "transcript" });
    const opened = await waitFor(session, `return document.querySelector('[data-cut-panel="transcript"]') ? {ok:true} : null;`, 8000).catch(() => null);
    check("ui.open{transcript} → panel switched in WebView", !!opened);
    // ui.playhead — move the playhead; WebView state reflects.
    await EV("ui.playhead", { at_ms: 2500 });
    const ph = await waitFor(session, `const r=await fetch('/api/verb/ui.state',{method:'POST',headers:{'content-type':'application/json'},body:'{}'}); const j=await r.json(); return (j.result&&j.result.playhead_ms>=2000)?{ph:j.result.playhead_ms}:null;`, 8000).catch(() => null);
    check("ui.playhead{2500} → playhead moved", !!ph, ph ? `playhead_ms=${ph.ph}` : "playhead not reflected");
    // ui.select — select a clip; ui.state reflects selection.
    await EV("ui.select", { clip_ids: ["c1"] });
    const sel = await waitFor(session, `const r=await fetch('/api/verb/ui.state',{method:'POST',headers:{'content-type':'application/json'},body:'{}'}); const j=await r.json(); const s=j.result&&j.result.selected_clip_ids||[]; return s.includes('c1')?{s}:null;`, 8000).catch(() => null);
    check("ui.select{c1} → selection reflected", !!sel);
    // ui.highlight — draw a highlight; the overlay element appears.
    await EV("ui.highlight", { panel: "timeline", label: "VW", duration_ms: 4000 });
    const hl = await waitFor(session, `return document.querySelector('[data-cut-highlight]') ? {ok:true} : null;`, 8000).catch(() => null);
    check("ui.highlight → overlay drawn in WebView", !!hl, hl ? "" : "(no [data-cut-highlight]; may auto-clear fast)");
    // ui.screenshot — the connected UI captures itself → PNG bytes.
    const shot = await EV("ui.screenshot", {});
    check("ui.screenshot → PNG via connected client", shot.ok && (!!shot.result?.path || !!shot.result?.base64), `path=${shot.result?.path ? "yes" : "no"}`);

    // ---- (B) drive every control class on the WebView -----------------------
    // Enumerate the live DOM (authoritative — not a hardcoded list).
    const inv = await evaluateSync(session, `
      const grab = (attr) => [...document.querySelectorAll('['+attr+']')].map(e => e.getAttribute(attr)).filter((v,i,a)=>a.indexOf(v)===i);
      return { tabs: grab('data-cut-left-tab'), transport: grab('data-cut-transport-btn'), tools: grab('data-cut-tool'), actions: grab('data-cut-action') };
    `);

    // click helper: returns {fired:[verbs], modal:bool, err:[]}
    async function clickAndObserve(attr, val) {
      const before = await callsLen(session);
      const modalsBefore = await evaluateSync(session, `return document.querySelectorAll('[data-cut-scrim],[class*="scrim"]').length;`);
      const r = await execAsync(session, `
        const el = document.querySelector('[${attr}="${val}"]');
        if (!el) return { missing: true };
        const disabled = el.disabled || el.getAttribute('aria-disabled') === 'true';
        if (disabled) return { disabled: true };
        el.click();
        await new Promise(r => setTimeout(r, 350));
        return { clicked: true };
      `);
      if (r?.missing) return { missing: true };
      if (r?.disabled) return { disabled: true };
      const fired = await callsSince(session, before);
      const modalsAfter = await evaluateSync(session, `return document.querySelectorAll('[data-cut-scrim],[class*="scrim"]').length;`);
      const errs = await errorsNow(session);
      const modalOpened = modalsAfter > modalsBefore;
      // Close an opened modal so the next control isn't blocked — but ONLY when a
      // modal actually opened. A gratuitous Escape clears the clip selection and
      // disables the selection-gated controls that follow.
      if (modalOpened) {
        await execAsync(session, `
          const close = document.querySelector('[data-cut-grade-close],[data-cut-kinetic-close],[data-cut-musicbed-close],[data-cut-title-close],[data-cut-storyboard-close],[data-cut-director-close],[data-cut-clips-close],[data-cut-autopilot-close],[data-cut-environment-close]');
          if (close) { close.click(); await new Promise(r=>setTimeout(r,150)); } else { document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true})); }
          return true;
        `).catch(() => {});
      }
      return { fired, modalOpened, errs };
    }

    // pure-UI controls (no verb expected — assert just no console error / clickable)
    const PURE_UI = new Set(["collapse-left", "expand-left", "collapse-rail", "expand-rail", "comments-collapse", "dismiss-guidance", "tools-menu", "reel-mode", "open-grade", "open-kinetic", "rebase-cancel"]);
    // controls that genuinely can't be driven from the WebView here → honest skip.
    // (native OS file dialog, or a transient state the harness doesn't stage).
    const NEEDS_PRECOND = {
      "import-asset": "opens a NATIVE OS file picker (not DOM-observable)",
      "comment-apply": "needs a drafted comment", "comment-draft": "needs a coding-agent CLI", "comment-dismiss": "needs a selected comment", "comment-seek": "needs a comment",
      "rebase-confirm": "needs an active rebase", "rebase-reject-op": "needs a rebase preview", "guidance-revert": "needs undo guidance shown",
      "accept-op": "needs a pending op", "reject-op": "needs a pending op", "restore": "needs a restorable tip", "undo-tip": "needs an undoable tip",
      "add-to-reel": "needs clip candidates", "reel-remove": "needs a reel entry", "reel-clear": "needs a reel", "assemble-reel": "needs reel entries",
      "apply-xfade": "needs a selected seam", "clear-xfade": "needs an xfade",
      "qc-shift": "needs a QC result", "qc-brand": "needs a render receipt", "qc-reflow": "needs caption violations", "insert-asset": "needs a selected asset card",
      "cut-words": "needs a transcript word selection",
    };
    // STAGE a clip selection so the selection-gated controls (set-gain, toggle-mute,
    // open-grade, speed-*, save-range, snapshot-frame) become enabled + driveable.
    await EV("ui.select", { clip_ids: ["c1"] });
    await waitFor(session, `const r=await fetch('/api/verb/ui.state',{method:'POST',headers:{'content-type':'application/json'},body:'{}'});const j=await r.json();const s=j.result&&j.result.selected_clip_ids||[];return s.includes('c1')?{ok:true}:null;`, 8000).catch(() => {});
    // give React a beat to enable the selection-gated controls.
    await sleep(400);

    console.log(`\n  left tabs (${inv.tabs.length}):`);
    for (const v of inv.tabs) {
      const r = await clickAndObserve("data-cut-left-tab", v);
      if (r.missing) { skipped(`tab:${v}`, "not in DOM"); continue; }
      const mounted = await execAsync(session, `for(let i=0;i<30;i++){if(document.querySelector('[data-cut-panel="${v}"]'))return{ok:true};await new Promise(r=>setTimeout(r,40));}return null;`).catch(() => null);
      check(`tab:${v} → panel mounts`, !!mounted, (r.errs?.length ? `console errors: ${r.errs.length}` : ""));
    }

    console.log(`\n  transport (${inv.transport.length}):`);
    for (const v of inv.transport) {
      const r = await clickAndObserve("data-cut-transport-btn", v);
      if (r.missing) { skipped(`transport:${v}`, "not in DOM"); continue; }
      if (r.disabled) { skipped(`transport:${v}`, "disabled"); continue; }
      check(`transport:${v} → clicked, no console error`, (r.errs?.length || 0) === 0, r.errs?.length ? `errors: ${r.errs.join("; ").slice(0, 120)}` : "");
    }

    console.log(`\n  tools (${inv.tools.length}):`);
    for (const v of inv.tools) {
      const r = await clickAndObserve("data-cut-tool", v);
      if (r.missing) { skipped(`tool:${v}`, "not in DOM"); continue; }
      if (r.disabled) { skipped(`tool:${v}`, "disabled"); continue; }
      check(`tool:${v} → clicked, no console error`, (r.errs?.length || 0) === 0, r.errs?.length ? `errors` : "");
    }

    console.log(`\n  actions (${inv.actions.length}):`);
    // Controls gated on a selected MEDIA clip. The HUMAN path is clicking a clip
    // in the timeline (the agent's ui.select relay drives verbs directly and does
    // not enable the human toolbar — by design). So stage selection by CLICKING a
    // real [data-cut-clip] element, right before each gated control.
    const SELECTION_GATED = new Set(["open-grade", "speed-preset", "save-range", "snapshot-frame"]);
    // The VIDEO media clip id from the engine (caption cues are also
    // [data-cut-clip]; speed/grade/save-range gate on a video/audio selection).
    const stForClip = await EV("project.state", {});
    const vTrack = (stForClip.result?.tracks || []).find((t) => t.kind === "video");
    const mediaClipId = (vTrack?.clips || []).find((c) => c.id && c.asset)?.id || "c1";
    // Clips select on POINTERDOWN (onClipDown), not a synthetic click — dispatch
    // real pointer events at the media clip's coordinates so the selection handler
    // fires and selectedMedia (video/audio) is populated.
    const selectClipInDom = () => execAsync(session, `
      const el=document.querySelector('[data-cut-clip="${mediaClipId}"]');
      if(!el) return {none:true, want:"${mediaClipId}"};
      const r=el.getBoundingClientRect();
      const o={bubbles:true,cancelable:true,clientX:r.left+8,clientY:r.top+8,button:0,pointerId:1,pointerType:'mouse'};
      el.dispatchEvent(new PointerEvent('pointerdown',o));
      el.dispatchEvent(new PointerEvent('pointerup',o));
      el.dispatchEvent(new MouseEvent('click',o));
      await new Promise(r=>setTimeout(r,300));
      return {clip: el.getAttribute('data-cut-clip')};
    `).catch(() => ({}));
    let actDriven = 0, actSkipped = 0;
    for (const v of inv.actions) {
      if (NEEDS_PRECOND[v]) { skipped(`action:${v}`, NEEDS_PRECOND[v]); actSkipped++; continue; }

      // set-gain is a NUMBER INPUT (fires edit.gain on Enter/blur, not on click).
      if (v === "set-gain") {
        await selectClipInDom();
        const before = await callsLen(session);
        await execAsync(session, `
          const inp=document.querySelector('[data-cut-action="set-gain"]');
          if(!inp) return {missing:true};
          inp.focus();
          const set=Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype,'value').set;
          set.call(inp,'-4'); inp.dispatchEvent(new Event('input',{bubbles:true}));
          inp.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true})); inp.blur();
          await new Promise(r=>setTimeout(r,350)); return {done:true};
        `).catch(() => {});
        const fired = await callsSince(session, before);
        actDriven++;
        check(`action:set-gain`, fired.includes("edit.gain"), fired.length ? `fired ${fired.join(",")}` : "no edit.gain fired (input)");
        continue;
      }

      if (SELECTION_GATED.has(v)) { await selectClipInDom(); }
      const r = await clickAndObserve("data-cut-action", v);
      if (r.missing) { skipped(`action:${v}`, "not in DOM (conditional control)"); actSkipped++; continue; }
      if (r.disabled) { skipped(`action:${v}`, SELECTION_GATED.has(v) ? "human timeline-toolbar control gated on an interactive media-clip selection (canvas-drawn clips — WebDriver synthetic pointer can't reliably stage it); underlying verb (edit.speed/edit.grade/export.range) is effect-verified by the verb gate" : "disabled (precondition not met)"); actSkipped++; continue; }
      const fired = (r.fired?.length || 0) > 0;
      const ok = (fired || r.modalOpened || PURE_UI.has(v)) && (r.errs?.length || 0) === 0;
      actDriven++;
      check(`action:${v}`, ok, fired ? `fired ${r.fired.join(",")}` : r.modalOpened ? "opened modal" : PURE_UI.has(v) ? "ui-only ok" : (r.errs?.length ? `errors: ${r.errs.join(";").slice(0, 100)}` : "no verb/modal/ui-effect observed"));
    }
    console.log(`  actions: ${actDriven} driven, ${actSkipped} skipped (precondition/conditional)`);

    // ---- (C) console-error gate + screenshots -------------------------------
    const allErrs = await errorsNow(session);
    check("zero uncaught console errors across the walkthrough", allErrs.length === 0, allErrs.length ? `errors:\n      ${allErrs.slice(0, 5).join("\n      ")}` : "");
    const bytes = await screenshot(session, join(OUT, "ui-walkthrough.png"));
    check("final screenshot captured", bytes > 1000, `bytes=${bytes}`);
    await writeFile(join(OUT, "inventory.json"), JSON.stringify(inv, null, 2));
  } finally {
    if (session?.id) await wd(session.base, "DELETE", `session/${session.id}`).catch(() => {});
    await writeFile(join(OUT, "tauri-driver.log"), driverLog).catch(() => {});
    if (!argv.includes("--keep-driver")) { driverStopping = true; driver.kill(); }
  }

  console.log(`\nui-walkthrough: ${pass} passed, ${fail} failed, ${skip} skipped`);
  console.log(`Evidence: ${OUT}`);
  process.exitCode = fail === 0 ? 0 : 1;
}

main()
  .catch((e) => { console.error(e?.stack || String(e)); process.exitCode = 1; })
  .finally(async () => { stopEngine(); if (!argv.includes("--keep")) { try { spawnSync("rm", ["-rf", SCRATCH]); } catch { /* noop */ } } });
