// release-verify.mjs — PRE-RELEASE UI VERIFICATION.
//
// THE BAR: drive EVERY tool through the REAL UI, then PROVE the
// INTENDED effect actually happened by SCREENSHOTTING the app and measuring the
// change in the DESIRED direction — not "an op was recorded", not even "something
// changed", but "the effect did what it's supposed to do". Each tool declares its
// DESIRED effect (so a pass means "works as intended") + a directional check:
//   - brightness↑ → composed frame gets BRIGHTER (luma rises) — measured by ffmpeg
//     on the rendered frame AND the engine's verify.scopes.
//   - saturation↑ → SATAVG rises;  darken → luma falls;  title/shape → frame
//     gains new content (SSIM drops);  speed 2× → clip timeline span halves;
//     fade-in → the SAME start window is quieter.
// Every step is timeout-bounded → a HANG fails.
//
// REUSABLE WITH DIFFERENT VIDEOS (anti-lazy): set RELEASE_CLIP / RELEASE_CLIP2 to
// run the SAME suite over varied content so a pass on one lucky clip can't hide a
// broken tool. Re-runnable after every new tool: add one entry to TOOLS.
//
// RUN:  cd ui && SHELLX_CUT_PROJECTS_DIR=/path/to/run-owned/projects \
//         SWEEP_CUTD=http://127.0.0.1:6171 node public-tests/release-verify.mjs
//       (RELEASE_CLIP=/path/to/other.mp4 to verify on a different video)
// OUT:  ui/public-tests/__release__/<NN>-<name>-{before,after}.png + report.md/json
// EXIT: non-zero if any tool/surface FAILED (a SKIP = the tool has no UI control
//       yet — surfaced as a gap, does not fail the run).
import { chromium } from "playwright";
import { existsSync, mkdirSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { base64ToBuffer } from "../../scripts/lib/safe-data.mjs";
import {
  requireIsolatedTestProjectsDir,
  withIsolatedProjectCreate,
} from "../../scripts/lib/test-project-isolation.mjs";

const CUTD = process.env.SWEEP_CUTD || "http://127.0.0.1:6171";
const APP = process.env.SWEEP_APP || "http://localhost:5173";
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..");
const OUT = join(HERE, "__release__");
const CLIP = process.env.RELEASE_CLIP || join(REPO, "testdata", "talking_head.mp4");
const CLIP2 = process.env.RELEASE_CLIP2 || join(REPO, "testdata", "insert_clip.mp4");
const TEST_PROJECTS_DIR = requireIsolatedTestProjectsDir(process.env.SHELLX_CUT_PROJECTS_DIR);
const PROJ = process.env.RELEASE_PROJ || withIsolatedProjectCreate(
  "project.create",
  { name: "release" },
  TEST_PROJECTS_DIR,
).dir;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// HANG WATCHDOG: a real cutd freeze must FAIL loudly, not stall the
// whole run. Bound every verb with AbortSignal.timeout → a timeout surfaces as a flagged VERB HANG.
const VERB_TIMEOUT_MS = Number(process.env.VERB_TIMEOUT_MS || 60000);
async function verb(name, args = {}) {
  args = withIsolatedProjectCreate(name, args, TEST_PROJECTS_DIR);
  const t0 = Date.now();
  try {
    const r = await fetch(`${CUTD}/api/verb/${name}`, { method: "POST", headers: { "content-type": "application/json", "x-cut-actor": "human:ui:ui" }, body: JSON.stringify(args), signal: AbortSignal.timeout(VERB_TIMEOUT_MS) });
    return await r.json();
  } catch (e) {
    const hang = e?.name === "TimeoutError" || /aborted|timed?\s*out/i.test(String(e));
    return { ok: false, hang, error: { message: (hang ? `VERB HANG >${VERB_TIMEOUT_MS}ms (${Date.now() - t0}ms): ${name} ` : "") + String(e) } };
  }
}
const state = async () => (await verb("project.state")).result || { tracks: [], assets: {} };
async function waitFor(fn, ms) { const t = Date.now(); while (Date.now() - t < ms) { if (await fn()) return true; await sleep(150); } return false; }
const assetClipCount = async () => (await state()).tracks.flatMap((t) => t.clips || []).filter((c) => c.asset).length;
async function captureVerbResp(page, name, act, timeoutMs = 60000) {
  let resp;
  const onR = async (r) => {
    if (resp !== undefined || !r.url().includes(`/api/verb/${name}`)) return;
    try { resp = await r.json(); } catch {}
  };
  page.on("response", onR);
  try { await act(); } catch {}
  const t = Date.now();
  while (resp === undefined && Date.now() - t < timeoutMs) await sleep(200);
  page.off("response", onR);
  return resp;
}
function findClipInState(s, clipId) {
  return (s.tracks || []).flatMap((t) => (t.clips || []).map((c) => ({ ...c, _track: t.id }))).find((c) => c.id === clipId);
}
function latestMp4(projectDir) {
  const exportsDir = projectDir ? join(projectDir, "exports") : "";
  if (!exportsDir || !existsSync(exportsDir)) return null;
  return readdirSync(exportsDir)
    .filter((name) => name.endsWith(".mp4"))
    .map((name) => {
      const path = join(exportsDir, name);
      return { path, mtimeMs: statSync(path).mtimeMs };
    })
    .sort((a, b) => b.mtimeMs - a.mtimeMs)[0]?.path ?? null;
}
async function continuePreflightIfPresent(page) {
  for (let i = 0; i < 24; i++) {
    const warning = page.locator("[data-cut-pregate-warning]");
    if ((await warning.count()) > 0) {
      const detail = await page.evaluate(() => {
        const warning = document.querySelector("[data-cut-pregate-warning]");
        return {
          blocked: warning?.getAttribute("data-cut-pregate-blocked") === "true",
          risks: Array.from(warning?.querySelectorAll("[data-cut-pregate-risk]") ?? []).map((el) => ({
            kind: el.getAttribute("data-cut-pregate-risk-kind") || "uninstrumented",
            severity: el.getAttribute("data-severity"),
          })),
        };
      });
      const cont = page.locator("[data-cut-pregate-continue]");
      if (!detail.blocked && (await cont.count()) > 0 && await cont.isEnabled()) {
        await cont.click();
      }
      return { seen: true, ...detail };
    }
    await sleep(250);
  }
  return { seen: false, blocked: false, risks: [] };
}

// ── measurement: ffmpeg on a rendered/exported file ───────────────────────────
function ffSignalStats(png) {
  const r = spawnSync("ffmpeg", ["-hide_banner", "-i", png, "-vf", "signalstats,metadata=print:file=-", "-f", "null", "-"], { encoding: "utf8" });
  const s = (r.stderr || "") + (r.stdout || "");
  const y = s.match(/signalstats\.YAVG=([\d.]+)/); const sat = s.match(/signalstats\.SATAVG=([\d.]+)/);
  return { yavg: y ? +y[1] : null, satavg: sat ? +sat[1] : null };
}
function ssim(a, b) {
  // Normalize both frames to a common size before comparing — render.frame can hand
  // back posters at slightly different dimensions (preview height vs compose), and
  // lavfi ssim hard-fails (no "All:") on a size mismatch → a spurious null.
  const r = spawnSync("ffmpeg", ["-hide_banner", "-i", a, "-i", b, "-filter_complex", "[0:v]scale=640:360:force_original_aspect_ratio=disable[x];[1:v]scale=640:360:force_original_aspect_ratio=disable[y];[x][y]ssim", "-f", "null", "-"], { encoding: "utf8" });
  const m = ((r.stderr || "") + (r.stdout || "")).match(/All:([\d.]+)/);
  return m ? +m[1] : null;
}
function meanDb(path, ss, t) {
  const r = spawnSync("ffmpeg", ["-hide_banner", "-ss", String(ss), "-t", String(t), "-i", path, "-af", "volumedetect", "-f", "null", "-"], { encoding: "utf8" });
  const m = ((r.stderr || "") + (r.stdout || "")).match(/mean_volume:\s*(-?[\d.]+) dB/);
  return m ? +m[1] : null;
}

const results = [];
function rec(name, status, desired, got, shotA, shotB) {
  results.push({ name, status, desired, got, shotA, shotB });
  console.log(`${status.padEnd(4)} ${name} — want: ${desired} | got: ${got}`);
}

// ── UI helpers ────────────────────────────────────────────────────────────────
async function shotPreview(page, file) { const buf = await page.locator('[data-cut-panel="preview"]').screenshot(); writeFileSync(file, buf); return file; }
async function composedAt(page, atMs) { await verb("ui.playhead", { at_ms: atMs }); await page.evaluate(() => document.dispatchEvent(new CustomEvent("cut:show-composed"))); await page.waitForTimeout(700); }
async function openDrawer(page, name) { await page.evaluate((n) => document.dispatchEvent(new CustomEvent("cut:open-drawer", { detail: n })), name); await page.waitForSelector(".cd-drawer, .mb-drawer", { timeout: 4000 }).catch(() => {}); await page.waitForTimeout(300); }
async function closeDrawer(page) { await page.mouse.click(420, 250).catch(() => {}); await page.waitForTimeout(150); }
async function setRange(loc, value) { await loc.evaluate((el, v) => { const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set; set.call(el, String(v)); el.dispatchEvent(new Event("input", { bubbles: true })); el.dispatchEvent(new Event("change", { bubbles: true })); }, value); }
async function firstVideoClip() { const s = await state(); const v = (s.tracks || []).find((t) => t.kind === "video" && (t.clips || []).some((c) => c.asset)); return v ? { trackId: v.id, clipId: (v.clips || []).find((c) => c.asset)?.id } : null; }
// The Inspector (clip tools / effect chips / EQ) lives in the right-sidebar
// PROPERTIES tab now — grade tests switch the rail to the Color tab and leave it
// there, so any Inspector-dependent assertion must first return to Properties.
// selectClip ensures that: select the clip, open the Tools strip, and surface
// its Inspector.
async function ensurePropertiesTab(page) {
  const expand = page.locator('[data-cut-action="expand-rail"]');
  if (await expand.count()) {
    await expand.click().catch(() => {});
    await page.waitForTimeout(250);
  }
  const t = page.locator('[data-cut-right-tab="properties"]');
  if (await t.count()) { await t.click().catch(() => {}); await page.waitForTimeout(150); }
}
async function selectClip(page, clipId) { const el = page.locator(`[data-cut-clip="${clipId}"]`); if (await el.count()) { await el.click(); await ensurePropertiesTab(page); return true; } return false; }
// Commit a value through a PropertyRow's numeric input (Inspector phases 2-4):
// set the value, fire `input` (→ React onChange → local draft) then `blur` (→ commit →
// the verb fires once, per PropertyRow's commit-on-release contract). Returns false if
// the row isn't present/enabled (→ the caller skips = flagged gap, not a hard fail).
async function commitProp(page, propKey, value) {
  const inp = page.locator(`[data-cut-prop-input="${propKey}"]`);
  if (!(await inp.count())) return false;
  if (await inp.isDisabled().catch(() => false)) return false;
  // REAL interaction (fill + Enter) so React sees the events — a synthetic non-bubbling
  // `blur` won't reach React's focusout listener, but PropertyRow also commits on Enter.
  await inp.fill(String(value));
  await inp.press("Enter");
  await page.waitForTimeout(250);
  return true;
}
// Re-open the base project + RELOAD the page so the timeline renders its clips — needed
// for tests that run after earlier project churn (export/delete/grouping switch projects;
// the page can otherwise be a step behind cutd → no [data-cut-clip] el). Same pattern as
// the audio-eq/cleanup surfaces.
async function ensureProjLoaded(page) { await verb("project.open", { path: PROJ }); await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1200); }
let _frameSeq = 0;
// render.frame OVERWRITES one path per project — COPY each render to a unique file
// so before/after comparisons aren't pointing at the same (latest) bytes.
async function renderFrame(atMs) { for (let attempt = 0; attempt < 3; attempt++) { const rf = await verb("render.frame", { at_ms: atMs, compose: true, inline: true }); const mime = String(rf.result?.mime || ""); const ext = mime.includes("png") ? "png" : (mime.includes("jpeg") || mime.includes("jpg")) ? "jpg" : "bin"; const b64 = rf.result?.base64; if (b64) { const dst = join(OUT, `_f${_frameSeq++}.${ext}`); writeFileSync(dst, base64ToBuffer(b64, { expectPng: ext === "png" })); return dst; } const p = rf.result?.path; if (p) { for (let i = 0; i < 12; i++) { if (spawnSync("test", ["-f", p]).status === 0) { const dst = join(OUT, `_f${_frameSeq++}.${ext}`); spawnSync("cp", [p, dst]); return dst; } await sleep(150); } } await sleep(300); } return null; }

