// interaction-verify.mjs — ACTION / INTERACTION coverage gate.
//
// release-verify proves each TOOL's EFFECT on the OUTPUT (does grade change pixels). This
// proves the UI BEHAVES: the right view opens, menus open, panels reopen after closing,
// toggles flip, frames don't leak between projects, empty states are honest. That's the
// CLASS that slipped past the effect-only gate. The default view, dead controls,
// panel reopening, cross-project frame isolation, and transcript empty state are
// each explicit checks here so those regressions cannot recur.
//
// RUN:  cd ui && SHELLX_CUT_PROJECTS_DIR=/path/to/run-owned/projects \
//         SWEEP_CUTD=http://127.0.0.1:6171 SWEEP_APP=http://localhost:5173 \
//         node public-tests/interaction-verify.mjs
// Exit 0 = all PASS; non-zero on any FAIL (CI/gate friendly).
import { chromium } from "playwright";
import { spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { base64ToBuffer } from "../../scripts/lib/safe-data.mjs";
import {
  requireIsolatedTestProjectsDir,
  withIsolatedProjectCreate,
} from "../../scripts/lib/test-project-isolation.mjs";

const CUTD = process.env.SWEEP_CUTD || "http://127.0.0.1:6171";
const APP = process.env.SWEEP_APP || "http://localhost:5173";
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..");
const CLIP = process.env.RELEASE_CLIP || join(REPO, "testdata", "talking_head.mp4");
const CLIP2 = process.env.RELEASE_CLIP2 || join(REPO, "testdata", "silent_screen.mp4");
// A DISTINCT asset that carries AUDIO (talking_head is the base; silent_screen is
// silent) — needed by the detach-audio extraction check: detaching is a no-op when
// an audio clip of the SAME asset already exists, so the overlay must be a different
// asset with its own audio and no sibling audio clip on the timeline.
const CLIP3 = process.env.RELEASE_CLIP3 || join(REPO, "testdata", "insert_clip.mp4");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tmp = mkdtempSync(join(tmpdir(), "iv-"));
const TEST_PROJECTS_DIR = requireIsolatedTestProjectsDir(process.env.SHELLX_CUT_PROJECTS_DIR);
let seq = 0;
let clickDiagSeq = 0;

// HANG WATCHDOG (see release-verify): bound every verb so a cutd freeze FAILS instead of stalling.
const VERB_TIMEOUT_MS = Number(process.env.VERB_TIMEOUT_MS || 60000);
async function verb(name, args = {}) {
  args = withIsolatedProjectCreate(name, args, TEST_PROJECTS_DIR);
  const t0 = Date.now();
  try {
    const r = await fetch(`${CUTD}/api/verb/${name}`, {
      method: "POST",
      headers: { "content-type": "application/json", "x-cut-actor": "human:ui:ui" },
      body: JSON.stringify(args),
      signal: AbortSignal.timeout(VERB_TIMEOUT_MS),
    });
    return r.json();
  } catch (e) {
    const hang = e?.name === "TimeoutError" || /aborted|timed?\s*out/i.test(String(e));
    return { ok: false, hang, error: { message: (hang ? `VERB HANG >${VERB_TIMEOUT_MS}ms (${Date.now() - t0}ms): ${name} ` : "") + String(e) } };
  }
}
async function state() {
  return (await verb("project.state")).result;
}
async function ops() {
  return (await verb("project.ops")).result?.ops || [];
}
async function frame(at) {
  // inline:true → frame bytes as base64 over HTTP, so this reads the frame whether
  // cutd is local (WSL) or the installed Windows engine (C:\ path WSL can't cp).
  const r = await verb("render.frame", { at_ms: at, compose: true, inline: true });
  const mime = String(r.result?.mime || "");
  const ext = mime.includes("png") ? "png" : (mime.includes("jpeg") || mime.includes("jpg")) ? "jpg" : "bin";
  const dst = join(tmp, `f${seq++}.${ext}`);
  const b64 = r.result?.base64;
  if (b64) {
    writeFileSync(dst, base64ToBuffer(b64, { expectPng: ext === "png" }));
    return dst;
  }
  const src = r.result?.path;
  if (!src) return null;
  spawnSync("cp", [src, dst]);
  return dst;
}
function ssim(a, b) {
  const r = spawnSync(
    "ffmpeg",
    ["-i", a, "-i", b, "-filter_complex", "[0:v]scale=320:180[x];[1:v]scale=320:180[y];[x][y]ssim", "-f", "null", "-"],
    { encoding: "utf8" }
  );
  const m = (r.stderr || "").match(/All:([\d.]+)/);
  return m ? parseFloat(m[1]) : null;
}
const cnt = async (page, sel) => await page.locator(sel).count();
const vis = async (page, sel) => await page.locator(sel).first().isVisible().catch(() => false);
function isVerbTraffic(entry, verbName) {
  try {
    return new URL(entry.url()).pathname === `/api/verb/${verbName}`;
  } catch {
    return false;
  }
}
async function ensureRightRail(page) {
  const expand = page.locator('[data-cut-action="expand-rail"]');
  if (await expand.count()) {
    await expand.click({ timeout: 2000 }).catch(() => {});
    await sleep(250);
  }
}
async function ensurePinnedRail(page) {
  await ensureRightRail(page);
  const pin = page.locator('[data-cut-rail-pin]');
  if (await pin.count()) {
    const pressed = await pin.getAttribute("aria-pressed").catch(() => null);
    if (pressed !== "true") {
      await pin.click({ timeout: 2000 }).catch(() => {});
      await sleep(250);
    }
  }
}
async function openRightTab(page, tab) {
  await ensureRightRail(page);
  await page.locator(`[data-cut-right-tab="${tab}"]`).click();
  await sleep(tab === "properties" ? 300 : 400);
}
async function expandInspectorSection(page, sectionKey) {
  const section = page.locator(`[data-cut-section="${sectionKey}"]`).first();
  if (!(await section.count())) return false;
  if ((await section.getAttribute("data-cut-section-collapsed")) === "true") {
    await page.locator(`[data-cut-section-toggle="${sectionKey}"]`).first().click();
    for (let attempt = 0; attempt < 10; attempt++) {
      if ((await section.getAttribute("data-cut-section-collapsed")) === "false") break;
      await sleep(50);
    }
  }
  return (await section.getAttribute("data-cut-section-collapsed")) === "false";
}
async function openTimelineAutomation(page) {
  const trigger = page.locator("[data-cut-timeline-automation-trigger]").first();
  if (!(await trigger.count())) return false;
  if ((await trigger.getAttribute("aria-expanded")) !== "true") {
    await trigger.click();
    await sleep(150);
  }
  return await page.locator("[data-cut-timeline-automation-menu]").first().isVisible().catch(() => false);
}
async function releaseModifiers(page) {
  for (const key of ["Control", "Meta", "Shift", "Alt"]) {
    await page.keyboard.up(key).catch(() => {});
  }
}
async function closeActiveDrawers(page) {
  await releaseModifiers(page);
  const closeSelectors = [
    "[data-cut-matte-close]",
    "[data-cut-shape-close]",
    "[data-cut-title-close]",
    "[data-cut-layer-close]",
    "[data-cut-assemble-close]",
    "[data-cut-generate-close]",
    "[data-cut-musicbed-close]",
    "[data-cut-autopilot-close]",
    "[data-cut-recipes-close]",
    "[data-cut-clips-close]",
    "[data-cut-environment-close]",
    "[data-cut-wizard-dismiss]",
  ];
  for (const sel of closeSelectors) {
    const btn = page.locator(sel).first();
    if (await btn.count()) {
      await btn.click({ timeout: 1200 }).catch(() => {});
      await sleep(160);
    }
  }
  await page.keyboard.press("Escape").catch(() => {});
  await sleep(120);
}
async function clickOrCenterHit(page, locator, timeout = 5000) {
  await locator.scrollIntoViewIfNeeded({ timeout: 1000 }).catch(() => {});
  try {
    await locator.click({ timeout });
    return { ok: true, mode: "locator" };
  } catch (e) {
    const err = String(e.message || e).slice(0, 260).replace(/\s+/g, " ");
    const diag = await locator.evaluate((el) => {
      const rect = el.getBoundingClientRect();
      const cx = rect.left + rect.width / 2;
      const cy = rect.top + rect.height / 2;
      const hit = document.elementFromPoint(cx, cy);
      const style = window.getComputedStyle(el);
      return {
        x: cx,
        y: cy,
        w: rect.width,
        h: rect.height,
        disabled: el instanceof HTMLButtonElement || el instanceof HTMLSelectElement || el instanceof HTMLInputElement ? el.disabled : false,
        visible: style.visibility !== "hidden" && style.display !== "none" && style.pointerEvents !== "none",
        hitSelf: !!hit && (hit === el || el.contains(hit)),
        hitTag: hit?.tagName ?? null,
        hitClass: hit instanceof HTMLElement ? hit.className : null,
        hitId: hit instanceof HTMLElement ? hit.id : null,
        hitAction: hit instanceof HTMLElement ? hit.getAttribute("data-cut-action") : null,
        hitCut: hit instanceof HTMLElement ? Array.from(hit.attributes).find((a) => a.name.startsWith("data-cut-"))?.name ?? null : null,
        stack: document.elementsFromPoint(cx, cy).slice(0, 6).map((node) => {
          if (!(node instanceof HTMLElement)) return node.tagName;
          const cut = Array.from(node.attributes).find((a) => a.name.startsWith("data-cut-"));
          return `${node.tagName}${node.id ? `#${node.id}` : ""}${node.className ? `.${String(node.className).replace(/\s+/g, ".").slice(0, 80)}` : ""}${node.getAttribute("data-cut-action") ? `[action=${node.getAttribute("data-cut-action")}]` : ""}${cut ? `[${cut.name}]` : ""}`;
        }),
      };
    }).catch((inner) => ({ error: String(inner.message || inner).slice(0, 160) }));
    if (diag && diag.hitSelf && !diag.disabled && diag.visible && Number.isFinite(diag.x) && Number.isFinite(diag.y)) {
      await page.mouse.click(diag.x, diag.y);
      return { ok: true, mode: "center", err, diag };
    }
    const shot = join(tmp, `click-fail-${++clickDiagSeq}.png`);
    await page.screenshot({ path: shot, fullPage: true }).catch(() => {});
    return { ok: false, mode: "failed", err, diag, shot };
  }
}
function clickDiagSummary(click) {
  if (!click?.diag) return "";
  const d = click.diag;
  if (d.error) return ` diagErr=${d.error}`;
  const hitName = d.hitAction || d.hitCut || d.hitId || String(d.hitClass || "").slice(0, 80) || "-";
  const stack = Array.isArray(d.stack) ? ` stack=${d.stack.join(">")}` : "";
  const shot = click.shot ? ` shot=${click.shot}` : "";
  return ` hitSelf=${d.hitSelf} hit=${d.hitTag || "?"}:${hitName} ` +
    `disabled=${d.disabled} visible=${d.visible} box=${Math.round(d.w || 0)}x${Math.round(d.h || 0)}${stack}${shot}`;
}

// ── checks ──────────────────────────────────────────────────────────────────
// Each: async (page) => { pass: bool, detail: string }. Throw = FAIL.

async function checkNoDeadExportMode(page) {
  // The mode bar must not carry a dead "Export" workspace button.
  const modes = await page.locator("[data-cut-mode]").evaluateAll((els) => els.map((e) => e.getAttribute("data-cut-mode")));
  return { pass: !modes.includes("export") && modes.includes("edit"), detail: `modes=[${modes.join(",")}]` };
}

async function checkExportMenuOpens(page) {
  // The real Export ▾ dropdown opens.
  const btn = page.locator("[data-cut-export-btn]");
  if (!(await btn.count())) return { pass: false, detail: "no Export ▾ button" };
  await btn.click();
  await sleep(400);
  const open = await vis(page, "[data-cut-export-menu]");
  await page.keyboard.press("Escape").catch(() => {});
  return { pass: open, detail: `menu visible=${open}` };
}

async function checkOtioImportEntry(page) {
  // Timeline import belongs with project media, not inside Export.
  // The actual file pick needs the native desktop dialog, so verify the Assets
  // entry exists and is visible without opening the OS picker.
  const assetsTab = page.locator('[data-cut-left-tab="assets"]');
  if (!(await assetsTab.count())) return { pass: false, detail: "no Assets tab" };
  await assetsTab.click();
  await sleep(400);
  const item = await vis(page, "[data-cut-import-otio]");
  return { pass: item, detail: `import-otio entry visible=${item}` };
}

async function checkRightTabsSwitch(page) {
  // The right sidebar is a tabbed panel — Properties · Color · Audio. Each tab
  // must switch its body: Properties = Inspector, Color = grade controls, Audio = mixer.
  await openRightTab(page, "properties");
  const props = await vis(page, '[data-cut-panel="inspector"]');
  await openRightTab(page, "color");
  const color = await vis(page, "[data-cut-grade-embed]");
  await openRightTab(page, "audio");
  const audio = await vis(page, "[data-cut-mixer-embed]");
  await openRightTab(page, "properties");
  return { pass: props && color && audio, detail: `properties=${props} color=${color} audio=${audio}` };
}

async function checkFindIsPermanentLeftSidebar(page) {
  // Find is a permanent LEFT sidebar tab. The header no longer owns a
  // redundant dropdown; the sidebar tab and its two search subtabs are available
  // directly at any time.
  const headerGone = (await cnt(page, "[data-cut-find-btn]")) === 0 && (await cnt(page, "[data-cut-find-menu]")) === 0;
  const leftTab = await vis(page, '[data-cut-left-tab="find"]');
  await page.locator('[data-cut-left-tab="find"]').click();
  await sleep(400);
  const mediaTab = await vis(page, '[data-cut-find-tab="find-media"]');
  await page.locator('[data-cut-find-tab="find-media"]').click();
  await sleep(300);
  const stock = await vis(page, "[data-cut-stock-embed]");
  const noScrim = !(await vis(page, "[data-cut-stock-scrim]"));
  await page.locator('[data-cut-find-tab="find-moment"]').click();
  await sleep(400);
  const moment = await vis(page, "[data-cut-search-embed]");
  return {
    pass: headerGone && leftTab && mediaTab && stock && noScrim && moment,
    detail: `headerFindGone=${headerGone} leftFindTab=${leftTab} mediaTab=${mediaTab} mediaEmbed=${stock} momentEmbed=${moment} noRightScrim=${noScrim}`,
  };
}

async function checkAssetsGenerateLaunch(page) {
  // Generated-media placement: "Generate (AI)" lives in the Assets tray's "Add media" area
  // (assets.generate CREATES media — the result lands in Assets — so it belongs beside
  // +Import, not under search). Assert the placement:
  //   (1) no old header Find/Generate menu remains;
  //   (2) the Assets tray has a "Generate (AI)" button (beside +Import) that opens the
  //       Generate drawer (prompt + cost notice render);
  //   (3) the paid-gen guard still holds — a FIRST click ARMS without dispatching
  //       assets.generate (no real CLI/money spent merely by arming).
  let genRequests = 0;
  const onReq = (req) => { if (isVerbTraffic(req, "assets.generate")) genRequests++; };
  page.on("request", onReq);
  try {
    // (1) Find must remain a search-only left tab; no old header Find/Generate menu remains.
    const findMenuNoGenerate =
      (await cnt(page, "[data-cut-find-btn]")) === 0
      && (await cnt(page, "[data-cut-find-menu]")) === 0
      && (await cnt(page, "[data-cut-find-generate]")) === 0;
    // (2) Assets tab → the "Generate (AI)" button → the Generate drawer opens.
    await page.locator('[data-cut-left-tab="assets"]').click();
    await sleep(250);
    const hasBtn = await vis(page, '[data-cut-action="generate-asset"]');
    await page.locator('[data-cut-action="generate-asset"]').click();
    await sleep(400);
    const drawer = await vis(page, "[data-cut-generate]");
    const promptBox = await vis(page, "[data-cut-generate-prompt]");
    const costNotice = await vis(page, "[data-cut-generate-cost-notice]");
    // (3) Type a prompt + first click → must ARM (button gets data-cut-generate-armed),
    // NOT fire the verb. (A project is loaded in main(), so arming is permitted.)
    await page.locator("[data-cut-generate-prompt]").fill("a test icon of a rocket");
    await page.locator("[data-cut-generate-run]").click();
    await sleep(400);
    const armed = (await cnt(page, "[data-cut-generate-run][data-cut-generate-armed]")) > 0;
    // Leave the shared verifier page on the normal Assets tab. The check above
    // intentionally opens Generate; later timeline shortcut checks should not
    // inherit that active workspace/focus state.
    await page.locator('[data-cut-left-tab="assets"]').click().catch(() => {});
    await sleep(250);
    return {
      pass: findMenuNoGenerate && hasBtn && drawer && promptBox && costNotice && armed && genRequests === 0,
      detail: `findMenuNoGenerate=${findMenuNoGenerate} assetsBtn=${hasBtn} drawer=${drawer} prompt=${promptBox} cost=${costNotice} armedNoSpend=${armed} genReqs=${genRequests}`,
    };
  } finally {
    page.off("request", onReq);
  }
}

async function checkColorReopen(page) {
  // Color is a RIGHT-SIDEBAR TAB (not a workspace mode or modal drawer).
  // Click Color tab → grade body shows → switch to Properties → grade hides → click
  // Color again → grade body REOPENS. (The grade controls must survive tab toggles.)
  await openRightTab(page, "color");
  const o1 = await vis(page, "[data-cut-grade-embed]");
  await openRightTab(page, "properties");
  const closed = !(await vis(page, "[data-cut-grade-embed]"));
  await openRightTab(page, "color");
  const o2 = await vis(page, "[data-cut-grade-embed]");
  // leave clean: back to Properties
  await openRightTab(page, "properties");
  return { pass: o1 && closed && o2, detail: `open=${o1} hidden=${closed} reopen=${o2}` };
}

async function checkCaptionTextCard(page) {
  // The Inspector (Properties tab, no clip selected = timeline scope) carries a
  // caption composer. Drive it for real: deselect → Properties tab → type text → pick
  // position → "Add caption at playhead" (captions.add_text), then assert a txt1 caption
  // clip with that exact text landed in project.state (the INTENDED effect, not just an
  // op record). Also exercises "Set style" (captions.set_style) returns ok.
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(200);
  // Clear any clip selection so the Inspector shows its project-scope (caption) surface.
  await page.locator("body").click().catch(() => {});
  await page.keyboard.press("Escape").catch(() => {});
  await sleep(200);
  await openRightTab(page, "properties").catch(() => {});
  await sleep(300);
  const input = page.locator("[data-cut-caption-text]");
  if (!(await input.count())) return { pass: false, detail: "no caption composer in Inspector" };
  const text = "IVCAP_" + Math.random().toString(36).slice(2, 6).toUpperCase();
  await input.fill(text);
  await page.locator("[data-cut-caption-position]").selectOption("bottom").catch(() => {});
  await sleep(150);
  await page.locator("[data-cut-caption-add]").click();
  await sleep(700);
  // Verify the effect: a txt1 (caption-kind) clip with the typed text now exists.
  const s = await state();
  const capTrack = (s.tracks || []).find((t) => t.id === "txt1" || t.kind === "caption");
  const landed = !!capTrack?.clips?.some((c) => c.text === text);
  // Style control returns ok (captions.set_style) — drive it too (no project mutation assert).
  await page.locator("[data-cut-caption-style]").click().catch(() => {});
  await sleep(300);
  // The "Import captions (SRT/VTT)…" control (captions.import) is present in
  // the same surface. The OS file picker can't run headless, so we assert the
  // control exists + is visible (the import handler is desktop-only), mirroring
  // how otio-import-entry verifies the OTIO import control.
  const importBtn = await vis(page, "[data-cut-caption-import]");
  return { pass: landed && importBtn, detail: `caption "${text}" on txt1=${landed}; import control=${importBtn}` };
}

async function checkCaptionEditInspector(page) {
  // caption-editing regression: selecting a caption clip used to fall through to
  // the empty project view (the sel resolver required an asset), and the right-click
  // "Edit text & style…" dead-ended there. Now a caption selects to a per-caption TEXT
  // editor. It proves selection shows the current text, editing changes state,
  // change+Save → captions.set_text changes the caption's words in state (the INTENDED
  // and the right-click "Edit text & style…" action lands on that editor.
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(150);
  const orig = "IVEDIT_" + Math.random().toString(36).slice(2, 6).toUpperCase();
  await verb("captions.add_text", { text: orig, range_ms: [0, 2500], position: "bottom" });
  await sleep(500);
  const s0 = await state();
  const cap = (s0.tracks || []).find((t) => t.kind === "caption")?.clips?.find((c) => c.text === orig);
  if (!cap) return { pass: false, detail: "caption not added" };
  // Select the caption clip → caption editor with its text.
  await page.locator(`[data-cut-clip="${cap.id}"]`).click({ force: true }).catch(() => {});
  await sleep(400);
  await openRightTab(page, "properties").catch(() => {});
  const kind = await page.locator('[data-cut-panel="inspector"]').getAttribute("data-cut-inspector-kind").catch(() => null);
  const ta = page.locator("[data-cut-caption-edit-text]");
  const hasEditor = (await ta.count()) > 0;
  const shownText = hasEditor ? await ta.inputValue().catch(() => "") : "";
  const selectionOk = kind === "caption" && hasEditor && shownText === orig;
  // The edit itself: change text + Save → captions.set_text → state reflects it.
  let edited = false;
  if (hasEditor) {
    const next = orig + "_X";
    await ta.fill(next);
    await page.locator('[data-cut-action="caption-save-text"]').click().catch(() => {});
    await sleep(500);
    const s1 = await state();
    edited = !!(s1.tracks || []).find((t) => t.kind === "caption")?.clips?.some((c) => c.id === cap.id && c.text === next);
  }
  // Right-click "Edit text & style…" lands on the editor.
  await page.locator("body").click().catch(() => {});
  await page.keyboard.press("Escape").catch(() => {});
  await sleep(200);
  await page.locator(`[data-cut-clip="${cap.id}"]`).click({ button: "right" }).catch(() => {});
  await sleep(250);
  await page.locator('[data-cut-ctx="caption-edit"]').click().catch(() => {});
  await sleep(350);
  await openRightTab(page, "properties").catch(() => {});
  const contextMenuOk = (await page.locator("[data-cut-caption-edit-text]").count()) > 0;
  return { pass: selectionOk && edited && contextMenuOk, detail: `selection editor+text=${selectionOk}; edit→state=${edited}; menu→editor=${contextMenuOk}` };
}

// Poll project.state until clip `clipId`'s asset SWAPS away from `assetBefore`.
// The render-backed verbs (title.update / shape.update) re-render a transparent
// overlay .mov before swapping the clip's asset — that takes several seconds, and
// MORE on a slow engine. A fixed sleep RACES it (an 1800ms
// wait failed on a 4-5s render even though the product path worked). Polling the
// real INTENDED effect (the asset id changing) is robust regardless of render time.
// Returns the updated clip once swapped, or null on timeout.
async function waitForAssetSwap(clipId, assetBefore, timeoutMs = 25000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const s = await state();
    for (const t of s.tracks || []) {
      const c = (t.clips || []).find((c) => c.id === clipId);
      if (c && c.asset !== assetBefore) return c;
    }
    await sleep(400);
  }
  return null;
}

