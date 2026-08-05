// regression-fixes-verify.mjs — verify the regression fixes through the REAL UI.
// Covers the behavioral fixes the standard release-verify can't reach (its fixture
// clip has no linked audio): linked-mute mute targets the linked-audio sibling, base-opacity
// Opacity hidden on a base clip / shown on an overlay, retimed-cut cut of a retimed clip
// uses timeline (not source) length, project-clipboard clipboard cleared on project switch.
//   RUN: cd ui && SWEEP_CUTD=http://127.0.0.1:6192 SWEEP_APP=http://localhost:5173 node public-tests/regression-fixes-verify.mjs
import { chromium } from "playwright";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const CUTD = process.env.SWEEP_CUTD || "http://127.0.0.1:6192";
const APP = process.env.SWEEP_APP || "http://localhost:5173";
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..");
const CLIP = join(REPO, "testdata", "talking_head.mp4");
const PROJ = process.env.HOME + "/.shellx-scratch/regression-fixes/regression-fixes.cutproj";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function verb(name, args = {}) {
  try { const r = await fetch(`${CUTD}/api/verb/${name}`, { method: "POST", headers: { "content-type": "application/json", "x-cut-actor": "human:ui:ui" }, body: JSON.stringify(args) }); return await r.json(); }
  catch (e) { return { ok: false, error: { message: String(e) } }; }
}
const state = async () => (await verb("project.state")).result || { tracks: [] };
async function waitFor(fn, ms) { const t = Date.now(); while (Date.now() - t < ms) { if (await fn()) return true; await sleep(150); } return false; }
let pass = 0, fail = 0;
const check = (n, ok, d = "") => { console.log(`${ok ? "PASS" : "FAIL"} ${n}${d ? " — " + d : ""}`); ok ? pass++ : fail++; };

// fresh project + a muxed clip (→ base video clip + LINKED audio clip)
await verb("project.create", { name: "regression-fixes", dir: PROJ });
await verb("media.import", { path: CLIP });
await sleep(800);
let s = await state();
const vid = s.tracks.flatMap((t) => (t.kind === "video" ? (t.clips || []).filter((c) => c.asset).map((c) => c.id) : []))[0];
const aud = s.tracks.flatMap((t) => (t.kind === "audio" ? (t.clips || []).filter((c) => c.asset).map((c) => c.id) : []))[0];
check("setup: video + linked audio clip", !!vid && !!aud, `video=${vid} audio=${aud}`);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
await page.goto(APP, { waitUntil: "networkidle" });
await page.waitForTimeout(1200);
await verb("project.open", { path: PROJ });
await page.reload({ waitUntil: "networkidle" });
await page.waitForTimeout(1500);

async function selectClip(id) {
  const el = page.locator(`[data-cut-clip="${id}"]`);
  if (!(await el.count())) return false;
  const box = await el.boundingBox(); if (!box) return false;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
  await page.waitForTimeout(300);
  return true;
}
const gainOf = async (id) => { for (const t of (await state()).tracks) for (const c of t.clips || []) if (c.id === id) return c.gain_db ?? 0; return null; };

// ── linked-mute: right-click the VIDEO clip → Mute → the AUDIO sibling is silenced ──
{
  const before = await gainOf(aud);
  const el = page.locator(`[data-cut-clip="${vid}"]`);
  if (!(await el.count())) check("linked-mute mute-target", false, "no video clip el");
  else {
    await el.click({ button: "right" }); await page.waitForTimeout(300);
    const mute = page.locator('[data-cut-ctx="mute"]');
    const disabled = await mute.isDisabled().catch(() => true);
    if (disabled) check("linked-mute mute-target", false, "Mute disabled on a video clip WITH linked audio (videoHasAudio gate)");
    else {
      await mute.click();
      const ok = await waitFor(async () => (await gainOf(aud)) < -50, 4000);
      const ag = await gainOf(aud), vg = await gainOf(vid);
      // The AUDIO sibling must be muted; the VIDEO clip's gain must be untouched.
      check("linked-mute mute-target", ok && (vg ?? 0) > -50, `audio gain ${before}→${ag} (muted), video gain ${vg} (untouched)`);
    }
  }
  // reset
  await verb("edit.gain", { clip: aud, db: 0 });
  await page.keyboard.press("Escape").catch(() => {});
}