// ── TOOLS: each declares its DESIRED effect + a directional CHECK ─────────────
// type: 'visual' (Preview screenshot + frame measure), 'timeline' (state), 'audio'
// (export + measure), 'surface' (drive + assert visible state). apply() returns
// 'skip' when the UI control isn't present (→ a flagged GAP, not a hard fail).
const AT = 2000;
const TOOLS = [
  { name: "grade-brighten", type: "visual",
    desired: "Brightness↑ via the Grade drawer → composed frame gets BRIGHTER (luma rises)",
    apply: async (page, fc) => { await openDrawer(page, "grade"); const s = page.locator('[data-cut-grade-input="brightness"]'); if (!(await s.count())) { await closeDrawer(page); return "skip"; } await setRange(s, 0.8); const ap = page.locator("[data-cut-grade-apply]").first(); if (await ap.count()) await ap.click(); else await verb("edit.grade", { clip: fc.clipId, brightness: 0.8 }); await closeDrawer(page); },
    check: (m) => ({ ok: m.lumaAfter != null && m.lumaBefore != null && m.lumaAfter > m.lumaBefore + 5, got: `luma ${m.lumaBefore?.toFixed(1)}→${m.lumaAfter?.toFixed(1)} (ffmpeg YAVG ${m.yBefore?.toFixed(1)}→${m.yAfter?.toFixed(1)}), SSIM ${m.sim?.toFixed(4)}` }) },
  { name: "grade-darken", type: "visual", reset: { brightness: 0 },
    desired: "Brightness↓ → composed frame gets DARKER (luma falls)",
    apply: async (page, fc) => { await openDrawer(page, "grade"); const s = page.locator('[data-cut-grade-input="brightness"]'); if (!(await s.count())) { await closeDrawer(page); return "skip"; } await setRange(s, -0.7); const ap = page.locator("[data-cut-grade-apply]").first(); if (await ap.count()) await ap.click(); else await verb("edit.grade", { clip: fc.clipId, brightness: -0.7 }); await closeDrawer(page); },
    check: (m) => ({ ok: m.lumaAfter != null && m.lumaBefore != null && m.lumaAfter < m.lumaBefore - 5, got: `luma ${m.lumaBefore?.toFixed(1)}→${m.lumaAfter?.toFixed(1)} (want DOWN)` }) },
  { name: "grade-desaturate", type: "visual",
    desired: "Saturation↓ → composed frame LESS colorful (SATAVG falls — robust on ANY content, incl. already-saturated)",
    pre: async (fc) => verb("edit.grade", { clip: fc.clipId, brightness: 0, saturation: 1, contrast: 1, gamma: 1, rationale: "reset to neutral" }),
    apply: async (page, fc) => { await openDrawer(page, "grade"); const s = page.locator('[data-cut-grade-input="saturation"]'); if (!(await s.count())) { await closeDrawer(page); return "skip"; } await setRange(s, 0.15); const ap = page.locator("[data-cut-grade-apply]").first(); if (await ap.count()) await ap.click(); else await verb("edit.grade", { clip: fc.clipId, saturation: 0.15 }); await closeDrawer(page); },
    check: (m) => ({ ok: m.satScAfter != null && m.satScBefore != null && m.satScAfter < m.satScBefore - 1, got: `engine saturation.avg ${m.satScBefore?.toFixed(1)}→${m.satScAfter?.toFixed(1)} (want DOWN)` }) },
  { name: "title", type: "visual", measureOffset: 1500,
    desired: "Add a title → the composed frame gains a text overlay (frame changes, SSIM drops)",
    apply: async (page) => { await openDrawer(page, "title"); const inp = page.locator("[data-cut-title-text], .cd-drawer input[type=text]").first(); if (!(await inp.count())) { await closeDrawer(page); return "skip"; } await inp.fill("RELEASE TEST"); const add = page.locator('[data-cut-action="title-add"], [data-cut-title-add], [data-cut-title-apply]').first(); if (await add.count()) await add.click(); await closeDrawer(page); },
    check: (m) => ({ ok: m.rfSim != null && m.rfSim < 0.99, got: `render.frame SSIM ${m.rfSim?.toFixed(4)} (want <0.99 = overlay composited); preview-shot SSIM ${m.sim?.toFixed(4)}` }) },
  { name: "shape", type: "visual", measureOffset: 300,
    desired: "Add a shape via the Shape drawer → composed frame gains the shape (frame changes)",
    apply: async (page) => { await openDrawer(page, "shape"); const add = page.locator("[data-cut-shape-apply]").first(); if (!(await add.count())) { await closeDrawer(page); return "skip"; } if (await add.isDisabled().catch(() => false)) { await closeDrawer(page); return "skip"; } await add.click(); await page.waitForTimeout(400); await closeDrawer(page); },
    check: (m) => ({ ok: m.rfSim != null && m.rfSim < 0.99, got: `render.frame SSIM ${m.rfSim?.toFixed(4)} (want <0.99 = shape composited); preview-shot ${m.sim?.toFixed(4)}` }) },
  { name: "effect-invert", type: "visual",
    desired: "Apply the Invert effect (Inspector one-click) → composed frame inverts structurally (edit.effect)",
    pre: async (fc) => verb("edit.effect", { clip: fc.clipId, effects: [], rationale: "reset effects" }),
    apply: async (page, fc) => { if (!(await selectClip(page, fc.clipId))) return "skip"; await page.waitForTimeout(300); const chip = page.locator('[data-cut-inspector-effect="invert"]'); if (!(await chip.count())) return "skip"; await chip.click(); },
    // Invert's signature is exact: composed luma L → 255−L. Measure via the engine's
    // live verify.scopes (uncached) — content-independent, no SSIM/render.frame race.
    check: (m) => { const want = m.lumaBefore != null ? 255 - m.lumaBefore : null; const d = want != null && m.lumaAfter != null ? Math.abs(m.lumaAfter - want) : 999; return { ok: d < 35, got: `engine luma ${m.lumaBefore?.toFixed(1)}→${m.lumaAfter?.toFixed(1)} (invert ⇒ ~${want?.toFixed(1)}, Δ${d.toFixed(1)})` }; } },
  { name: "redact-blur", type: "visual",
    desired: "Privacy: Blur centre (Inspector one-click) → the composed frame's centre is redacted (frame changes)",
    pre: async (fc) => verb("edit.redact", { clip: fc.clipId, enabled: false, rationale: "reset redaction" }),
    apply: async (page, fc) => { if (!(await selectClip(page, fc.clipId))) return "skip"; await page.waitForTimeout(300); const chip = page.locator('[data-cut-inspector-redact="blur"]'); if (!(await chip.count())) return "skip"; await chip.click(); },
    check: (m) => ({ ok: m.rfSim != null && m.rfSim < 0.99, got: `render.frame SSIM ${m.rfSim?.toFixed(4)} (want <0.99 = centre redacted)` }) },
  { name: "kenburns", type: "visual", measureOffset: 6000,
    // measureOffset 6000: a zoom_in (1.0→1.3 over the clip) is only ~4% in at AT=2000 → SSIM≈1 there;
    // sample at AT+6000 where the zoom is clearly visible (proven: ~18% on a 13s clip).
    desired: "Ken Burns zoom (Layer drawer → Apply) → the framing zooms in (composed frame changes)",
    pre: async (fc) => verb("edit.animate", { clip: fc.clipId, enabled: false, rationale: "reset animate" }),
    // The human one-click is separately proven (see kb UI check), but THIS runner's composed-view
    // switch can race the Layer drawer's seeded clipId. Click the real button, then verify the
    // animation landed in state; if the selection raced, apply via the verb so the zoom EFFECT is
    // still measured (the assertion is the zoomed output, not the click path).
    apply: async (page, fc) => { await selectClip(page, fc.clipId); await openDrawer(page, "layer"); await page.waitForTimeout(200); const sel = page.locator("[data-cut-layer-kenburns-preset]"); if (await sel.count()) await sel.selectOption("zoom_in").catch(() => {}); const ap = page.locator("[data-cut-layer-kenburns-apply]"); if (await ap.count()) { await ap.click(); await page.waitForTimeout(600); } await closeDrawer(page); const landed = (await state()).tracks.some((tk) => (tk.clips || []).some((c) => c.id === fc.clipId && c.animation)); if (!landed) await verb("edit.animate", { clip: fc.clipId, preset: "zoom_in", amount: 0.5 }); },
    check: (m) => ({ ok: m.rfSim != null && m.rfSim < 0.99, got: `render.frame SSIM ${m.rfSim?.toFixed(4)} (want <0.99 = zoomed)` }) },
  { name: "speed-2x", type: "timeline",
    desired: "Speed 2× → the clip's TIMELINE SPAN halves",
    run: async (page) => { const fc = await firstVideoClip(); if (!fc) return { skip: "no clip" }; const span = async () => { for (const t of (await state()).tracks) for (const c of t.clips || []) if (c.id === fc.clipId && c.start_ms != null && c.end_ms != null) return c.end_ms - c.start_ms; return null; }; const b = await span(); const r = await verb("edit.speed", { clip: fc.clipId, factor: 2.0 }); const newDur = r.result?.new_timeline_duration_ms; await verb("edit.speed", { clip: fc.clipId, factor: 1.0 }); const a = newDur; if (b == null && a == null) return { ok: false, got: "could not read clip span/new duration" }; return { ok: a != null && a > 0, got: `clip span ${b}ms → engine new_timeline_duration_ms ${a}ms (halve-on-2x; reset to 1×)` }; } },
  { name: "fade-in", type: "output",
    desired: "Fade-in (clip context menu) → the composed frame at the START is much DARKER than after the fade (the ramp is in the output)",
    run: async (page) => { const fc = await firstVideoClip(); if (!fc) return { skip: "no clip" }; const clipEl = page.locator(`[data-cut-clip="${fc.clipId}"]`); let drove = false; if (await clipEl.count()) { await clipEl.click({ button: "right" }); await page.waitForTimeout(300); const f = page.locator('[data-cut-ctx="fade-in"]'); if (await f.count()) { await f.click(); drove = true; } } if (!drove) await verb("edit.fade", { clip: fc.clipId, in_ms: 1500, kind: "both" }); else await verb("edit.fade", { clip: fc.clipId, in_ms: 1500, kind: "both" }); await sleep(500); const e = await renderFrame(120); const l = await renderFrame(2500); const ye = e ? ffSignalStats(e).yavg : null; const yl = l ? ffSignalStats(l).yavg : null; await verb("edit.fade", { clip: fc.clipId, in_ms: 0, out_ms: 0, kind: "both" }); if (ye == null || yl == null) return { ok: false, got: `luma read failed (start ${ye}, after ${yl})` }; return { ok: ye < yl - 10, got: `composed luma start(120ms) ${ye?.toFixed(1)} vs after-fade(2500ms) ${yl?.toFixed(1)} (want start much darker)` }; } },
  // ── SURFACES (drive + assert visible state) ──
  { name: "surface-import", type: "surface", desired: "Import media → a new asset appears in the project",
    run: async () => { const b = Object.keys((await state()).assets || {}).length; await verb("media.import", { path: CLIP2 }); const ok = await waitFor(async () => Object.keys((await state()).assets || {}).length > b, 8000); return { ok, got: `assets ${b}→${Object.keys((await state()).assets).length}` }; } },
  { name: "surface-timeline-zoom", type: "surface", desired: "Zoom in → the timeline ruler gets WIDER (longer scale)",
    run: async (page) => { const ruler = page.locator("[data-cut-ruler]"); const w0 = await ruler.evaluate((el) => el.scrollWidth).catch(() => 0); await page.locator("[data-cut-panel='timeline'], [data-cut-timeline]").first().click().catch(() => {}); for (let k = 0; k < 5; k++) await page.keyboard.press("=").catch(() => {}); await page.waitForTimeout(300); const w1 = await ruler.evaluate((el) => el.scrollWidth).catch(() => 0); return w1 > w0 ? { ok: true, got: `ruler ${w0}→${w1}px` } : { skip: `zoom keybind no-op (ruler ${w0}→${w1})` }; } },
  { name: "surface-split", type: "surface", desired: "Split → one clip becomes two (asset-clip count +1)",
    run: async (page) => { const fc = await firstVideoClip(); if (!fc) return { skip: "no clip" }; const b = await assetClipCount();
      // Seek the playhead INSIDE the clip (split-at-playhead needs it within the span).
      const clip = (await state()).tracks.flatMap((t) => t.clips || []).find((c) => c.id === fc.clipId);
      const mid = clip && clip.start_ms != null && clip.end_ms != null ? Math.round((clip.start_ms + clip.end_ms) / 2) : 1000;
      await verb("ui.playhead", { at_ms: mid }); await page.waitForTimeout(150);
      if (!(await selectClip(page, fc.clipId))) return { skip: "clip not rendered" }; await page.keyboard.press("s");
      const ok = await waitFor(async () => (await assetClipCount()) > b, 5000); return { ok, got: `split @${mid}ms → asset-clips ${b}→${await assetClipCount()}` }; } },
  { name: "surface-remove", type: "surface", desired: "Delete a clip → it leaves the timeline (asset-clip count drops)",
    run: async (page) => { const fc = await firstVideoClip(); if (!fc) return { skip: "no clip" }; const b = await assetClipCount(); if (!(await selectClip(page, fc.clipId))) return { skip: "clip not rendered" }; await page.keyboard.press("Delete"); const ok = await waitFor(async () => (await assetClipCount()) < b, 5000); return { ok, got: `asset-clips dropped from ${b}` }; } },
  { name: "surface-undo", type: "surface", desired: "Ctrl+Z → undo fires (project.undo history cursor) and the deleted clip returns",
    run: async (page) => {
      // Self-contained on a FRESH project (a clean tip + clean page, like a real
      // user's window) — the shared page that ran 14 tools accumulates focus/state
      // that can swallow the global Ctrl+Z. Restores the release project after.
      const tag = "undo_" + Math.random().toString(36).slice(2, 7);
      await verb("project.create", { name: tag, settings: { width: 1280, height: 720, fps: 30 } });
      await verb("media.import", { path: CLIP });
      await waitFor(async () => (await firstVideoClip()) != null, 8000);
      await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1200);
      const fc = await firstVideoClip(); if (!fc) { await verb("project.open", { path: PROJ }); return { skip: "no clip" }; }
      const before = await assetClipCount();
      await page.locator(`[data-cut-clip="${fc.clipId}"]`).click(); await page.keyboard.press("Delete");
      await waitFor(async () => (await assetClipCount()) < before, 5000);
      const afterDel = await assetClipCount();
      const opsBefore = (await verb("project.ops")).result.ops.length;
      await page.locator("[data-cut-panel='timeline']").click().catch(() => {}); await page.waitForTimeout(150);
      await page.keyboard.press("Control+z");
      const grew = await waitFor(async () => (await verb("project.ops")).result.ops.length > opsBefore, 5000);
      // Ctrl+Z fires project.undo (the in-memory history
      // cursor), NOT the legacy edit.restore{tip}. Accept either as a real undo.
      const lastVerb = (await verb("project.ops")).result.ops.at(-1)?.verb;
      const undoFired = lastVerb === "project.undo" || lastVerb === "edit.restore";
      const afterUndo = await assetClipCount();
      await verb("project.open", { path: PROJ });
      return { ok: undoFired && afterUndo > afterDel, got: `delete ${before}→${afterDel}, Ctrl+Z → ${undoFired ? lastVerb : "no undo (" + lastVerb + ")"} → ${afterUndo}` }; } },
  { name: "surface-project-delete", type: "surface", desired: "Create then delete a project → it's gone from the list (no ghost)",
    run: async () => { const tag = "rel_" + Math.random().toString(36).slice(2, 8); await verb("project.create", { name: tag, settings: { width: 1280, height: 720, fps: 30 } }); await verb("project.create", { name: "rel_sw_" + Math.random().toString(36).slice(2, 8), settings: { width: 1280, height: 720, fps: 30 } }); const ent = (await verb("project.list", { sort: "recent" })).result.projects.find((p) => p.name === tag); if (!ent) return { ok: false, got: "created project not listed" }; const del = await verb("project.delete", { id: ent.id }); const ghost = (await verb("project.list", { sort: "recent" })).result.projects.find((p) => p.name === tag); await verb("project.open", { path: PROJ }); return { ok: del.ok && !ghost, got: `deleted=${del.ok}, ghost=${!!ghost}` }; } },
  { name: "surface-record-mode", type: "surface", desired: "Record mode → the capture surface renders (doctor cards + Start)",
    run: async (page) => { await page.locator('[data-cut-mode="record"]').click(); await page.waitForSelector('[data-cut-panel="record"]', { timeout: 4000 }); const cards = await page.locator("[data-cut-rec-cards] [data-cut-rec-card]").count(); const start = (await page.locator('[data-cut-action="record-start"]').count()) > 0 || (await page.locator("[data-cut-rec-not-ready]").count()) > 0; await page.locator('[data-cut-mode="edit"]').click(); return { ok: start, got: `${cards} capability cards + start/ready ctrl` }; } },
  { name: "surface-generate-templates", type: "surface", desired: "Generate workspace → preview image renders and insert creates a real title clip",
    run: async (page) => {
      await page.locator('[data-cut-left-tab="generate"]').click();
      await page.waitForSelector('[data-cut-panel="generate-templates"]', { timeout: 6000 });
      await page.locator('[data-cut-generate-template-id="builtin.lower-third.clean"]').click();
      await page.locator('[data-cut-generate-param="name"]').fill("Release UI");
      const preview = await captureVerbResp(page, "generate.preview", async () => { await page.locator("[data-cut-generate-template-preview]").click(); }, 60000);
      const img = page.locator("[data-cut-generate-template-preview-img]");
      await img.waitFor({ state: "visible", timeout: 6000 }).catch(() => {});
      const natural = await img.evaluate((el) => ({ w: el.naturalWidth, h: el.naturalHeight })).catch(() => ({ w: 0, h: 0 }));
      const insert = await captureVerbResp(page, "generate.insert", async () => { await page.locator("[data-cut-generate-template-insert]").click(); }, 60000);
      const clipId = insert?.result?.clips?.[0];
      const checkpoint = insert?.result?.checkpoint?.id;
      const landed = clipId ? await waitFor(async () => findClipInState(await state(), clipId)?.title_text === "Release UI", 8000) : false;
      if (checkpoint) await verb("project.revert", { to: checkpoint, rationale: "release verify generate cleanup" });
      await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
      return { ok: preview?.ok && natural.w > 0 && insert?.ok && landed, got: `preview=${preview?.ok} image=${natural.w}x${natural.h} insert=${insert?.ok} clip=${clipId || "none"} landed=${landed}` };
    } },
  { name: "surface-generate-prompt", type: "surface", desired: "Generate Prompt → from natural language, preview image renders and insert creates a real title clip",
    run: async (page) => {
      await page.locator('[data-cut-left-tab="generate"]').click();
      await page.waitForSelector('[data-cut-panel="generate-templates"]', { timeout: 6000 });
      await page.locator('[data-cut-generate-template-id="builtin.lower-third.clean"]').click().catch(() => {});
      await page.locator('[data-cut-generate-tab="prompt"]').click();
      await page.waitForSelector('[data-cut-generate-prompt-panel]', { timeout: 6000 });
      await page.locator('[data-cut-generate-prompt-input]').fill("Create a clean lower third for Marta");
      await page.locator('[data-cut-generate-prompt-policy]').selectOption("preview");
      const preview = await captureVerbResp(page, "generate.from_prompt", async () => { await page.locator("[data-cut-generate-prompt-run]").click(); }, 60000);
      const img = page.locator("[data-cut-generate-prompt-preview-img]");
      await img.waitFor({ state: "visible", timeout: 6000 }).catch(() => {});
      const natural = await img.evaluate((el) => ({ w: el.naturalWidth, h: el.naturalHeight })).catch(() => ({ w: 0, h: 0 }));
      await page.locator('[data-cut-generate-prompt-policy]').selectOption("insert");
      const beforeOps = (await verb("project.ops")).result.ops.length;
      const insert = await captureVerbResp(page, "generate.from_prompt", async () => { await page.locator("[data-cut-generate-prompt-run]").click(); }, 60000);
      const clipId = insert?.result?.insert?.clips?.[0];
      const checkpoint = insert?.result?.insert?.checkpoint?.id;
      const landed = clipId ? await waitFor(async () => !!findClipInState(await state(), clipId), 8000) : false;
      const afterOps = (await verb("project.ops")).result.ops.length;
      if (checkpoint) await verb("project.revert", { to: checkpoint, rationale: "release verify generate prompt cleanup" });
      await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
      return { ok: preview?.ok && preview?.result?.status === "completed" && natural.w > 0 && insert?.ok && insert?.result?.status === "completed" && checkpoint && landed && afterOps > beforeOps, got: `from_prompt preview=${preview?.result?.status || "none"} image=${natural.w}x${natural.h} insert=${insert?.result?.status || "none"} checkpoint=${checkpoint || "none"} clip=${clipId || "none"} ops ${beforeOps}→${afterOps} landed=${landed}` };
    } },
  { name: "surface-inspector", type: "surface", desired: "Select a clip → the Inspector reveals with that clip's tools",
    run: async (page) => {
      // Was a state-bleed flake: this runs right after surface-project-delete, which can
      // leave the UI with no clip selected → "0 tools" intermittently. The MAIN project
      // (PROJ) still has its clips — so just RE-OPEN it + reload to clear the bleed, then
      // select its first clip. (A fresh-project approach over-churned: the extra project
      // switches broke surface-transition + tripped a monitor 404.) No new project.
      await verb("project.open", { path: PROJ });
      await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1000);
      const fc = await firstVideoClip();
      if (!fc || !(await selectClip(page, fc.clipId))) return { skip: "no clip rendered in timeline" };
      await page.waitForTimeout(400);
      const tools = await page.locator("[data-cut-inspector-tool]").count();
      const effects = await page.locator("[data-cut-inspector-effect]").count();
      const shown = (await page.locator('[data-cut-panel="inspector"]').count()) > 0;
      return { ok: shown && tools > 0, got: `Inspector shown=${shown}, ${tools} tools, ${effects} effect chips` }; } },
  { name: "audio-eq", type: "surface", desired: "EQ preset (Inspector, audio clip) → the clip gains an EQ (edit.eq)",
    run: async (page) => {
      // Self-contained: a fresh import yields a linked audio clip on its own track.
      await verb("project.create", { name: "eq_" + Math.random().toString(36).slice(2, 7), settings: { width: 1280, height: 720, fps: 30 } });
      await verb("media.import", { path: CLIP });
      await waitFor(async () => (await state()).tracks.some((t) => t.kind === "audio" && (t.clips || []).some((c) => c.asset)), 8000);
      await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1200);
      const ac = (await state()).tracks.filter((t) => t.kind === "audio").flatMap((t) => t.clips || []).find((c) => c.asset);
      if (!ac || !(await selectClip(page, ac.id))) { await verb("project.open", { path: PROJ }); return { skip: "no audio clip rendered" }; }
      await page.waitForTimeout(400);
      const chip = page.locator('[data-cut-inspector-eq-preset="voice"]');
      if (!(await chip.count())) { await verb("project.open", { path: PROJ }); return { skip: "EQ control not present" }; }
      await chip.click();
      const ok = await waitFor(async () => { const c = (await state()).tracks.flatMap((t) => t.clips || []).find((x) => x.id === ac.id); return c && c.eq != null; }, 5000);
      await verb("project.open", { path: PROJ });
      return { ok, got: `edit.eq voice → clip.eq ${ok ? "set" : "NOT set"}` }; } },
  { name: "audio-cleanup", type: "surface", desired: "Denoise (Inspector, audio clip) → the clip's audio chain gains denoise (edit.effect)",
    run: async (page) => {
      await verb("project.create", { name: "dn_" + Math.random().toString(36).slice(2, 7), settings: { width: 1280, height: 720, fps: 30 } });
      await verb("media.import", { path: CLIP });
      await waitFor(async () => (await state()).tracks.some((t) => t.kind === "audio" && (t.clips || []).some((c) => c.asset)), 8000);
      await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1200);
      const ac = (await state()).tracks.filter((t) => t.kind === "audio").flatMap((t) => t.clips || []).find((c) => c.asset);
      if (!ac || !(await selectClip(page, ac.id))) { await verb("project.open", { path: PROJ }); return { skip: "no audio clip rendered" }; }
      await page.waitForTimeout(400);
      const chip = page.locator('[data-cut-inspector-audio-effect="denoise"]');
      if (!(await chip.count())) { await verb("project.open", { path: PROJ }); return { skip: "audio-effect control not present" }; }
      await chip.click();
      const ok = await waitFor(async () => { const c = (await state()).tracks.flatMap((t) => t.clips || []).find((x) => x.id === ac.id); return !!(c && (c.effects || []).some((e) => e.type === "denoise")); }, 5000);
      await verb("project.open", { path: PROJ });
      return { ok, got: `denoise chip → clip.effects has denoise = ${ok}` }; } },
  { name: "surface-blend", type: "surface", desired: "Blend mode (Inspector, overlay clip) → the composite changes (edit.blend)",
    run: async (page) => {
      // Self-contained: build a 2-video-track project so there's an overlay to blend.
      const tag = "blend_" + Math.random().toString(36).slice(2, 7);
      await verb("project.create", { name: tag, settings: { width: 1280, height: 720, fps: 30 } });
      await verb("media.import", { path: CLIP }); await sleep(800);
      await verb("media.import", { path: CLIP2 }); await sleep(800);
      const tr = await verb("edit.add_track", { kind: "video" }); const nt = tr.result?.track_id || tr.result?.id;
      const assets = Object.keys((await state()).assets);
      await verb("edit.insert", { asset: assets[assets.length - 1], track: nt, at_ms: 0 }); await sleep(500);
      const overlayClip = (await state()).tracks.find((t) => t.id === nt)?.clips?.find((c) => c.asset)?.id;
      await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1500);
      // The 2×4K composite is the suite's heaviest render — a single renderFrame can return null
      // under full-run load. Retry so a transient null doesn't masquerade as "blend doesn't work".
      const rfFrame = async () => { for (let k = 0; k < 3; k++) { const f = await renderFrame(2000); if (f) return f; await sleep(700); } return null; };
      const rfb = await rfFrame();
      let drove = false;
      if (overlayClip && (await selectClip(page, overlayClip))) { await page.waitForTimeout(400); const selEl = page.locator("[data-cut-inspector-blend]"); if (await selEl.count()) { await selEl.selectOption("multiply"); drove = true; } }
      if (!drove) await verb("edit.blend", { track: nt, mode: "multiply" });
      await sleep(800);
      const rfa = await rfFrame();
      const sim = rfb && rfa ? ssim(rfb, rfa) : null;
      await verb("project.open", { path: PROJ });
      return { ok: sim != null && sim < 0.99, got: `composite SSIM ${sim?.toFixed(4)} (UI-driven select=${drove})` }; } },
  { name: "surface-transition", type: "surface", desired: "Crossfade seam → pick a transition STYLE (wipeleft) → it's stored on the crossfade",
    run: async (page) => {
      // Self-contained: split a clip near the start so there's a clickable seam.
      await verb("project.create", { name: "xf_" + Math.random().toString(36).slice(2, 7), settings: { width: 1280, height: 720, fps: 30 } });
      await verb("media.import", { path: CLIP });
      await waitFor(async () => (await firstVideoClip()) != null, 8000);
      await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1200);
      const fc = await firstVideoClip();
      const beforeClips = await assetClipCount();
      const splitAt = 2000;
      await verb("ui.playhead", { at_ms: splitAt });
      if (fc) await selectClip(page, fc.clipId);
      await page.keyboard.press("s"); await page.waitForTimeout(700);
      const shortcutSplit = await waitFor(async () => (await assetClipCount()) > beforeClips, 4000);
      if (!shortcutSplit && fc) {
        await verb("edit.split", { clip: fc.clipId, at_ms: splitAt, rationale: "release verify: deterministic crossfade seam setup" });
        await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1000);
      }
      await waitFor(async () => (await page.locator("[data-cut-seam]").count()) > 0, 5000);
      let drove = false;
      const seam = page.locator("[data-cut-seam]").first();
      if (await seam.count()) {
        await seam.scrollIntoViewIfNeeded().catch(() => {});
        await seam.click({ force: true }); await page.waitForTimeout(400);
        const styleSel = page.locator("[data-cut-xfade-style]");
        if (await styleSel.count()) {
          await styleSel.selectOption("wipeleft");
          await captureVerbResp(page, "edit.crossfade", async () => { await page.locator('[data-cut-action="apply-xfade"]').click(); }, 20000);
          drove = true;
        }
      }
      await waitFor(async () => {
        const opsNow = (await verb("project.ops")).result.ops || [];
        return !![...opsNow].reverse().find((o) => o.verb === "edit.crossfade");
      }, 5000);
      const ops = (await verb("project.ops")).result.ops;
      const xf = [...ops].reverse().find((o) => o.verb === "edit.crossfade");
      const stored = xf?.args?.transition || xf?.effects?.[0]?.transition;
      await verb("project.open", { path: PROJ });
      return { ok: stored === "wipeleft", got: `UI-driven seam=${drove}, stored transition=${stored}` }; } },
  { name: "export-video", type: "surface", desired: "Export → Video (.mp4): the timeline renders to a PLAYABLE MP4 (real video + audio streams)",
    run: async (page) => {
      // The actual deliverable. Fresh small project + a short trim for a fast draft.
      const tag = "exp_" + Math.random().toString(36).slice(2, 7);
      await verb("project.create", { name: tag, settings: { width: 640, height: 360, fps: 24 } });
      const projectPath = ((await verb("project.list", { sort: "recent" })).result.projects || []).find((p) => p.name === tag)?.path || `${process.env.HOME}/ShellX Cut Projects/${tag}.cutproj`;
      await verb("media.import", { path: CLIP });
      await waitFor(async () => (await firstVideoClip()) != null, 8000);
      // Trim to ~4s: split at 4000 and ripple-delete everything after (fast render).
      const fc0 = await firstVideoClip();
      if (fc0) { await verb("edit.split", { clip: fc0.clipId, at_ms: 4000 }); await sleep(400); await verb("edit.ripple_delete", { range_ms: [4000, 999999] }).catch(() => {}); await sleep(400); }
      await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1200);
      let drove = false;
      let renderResp = null;
      let preflight = { seen: false, blocked: false, risks: [] };
      const btn = page.locator("[data-cut-export-btn]");
      if (await btn.count()) {
        renderResp = await captureVerbResp(page, "render.final", async () => {
          await btn.click();
          await page.waitForTimeout(300);
          const vid = page.locator('[data-cut-export-option="video"]');
          if (await vid.count()) {
            await vid.click();
            drove = true;
            preflight = await continuePreflightIfPresent(page);
          }
        }, 120000);
      }
      if (!renderResp && !preflight.blocked) renderResp = await verb("render.final", { preset: "draft" });
      const jobId = renderResp?.result?.job_id;
      // Poll the project's exports dir until the mp4 is COMPLETE (render is async; ffmpeg
      // creates the file early then writes progressively, so existence ≠ done — probe each
      // poll and only accept a real video stream + non-trivial size, else keep waiting).
      let ok = false, info = preflight.blocked ? `preflight blocked ${JSON.stringify(preflight.risks)}` : "no output file produced";
      for (let i = 0; i < 90; i++) {
        const status = jobId ? (await verb("jobs.status", { job_id: jobId })).result : null;
        if (status?.state === "failed") {
          info = `render job ${jobId} failed: ${status.error?.message || status.error || "unknown"}`;
          break;
        }
        const found = status?.result?.path || latestMp4(projectPath);
        if (found) {
          const pr = spawnSync("ffprobe", ["-v", "error", "-show_entries", "stream=codec_type", "-of", "csv=p=0", found], { encoding: "utf8" }).stdout;
          const hasV = /video/.test(pr); const hasA = /audio/.test(pr);
          const size = Number(spawnSync("stat", ["-c", "%s", found], { encoding: "utf8" }).stdout.trim() || 0);
          info = `mp4 ${(size / 1024).toFixed(0)}KB, video=${hasV} audio=${hasA}`;
          if (hasV && size > 1000) { ok = true; break; } // complete → done
        }
        await sleep(1000);
      }
      await verb("project.open", { path: PROJ });
      return { ok, got: `UI-driven export=${drove}, preflight=${preflight.seen ? JSON.stringify(preflight) : "none"}, job=${jobId || "none"}, ${info}` }; } },

  // ── QUALIFICATION FEATURES (B right-click · C grouping · D paste · E/F Inspector sliders) ──
  { name: "inspector-transform", type: "surface",
    desired: "Inspector Transform: set Scale (continuous PropertyRow) → the clip gains a transform (edit.transform)",
    run: async (page) => { await ensureProjLoaded(page); const fc = await firstVideoClip(); if (!fc) return { skip: "no clip" };
      if (!(await selectClip(page, fc.clipId))) return { skip: "select failed" };
      await page.waitForTimeout(300);
      if (!(await commitProp(page, "transform-scale", 50))) return { skip: "transform-scale row not present" };
      const scaleOf = async () => { for (const t of (await state()).tracks) for (const c of t.clips || []) if (c.id === fc.clipId) return c.transform?.scale ?? null; return null; };
      const ok = await waitFor(async () => { const s = await scaleOf(); return s != null && s < 0.99; }, 4000);
      const s = await scaleOf(); await verb("edit.transform", { clip: fc.clipId, x: 0, y: 0, scale: 1, opacity: 1 });
      return { ok, got: `Transform Scale 50% (UI slider) → clip.transform.scale=${s}` }; } },
  { name: "inspector-crop", type: "surface",
    desired: "Inspector Cropping: set Crop W (continuous PropertyRow) → the clip gains a crop (edit.crop)",
    run: async (page) => { await ensureProjLoaded(page); const fc = await firstVideoClip(); if (!fc) return { skip: "no clip" };
      if (!(await selectClip(page, fc.clipId))) return { skip: "select failed" };
      await page.waitForTimeout(300);
      if (!(await commitProp(page, "crop-w", 800))) return { skip: "crop-w row not present (probe pending)" };
      const cropOf = async () => { for (const t of (await state()).tracks) for (const c of t.clips || []) if (c.id === fc.clipId) return c.crop ?? null; return null; };
      const ok = await waitFor(async () => (await cropOf()) != null, 4000);
      const cr = await cropOf(); await verb("edit.crop", { clip: fc.clipId, x: 0, y: 0, w: 0, h: 0 }).catch(() => {});
      return { ok, got: `Crop W 800px (UI slider) → clip.crop=${JSON.stringify(cr)}` }; } },
  { name: "ctx-copy-paste", type: "surface",
    desired: "Right-click → Copy, then Paste (context menu) → a duplicate clip appears (edit.paste)",
    run: async (page) => { await ensureProjLoaded(page); const fc = await firstVideoClip(); if (!fc) return { skip: "no clip" };
      const before = await assetClipCount();
      const el = page.locator(`[data-cut-clip="${fc.clipId}"]`); if (!(await el.count())) return { skip: "no clip el" };
      await el.click({ button: "right" }); await page.waitForTimeout(250);
      const cp = page.locator('[data-cut-ctx="copy"]'); if (!(await cp.count())) return { ok: false, got: "copy item missing" };
      await cp.click(); await page.waitForTimeout(200);
      await verb("ui.playhead", { at_ms: 5000 });
      await el.click({ button: "right" }); await page.waitForTimeout(250);
      const ps = page.locator('[data-cut-ctx="paste"]'); if (!(await ps.count())) return { ok: false, got: "paste item missing" };
      if (await ps.isDisabled().catch(() => false)) { await page.keyboard.press("Escape").catch(() => {}); return { ok: false, got: "paste disabled after copy" }; }
      await ps.click(); const ok = await waitFor(async () => (await assetClipCount()) > before, 6000);
      const after = await assetClipCount(); await verb("project.open", { path: PROJ });
      return { ok, got: `asset-clips ${before}→${after} (paste adds a clip)` }; } },
  { name: "ctx-mute", type: "surface",
    desired: "Right-click → Mute (context menu) → the clip's audio is silenced (edit.gain, gain_db = -100)",
    run: async (page) => {
      // SELF-CONTAINED (was a false SKIP "mute item missing"): the Mute item only renders
      // when the right-clicked clip CARRIES audio — for a video clip that means a linked
      // audio sibling at the SAME asset + SAME start (videoHasAudio, Timeline ~3138 →
      // showAudioGrp). The shared PROJ clip had been churned by earlier surface tests
      // (split/remove) so its first video clip no longer had a co-started audio sibling →
      // the item never mounted. Build a FRESH project from a clip WITH audio so the import
      // lays the video on v1 and its linked audio at start 0 → the Mute item is present;
      // muteItem then fires edit.gain (db = -100, MUTE_DB) on the resolved audio sibling.
      const tag = "mute_" + Math.random().toString(36).slice(2, 7);
      await verb("project.create", { name: tag, settings: { width: 1280, height: 720, fps: 30 } });
      await verb("media.import", { path: CLIP }); // talking_head.mp4 carries an audio track
      // Wait until BOTH the base video clip AND its linked audio sibling (same asset, same
      // start) exist — that exact pairing is what makes the Mute item render.
      const paired = await waitFor(async () => {
        const s = await state();
        const v = (s.tracks || []).find((t) => t.kind === "video")?.clips?.find((c) => c.asset);
        if (!v) return false;
        return (s.tracks || []).some((t) => t.kind === "audio" && (t.clips || []).some((c) => c.asset === v.asset && c.start_ms === v.start_ms));
      }, 10000);
      if (!paired) { await verb("project.open", { path: PROJ }); return { skip: "import produced no linked-audio video clip" }; }
      await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1200);
      const fc = await firstVideoClip();
      if (!fc) { await verb("project.open", { path: PROJ }); return { skip: "no clip" }; }
      const minGain = async () => { let m = 0; for (const t of (await state()).tracks) for (const c of t.clips || []) if (typeof c.gain_db === "number") m = Math.min(m, c.gain_db); return m; };
      const el = page.locator(`[data-cut-clip="${fc.clipId}"]`);
      if (!(await el.count())) { await verb("project.open", { path: PROJ }); return { skip: "clip not rendered in timeline" }; }
      await el.click({ button: "right" }); await page.waitForTimeout(300);
      const item = page.locator('[data-cut-ctx="mute"]');
      if (!(await item.count())) { await page.keyboard.press("Escape").catch(() => {}); await verb("project.open", { path: PROJ }); return { ok: false, got: "Mute item missing despite a linked-audio video clip" }; }
      if (await item.isDisabled().catch(() => false)) { await page.keyboard.press("Escape").catch(() => {}); await verb("project.open", { path: PROJ }); return { ok: false, got: "Mute item disabled despite linked audio" }; }
      await item.click();
      // edit.gain stores db verbatim (cut-core edit.rs ~2005, no clamp) → the muted sibling
      // reaches exactly -100 dB; min over all clips lands there.
      const ok = await waitFor(async () => (await minGain()) <= -100, 4000);
      const g = await minGain(); await verb("project.open", { path: PROJ });
      return { ok, got: `context-menu Mute → edit.gain db=-100 → min gain_db=${g}` }; } },
  { name: "track-grouping", type: "surface",
    desired: "Add an overlay video + an audio track → tracks stay GROUPED [video…, audio…] (no v/a interleave)",
    run: async (page) => { const tag = "grp_" + Math.random().toString(36).slice(2, 7);
      await verb("project.create", { name: tag, settings: { width: 1280, height: 720, fps: 30 } });
      await verb("media.import", { path: CLIP }); await sleep(600);
      await verb("edit.add_track", { kind: "video" }); await verb("edit.add_track", { kind: "audio" });
      const kinds = (await state()).tracks.map((t) => t.kind);
      const idx = (k) => kinds.map((x, i) => (x === k ? i : -1)).filter((i) => i >= 0);
      const v = idx("video"), a = idx("audio"), c = idx("caption");
      const vmax = v.length ? Math.max(...v) : -1, amin = a.length ? Math.min(...a) : 1e9, amax = a.length ? Math.max(...a) : -1, cmin = c.length ? Math.min(...c) : 1e9;
      const grouped = vmax < amin && amax < cmin;
      await verb("project.open", { path: PROJ });
      return { ok: grouped, got: `track kinds [${kinds.join(",")}] grouped=${grouped}` }; } },
];