async function checkTitleEditInspector(page) {
  // title-editing regression: a placed title is a MEDIA clip (its pixels are a
  // pre-rendered transparent overlay .mov), so selecting one gave a generic "Video clip"
  // inspector with NO way to change its words. Now a title selects to a per-title TEXT
  // editor. It proves selection shows the current text (seeded from
  // the title_text annotation); (edit) change+Save → title.update RE-RENDERS the overlay
  // and SWAPS the clip's asset IN PLACE, and the new text is reflected in state — the
  // INTENDED effect (asset id changes; clip id/track stay).
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(150);
  const orig = "IVTITLE_" + Math.random().toString(36).slice(2, 6).toUpperCase();
  const add = await verb("title.add", { text: orig, range_ms: [0, 3000] });
  if (!add.ok) return { pass: false, detail: `title.add failed: ${add.error?.message || add.error?.code}` };
  await sleep(1200); // the title renders a transparent overlay .mov
  const s0 = await state();
  const ttrack = (s0.tracks || []).find((t) => (t.id || "").startsWith("title"));
  const tclip = ttrack?.clips?.find((c) => c.title_text === orig);
  if (!tclip) return { pass: false, detail: "title clip not in state / no title_text annotation" };
  const assetBefore = tclip.asset;
  // Select the title clip → title editor with its current text.
  await page.locator(`[data-cut-clip="${tclip.id}"]`).click({ force: true }).catch(() => {});
  await sleep(400);
  await openRightTab(page, "properties").catch(() => {});
  const kind = await page.locator('[data-cut-panel="inspector"]').getAttribute("data-cut-inspector-kind").catch(() => null);
  const ta = page.locator("[data-cut-title-edit-text]");
  const hasEditor = (await ta.count()) > 0;
  const shownText = hasEditor ? await ta.inputValue().catch(() => "") : "";
  const editorOk = kind === "title" && hasEditor && shownText === orig;
  // The edit: change text + Save → title.update → state asset SWAPPED + new title_text.
  let edited = false;
  let titleReqs = 0;
  let titleClickError = "";
  if (hasEditor) {
    const next = orig + "_X";
    await ta.fill(next);
    const save = page.locator('[data-cut-action="title-save-text"]');
    const onReq = (req) => { if (isVerbTraffic(req, "title.update")) titleReqs++; };
    page.on("request", onReq);
    await save.scrollIntoViewIfNeeded().catch(() => {});
    await save.click({ timeout: 8000 }).catch((e) => { titleClickError = String(e.message || e).slice(0, 220).replace(/\s+/g, " "); });
    // Render-backed: poll until the asset swaps rather than a fixed sleep that races
    // a slow render. See waitForAssetSwap.
    const t1 = await waitForAssetSwap(tclip.id, assetBefore, 45000);
    page.off("request", onReq);
    edited = !!t1 && t1.title_text === next && t1.asset !== assetBefore;
  }
  return { pass: editorOk && edited, detail: `editor+text=${editorOk}; edit→state(asset swap+text)=${edited}; reqs=${titleReqs}${titleClickError ? ` clickErr=${titleClickError}` : ""}` };
}

async function checkShapeEditInspector(page) {
  // Shape-editing regression: a placed shape
  // (edit.add_shape) is a MEDIA clip whose pixels are a pre-rendered transparent
  // overlay .mov on a `title*` track — like a title — so selecting one gave a generic
  // "Video clip" inspector with NO way to change it. Now a shape selects to a per-shape
  // editor (type / label / color). It proves selection shows the current
  // current label (seeded from the shape_label annotation) and inspector kind "shape";
  // (edit) change label+color + Save → shape.update RE-RENDERS the overlay and SWAPS the
  // clip's asset IN PLACE, and the new label is reflected in state — the INTENDED effect
  // (asset id changes; clip id/track stay). Also asserts a shape clip is NOT mistaken for
  // a title (no title_text marker → it routes to the shape editor, not the title editor).
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(150);
  const orig = "IVSHAPE_" + Math.random().toString(36).slice(2, 6).toUpperCase();
  const add = await verb("edit.add_shape", { shape: "rect", fill: "#FF0000", text: orig, range_ms: [0, 3000] });
  if (!add.ok) return { pass: false, detail: `edit.add_shape failed: ${add.error?.message || add.error?.code}` };
  await sleep(1200); // the shape renders a transparent overlay .mov
  const s0 = await state();
  // The shape clip carries shape_kind/shape_label and NOT title_text.
  let sclip = null;
  for (const t of s0.tracks || []) {
    if (!(t.id || "").startsWith("title")) continue;
    const c = (t.clips || []).find((c) => c.shape_kind === "rect" && c.shape_label === orig);
    if (c) { sclip = c; break; }
  }
  if (!sclip) return { pass: false, detail: "shape clip not in state / no shape_kind+shape_label annotation" };
  if (sclip.title_text != null) return { pass: false, detail: "shape clip wrongly carries title_text (would shadow shape editor)" };
  const assetBefore = sclip.asset;
  // Select the shape clip → shape editor with its current label + kind "shape".
  await page.locator(`[data-cut-clip="${sclip.id}"]`).click({ force: true }).catch(() => {});
  await sleep(400);
  await openRightTab(page, "properties").catch(() => {});
  const kind = await page.locator('[data-cut-panel="inspector"]').getAttribute("data-cut-inspector-kind").catch(() => null);
  const labelInput = page.locator("[data-cut-shape-edit-label]");
  const hasEditor = (await labelInput.count()) > 0;
  const shownLabel = hasEditor ? await labelInput.inputValue().catch(() => "") : "";
  const editorOk = kind === "shape" && hasEditor && shownLabel === orig;
  // The edit: change label + color + Save → shape.update → state asset SWAPPED + new label.
  let edited = false;
  let shapeReqs = 0;
  let shapeClickError = "";
  if (hasEditor) {
    const next = orig + "_X";
    await labelInput.fill(next);
    await page.locator("[data-cut-shape-edit-color]").fill("#00FF00").catch(() => {});
    const save = page.locator('[data-cut-action="shape-save"]');
    const onReq = (req) => { if (isVerbTraffic(req, "shape.update")) shapeReqs++; };
    page.on("request", onReq);
    await save.scrollIntoViewIfNeeded().catch(() => {});
    await save.click({ timeout: 8000 }).catch((e) => { shapeClickError = String(e.message || e).slice(0, 220).replace(/\s+/g, " "); });
    // Render-backed: poll until the asset swaps (see waitForAssetSwap).
    const c1 = await waitForAssetSwap(sclip.id, assetBefore, 45000);
    page.off("request", onReq);
    edited = !!c1 && c1.shape_label === next && c1.asset !== assetBefore;
  }
  return { pass: editorOk && edited, detail: `editor+label=${editorOk}; edit→state(asset swap+label)=${edited}; reqs=${shapeReqs}${shapeClickError ? ` clickErr=${shapeClickError}` : ""}` };
}