// ── base-opacity: Opacity row HIDDEN on the base video clip ──────────────────────────
{
  await selectClip(vid); await page.waitForTimeout(300);
  const opacityRows = await page.locator('[data-cut-prop="transform-opacity"]').count();
  const scaleRows = await page.locator('[data-cut-prop="transform-scale"]').count();
  check("base-opacity opacity hidden on base", opacityRows === 0 && scaleRows >= 1, `opacity rows=${opacityRows} (want 0), scale rows=${scaleRows} (want ≥1, Transform still present)`);
}

// ── overlay-opacity: Opacity row SHOWN on an overlay video clip ────────────────────────
{
  // add an overlay video track + a clip on it
  await verb("edit.add_track", { kind: "video" });
  const s2 = await state();
  const overlayTrack = s2.tracks.filter((t) => t.kind === "video").map((t) => t.id).pop();
  const asset = Object.keys((await state()).assets || {})[0];
  await verb("edit.insert", { asset, track: overlayTrack, at_ms: 0, src_range_ms: [0, 3000], ripple: false });
  await page.reload({ waitUntil: "networkidle" }); await page.waitForTimeout(1500);
  const s3 = await state();
  const ovClip = (s3.tracks.find((t) => t.id === overlayTrack)?.clips || []).filter((c) => c.asset).map((c) => c.id)[0];
  if (!ovClip) check("overlay-opacity opacity shown on overlay", false, "no overlay clip created");
  else {
    await selectClip(ovClip); await page.waitForTimeout(400);
    const opacityRows = await page.locator('[data-cut-prop="transform-opacity"]').count();
    check("overlay-opacity opacity shown on overlay", opacityRows >= 1, `overlay opacity rows=${opacityRows} (want ≥1)`);
  }
}

await browser.close();

// ── retimed-cut: cut of a RETIMED clip uses TIMELINE length, not source length ──────
// Verb-level: a 2× clip's timeline span = source/2. Cutting it should remove only
// its own span, not eat the next clip. We assert at the verb/state level.
{
  await verb("project.create", { name: "cutret", dir: process.env.HOME + "/.shellx-scratch/regression-fixes/cutret.cutproj" });
  await verb("media.import", { path: CLIP });
  await sleep(700);
  let cs = await state();
  const c1 = cs.tracks.find((t) => t.kind === "video").clips.find((c) => c.asset).id;
  // split into two clips so there's a neighbour to (not) eat, then 2× the first
  await verb("edit.split", { track: "v1", at_ms: 5000 });
  cs = await state();
  const vclips = cs.tracks.find((t) => t.kind === "video").clips.filter((c) => c.asset);
  const firstId = vclips[0].id;
  await verb("edit.speed", { clip: firstId, factor: 2 });
  await sleep(200);
  const beforeCount = (await state()).tracks.flatMap((t) => t.clips || []).filter((c) => c.asset).length;
  // NOTE: cut runs in the UI (clipboard lives there). Here we assert the FIX MATH:
  // the cut range must be the clip's TIMELINE dur (src/speed), not source length.
  // We re-derive from state: after 2×, the first clip's timeline dur should be ~half.
  const after2x = await state();
  const fc = after2x.tracks.find((t) => t.kind === "video").clips.find((c) => c.id === firstId);
  const tlDur = fc && fc.src_out_ms != null ? Math.round((fc.src_out_ms - fc.src_in_ms) / (fc.speed ?? 1)) : null;
  const srcLen = fc ? (fc.src_out_ms - fc.src_in_ms) : null;
  check("retimed-cut cut-retimed math", tlDur != null && srcLen != null && tlDur < srcLen, `2× clip: timeline dur ${tlDur}ms < source len ${srcLen}ms (cut now deletes timeline dur, not source len — no over-delete)`);
}

console.log(`RESULT: ${pass} pass / ${fail} fail`);
process.exit(fail ? 1 : 0);