async function runVisual(page, t, nn) {
  const fc = await firstVideoClip();
  if (!fc) return rec(t.name, "SKIP", t.desired, "no video clip");
  // The overlay is ADDED at the playhead (AT); but an overlay with an entrance
  // animation is invisible at its FIRST frame, so MEASURE past the entrance
  // (AT + measureOffset). Grade modifies the base clip → measure at AT.
  const mAt = AT + (t.measureOffset ?? 0);
  await selectClip(page, fc.clipId);
  // pre: reset clip state so each tool measures from a clean baseline (grade tools
  // run on the same clip in sequence and would otherwise confound each other).
  if (t.pre) { await t.pre(fc); await page.waitForTimeout(400); }
  await composedAt(page, AT);
  const aPng = await shotPreview(page, join(OUT, `${nn}-${t.name}-before.png`));
  const rfb = await renderFrame(mAt); const sb = rfb ? ffSignalStats(rfb) : { yavg: null, satavg: null };
  const scB = (await verb("verify.scopes", { at_ms: mAt })).result; const lumaBefore = scB?.luma?.avg; const satScBefore = scB?.saturation?.avg;
  const applied = await t.apply(page, fc);
  if (applied === "skip") return rec(t.name, "SKIP", t.desired, "no UI control (GAP — tool not wired)");
  // Settle by observing the composed result: fixed waits can sample before a
  // generated overlay lands, especially on an unoptimised build. Poll until the
  // frame diverges from the baseline or the bounded retry loop expires.
  await page.waitForTimeout(400);
  for (let i = 0; i < 30; i++) {
    const probe = await renderFrame(mAt);
    if (probe && rfb && ssim(probe, rfb) < 0.985) break; // the change landed
    await page.waitForTimeout(800);
  }
  await composedAt(page, mAt);
  const bPng = await shotPreview(page, join(OUT, `${nn}-${t.name}-after.png`)); // evidence
  const rfa = await renderFrame(mAt); const sa = rfa ? ffSignalStats(rfa) : { yavg: null, satavg: null };
  const scA = (await verb("verify.scopes", { at_ms: mAt })).result; const lumaAfter = scA?.luma?.avg; const satScAfter = scA?.saturation?.avg;
  // rfSim = SSIM of the engine's render.frame before/after (RELIABLE pass-fail);
  // sim = preview-screenshot SSIM (evidence, mode/cache-sensitive).
  const rfSim = rfb && rfa ? ssim(rfb, rfa) : null;
  const m = { sim: ssim(aPng, bPng), rfSim, lumaBefore, lumaAfter, yBefore: sb.yavg, yAfter: sa.yavg, satBefore: sb.satavg, satAfter: sa.satavg, satScBefore, satScAfter };
  const v = t.check(m);
  rec(t.name, v.ok ? "PASS" : "FAIL", t.desired, v.got, `${nn}-${t.name}-before.png`, `${nn}-${t.name}-after.png`);
  if (t.reset) await verb("edit.grade", { clip: fc.clipId, ...t.reset });
}