async function checkMarkerDeleteAndSeek(page) {
  // Markers could be added and dragged but also need visible delete and seek paths.
  // These actions use edit.remove_marker / edit.seek_marker
  // exist. This drives the new UI affordances and asserts the INTENDED effect:
  //  (a) right-click a marker → "Delete marker" → it leaves project.markers;
  //  (b) click a marker → the playhead seeks to its at_ms (left style moves).
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(200);
  // --- (a) DELETE: add a plain marker, right-click it, choose Delete, assert gone.
  const delLabel = "IVDEL_" + Math.random().toString(36).slice(2, 6).toUpperCase();
  await verb("edit.add_marker", { at_ms: 1200, label: delLabel });
  await sleep(500);
  const before = await state();
  const delId = (before.markers || []).find((m) => m.label === delLabel)?.id;
  if (!delId) return { pass: false, detail: "add_marker did not land in project.markers" };
  const tri = page.locator(`[data-cut-marker="${delId}"]`);
  if (!(await tri.count())) return { pass: false, detail: `marker ${delId} not rendered on the ruler` };
  await tri.click({ button: "right" });
  await sleep(250);
  const menuOpen = await vis(page, "[data-cut-marker-menu]");
  if (!menuOpen) return { pass: false, detail: "marker context menu did not open on right-click" };
  await page.locator('[data-cut-marker-ctx="delete"]').click();
  await sleep(600);
  const afterDel = await state();
  const stillThere = (afterDel.markers || []).some((m) => m.id === delId);
  if (stillThere) return { pass: false, detail: `marker ${delId} still in project.markers after Delete` };

  // --- (b) SEEK: add a marker far down the timeline, park the playhead at 0,
  // click the marker, assert the imperative playhead element MOVED right.
  const seekLabel = "IVSEEK_" + Math.random().toString(36).slice(2, 6).toUpperCase();
  await verb("edit.add_marker", { at_ms: 3500, label: seekLabel });
  await sleep(500);
  const s2 = await state();
  const seekId = (s2.markers || []).find((m) => m.label === seekLabel)?.id;
  if (!seekId) return { pass: false, detail: "seek marker did not land" };
  // Park the playhead at the very start by clicking the ruler near its left edge.
  const ruler = page.locator("[data-cut-ruler]");
  const rb = await ruler.boundingBox();
  if (rb) { await page.mouse.click(rb.x + 90, rb.y + rb.height - 3); await sleep(250); }
  const leftBefore = await page.locator("[data-cut-playhead]").evaluate((el) => parseFloat(el.style.left) || 0);
  await page.locator(`[data-cut-marker="${seekId}"]`).click();
  await sleep(400);
  const leftAfter = await page.locator("[data-cut-playhead]").evaluate((el) => parseFloat(el.style.left) || 0);
  const moved = leftAfter - leftBefore >= 8; // marker @3.5s sits well right of 0
  // Clean up the seek marker so re-runs stay tidy.
  await verb("edit.remove_marker", { id: seekId });
  return {
    pass: moved,
    detail: `delete: ${delLabel} gone=${!stillThere}; seek: playhead left ${leftBefore.toFixed(0)}→${leftAfter.toFixed(0)}px (moved=${moved})`,
  };
}

async function checkSttModelSelector(page) {
  // Settings>Environment perception card carries a real STT model
  // selector (system.set_stt_model). The engine ships three first-class choices:
  // Parakeet v3 default, Canary-1B-v2 + MMS_FA for weak-language word timestamps,
  // and Whisper large-v3 fallback. Open the environment drawer, assert all three
  // options exist, switch to Canary → a /api/verb/system.set_stt_model fires →
  // doctor reports the new active model, then reset to default (leave clean).
  let setReqs = 0;
  const onReq = (req) => { if (isVerbTraffic(req, "system.set_stt_model")) setReqs++; };
  page.on("request", onReq);
  try {
    // Open Settings>Environment via the status-bar environment chip.
    await closeActiveDrawers(page);
    await sleep(150);
    const envClick = await clickOrCenterHit(page, page.locator("[data-cut-env-chip]").first(), 5000);
    await page.locator("[data-cut-environment]").waitFor({ timeout: 5000 }).catch(() => {});
    await page.locator('[data-cut-settings-category="ai-transcription"]').click().catch(() => {});
    await sleep(600);
    const sel = page.locator("[data-cut-env-stt-model]");
    if (!(await sel.count())) {
      // The perception card may render the read-only custom path if an advanced model
      // is set; close + report honestly rather than failing on a non-default machine.
      const custom = await page.locator("[data-cut-env-stt-custom]").count();
      const envOpen = await page.locator("[data-cut-environment]").count();
      await page.keyboard.press("Escape").catch(() => {});
      return {
        pass: custom > 0,
        detail: custom > 0
          ? `STT shows custom model read-only (no recommended switch); envClick=${envClick.mode}`
          : `no STT control on perception card; envOpen=${envOpen} envClick=${envClick.mode}${envClick.err ? ` clickErr=${envClick.err}` : ""}${clickDiagSummary(envClick)}`,
      };
    }
    const opts = await sel.locator("option").evaluateAll((els) => els.map((e) => e.value));
    const hasModels =
      opts.includes("nemo-parakeet-tdt-0.6b-v3") &&
      opts.includes("nemo-canary-1b-v2") &&
      opts.includes("whisperx-large-v3");
    await sel.selectOption("nemo-canary-1b-v2");
    // The UI onChange posts system.set_stt_model {model} (async) → re-fetches doctor.
    // Poll a FRESH doctor (the engine rescans on set) until the active model flips.
    let active = null;
    for (let i = 0; i < 8; i++) {
      await sleep(300);
      active = await verb("system.doctor", { refresh: true }).then((r) => (r.result?.cards || []).find((c) => c.kind === "perception")?.details?.stt_model);
      if (active === "nemo-canary-1b-v2") break;
    }
    const switched = active === "nemo-canary-1b-v2";
    // leave clean: reset to the default model + close the drawer.
    await verb("system.set_stt_model", { clear: true });
    await page.keyboard.press("Escape").catch(() => {});
    return { pass: hasModels && switched && setReqs >= 1, detail: `models=${opts.join(",")} switchedTo=${active} setReqs=${setReqs} envClick=${envClick.mode}` };
  } finally {
    page.off("request", onReq);
  }
}

async function checkTranscriptHonestEmpty(page) {
  // The transcript pane is never blank: setup card / pending / words / guidance.
  await page.getByText("Transcript", { exact: true }).first().click();
  await sleep(800);
  const any =
    (await cnt(page, "[data-cut-perception-setup]")) +
    (await cnt(page, "[data-cut-transcribe-pending]")) +
    (await cnt(page, "[data-word-idx]")) +
    (await cnt(page, "[data-cut-timeline-empty], .tx__empty"));
  return { pass: any > 0, detail: `non-blank markers=${any}` };
}

async function checkCrossProjectFrameIntegrity() {
  // Two projects with DIFFERENT content must not share cached frames.
  await verb("project.create", { name: "iv_a_" + Math.random().toString(36).slice(2, 6), settings: { width: 1280, height: 720, fps: 30 } });
  await verb("media.import", { path: CLIP });
  await sleep(1200);
  const fa = await frame(1000);
  await verb("project.create", { name: "iv_b_" + Math.random().toString(36).slice(2, 6), settings: { width: 1280, height: 720, fps: 30 } });
  await verb("media.import", { path: CLIP2 });
  await sleep(1200);
  const fb = await frame(1000);
  const s = fa && fb ? ssim(fa, fb) : null;
  // distinct content → must differ clearly; a leak shows up as ~1.0
  return { pass: s != null && s < 0.95, detail: `projA-vs-projB frame SSIM ${s?.toFixed(4)} (≥0.95 ⇒ leak)` };
}

async function checkDefaultViewIsEditor(page) {
  // A prior session left in a sub-mode must relaunch into the editor, not the mixer.
  await page.evaluate(() => {
    const raw = localStorage.getItem("cut.layout.v1");
    const o = raw ? JSON.parse(raw) : {};
    o.workspaceMode = "audio";
    localStorage.setItem("cut.layout.v1", JSON.stringify(o));
  });
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(1000);
  const mode = await page.locator("[data-cut-modes]").getAttribute("data-cut-modes").catch(() => null);
  const mixerOpen = await vis(page, '[data-cut-panel="mixer"],[data-cut-drawer="mixer"]');
  return { pass: mode === "edit" && !mixerOpen, detail: `launched mode=${mode} mixerOpen=${mixerOpen}` };
}

async function checkEffectChipPreviewsComposed(page) {
  // The Windows full-test found color/filter CHIP effects (sepia/invert/…) applied to the
  // engine + export but did NOT flip the live preview to composed — clicking gave no visual
  // feedback. Fix: an effect chip now dispatches cut:show-composed (like grade/title). Assert
  // the chip flips composed ON *and* the composed frame actually changes (proves both the
  // preview flip and that the effect applies). Force composed OFF first so the flip is real.
  await freshProject(page, "effectchip");
  // Re-select a clip (mode toggles drop the selection) so the Inspector shows the effect chips.
  await page.locator("[data-cut-clip]").first().click().catch(() => {});
  await sleep(400);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "video-effects"))) {
    return { pass: false, detail: "Effects & compositing section did not expand" };
  }
  const chip = page.locator('[data-cut-inspector-effect="invert"]');
  if (!(await chip.count())) return { pass: false, detail: "no invert effect chip (clip not selected?)" };
  // ensure composed starts OFF
  const cur = await page.locator("[data-cut-composed]").getAttribute("data-cut-composed").catch(() => null);
  if (cur === "true") { await page.locator("[data-cut-composed]").click(); await sleep(250); }
  const before = await page.locator("[data-cut-composed]").getAttribute("data-cut-composed").catch(() => null);
  const f0 = await frame(1500);
  await chip.click();
  await sleep(900);
  const after = await page.locator("[data-cut-composed]").getAttribute("data-cut-composed").catch(() => null);
  const f1 = await frame(1500);
  const s = f0 && f1 ? ssim(f0, f1) : null;
  await chip.click().catch(() => {}); // toggle invert back off → leave clean
  await sleep(300);
  return {
    pass: before === "false" && after === "true" && s != null && s < 0.99,
    detail: `composed ${before}→${after}; invert frame SSIM ${s?.toFixed(4)} (<0.99 ⇒ applied+previewed)`,
  };
}

async function checkGradeSliderNoHang(page) {
  // Color-slider remount regression: the grade Slider was defined INSIDE
  // GradeDrawer's render → a fresh component identity every onChange → React remounted the
  // <input>, dropping focus/the drag mid-interaction so the slider couldn't be moved (felt
  // frozen). Now module-level. Proof: focus Contrast, press ArrowRight 8×; the value MUST
  // advance ~8 steps (×0.01 = 0.08 since the 0.6.106 precision fix; ≥0.06 tolerates a
  // couple of dropped presses). With the remount bug focus is lost after step 1 (~0.01).
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(200);
  await page.locator("[data-cut-clip]").first().click().catch(() => {});
  await sleep(300);
  // Grade lives in the right-sidebar Color tab.
  await openRightTab(page, "color");
  const input = page.locator('[data-cut-grade-input="contrast"]');
  if (!(await input.count())) return { pass: false, detail: "grade Color tab / contrast slider not open" };
  const readVal = async () => parseFloat(await page.locator('[data-cut-grade-val="contrast"]').first().innerText());
  const v0 = await readVal();
  await input.focus();
  for (let i = 0; i < 8; i++) { await page.keyboard.press("ArrowRight"); await sleep(40); }
  const v1 = await readVal();
  await openRightTab(page, "properties").catch(() => {});
  await sleep(200);
  return {
    pass: v1 - v0 >= 0.06,
    detail: `contrast ${v0}→${v1} after 8×ArrowRight (≥0.06 at 0.01/step ⇒ focus survived renders — no remount/hang)`,
  };
}

async function checkTimelineMoveNoListenerLeak(page) {
  // Moving a clip several times must not hang. Window 'mousemove' handlers once leaked
  // across drag gestures — endGesture (a stable empty-dep closure) removed the FIRST-render
  // onWinMove, but beginGesture added the CURRENT one (recreated whenever zoom/scroll changed
  // clientXToMs), so the live handler was never removed and PILED UP → every later mousemove fired
  // N handlers → freeze. Instrument the REAL set of window mousemove listeners, drag 5× with a zoom
  // between each (forces handler recreation so the bug WOULD manifest); the idle set must NOT grow.
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(200);
  await page.evaluate(() => {
    const w = window;
    if (!w.__mmset) {
      w.__mmset = new Set();
      const add = w.addEventListener.bind(w), rem = w.removeEventListener.bind(w);
      w.addEventListener = (t, f, o) => { if (t === "mousemove") w.__mmset.add(f); return add(t, f, o); };
      w.removeEventListener = (t, f, o) => { if (t === "mousemove") w.__mmset.delete(f); return rem(t, f, o); };
    }
  });
  const sizes = [];
  for (let i = 0; i < 5; i++) {
    const box = await page.locator("[data-cut-clip]").first().boundingBox();
    if (!box) return { pass: false, detail: "no clip to drag" };
    const y = box.y + box.height / 2;
    await page.mouse.move(box.x + 15, y);
    await page.mouse.down();
    await page.mouse.move(box.x + 55, y, { steps: 5 });
    await page.mouse.move(box.x + 95, y, { steps: 5 });
    await page.mouse.up();
    await sleep(220);
    await page.locator(i % 2 ? "[data-cut-zoom-out]" : "[data-cut-zoom-in]").click().catch(() => {});
    await sleep(150);
    sizes.push(await page.evaluate(() => window.__mmset.size));
  }
  const growth = sizes[sizes.length - 1] - sizes[0];
  return { pass: growth <= 0, detail: `idle mousemove listeners across 5 drags+zooms = [${sizes.join(",")}] (growth ${growth}; >0 ⇒ leak/hang)` };
}

// Human-control parity: each of these verbs also needs a working visible control.
// These run LATE (after the Settings/Find/cross-project checks have mutated the shared
// page state), so each starts from a FRESH project + a page reload — the clean editor
// state the controls need (selecting the audio clip, arming redact, the right tabs).
async function freshProject(page, tag) {
  await verb("project.create", { name: `iv_${tag}_` + Math.random().toString(36).slice(2, 6), settings: { width: 1280, height: 720, fps: 30 } });
  await verb("media.import", { path: CLIP });
  await sleep(1500);
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(1200);
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(400);
}

async function checkAudioCleanupVoice(page) {
  await freshProject(page, "cleanup");
  // audio.cleanup_voice is an ORCHESTRATOR — it records its sub-ops (edit.eq +
  // edit.effect on the audio clip), not its own op. Click "Clean voice" and assert
  // the chain landed on the selected audio clip.
  const s = await state();
  const audioClip = s.tracks.find((t) => t.kind === "audio")?.clips?.find((c) => c.asset)?.id;
  if (!audioClip) return { pass: false, detail: "no audio clip to clean" };
  const before = ((await verb("project.ops")).result?.ops || []).length;
  await page.locator(`[data-cut-clip="${audioClip}"]`).click().catch(() => {});
  await sleep(400);
  await openRightTab(page, "properties").catch(() => {});
  await sleep(300);
  if (!(await expandInspectorSection(page, "audio-cleanup"))) {
    return { pass: false, detail: "Voice cleanup section did not expand" };
  }
  await page.locator("[data-cut-inspector-cleanup-strength]").selectOption("strong").catch(() => {});
  const btn = page.locator('[data-cut-action="audio-cleanup-voice"]');
  if (!(await btn.count())) return { pass: false, detail: "no clean-voice button" };
  await btn.click();
  await sleep(1800);
  const newOps = ((await verb("project.ops")).result?.ops || []).slice(before);
  const hasEq = newOps.some((o) => o.verb === "edit.eq" && o.args?.clip === audioClip);
  const hasEffect = newOps.some((o) => o.verb === "edit.effect" && o.args?.clip === audioClip);
  return { pass: hasEq && hasEffect, detail: `eq=${hasEq} effect=${hasEffect} (+${newOps.length} ops on ${audioClip})` };
}

async function checkRedactDraw(page) {
  await freshProject(page, "redact");
  // edit.redact draw-region: arm draw mode, drag a box on the Preview stage, assert an
  // edit.redact op with rect points that are FRACTIONS in [0,1] matching the drag.
  const s = await state();
  const videoClip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (!videoClip) return { pass: false, detail: "no video clip" };
  await page.locator(`[data-cut-clip="${videoClip}"]`).click().catch(() => {});
  await sleep(400);
  await openRightTab(page, "properties").catch(() => {});
  await sleep(300);
  if (!(await expandInspectorSection(page, "video-privacy"))) {
    return { pass: false, detail: "Privacy & redaction section did not expand" };
  }
  await page.locator("[data-cut-inspector-redact-mode]").selectOption("pixelate").catch(() => {});
  await page.locator('[data-cut-action="redact-draw"]').click().catch(() => {});
  await sleep(300);
  if (!(await page.locator("[data-cut-redact-capture]").count())) return { pass: false, detail: "draw capture layer didn't arm" };
  const stage = await page.locator("[data-cut-stage]").boundingBox();
  if (!stage) return { pass: false, detail: "no stage box" };
  const before = ((await verb("project.ops")).result?.ops || []).length;
  const x0 = stage.x + stage.width * 0.25, y0 = stage.y + stage.height * 0.3;
  const x1 = stage.x + stage.width * 0.7, y1 = stage.y + stage.height * 0.8;
  await page.mouse.move(x0, y0); await page.mouse.down();
  await page.mouse.move((x0 + x1) / 2, (y0 + y1) / 2, { steps: 5 });
  await page.mouse.move(x1, y1, { steps: 5 }); await page.mouse.up();
  await sleep(800);
  const redact = ((await verb("project.ops")).result?.ops || []).slice(before).reverse().find((o) => o.verb === "edit.redact");
  const pts = redact?.args?.points;
  const inRange = Array.isArray(pts) && pts.length === 2 && pts.flat().every((v) => v >= 0 && v <= 1);
  return { pass: !!(redact && redact.args?.shape === "rect" && inRange), detail: `shape=${redact?.args?.shape} points=${JSON.stringify(pts)}` };
}

async function checkMixerLoudnessReadout(page) {
  // verify.loudness integrated-LUFS badge in the Audio (mixer) tab — the badge value
  // must reflect the verb's measured loudness.
  let resp = null;
  const onResp = async (r) => { if (isVerbTraffic(r, "verify.loudness")) { try { resp = await r.json(); } catch {} } };
  page.on("response", onResp);
  try {
    await freshProject(page, "loud");
    await page.locator("[data-cut-clip]").first().click().catch(() => {});
    await sleep(500);
    await openRightTab(page, "audio");
    await page.locator("[data-cut-mixer-loud-target-select]").selectOption("-14").catch(() => {});
    const btn = page.locator('[data-cut-action="verify-loudness"]:not([disabled])').first();
    if (!(await btn.count())) return { pass: false, detail: "no enabled Measure-loudness button" };
    await btn.click();
    let lufs = "";
    for (let i = 0; i < 24; i++) { await sleep(250); lufs = (await page.locator("[data-cut-loudness-lufs]").first().getAttribute("data-cut-loudness-lufs").catch(() => "")) || ""; if (lufs.trim()) break; }
    const numeric = lufs && !Number.isNaN(parseFloat(lufs));
    const measured = resp?.ok && typeof resp?.result?.integrated_lufs === "number";
    const matches = measured && Math.abs(parseFloat(lufs) - resp.result.integrated_lufs) < 0.15;
    await openRightTab(page, "properties").catch(() => {});
    return { pass: !!(numeric && measured && matches), detail: `badge=${lufs} verb=${resp?.result?.integrated_lufs} gap=${resp?.result?.gap_lu}` };
  } finally { page.off("response", onResp); }
}

async function checkLayerSlide(page) {
  // edit.slide animated-PiP slide: import a 2nd source as an overlay clip, open Layer,
  // apply a slide, assert pos_x/pos_y keyframes (>=2 points) land on the clip.
  await freshProject(page, "slide");
  const imp = await verb("media.import", { path: CLIP2 });
  const a2 = imp.result?.asset_id;
  await sleep(1500); // let the import probe finish (insert needs the duration)
  const at = await verb("edit.add_track", { kind: "video", rationale: "overlay" });
  const ovTrackId = at.result?.track_id || (await state()).tracks.filter((t) => t.kind === "video").pop()?.id;
  await verb("edit.insert", { asset: a2, track: ovTrackId, at_ms: 0 });
  // Poll for the inserted overlay clip (don't assume an id) so a slow probe can't flake it.
  let ovClip;
  for (let i = 0; i < 12; i++) {
    await sleep(400);
    const ot = (await state()).tracks.find((t) => t.id === ovTrackId);
    ovClip = (ot?.clips || []).find((c) => c.asset === a2)?.id;
    if (ovClip) break;
  }
  if (!ovClip) return { pass: false, detail: "overlay clip never landed" };
  await page.locator(`[data-cut-clip="${ovClip}"]`).waitFor({ timeout: 8000 }).catch(() => {});
  await page.locator(`[data-cut-clip="${ovClip}"]`).click({ timeout: 5000 }).catch(() => {}); await sleep(400);
  await page.locator('[data-cut-action="open-layer"]').click().catch(() => {}); await sleep(600);
  await page.locator("[data-cut-layer-slide-edge]").selectOption("top").catch(() => {});
  await page.locator("[data-cut-layer-slide-mode]").selectOption("in").catch(() => {});
  await page.locator('[data-cut-layer-input="slide_ms"]').fill("400").catch(() => {});
  await sleep(150);
  await page.locator('[data-cut-action="edit-slide"]').click().catch(() => {}); await sleep(900);
  const after = await state();
  const clip = (after.tracks.filter((t) => t.kind === "video").pop().clips || []).find((c) => c.id === ovClip);
  const kf = (clip?.keyframes || []).find((k) => k.param === "pos_x" || k.param === "pos_y");
  return { pass: !!(kf && (kf.points || []).length >= 2), detail: `param=${kf?.param} points=${JSON.stringify(kf?.points)}` };
}

async function checkRecordControlsRender(page) {
  // The Record surface must expose auto-polish, system-audio, mic-warm, and
  // export controls with safe defaults.
  await page.locator('[data-cut-mode="record"]').click().catch(() => {});
  await sleep(800);
  const autopolish = await cnt(page, "[data-cut-rec-autopolish-toggle]");
  const sysAudioOn = await page.locator("[data-cut-rec-system-audio-toggle] input").isChecked().catch(() => false);
  let micWarm = 0;
  for (let i = 0; i < 14; i++) { micWarm = await cnt(page, "[data-cut-rec-mic-warm]"); if (micWarm) break; await sleep(400); }
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(300);
  // System-audio defaults OFF because the native loopback can hold the
  // render device on a hung capture) — so assert the toggle is present + UNCHECKED by default.
  return { pass: autopolish > 0 && !sysAudioOn, detail: `autopolish=${autopolish} sysAudioDefaultOff=${!sysAudioOn} micWarmIndicator=${micWarm}` };
}

// ── runner ──────────────────────────────────────────────────────────────────
async function checkKeymapMarksAndZoom(page) {
  // I/O mark the export-range in/out at the
  // playhead (feeds the SAME [data-cut-range] band the ruler-drag paints, which
  // Preview turns into export.range), Shift+Z fits the whole timeline to the
  // window, and Ctrl/Cmd+= is the zoom-in alias. Pure UI wiring onto existing,
  // already-proven plumbing — this check guards the bindings from regressing.
  await closeActiveDrawers(page);
  await page.locator('[data-cut-left-tab="assets"]').click().catch(() => {});
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(250);
  const clip = page.locator("[data-cut-clip]").first();
  if (!(await clip.count())) return { pass: false, detail: "no clip" };
  // Seek the playhead into the timeline via a ruler click (a pure click seeks and
  // clears any prior range), then press I → a range from the playhead to the end.
  const ruler = page.locator("[data-cut-ruler]").first();
  const rb = await ruler.boundingBox();
  if (!rb) return { pass: false, detail: "no ruler" };
  await page.mouse.click(rb.x + rb.width * 0.4, rb.y + 6);
  await sleep(250);
  await page.keyboard.press("i");
  await sleep(250);
  const band = page.locator("[data-cut-range]").first();
  const markedIn = (await band.count()) > 0;
  const rangeAttr = markedIn ? await band.getAttribute("data-cut-range") : "none";
  // Zoom: Shift+Z fits (small clip), Ctrl+= twice grows it (alias works).
  await page.keyboard.press("Shift+Z");
  await sleep(250);
  const wFit = (await clip.boundingBox())?.width ?? 0;
  await page.keyboard.press("Control+=");
  await page.keyboard.press("Control+=");
  await sleep(250);
  const wZoom = (await clip.boundingBox())?.width ?? 0;
  const zoomAlias = wZoom > wFit + 1;
  const matteOpened = (await cnt(page, "[data-cut-matte]")) > 0;
  if (matteOpened) await closeActiveDrawers(page);
  return {
    pass: markedIn && zoomAlias && !matteOpened,
    detail: `range=${rangeAttr} fitW=${wFit.toFixed(0)} zoomW=${wZoom.toFixed(0)} zoomAlias=${zoomAlias} matteOpened=${matteOpened}`,
  };
}

async function checkUndoRedoRoundtrip(page) {
  // Visible undo/redo paths. The backend owns the multi-step
  // linear-history invariant; this UI check proves Ctrl+Z, Ctrl+Shift+Z, and the
  // Review rail Undo button dispatch the real verbs and converge in project.state.
  const focusTimeline = async () => {
    await page.locator("[data-cut-ruler]").first().click({ timeout: 2000 }).catch(() => {});
    await sleep(120);
  };
  const trace = (label) => { if (process.env.IV_TRACE_UNDO) console.log(`UNDO_TRACE ${label}`); };
  const mc = async () => (await state()).markers?.length ?? 0;
  trace("focus-initial:start");
  await focusTimeline();
  trace("focus-initial:done");
  const m0 = await mc();
  trace(`m0:${m0}`);
  // One deterministic forward edit.
  await verb("edit.add_marker", { at_ms: 800, label: "ur1" });
  trace("add-marker:1");
  await waitForState((st) => (st.markers?.length ?? 0) === m0 + 1, 10000);
  const m1 = await mc();
  trace(`m1:${m1}`);

  let keyUndoReqs = 0;
  const onKeyUndoReq = (req) => { if (isVerbTraffic(req, "project.undo")) keyUndoReqs++; };
  page.on("request", onKeyUndoReq);
  const undoKey = async (expected) => {
    trace("undo-key:focus:start");
    await focusTimeline();
    trace("undo-key:press:start");
    await page.keyboard.press("Control+z");
    trace("undo-key:press:done");
    await waitForState((st) => (st.markers?.length ?? 0) === expected, 10000);
    const count = await mc();
    trace(`undo-key:count:${count}`);
    return count;
  };
  let redoReqs = 0;
  const onRedoReq = (req) => { if (isVerbTraffic(req, "project.redo")) redoReqs++; };
  page.on("request", onRedoReq);
  const redoKey = async (expected) => {
    trace("redo-key:focus:start");
    await focusTimeline();
    trace("redo-key:press:start");
    await page.keyboard.press("Control+Shift+Z");
    trace("redo-key:press:done");
    await waitForState((st) => (st.markers?.length ?? 0) === expected, 10000);
    const count = await mc();
    trace(`redo-key:count:${count}`);
    return count;
  };
  const u1 = await undoKey(m0);
  page.off("request", onKeyUndoReq);
  const r1 = await redoKey(m0 + 1);
  page.off("request", onRedoReq);

  // Rail buttons present + reflect availability (we're at the tip → redo off).
  await releaseModifiers(page);
  trace("rail:pin:start");
  await closeActiveDrawers(page);
  await ensurePinnedRail(page);
  trace("rail:pin:done");
  const review = page.locator('[data-cut-panel="review"]').first();
  trace("rail:ops-click:start");
  await review.locator('[data-cut-tab="ops"]').click({ timeout: 2000 }).catch(() => {});
  trace("rail:ops-click:done");
  await sleep(250);
  const undoBtn = review.locator('[data-cut-action="undo"]').first();
  const redoBtn = review.locator('[data-cut-action="redo"]').first();
  await sleep(250);
  const haveButtons = (await undoBtn.count()) > 0 && (await redoBtn.count()) > 0;
  trace(`rail:buttons:${haveButtons}`);
  let undoEnabled = false;
  for (let i = 0; i < 20; i++) {
    undoEnabled = !(await undoBtn.isDisabled().catch(() => true));
    if (undoEnabled) break;
    await sleep(250);
  }
  const redoDisabledAtTip = await redoBtn.isDisabled().catch(() => false);
  trace(`rail:redo-off-tip:${redoDisabledAtTip}`);
  await sleep(500);
  // The rail Undo BUTTON also steps back (not just the keyboard).
  let undoReqs = 0;
  const onUndoReq = (req) => { if (isVerbTraffic(req, "project.undo")) undoReqs++; };
  page.on("request", onUndoReq);
  let undoClick = { ok: false, mode: "not-run" };
  trace("rail:undo-click:start");
  undoClick = await clickOrCenterHit(page, undoBtn, 8000);
  trace("rail:undo-click:done");
  await waitForState((st) => (st.markers?.length ?? 0) === m0, 10000);
  page.off("request", onUndoReq);
  const afterBtnUndo = await mc();
  trace(`rail:btn-undo-count:${afterBtnUndo}`);

  // A fresh edit after an undo clears the redo branch.
  await verb("edit.add_marker", { at_ms: 3200, label: "fresh" });
  trace("fresh-marker:done");
  await waitForState((st) => (st.markers?.length ?? 0) === m0 + 1, 10000);
  await sleep(300);
  const redoDisabledAfterFresh = await redoBtn.isDisabled().catch(() => false);
  trace(`rail:redo-off-fresh:${redoDisabledAfterFresh}`);

  const pass =
    m1 === m0 + 1 &&
    u1 === m0 &&
    r1 === m0 + 1 &&
    haveButtons && undoEnabled && redoDisabledAtTip && afterBtnUndo === m0 && redoDisabledAfterFresh;
  return {
    pass,
    detail: `m0=${m0} fwd=${m1} undoKey=${u1} redoKey=${r1} ` +
      `keyUndoReqs=${keyUndoReqs} redoReqs=${redoReqs} btns=${haveButtons} undoOn=${undoEnabled} undoReqs=${undoReqs} undoClick=${undoClick.mode}${undoClick.err ? ` clickErr=${undoClick.err}` : ""}${clickDiagSummary(undoClick)} ` +
      `redoOffTip=${redoDisabledAtTip} btnUndo=${afterBtnUndo} redoOffFresh=${redoDisabledAfterFresh}`,
  };
}