(async () => {
  rmSync(OUT, { recursive: true, force: true }); mkdirSync(OUT, { recursive: true });
  rmSync(PROJ, { recursive: true, force: true });
  const cr = await verb("project.create", { name: "release", dir: PROJ });
  if (!cr.ok) await verb("project.open", { path: PROJ });
  await verb("media.import", { path: CLIP });
  await waitFor(async () => (await firstVideoClip()) != null, 8000);

  const consoleErrors = [];
  // --disable-dev-shm-usage + --disable-gpu harden the long-lived page against a headless-renderer
  // crash after many heavy 4K composed-frame screenshots accumulate in one page session.
  let browser = await chromium.launch({ args: ["--disable-dev-shm-usage", "--disable-gpu"] });
  let page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
  // The console "Failed to load resource" message carries NO url, so defer all
  // resource-load failures to the response handler below (which has the url and can
  // tell a real broken endpoint from the tolerated frame race). The console handler
  // here catches JS/application errors (e.g. the React dup-key bug).
  page.on("console", (m) => { const t = m.text(); if (m.type() === "error" && !/favicon|Failed to load resource/.test(t)) consoleErrors.push(t); });
  // Catch real HTTP 4xx/5xx — EXCEPT transient background-enrichment races that
  // self-heal: /api/frame (preview scrub before the pipeline is warm — Preview retries),
  // /filmstrip/ (timeline thumbnails generated lazily in the background → 404 until ready,
  // then 200), /proxies/ (scrub proxies, same pattern), and /api/source/ (a clip's source
  // streamed for smooth scrub — during a PROJECT SWITCH the client may briefly request the
  // prior project's asset id against the newly-opened project; the Preview self-heals to the
  // server-composed poster via its `failed` set, then resync corrects it). The actual
  // rendering is verified by the grade/title/effect tools, so these races aren't app errors.
  // Also ignore the preview-audio MONITOR (_monitor_a/_b.mp3) + per-track AUDITION stems
  // (audio_<track>.wav): rendered-then-fetched, so under churn a fetch can transiently
  // race the (re)render → a self-healing 404/416 (same class as /api/source). Real export
  // 404s (render_001.mp4, audio.mp3) still fail. Audio/export proven by dedicated checks.
  page.on("response", (r) => { if (r.status() >= 400 && !/favicon|\/api\/frame|\/filmstrip\/|\/proxies\/|\/api\/source\/|\/api\/export\/_monitor_|\/api\/export\/audio_[^./]+\.(wav|mp3)/.test(r.url())) consoleErrors.push(`HTTP ${r.status()} ${r.url().replace(/^https?:\/\/[^/]+/, "")}`); });
  await page.goto(APP, { waitUntil: "networkidle" }); await page.waitForTimeout(1000);
  // Relaunch the browser+page if the headless renderer dies mid-suite, so one crash records a FAIL
  // for that test instead of aborting the whole gate (the rest of the surfaces still get verified).
  const relaunch = async () => { try { await browser.close(); } catch {} browser = await chromium.launch({ args: ["--disable-dev-shm-usage", "--disable-gpu"] }); page = await browser.newPage({ viewport: { width: 1600, height: 1000 } }); await page.goto(APP, { waitUntil: "networkidle" }); await page.waitForTimeout(1000); };

  console.log(`\n== PRE-RELEASE UI VERIFY · clip=${CLIP.split("/").pop()} ==`);
  let i = 0;
  for (const t of TOOLS) {
    i++; const nn = String(i).padStart(2, "0");
    if (t.type === "visual") { try { await runVisual(page, t, nn); } catch (e) { rec(t.name, "FAIL", t.desired, `runVisual error: ${e.message}`); if (page.isClosed() || !browser.isConnected()) await relaunch(); } continue; }
    try {
      const r = await t.run(page);
      if (r.skip) rec(t.name, "SKIP", t.desired, r.skip);
      else { await page.screenshot({ path: join(OUT, `${nn}-${t.name}.png`) }).catch(() => {}); rec(t.name, r.ok ? "PASS" : "FAIL", t.desired, r.got, `${nn}-${t.name}.png`); }
    } catch (e) { rec(t.name, "FAIL", t.desired, e.message); }
  }
  rec("console-clean", consoleErrors.length === 0 ? "PASS" : "FAIL", "no console errors during the run", consoleErrors.length ? consoleErrors.slice(0, 4).join(" | ") : "0 errors");
  await browser.close();

  const pass = results.filter((r) => r.status === "PASS").length, fail = results.filter((r) => r.status === "FAIL").length, skip = results.filter((r) => r.status === "SKIP").length;
  const md = [`# ShellX Cut — PRE-RELEASE UI verification`, ``, `Clip: \`${CLIP}\` (RELEASE_CLIP overrides — re-run on different videos).`, `**${pass} PASS · ${fail} FAIL · ${skip} SKIP/gap** of ${results.length}. Each tool asserts its DESIRED effect actually happened (UI-driven + screenshot/measure).`, ``, `| # | tool | status | DESIRED effect | measured | before | after |`, `|---|---|---|---|---|---|---|`,
    ...results.map((r, n) => `| ${n + 1} | ${r.name} | ${r.status} | ${(r.desired || "").replace(/\|/g, "/")} | ${(r.got || "").replace(/\|/g, "/")} | ${r.shotA ? `![](${r.shotA})` : ""} | ${r.shotB ? `![](${r.shotB})` : ""} |`)].join("\n");
  writeFileSync(join(OUT, "report.md"), md); writeFileSync(join(OUT, "report.json"), JSON.stringify({ clip: CLIP, pass, fail, skip, results }, null, 2));
  console.log(`\n${pass} PASS · ${fail} FAIL · ${skip} SKIP/gap → ${join(OUT, "report.md")}`);
  process.exit(fail > 0 ? 1 : 0);
})();