async function checkGroupedUndo(page) {
  // Two edits sharing a group_id are ONE undo step. Drive
  // two group-tagged edit.insert ops (the SAME meta-arg the linked A/V paste /
  // delete pass — now declared on the schema), then a SINGLE Ctrl+Z through the
  // real UI must remove BOTH inserted clips (the cursor steps over the whole
  // group), and one redo restores both. Sequential awaited verbs = deterministic
  // (no concurrent-op race), isolating the grouping behavior under test.
  const ac = async () => {
    const s = await state();
    return s.tracks.reduce((n, t) => n + (t.clips?.filter((c) => c.asset).length ?? 0), 0);
  };
  const s = await state();
  const vtrack = s.tracks.find((t) => t.kind === "video");
  const atrack = s.tracks.find((t) => t.kind === "audio");
  const asset = vtrack?.clips?.find((c) => c.asset)?.asset;
  if (!asset || !atrack) return { pass: false, detail: "no base asset / audio track" };
  const before = await ac();
  const gid = "iv-grp-" + Math.random().toString(36).slice(2, 8);
  // Two inserts tagged as ONE group (sequential → consecutive in the log).
  await verb("edit.insert", { asset, track: vtrack.id, at_ms: 0, group_id: gid });
  await verb("edit.insert", { asset, track: atrack.id, at_ms: 0, group_id: gid });
  await sleep(350);
  const afterInsert = await ac();
  // ONE Ctrl+Z must undo the WHOLE group (both clips).
  await page.locator("[data-cut-ruler]").first().click().catch(() => {});
  await page.keyboard.press("Control+z");
  await sleep(520);
  const afterUndo = await ac();
  // One redo re-applies the whole group.
  await page.locator("[data-cut-ruler]").first().click().catch(() => {});
  await page.keyboard.press("Control+Shift+Z");
  await sleep(520);
  const afterRedo = await ac();
  const pass = afterInsert === before + 2 && afterUndo === before && afterRedo === before + 2;
  return {
    pass,
    detail: `before=${before} grouped=${afterInsert} undo1=${afterUndo} redo1=${afterRedo} ` +
      `(2 group-tagged inserts → ONE Ctrl+Z removes both)`,
  };
}

async function checkAgentChatPanel(page) {
  // The agent chat box (right-rail Chat tab) renders with an empty state + a
  // working compose row (send disabled when empty, enabled after typing). The
  // END-TO-END turn (agent edits the timeline via MCP) is proven by the backend
  // live proof — spending a real subscription turn is not a gate concern.
  await openRightTab(page, "chat").catch(() => {});
  const panel = page.locator("[data-cut-chat]").first();
  const hasPanel = (await panel.count()) > 0 && (await panel.isVisible().catch(() => false));
  const empty = (await page.locator("[data-cut-chat-empty]").count()) > 0;
  const input = page.locator("[data-cut-chat-input]").first();
  const send = page.locator("[data-cut-chat-send]").first();
  const sendOffEmpty = await send.isDisabled().catch(() => true);
  await input.fill("add a marker at 2 seconds").catch(() => {});
  await sleep(150);
  const sendOnTyped = !(await send.isDisabled().catch(() => true));
  await input.fill("").catch(() => {}); // leave it clean for later checks
  // switch back to Properties so the rest of the rail checks see a normal tab.
  await openRightTab(page, "properties").catch(() => {});
  return {
    pass: hasPanel && empty && sendOffEmpty && sendOnTyped,
    detail: `panel=${hasPanel} empty=${empty} sendDisabledEmpty=${sendOffEmpty} sendEnabledTyped=${sendOnTyped}`,
  };
}

async function checkAssembleDrawerSurface(page) {
  // The Assemble (AI) drawer surfaces the assemble.* verbs.
  // Prove the SURFACE is wired: the topbar button opens the drawer, all three
  // modes render their own inputs, and Run fires the REAL verb + renders a result
  // OR an honest error. The fixture clip has no transcribed source, so the verb's
  // honest "is the source transcribed?" degradation is the expected, CORRECT
  // response; the verb logic itself is proven by its own backend tests.
  let repurposeFired = 0;
  const onReq = (req) => { if (isVerbTraffic(req, "assemble.repurpose")) repurposeFired++; };
  page.on("request", onReq);
  await page.locator("[data-cut-assemble-btn]").click().catch(() => {});
  await sleep(350);
  const drawer = page.locator("[data-cut-assemble]").first();
  const opened = (await drawer.count()) > 0 && (await drawer.isVisible().catch(() => false));
  const modes = await page.locator("[data-cut-assemble-mode-opt]").count();
  // default mode is "shorts" (the flagship) → its aspect select shows on open
  const aspectInput = (await page.locator("[data-cut-assemble-aspect]").count()) > 0;
  await page.locator('[data-cut-assemble-mode-opt="from_script"]').click().catch(() => {});
  await sleep(150);
  const scriptInput = (await page.locator("[data-cut-assemble-script]").count()) > 0;
  await page.locator('[data-cut-assemble-mode-opt="broll"]').click().catch(() => {});
  await sleep(150);
  const queryInput = (await page.locator("[data-cut-assemble-query]").count()) > 0;
  await page.locator('[data-cut-assemble-mode-opt="repurpose"]').click().catch(() => {});
  await sleep(150);
  await page.locator("[data-cut-assemble-run]").click().catch(() => {});
  await sleep(1600);
  const responded =
    (await page.locator("[data-cut-assemble-results]").count()) > 0 ||
    (await page.locator("[data-cut-assemble-error]").count()) > 0 ||
    (await page.locator("[data-cut-assemble-note]").count()) > 0;
  await page.locator("[data-cut-assemble-close]").click().catch(() => {});
  await sleep(150);
  page.off("request", onReq);
  return {
    pass: opened && modes === 4 && aspectInput && scriptInput && queryInput && repurposeFired > 0 && responded,
    detail: `opened=${opened} modes=${modes} aspect=${aspectInput} script=${scriptInput} query=${queryInput} verbFired=${repurposeFired} responded=${responded}`,
  };
}

async function checkScoreClipEngagement(page) {
  // The Inspector "Engagement" section surfaces score.clip
  // verb. Select a media clip on the Properties tab, click "Score this clip": the
  // verb must fire and render EITHER a numeric score OR an honest error (an
  // un-analyzed source → the verb's error, shown verbatim). Either proves the
  // surface is wired; the scoring logic is proven by score.clip's own tests.
  await freshProject(page, "score");
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (clip) { await page.locator(`[data-cut-clip="${clip}"]`).click().catch(() => {}); await sleep(300); }
  await openRightTab(page, "properties").catch(() => {});
  await sleep(200);
  if (!(await expandInspectorSection(page, "engagement"))) {
    return { pass: false, detail: "Short-form score section did not expand" };
  }
  const section = (await page.locator('[data-cut-inspector-group="engagement"]').count()) > 0;
  let scoreFired = 0;
  const onReq = (req) => { if (isVerbTraffic(req, "score.clip")) scoreFired++; };
  page.on("request", onReq);
  await page.locator('[data-cut-action="score-clip"]').click().catch(() => {});
  await sleep(1600);
  const scored = (await page.locator("[data-cut-inspector-score]").count()) > 0;
  const errored = (await page.locator("[data-cut-inspector-score-error]").count()) > 0;
  page.off("request", onReq);
  return {
    pass: section && scoreFired > 0 && (scored || errored),
    detail: `section=${section} verbFired=${scoreFired} scored=${scored} honestError=${errored}`,
  };
}

async function checkJudgeButtonRenders(page) {
  // The "Get AI review" button (verify.judge) in the Review→QC tab. We
  // assert it RENDERS + is enabled, but DO NOT click it: verify.judge spends a
  // real subscription-CLI turn, so the always-run gate must not fire it. The full
  // click→job→verdict flow is verified separately (the backend contract proof).
  await ensurePinnedRail(page);
  await page.locator('[data-cut-tab="qc"]').click().catch(() => {});
  await sleep(300);
  const btn = page.locator('[data-cut-action="judge-run"]').first();
  const present = (await btn.count()) > 0;
  const enabled = present ? !(await btn.isDisabled().catch(() => true)) : false;
  // leave the rail on the ops tab for any later checks.
  await page.locator('[data-cut-tab="ops"]').click().catch(() => {});
  return {
    pass: present && enabled,
    detail: `judgeButton=${present} enabled=${enabled}`,
  };
}

async function checkSyncByAudio(page) {
  // "Sync by audio" (edit.multicam_sync + edit.move) aligns 2+ selected
  // media clips. The base import yields a video clip + its LINKED audio clip (two
  // media clips). Ctrl-multi-select both, assert the toolbar button GATES
  // correctly (disabled with 1 media selected, enabled with 2) and firing it
  // calls multicam_sync + shows a status. video + its own audio are the same
  // source → offset 0 → "already aligned"; that still proves the measure→move
  // surface is wired (the verb fires + the compose runs + the honest note shows).
  const s = await state();
  const vid = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  const aud = s.tracks.find((t) => t.kind === "audio")?.clips?.find((c) => c.asset)?.id;
  if (!vid || !aud) return { pass: false, detail: `need a video+audio clip, got vid=${vid} aud=${aud}` };
  if (!(await openTimelineAutomation(page))) return { pass: false, detail: "Automate menu did not open" };
  const btn = page.locator('[data-cut-action="sync-by-audio"]').first();
  await page.locator(`[data-cut-clip="${vid}"]`).click().catch(() => {});
  await sleep(200);
  const disabled1 = await btn.isDisabled().catch(() => true);
  await page.locator(`[data-cut-clip="${aud}"]`).click({ modifiers: ["Control"] }).catch(() => {});
  await sleep(250);
  const enabled2 = !(await btn.isDisabled().catch(() => true));
  let syncFired = 0;
  const onReq = (req) => { if (isVerbTraffic(req, "edit.multicam_sync")) syncFired++; };
  page.on("request", onReq);
  await btn.click().catch(() => {});
  await sleep(2000);
  const noted = (await page.locator("[data-cut-sync-note]").count()) > 0;
  page.off("request", onReq);
  return {
    pass: disabled1 && enabled2 && syncFired > 0 && noted,
    detail: `disabledWith1=${disabled1} enabledWith2=${enabled2} verbFired=${syncFired} status=${noted}`,
  };
}

// Human-control surfacing checks
// Each surfaces an agent verb as a human control. They drive the
// real control and assert the INTENDED state effect (a grade lands, an audio clip
// appears, a speed_ramp field is set…). Render-backed effects are POLLED (not
// fixed-sleep) per the async UI regression. Verbs that depend on perception (auto_zoom
// needs loudness windows) tolerate the verb's honest error — like score.clip — so
// the check proves the SURFACE is wired even when the dev engine lacks STT/loudness.

// Poll project.state until `pred(state)` is truthy; returns the state or null.
async function waitForState(pred, timeoutMs = 20000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const s = await state();
    try { if (pred(s)) return s; } catch {}
    await sleep(400);
  }
  return null;
}
const flatClips = (s) => (s.tracks || []).flatMap((t) => (t.clips || []).map((c) => ({ ...c, _track: t.id, _kind: t.kind })));
const findClip = (s, id) => flatClips(s).find((c) => c.id === id);

async function checkAutoBalance(page) {
  // edit.auto_balance — one-click reference-free auto white-balance/exposure. Select a
  // video clip, Properties tab, click "Auto balance": the clip must gain a `grade`
  // (the verb derives + commits an edit.grade). The INTENDED effect, not just an op.
  await freshProject(page, "autobal");
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (!clip) return { pass: false, detail: "no video clip" };
  await page.locator(`[data-cut-clip="${clip}"]`).click().catch(() => {}); await sleep(300);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "video-color"))) {
    return { pass: false, detail: "Color section did not expand" };
  }
  const btn = page.locator('[data-cut-action="auto-balance"]');
  if (!(await btn.count())) return { pass: false, detail: "no auto-balance button" };
  await btn.click();
  const after = await waitForState((st) => !!findClip(st, clip)?.grade, 20000);
  return { pass: !!after, detail: `clip ${clip} gained a grade=${!!after}` };
}

async function checkColorMatch(page) {
  // edit.color_match — match a clip's colour to a REFERENCE clip. Proves: (gate) the
  // reference picker is DISABLED with only 1 video clip; (wire) after a 2nd distinct
  // video clip exists it can be picked and "Match" fires edit.color_match ok.
  await freshProject(page, "cmatch");
  const s0 = await state();
  const baseClip = s0.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (!baseClip) return { pass: false, detail: "no base video clip" };
  await page.locator(`[data-cut-clip="${baseClip}"]`).click().catch(() => {}); await sleep(300);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "video-color"))) {
    return { pass: false, detail: "Color section did not expand" };
  }
  const refSel = page.locator("[data-cut-colormatch-ref]");
  const disabled1 = await refSel.isDisabled().catch(() => false);
  // Add a DISTINCT 2nd video clip (silent_screen) on an overlay track.
  const imp = await verb("media.import", { path: CLIP2 });
  const a2 = imp.result?.asset_id;
  await sleep(1500);
  const at = await verb("edit.add_track", { kind: "video" });
  const ov = at.result?.track_id || (await state()).tracks.filter((t) => t.kind === "video").pop()?.id;
  await verb("edit.insert", { asset: a2, track: ov, at_ms: 0 });
  let refClip;
  for (let i = 0; i < 14; i++) { await sleep(400); refClip = (await state()).tracks.find((t) => t.id === ov)?.clips?.find((c) => c.asset)?.id; if (refClip) break; }
  if (!refClip) return { pass: false, detail: "reference clip never landed" };
  // Re-select the base clip (insert may have moved selection) + pick the reference.
  const baseClick = await clickOrCenterHit(page, page.locator(`[data-cut-clip="${baseClip}"]`));
  if (!baseClick.ok) return { pass: false, detail: `base clip not interactable:${clickDiagSummary(baseClick)}` };
  await sleep(300);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "video-color"))) {
    return { pass: false, detail: "Color section did not re-expand" };
  }
  await refSel.selectOption(refClip).catch(() => {});
  const matchBtn = page.locator('[data-cut-action="color-match"]');
  const enabled2 = !(await matchBtn.isDisabled().catch(() => true));
  // Capture the verb response if it lands in time, but assert the INTENDED EFFECT
  // (the base clip gains a `grade` — edit.color_match commits an edit.grade) by
  // POLLING state, not a fixed sleep: the first call decodes a mid-clip frame from
  // each side, which can exceed a fixed window under full-suite load (the
  // response-race flaked here). Mirrors checkAutoBalance's effect-poll. The base
  // clip starts ungraded (fresh import), so a grade appearing = the match landed.
  let resp = null;
  const onR = async (r) => { if (isVerbTraffic(r, "edit.color_match")) { try { resp = await r.json(); } catch {} } };
  page.on("response", onR);
  await matchBtn.click().catch(() => {});
  const graded = await waitForState((st) => !!findClip(st, baseClip)?.grade, 25000);
  page.off("response", onR);
  const effect = !!graded;
  return { pass: disabled1 && enabled2 && effect, detail: `gate(disabledWith1)=${disabled1} enabledWith2=${enabled2} matchGradeLanded=${effect} verbOk=${resp?.ok === true} identity=${resp?.result?.identity}` };
}

async function checkAutoZoom(page) {
  // edit.auto_zoom — emphasis-driven punch-in zooms. Like score.clip, the verb reads
  // perception (loudness/transcript): on an engine without that it honestly errors.
  // So we assert the verb FIRED and EITHER scale keyframes landed OR an honest note
  // shows — the surface is wired regardless of the dev engine's perception tier.
  await freshProject(page, "autozoom");
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (!clip) return { pass: false, detail: "no video clip" };
  await page.locator(`[data-cut-clip="${clip}"]`).click().catch(() => {}); await sleep(300);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "video-motion"))) {
    return { pass: false, detail: "Stabilization & auto zoom section did not expand" };
  }
  await page.locator("[data-cut-autozoom-intensity]").selectOption("0.2").catch(() => {});
  const btn = page.locator('[data-cut-action="auto-zoom"]');
  if (!(await btn.count())) return { pass: false, detail: "no auto-zoom button" };
  let fired = 0;
  const onReq = (req) => { if (isVerbTraffic(req, "edit.auto_zoom")) fired++; };
  page.on("request", onReq);
  await btn.click();
  await sleep(1800);
  page.off("request", onReq);
  const c = findClip(await state(), clip);
  const hasKf = (c?.keyframes || []).some((k) => k.param === "scale");
  const noted = (await page.locator("[data-cut-inspector-auto-note]").count()) > 0;
  return { pass: fired > 0 && (hasKf || noted), detail: `verbFired=${fired} scaleKeyframes=${hasKf} honestNote=${noted}` };
}

async function checkAdjustment(page) {
  // edit.adjustment — add an adjustment layer (a grade/effect over a time span on the
  // composite beneath). Pick the "Vignette" look, "Add over clip": a root-level
  // adjustment with a range_ms must appear in project.state (the INTENDED effect).
  await freshProject(page, "adjust");
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (!clip) return { pass: false, detail: "no video clip" };
  await page.locator(`[data-cut-clip="${clip}"]`).click().catch(() => {}); await sleep(300);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "video-color"))) {
    return { pass: false, detail: "Color section did not expand" };
  }
  await page.locator("[data-cut-adjustment-look]").selectOption("vignette").catch(() => {});
  const btn = page.locator('[data-cut-action="adjustment"]');
  if (!(await btn.count())) return { pass: false, detail: "no adjustment button" };
  await btn.click();
  const after = await waitForState((st) => (st.adjustments || []).some((a) => Array.isArray(a.range_ms)), 15000);
  const adj = (after?.adjustments || [])[0];
  return { pass: !!after, detail: `adjustment landed=${!!after} range=${JSON.stringify(adj?.range_ms)}` };
}

async function checkSpeedRamp(page) {
  // edit.speed_ramp — a variable-speed curve via presets. Pick
  // "slow → fast → slow", Apply ramp: the clip must gain a `speed_ramp` field in state
  // (the realized piecewise-speed curve), and Clear must remove it.
  await freshProject(page, "ramp");
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (!clip) return { pass: false, detail: "no video clip" };
  await page.locator(`[data-cut-clip="${clip}"]`).click().catch(() => {}); await sleep(300);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "speed"))) {
    return { pass: false, detail: "Speed / Retime section did not expand" };
  }
  const preset = page.locator("[data-cut-speed-ramp-preset]");
  if (!(await preset.count())) return { pass: false, detail: "no speed-ramp control (blocked?)" };
  await preset.selectOption("slow_fast_slow").catch(() => {});
  const rampBtn = page.locator('[data-cut-action="speed-ramp"]');
  const rampClick = await clickOrCenterHit(page, rampBtn);
  if (!rampClick.ok) return { pass: false, detail: `speed-ramp not interactable:${clickDiagSummary(rampClick)}` };
  const ramped = await waitForState((st) => !!findClip(st, clip)?.speed_ramp, 15000);
  // Clear it again so the clip is left clean using the same real pointer path.
  const clearClick = await clickOrCenterHit(page, page.locator('[data-cut-action="speed-ramp-clear"]'));
  if (!clearClick.ok) return { pass: false, detail: `speed-ramp clear not interactable:${clickDiagSummary(clearClick)}` };
  const cleared = await waitForState((st) => !findClip(st, clip)?.speed_ramp, 12000);
  return { pass: !!ramped && !!cleared, detail: `ramp set=${!!ramped} cleared=${!!cleared}` };
}

async function checkDetachAudio(page) {
  // edit.detach_audio — extract a video clip's audio onto its own track. Build the
  // EXTRACT case (not the no-op): a DISTINCT audio-bearing asset (insert_clip) placed
  // VIDEO-ONLY on an overlay track has no sibling audio clip → detach creates one.
  // Right-click the clip → "Detach audio" → a new audio clip must appear.
  await freshProject(page, "detach");
  const imp = await verb("media.import", { path: CLIP3 });
  const a2 = imp.result?.asset_id;
  await sleep(1500);
  const at = await verb("edit.add_track", { kind: "video" });
  const ov = at.result?.track_id || (await state()).tracks.filter((t) => t.kind === "video").pop()?.id;
  await verb("edit.insert", { asset: a2, track: ov, at_ms: 0 });
  let clip;
  for (let i = 0; i < 14; i++) { await sleep(400); clip = (await state()).tracks.find((t) => t.id === ov)?.clips?.find((c) => c.asset)?.id; if (clip) break; }
  if (!clip) return { pass: false, detail: "overlay clip never landed" };
  const audioBefore = flatClips(await state()).filter((c) => c._kind === "audio").length;
  await page.locator(`[data-cut-clip="${clip}"]`).waitFor({ timeout: 8000 }).catch(() => {});
  await page.locator(`[data-cut-clip="${clip}"]`).click({ button: "right", force: true }).catch(() => {});
  await sleep(300);
  const item = page.locator('[data-cut-ctx="detach-audio"]');
  if (!(await item.count())) return { pass: false, detail: "no Detach-audio menu item" };
  await item.click();
  const after = await waitForState((st) => flatClips(st).filter((c) => c._kind === "audio").length > audioBefore, 15000);
  return { pass: !!after, detail: `audio clips ${audioBefore}→${after ? flatClips(after).filter((c) => c._kind === "audio").length : "?"} (extracted=${!!after})` };
}

async function checkSplitEdit(page) {
  // edit.split_edit — J-cut / L-cut: roll the audio transition against a video cut.
  // Build the precondition (two butted video + two butted audio clips at a cut) by
  // splitting BOTH base tracks at 2000ms, then right-click the left video clip → J-cut.
  // The INTENDED effect: the outgoing audio clip's source out-point rolls earlier.
  await freshProject(page, "splitedit");
  const s0 = await state();
  const vTrack = s0.tracks.find((t) => t.kind === "video")?.id;
  const aTrack = s0.tracks.find((t) => t.kind === "audio")?.id;
  if (!vTrack || !aTrack) return { pass: false, detail: "missing base tracks" };
  await verb("edit.split", { track: vTrack, at_ms: 2000 });
  await verb("edit.split", { track: aTrack, at_ms: 2000 });
  await sleep(800);
  const s1 = await state();
  const leftVid = s1.tracks.find((t) => t.id === vTrack)?.clips?.find((c) => c.asset)?.id; // first video clip
  const firstAudio = s1.tracks.find((t) => t.id === aTrack)?.clips?.find((c) => c.asset);
  if (!leftVid || !firstAudio) return { pass: false, detail: "split didn't produce butted clips" };
  const audioOutBefore = firstAudio.src_out_ms;
  await page.locator(`[data-cut-clip="${leftVid}"]`).click({ button: "right", force: true }).catch(() => {});
  await sleep(300);
  const item = page.locator('[data-cut-ctx="split-edit-j"]');
  if (!(await item.count())) return { pass: false, detail: "no J-cut menu item (no video seam?)" };
  await item.click();
  const after = await waitForState((st) => {
    const a = st.tracks.find((t) => t.id === aTrack)?.clips?.find((c) => c.id === firstAudio.id);
    return a && a.src_out_ms !== audioOutBefore;
  }, 12000);
  return { pass: !!after, detail: `audio out-point rolled ${audioOutBefore}→${after ? findClip(after, firstAudio.id)?.src_out_ms : "?"}` };
}

async function checkCutToBeat(page) {
  // edit.cut_to_beat — split the video track on each music beat. Proves: (gate) the
  // "Cut to beat" toolbar button is DISABLED without beat markers; (wire) after beat
  // markers exist it enables and clicking splits the track (more video clips appear).
  await freshProject(page, "beat");
  if (!(await openTimelineAutomation(page))) return { pass: false, detail: "Automate menu did not open" };
  const btn = page.locator('[data-cut-action="cut-to-beat"]').first();
  const disabled1 = await btn.isDisabled().catch(() => false);
  // Add beat markers inside the clip content (audio.add_music surfaces these; here we
  // add them directly — cut_to_beat reads any marker labelled "beat").
  for (const ms of [1000, 2000, 3000, 4000]) await verb("edit.add_marker", { at_ms: ms, label: "beat" });
  // Poll until the marker op_applied propagates to the UI and the gate enables
  // (fixed-sleep here raced the op→UI update — async UI regression: poll, don't sleep).
  let enabled2 = false;
  for (let i = 0; i < 20; i++) { await sleep(400); enabled2 = !(await btn.isDisabled().catch(() => true)); if (enabled2) break; }
  const s0 = await state();
  const vTrack = s0.tracks.find((t) => t.kind === "video")?.id;
  const vidBefore = s0.tracks.find((t) => t.id === vTrack)?.clips?.filter((c) => c.asset).length || 0;
  let fired = 0;
  const onReq = (req) => { if (isVerbTraffic(req, "edit.cut_to_beat")) fired++; };
  page.on("request", onReq);
  await btn.click().catch(() => {});
  const after = await waitForState((st) => (st.tracks.find((t) => t.id === vTrack)?.clips?.filter((c) => c.asset).length || 0) > vidBefore, 15000);
  page.off("request", onReq);
  return { pass: disabled1 && enabled2 && fired > 0 && !!after, detail: `gate(disabledNoBeats)=${disabled1} enabledWithBeats=${enabled2} verbFired=${fired} clips ${vidBefore}→split=${!!after}` };
}

async function checkMulticamSwitch(page) {
  // edit.multicam_switch — cut a `program` track to the active speaker across ≥2 synced
  // angles. Proves: (gate) the "Auto multicam" button is DISABLED with 1 video track;
  // (wire) with a 2nd video track holding media it enables and firing builds a
  // `program` track. The active-speaker SWITCH reads each angle's perception
  // (Loudness.windows) — like auto_zoom, an engine without that honestly errors,
  // so we accept EITHER a program track OR an honest status note (the surface is
  // wired regardless of the dev engine's perception tier).
  await freshProject(page, "multicam");
  if (!(await openTimelineAutomation(page))) return { pass: false, detail: "Automate menu did not open" };
  const btn = page.locator('[data-cut-action="multicam-switch"]').first();
  const disabled1 = await btn.isDisabled().catch(() => false);
  const s0 = await state();
  const a1 = s0.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.asset;
  const at = await verb("edit.add_track", { kind: "video" });
  const ov = at.result?.track_id || (await state()).tracks.filter((t) => t.kind === "video").pop()?.id;
  await verb("edit.insert", { asset: a1, track: ov, at_ms: 0 });
  // Poll until the 2nd video track's insert propagates and the gate enables (poll,
  // don't fixed-sleep — the op→UI update can lag, async UI regression).
  let enabled2 = false;
  for (let i = 0; i < 20; i++) { await sleep(400); enabled2 = !(await btn.isDisabled().catch(() => true)); if (enabled2) break; }
  let fired = 0;
  const onReq = (req) => { if (isVerbTraffic(req, "edit.multicam_switch")) fired++; };
  page.on("request", onReq);
  await btn.click().catch(() => {});
  await sleep(1500);
  const noted = (await page.locator("[data-cut-sync-note]").count()) > 0;
  const after = await waitForState((st) => (st.tracks || []).some((t) => (t.id || "").startsWith("program")), 8000);
  page.off("request", onReq);
  return { pass: disabled1 && enabled2 && fired > 0 && (!!after || noted), detail: `gate(disabledWith1Track)=${disabled1} enabledWith2=${enabled2} verbFired=${fired} programTrack=${!!after} statusNote=${noted}` };
}

async function checkCaptionsTranslate(page) {
  // captions.translate / transcript.translate (no-selection Captions surface). Proves
  // RENDER + GATING + the language picker. We DELIBERATELY do NOT click "Translate
  // captions": it spends a real subscription-CLI turn (the same reason the judge check
  // doesn't click). The full click→translated-track path is proven live separately
  // (the CLI backend creates a target-language track, cues_translated>0). Here: the
  // button is DISABLED with no captions, ENABLED after a caption source exists, and the
  // transcript-translate sibling is correctly DISABLED with no transcript.
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {}); await sleep(150);
  await freshProject(page, "translate");
  await page.locator("body").click().catch(() => {});
  await page.keyboard.press("Escape").catch(() => {});
  await sleep(150);
  await openRightTab(page, "properties").catch(() => {});
  const capBtn = page.locator('[data-cut-action="translate-captions"]');
  if (!(await capBtn.count())) return { pass: false, detail: "no translate-captions control" };
  const disabledNoCaptions = await capBtn.isDisabled().catch(() => false);
  // Import a caption source (cap1) so the gate flips to enabled (no STT needed).
  const srt = join(tmp, "iv_translate.srt");
  writeFileSync(srt, "1\n00:00:00,000 --> 00:00:02,000\nHello world\n\n2\n00:00:02,000 --> 00:00:04,000\nA caption to translate\n");
  await verb("captions.import", { path: srt });
  await sleep(900);
  // Reload so the caption source + gates refresh deterministically, then reselect
  // nothing → Properties → the caption-translate gate should now be enabled.
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(1000);
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await page.locator("body").click().catch(() => {});
  await page.keyboard.press("Escape").catch(() => {});
  await openRightTab(page, "properties").catch(() => {});
  const enabledWithCaptions = !(await capBtn.isDisabled().catch(() => true));
  await page.locator("[data-cut-translate-lang]").selectOption("es").catch(() => {});
  const langVal = await page.locator("[data-cut-translate-lang]").inputValue().catch(() => "");
  // transcript.translate ride-along is surfaced in the same row. Its enabled/disabled
  // state reflects whether a transcript exists (correctly enabled WHEN one does), so we
  // assert it's PRESENT rather than hard-asserting a gate state that depends on perception.
  const transPresent = (await page.locator('[data-cut-action="translate-transcript"]').count()) > 0;
  return {
    pass: disabledNoCaptions && enabledWithCaptions && langVal === "es" && transPresent,
    detail: `gate(disabledNoCaptions)=${disabledNoCaptions} enabledWithCaptions=${enabledWithCaptions} lang=${langVal} transcriptRideAlongPresent=${transPresent}`,
  };
}

// ── COLOR surfacing — the new color verbs given Inspector controls.
// Each: open Inspector → Color management / Grade gallery / Layered grades / Power
// window → drive the real control → assert the INTENDED state effect.
// Project.color reflects in settings.color; clip tags in the clip.

async function checkColorManagement(page) {
  // project.color (working+output) + edit.color_space (clip input). Select a video
  // clip, Properties tab; set working→rec2020, output→srgb (project.settings.color),
  // and the clip input→srgb (clip.input_color_space). Each must land in state.
  await freshProject(page, "colormgmt");
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (!clip) return { pass: false, detail: "no video clip" };
  await page.locator(`[data-cut-clip="${clip}"]`).click().catch(() => {}); await sleep(300);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "video-color"))) {
    return { pass: false, detail: "Color section did not expand" };
  }
  const working = page.locator("[data-cut-color-working]");
  if (!(await working.count())) return { pass: false, detail: "no working-space selector" };
  await working.selectOption("rec2020").catch(() => {});
  const w = await waitForState((st) => st.settings?.color?.working === "rec2020", 12000);
  await page.locator("[data-cut-color-output]").selectOption("srgb").catch(() => {});
  const o = await waitForState((st) => st.settings?.color?.output === "srgb", 12000);
  await page.locator("[data-cut-color-input]").selectOption("srgb").catch(() => {});
  const i = await waitForState((st) => findClip(st, clip)?.input_color_space === "srgb", 12000);
  return {
    pass: !!w && !!o && !!i,
    detail: `working=${w?.settings?.color?.working} output=${o?.settings?.color?.output} clipInput=${i ? findClip(i, clip)?.input_color_space : "?"}`,
  };
}

async function checkGradeGallery(page) {
  // grade.save / grade.list / grade.apply. Give the clip a known grade (contrast
  // 1.4), Save look from the Inspector → it appears in grade.list. Change the grade
  // (contrast 0.7), then Apply the saved look back → the clip's contrast returns to
  // ~1.4 (the INTENDED copy-a-look effect, proven on the params not just an op).
  await freshProject(page, "gallery");
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (!clip) return { pass: false, detail: "no video clip" };
  await verb("edit.grade", { clip, contrast: 1.4 });
  await waitForState((st) => Math.abs((findClip(st, clip)?.grade?.contrast ?? 1) - 1.4) < 0.01, 10000);
  // Reload so the Inspector seeds the clip's grade (Save look gates on it).
  await page.reload({ waitUntil: "domcontentloaded" }); await sleep(1000);
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await page.locator(`[data-cut-clip="${clip}"]`).click().catch(() => {}); await sleep(300);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "video-color"))) {
    return { pass: false, detail: "Color section did not expand" };
  }
  const nameInput = page.locator("[data-cut-grade-save-name]");
  if (!(await nameInput.count())) return { pass: false, detail: "no save-name input" };
  const look = "iv_look_" + Math.random().toString(36).slice(2, 5);
  await nameInput.fill(look).catch(() => {});
  await page.locator('[data-cut-action="grade-save"]').click().catch(() => {});
  // grade.list (the read path the dropdown uses) must now carry the preset.
  let listed = false;
  for (let k = 0; k < 14; k++) { await sleep(400); const r = await verb("grade.list", {}); if ((r.result?.presets || []).some((p) => p.name === look)) { listed = true; break; } }
  // The dropdown option appears (UI reloaded the gallery after save).
  await page.locator(`[data-cut-grade-preset] option[value="${look}"]`).waitFor({ timeout: 6000 }).catch(() => {});
  // Change the clip's grade away from the saved look, then Apply the look back.
  await verb("edit.grade", { clip, contrast: 0.7 });
  await waitForState((st) => (findClip(st, clip)?.grade?.contrast ?? 1) < 0.8, 8000);
  await page.locator("[data-cut-grade-preset]").selectOption(look).catch(() => {});
  await page.locator('[data-cut-action="grade-apply"]').click().catch(() => {});
  const after = await waitForState((st) => Math.abs((findClip(st, clip)?.grade?.contrast ?? 1) - 1.4) < 0.05, 12000);
  return { pass: listed && !!after, detail: `saved=${listed} appliedBackContrast=${after ? findClip(after, clip)?.grade?.contrast : "?"}` };
}

async function checkGradeStack(page) {
  // edit.grade_stack — layered grading. Add a "Contrast +" layer (stack length 1),
  // add a "Warm" layer (length 2), then Remove the first row (length 1). The stack
  // length in project.state is the INTENDED effect at each step.
  await freshProject(page, "gradestack");
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (!clip) return { pass: false, detail: "no video clip" };
  await page.locator(`[data-cut-clip="${clip}"]`).click().catch(() => {}); await sleep(300);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "video-color"))) {
    return { pass: false, detail: "Color section did not expand" };
  }
  const addBtn = page.locator('[data-cut-action="grade-stack-add"]');
  if (!(await addBtn.count())) return { pass: false, detail: "no grade-stack add button" };
  await page.locator("[data-cut-grade-stack-layer]").selectOption("contrast").catch(() => {});
  await addBtn.click();
  const one = await waitForState((st) => (findClip(st, clip)?.grade_stack?.length ?? 0) >= 1, 12000);
  await page.locator("[data-cut-grade-stack-layer]").selectOption("warm").catch(() => {});
  await addBtn.click();
  const two = await waitForState((st) => (findClip(st, clip)?.grade_stack?.length ?? 0) >= 2, 12000);
  await page.locator('[data-cut-grade-stack-row] [data-cut-action="grade-stack-remove"]').first().click().catch(() => {});
  const removed = await waitForState((st) => (findClip(st, clip)?.grade_stack?.length ?? 0) === 1, 12000);
  return { pass: !!one && !!two && !!removed, detail: `len 0→1=${!!one} →2=${!!two} →remove→1=${!!removed}` };
}

async function checkPowerWindow(page) {
  // edit.grade_window — a geometric power window (region-scoped grade). Add a
  // "Center box"/"Brighten" window: it lands in clip.grade_windows with a rect of
  // two points AND a composed render frame visibly CHANGES (SSIM<1 — the region
  // actually affects pixels). Then add a 2nd window and Remove the first with one
  // atomic remove_index op → back to 1, proving real per-window removal.
  await freshProject(page, "powerwin");
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  if (!clip) return { pass: false, detail: "no video clip" };
  await page.locator(`[data-cut-clip="${clip}"]`).click().catch(() => {}); await sleep(300);
  await openRightTab(page, "properties").catch(() => {});
  if (!(await expandInspectorSection(page, "video-color"))) {
    return { pass: false, detail: "Color section did not expand" };
  }
  const addBtn = page.locator('[data-cut-action="grade-window-add"]');
  if (!(await addBtn.count())) return { pass: false, detail: "no grade-window add button" };
  await page.locator("[data-cut-grade-window-region]").selectOption("center").catch(() => {});
  await page.locator("[data-cut-grade-window-look]").selectOption("brighten").catch(() => {});
  // Render a composed frame BEFORE the window (for the pixel-delta proof).
  const before = await frame(500);
  await addBtn.click();
  const after1 = await waitForState((st) => (findClip(st, clip)?.grade_windows?.length ?? 0) >= 1, 15000);
  const win = findClip(after1 || (await state()), clip)?.grade_windows?.[0];
  const rectOk = win?.window?.shape === "rect" && Array.isArray(win?.window?.points) && win.window.points.length === 2;
  // Pixel-delta: the windowed region must change the composed frame. ffmpeg-less
  // environments return null → we don't fail on a missing tool, but a measured
  // NO-change fails (the window would be inert).
  const afterImg = await frame(500);
  const sv = before && afterImg ? ssim(before, afterImg) : null;
  const renderChanged = sv == null ? null : sv < 0.999;
  // Add a 2nd window, then remove the first atomically → length back to 1.
  await page.locator("[data-cut-grade-window-region]").selectOption("left").catch(() => {});
  await addBtn.click();
  const two = await waitForState((st) => (findClip(st, clip)?.grade_windows?.length ?? 0) >= 2, 15000);
  const opBeforeRemove = (await ops()).length;
  let removeRequests = 0;
  const onRemoveRequest = (req) => {
    if (isVerbTraffic(req, "edit.grade_window") && req.postDataJSON()?.remove_index === 0) removeRequests++;
  };
  page.on('request', onRemoveRequest);
  await page.locator('[data-cut-grade-window-row] [data-cut-action="grade-window-remove"]').first().click().catch(() => {});
  const removed = await waitForState((st) => (findClip(st, clip)?.grade_windows?.length ?? 0) === 1, 18000);
  page.off('request', onRemoveRequest);
  const removeOps = (await ops()).slice(opBeforeRemove).filter((o) => o.verb === 'edit.grade_window');
  const atomicRemove = removeRequests === 1 && removeOps.length === 1 && removeOps[0]?.args?.remove_index === 0;
  return {
    pass: !!after1 && rectOk && renderChanged !== false && !!two && !!removed && atomicRemove,
    detail: `added=${!!after1} rect2pts=${rectOk} ssim=${sv == null ? "n/a" : sv.toFixed(4)} renderChanged=${renderChanged} two=${!!two} remove→1=${!!removed} atomic=${atomicRemove} reqs=${removeRequests} ops=${removeOps.length}`,
  };
}

// ── Taxonomy direct-edit menu controls (Replace / Fit-to-fill / Nest) + the batch
//    render queue + the chat suggestion chips UI surfacing). Each opens
//    the REAL menu/modal, clicks the control, and asserts the verb landed + its
//    effect on project.state/output — the interaction-class proof (not just "a
//    button exists").

async function checkReplaceClip(page) {
  // edit.replace (right-click → Replace with… → pick asset). Import a 2nd asset so a
  // compatible SOURCE exists, then swap the base video clip's source for it. The clip
  // KEEPS its id (3-point replace), so the proof is its `asset` flipping to the new one.
  await freshProject(page, "replace");
  const imp = await verb("media.import", { path: CLIP2 });
  const a2 = imp.result?.asset_id;
  await sleep(1200);
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  const before = clip ? findClip(s, clip)?.asset : null;
  if (!clip || !a2) return { pass: false, detail: `clip=${clip} a2=${a2}` };
  await page.locator(`[data-cut-clip="${clip}"]`).click({ button: "right", force: true }).catch(() => {});
  await sleep(300);
  const btn = page.locator('[data-cut-ctx="replace"]');
  if (!(await btn.count())) return { pass: false, detail: "no Replace menu item" };
  if (await btn.isDisabled().catch(() => true)) return { pass: false, detail: "Replace disabled (no compatible source asset?)" };
  await btn.click(); // expand the inline asset picker
  await sleep(250);
  const opt = page.locator(`[data-cut-ctx-replace-asset="${a2}"]`);
  if (!(await opt.count())) return { pass: false, detail: "replacement asset not offered in picker" };
  await opt.click();
  const after = await waitForState((st) => findClip(st, clip)?.asset === a2, 15000);
  return { pass: !!after, detail: `clip ${clip} asset ${before}→${after ? findClip(after, clip)?.asset : "?"} (target=${a2})` };
}

async function checkFitToFill(page) {
  // edit.fit_to_fill (right-click → Fit to fill gap… → pick asset). Build a GAP next to
  // a clip (split the base video at 3s + 11s, LIFT the middle → an 8s gap), then fit a
  // 10s asset into it (speed 10/8=1.25×, in [0.25,4]×). Proof: a clip backed by the fill
  // asset lands on the video track (the empty slot is filled).
  await freshProject(page, "fitfill");
  const imp = await verb("media.import", { path: CLIP3 }); // 10s source
  const a2 = imp.result?.asset_id;
  await sleep(1200);
  const s0 = await state();
  const vTrack = s0.tracks.find((t) => t.kind === "video")?.id;
  if (!vTrack || !a2) return { pass: false, detail: `vTrack=${vTrack} a2=${a2}` };
  await verb("edit.split", { track: vTrack, at_ms: 3000 });
  await verb("edit.split", { track: vTrack, at_ms: 11000 });
  await sleep(600);
  await verb("edit.ripple_delete", { track: vTrack, range_ms: [3000, 11000], ripple: false }); // LIFT → 8s gap
  // wait for the gap to propagate to the UI (a clip ends at 3000 with a gap after it).
  await waitForState((st) => {
    const cs = st.tracks.find((t) => t.id === vTrack)?.clips || [];
    return cs.some((c) => c.kind === "gap");
  }, 10000);
  await sleep(600);
  const s1 = await state();
  const first = s1.tracks.find((t) => t.id === vTrack)?.clips?.find((c) => c.asset)?.id;
  if (!first) return { pass: false, detail: "no first clip after split/lift" };
  await page.locator(`[data-cut-clip="${first}"]`).click({ button: "right", force: true }).catch(() => {});
  await sleep(300);
  const btn = page.locator('[data-cut-ctx="fit-to-fill"]');
  if (!(await btn.count())) return { pass: false, detail: "no Fit-to-fill menu item" };
  if (await btn.isDisabled().catch(() => true)) return { pass: false, detail: "Fit-to-fill disabled (no adjacent gap detected in UI)" };
  await btn.click();
  await sleep(250);
  const opt = page.locator(`[data-cut-ctx-fit-asset="${a2}"]`);
  if (!(await opt.count())) return { pass: false, detail: "fill asset not offered in picker" };
  await opt.click();
  const filled = await waitForState((st) => flatClips(st).some((c) => c.asset === a2 && c._kind === "video"), 25000);
  return { pass: !!filled, detail: `gap filled by asset ${a2} = ${!!filled}` };
}

async function checkNestSelection(page) {
  // edit.nest (multi-select → right-click → Nest selection). Split the base video into
  // 2 clips, select BOTH, right-click within the selection (kept, NLE convention), Nest.
  // Proof: the 2-clip run collapses to ONE nest clip (its `nest` field set); curation
  // also proven — Nest is DISABLED before the multi-select, ENABLED after.
  await freshProject(page, "nest");
  const s0 = await state();
  const vTrack = s0.tracks.find((t) => t.kind === "video")?.id;
  if (!vTrack) return { pass: false, detail: "no video track" };
  await verb("edit.split", { track: vTrack, at_ms: 3000 });
  await sleep(700);
  const s1 = await state();
  const clips = (s1.tracks.find((t) => t.id === vTrack)?.clips?.filter((c) => c.asset).map((c) => c.id)) || [];
  if (clips.length < 2) return { pass: false, detail: `need 2 clips, have ${clips.length}` };
  const vidBefore = clips.length;
  // Gate (disabled) check: right-click ONE clip (single selection) → Nest disabled.
  await page.locator(`[data-cut-clip="${clips[0]}"]`).click().catch(() => {});
  await sleep(200);
  await page.locator(`[data-cut-clip="${clips[0]}"]`).click({ button: "right", force: true }).catch(() => {});
  await sleep(300);
  const disabledSingle = await page.locator('[data-cut-ctx="nest"]').isDisabled().catch(() => false);
  await page.keyboard.press("Escape").catch(() => {});
  await sleep(150);
  // Now multi-select both, right-click within → Nest enabled → fire it.
  await page.locator(`[data-cut-clip="${clips[0]}"]`).click().catch(() => {});
  await page.locator(`[data-cut-clip="${clips[1]}"]`).click({ modifiers: ["Control"] }).catch(() => {});
  await sleep(300);
  await page.locator(`[data-cut-clip="${clips[1]}"]`).click({ button: "right", force: true }).catch(() => {});
  await sleep(300);
  const btn = page.locator('[data-cut-ctx="nest"]');
  if (!(await btn.count())) return { pass: false, detail: "no Nest menu item" };
  const enabledMulti = !(await btn.isDisabled().catch(() => true));
  if (!enabledMulti) return { pass: false, detail: `Nest disabled despite 2 contiguous clips (gateSingle=${disabledSingle})` };
  await btn.click();
  const after = await waitForState((st) => {
    const media = st.tracks.find((t) => t.id === vTrack)?.clips?.filter((c) => c.asset) || [];
    return media.length < vidBefore && media.some((c) => c.nest);
  }, 15000);
  const post = after ? after.tracks.find((t) => t.id === vTrack)?.clips?.filter((c) => c.asset) : [];
  return { pass: !!after && disabledSingle && enabledMulti, detail: `gate(single disabled)=${disabledSingle} enabled(multi)=${enabledMulti} mediaClips ${vidBefore}→${post.length} nestClip=${post.some((c) => c.nest)}` };
}

async function checkRenderQueue(page) {
  // render.queue (Export ▾ → Render queue / batch deliver…). Shorten the timeline so 2
  // draft renders are fast, open the modal (it seeds 2 rows), set both to draft·project,
  // submit, and poll the QUEUE JOB to a terminal state. Proof: render.queue fired with 2
  // jobs AND the queue job reaches done (it actually rendered both deliveries).
  await freshProject(page, "renderq");
  const s0 = await state();
  const vTrack = s0.tracks.find((t) => t.kind === "video")?.id;
  const aTrack = s0.tracks.find((t) => t.kind === "audio")?.id;
  // trim to ~1.5s so each render is quick (ripple the tail closed on both base tracks).
  if (vTrack) await verb("edit.ripple_delete", { track: vTrack, range_ms: [1500, 999000], ripple: true });
  if (aTrack) await verb("edit.ripple_delete", { track: aTrack, range_ms: [1500, 999000], ripple: true });
  await sleep(800);
  let queueResp = null;
  const onResp = async (r) => { if (isVerbTraffic(r, "render.queue")) { try { queueResp = await r.json(); } catch {} } };
  page.on("response", onResp);
  try {
    await page.locator("[data-cut-export-btn]").click().catch(() => {});
    await sleep(350);
    const open = page.locator("[data-cut-render-queue-open]");
    if (!(await open.count())) return { pass: false, detail: "no Render-queue entry in Export menu" };
    await open.click();
    await sleep(350);
    const modal = page.locator("[data-cut-render-queue]");
    if (!((await modal.count()) > 0)) return { pass: false, detail: "render-queue modal did not open" };
    const rows0 = await page.locator("[data-cut-render-queue-row]").count();
    const pickers = await page.locator("[data-cut-render-queue-output-pick]").count();
    await page.locator('[data-cut-render-queue-output-pick="0"]').click().catch(() => {});
    await sleep(150);
    const pickerNote = await page.locator("[data-cut-render-queue-note]").textContent().catch(() => "");
    if (pickers < rows0) return { pass: false, detail: `render-queue output picker missing on some rows (${pickers}/${rows0})` };
    // normalize both rows to draft · project (fast, no reframe).
    await page.locator('[data-cut-render-queue-aspect="1"]').selectOption("project").catch(() => {});
    await page.locator('[data-cut-render-queue-preset="0"]').selectOption("draft").catch(() => {});
    await page.locator('[data-cut-render-queue-preset="1"]').selectOption("draft").catch(() => {});
    await page.locator("[data-cut-render-queue-start]").click().catch(() => {});
    // wait for the render.queue response (queue_id + count).
    for (let i = 0; i < 30 && !queueResp; i++) await sleep(300);
    const qid = queueResp?.result?.queue_id;
    const count = queueResp?.result?.count;
    if (!qid) return { pass: false, detail: `render.queue gave no queue_id (resp=${JSON.stringify(queueResp?.error || queueResp).slice(0, 120)})` };
    // poll the queue job to a terminal state (sequential renders of a ~1.5s draft).
    let final = null;
    for (let i = 0; i < 220; i++) { // ~220*700ms ≈ 154s budget
      const js = await verb("jobs.status", { job_id: qid });
      const st = js.result?.state;
      if (st === "done" || st === "failed") { final = js.result; break; }
      await sleep(700);
    }
    const doneShown = (await page.locator('[data-cut-render-queue-progress="done"]').count()) > 0;
    return {
      pass: count === 2 && final?.state === "done",
      detail: `rows=${rows0} pickers=${pickers} pickerNote=${pickerNote ? "shown" : "none"} queued=${count} queueId=${qid?.slice(0, 10)} finalState=${final?.state ?? "timeout"} modalDone=${doneShown}`,
    };
  } finally {
    page.off("response", onResp);
    // Clean up: close the queue modal so its overlay can't block later checks
    // (e.g. the chat-chips check, which clicks the right-rail Chat tab).
    await page.locator("[data-cut-render-queue-close]").click().catch(() => {});
    await page.keyboard.press("Escape").catch(() => {});
  }
}

async function checkChatChips(page) {
  // AgentChat suggestion chips — discoverability quick-prompts (dub / diarize /
  // repurpose). Proof: ≥3 chips exist and clicking one PRE-FILLS the compose input
  // (does NOT auto-send → no agent.chat turn spent). Runs in the base project (chips
  // are disabled without an open project; the base project is open here).
  await page.keyboard.press("Escape").catch(() => {}); // dismiss any stray overlay/menu
  await sleep(150);
  await openRightTab(page, "chat").catch(() => {});
  const chips = page.locator("[data-cut-chat-chip]");
  const n = await chips.count();
  const input = page.locator("[data-cut-chat-input]").first();
  await input.fill("").catch(() => {});
  const label = n ? await chips.first().getAttribute("data-cut-chat-chip") : null;
  let chatFired = 0;
  const onReq = (req) => { if (isVerbTraffic(req, "agent.chat")) chatFired++; };
  page.on("request", onReq);
  await chips.first().click().catch(() => {});
  await sleep(300);
  const val = (await input.inputValue().catch(() => "")) || "";
  page.off("request", onReq);
  await openRightTab(page, "properties").catch(() => {});
  return {
    pass: n >= 3 && val.trim().length > 0 && chatFired === 0,
    detail: `chips=${n} firstChip="${label}" inputFilled=${val.length > 0} noAutoSend=${chatFired === 0}`,
  };
}

async function main() {
  // Base project + a clip for the checks that need one.
  await verb("project.create", { name: "iv_base_" + Math.random().toString(36).slice(2, 6), settings: { width: 1280, height: 720, fps: 30 } });
  await verb("media.import", { path: CLIP });
  await sleep(1200);
  // select the imported clip so Color/Grade has a target.
  const s = await state();
  const clip = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;

  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  const errors = [];
  // Ignore SELF-HEALING preview-fetch transients (same class as the already-ignored
  // /api/frame, /api/source, /proxies): the preview-audio MONITOR (_monitor_a/_b.mp3)
  // and the per-track AUDITION stems (audio_<track>.wav) are rendered-then-fetched, so
  // under rapid automated editing a fetch can briefly race the (re)render → a transient
  // 404/416 that heals on the next render. The actual audio + export functionality is
  // proven by dedicated checks (verify-audio-layer, export-formats, surface-sweep's
  // wf-audio-monitor-loads) — these intermediate fetches are not deliverables. Real
  // export/render 404s (e.g. render_001.mp4, audio.mp3) still fail console-clean.
  const benignFetch = /favicon|\/api\/frame|\/filmstrip\/|\/proxies\/|\/api\/source\/|\/api\/export\/_monitor_|\/api\/export\/audio_[^./]+\.(wav|mp3)/;
  page.on("response", (r) => {
    if (r.status() >= 400 && !benignFetch.test(r.url())) errors.push(`HTTP ${r.status()} ${r.url().replace(/^https?:\/\/[^/]+/, "")}`);
  });
  await page.goto(APP, { waitUntil: "domcontentloaded" });
  await sleep(1200);
  if (clip) {
    await page.locator(`[data-cut-clip="${clip}"]`).click().catch(() => {});
    await sleep(400);
  }

  const results = [];
  // IV_ONLY=<substr> runs only the checks whose name contains <substr> (targeted
  // re-runs, e.g. IV_ONLY=auto-balance-control). IV_FROM/IV_TO run a contiguous
  // slice for order-dependent debugging. The base setup above still runs; all checks
  // are self-contained (each calls freshProject), so the filters are safe.
  const only = process.env.IV_ONLY;
  const from = process.env.IV_FROM;
  const to = process.env.IV_TO;
  let inRange = !from;
  const run = async (name, fn) => {
    if (from && name.includes(from)) inRange = true;
    const selected = (!only || name.includes(only)) && inRange;
    const stopAfter = to && name.includes(to);
    if (!selected) {
      if (stopAfter) inRange = false;
      return;
    }
    console.log(`RUN   ${name}`);
    try {
      const r = await fn(page);
      results.push({ name, ...r });
      console.log(`${r.pass ? "PASS" : "FAIL"}  ${name}  ${r.detail}`);
    } catch (e) {
      const detail = String(e.message || e).slice(0, 120);
      results.push({ name, pass: false, detail });
      console.log(`FAIL  ${name}  ${detail}`);
    }
    if (process.env.IV_TRACE_DRAWERS) {
      const drawers = await page.evaluate(() => ({
        matte: !!document.querySelector("[data-cut-matte]"),
        generate: !!document.querySelector("[data-cut-generate]"),
        assemble: !!document.querySelector("[data-cut-assemble]"),
        stock: !!document.querySelector("[data-cut-stock]"),
        search: !!document.querySelector("[data-cut-search]"),
      })).catch(() => null);
      if (drawers) console.log(`DRAWERS after ${name}: ${JSON.stringify(drawers)}`);
    }
    if (stopAfter) inRange = false;
  };

  await run("no-dead-export-mode", checkNoDeadExportMode);
  await run("export-menu-opens", checkExportMenuOpens);
  await run("otio-import-entry", checkOtioImportEntry);
  await run("right-tabs-switch", checkRightTabsSwitch);
  await run("agent-chat-panel-renders", checkAgentChatPanel);
  await run("assemble-drawer-surface", checkAssembleDrawerSurface);
  await run("score-clip-engagement", checkScoreClipEngagement);
  await run("judge-button-renders", checkJudgeButtonRenders);
  await run("sync-by-audio", checkSyncByAudio); // early: clean 2-clip timeline (video + linked audio)
  await run("color-drawer-reopen", checkColorReopen);
  await run("grade-slider-no-hang", checkGradeSliderNoHang);
  await run("find-permanent-left-sidebar", checkFindIsPermanentLeftSidebar);
  await run("assets-generate-launch", checkAssetsGenerateLaunch);
  await run("effect-chip-previews-composed", checkEffectChipPreviewsComposed);
  await run("timeline-move-no-listener-leak", checkTimelineMoveNoListenerLeak);
  await run("transcript-honest-empty", checkTranscriptHonestEmpty);
  await run("caption-text-card", checkCaptionTextCard);
  await run("caption-edit-inspector", checkCaptionEditInspector);
  await run("title-edit-inspector", checkTitleEditInspector);
  await run("shape-edit-inspector", checkShapeEditInspector);
  await run("marker-delete-and-seek", checkMarkerDeleteAndSeek);
  await run("keymap-marks-and-zoom", checkKeymapMarksAndZoom);
  await run("undo-redo-roundtrip", checkUndoRedoRoundtrip);
  await run("grouped-undo", checkGroupedUndo);
  await run("stt-model-selector", checkSttModelSelector);
  // engine→UI gap wirings + Record cluster (run in the base project, before
  // the project-switch checks; slide adds an overlay so keep it after the base-clip ones).
  await run("audio-cleanup-voice", checkAudioCleanupVoice);
  // Agent verbs that also have human controls.
  await run("auto-balance-control", checkAutoBalance);
  await run("color-match-control", checkColorMatch);
  await run("auto-zoom-control", checkAutoZoom);
  await run("adjustment-control", checkAdjustment);
  await run("speed-ramp-control", checkSpeedRamp);
  await run("detach-audio-control", checkDetachAudio);
  await run("split-edit-control", checkSplitEdit);
  await run("cut-to-beat-control", checkCutToBeat);
  await run("multicam-switch-control", checkMulticamSwitch);
  await run("captions-translate-control", checkCaptionsTranslate);
  // taxonomy direct-edit menu controls + batch render queue + chat chips.
  await run("clip-menu-replace", checkReplaceClip);
  await run("clip-menu-fit-to-fill", checkFitToFill);
  await run("clip-menu-nest", checkNestSelection);
  await run("render-queue-batch", checkRenderQueue);
  await run("agent-chat-chips", checkChatChips);
  // COLOR surfacing — Inspector › Color management / Grade gallery /
  // Layered grades / Power window, each wired to its color verb.
  await run("color-management", checkColorManagement);
  await run("grade-gallery", checkGradeGallery);
  await run("grade-stack", checkGradeStack);
  await run("power-window", checkPowerWindow);
  await run("redact-draw", checkRedactDraw);
  await run("mixer-loudness-readout", checkMixerLoudnessReadout);
  await run("layer-slide", checkLayerSlide);
  await run("record-controls-render", checkRecordControlsRender);

  await run("cross-project-frame-integrity", () => checkCrossProjectFrameIntegrity());
  await run("default-view-is-editor", checkDefaultViewIsEditor); // reloads — keep last
  // console cleanliness across the whole run (interaction class, not background races).
  results.push({ name: "console-clean", pass: errors.length === 0, detail: errors.length ? errors.slice(0, 4).join(" | ") : "0 errors" });

  await browser.close();

  let fail = 0;
  console.log("\n== INTERACTION VERIFY ==");
  for (const r of results) {
    console.log(`  ${r.pass ? "PASS" : "FAIL"}  ${r.name.padEnd(30)} ${r.detail}`);
    if (!r.pass) fail++;
  }
  const pass = results.length - fail;
  console.log(`\n${pass} PASS, ${fail} FAIL  (${results.length} checks)`);
  process.exit(fail ? 1 : 0);
}
main().catch((e) => {
  console.error(e);
  process.exit(2);
});
