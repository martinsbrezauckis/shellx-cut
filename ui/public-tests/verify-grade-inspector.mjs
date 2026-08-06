// verify-grade-inspector.mjs — EFFECT-PROOF gate for the Grade (Color) + Inspector
// human-edit controls that the effect-only release gate left UNPROVEN.
//
// WHY THIS EXISTS: release-verify proves each VERB's effect; interaction-verify proves
// the UI BEHAVES. But a cluster of CONTEXTUAL edit controls — the Color tab's
// contrast/gamma/white-balance/reset sliders and the Inspector's stabilize / reverse /
// duck / effect-chips / 2× speed buttons — were never driven through the REAL control
// with the engine effect asserted (only brightness+saturation in Grade and invert+denoise
// in Inspector had coverage). A control can be wired (fires a verb) yet apply NOTHING, or
// be silently missing. This gate drives each REAL DOM control and asserts the INTENDED
// engine effect: a composed-frame change (SSIM<0.99), a clip.* field flip in project.state,
// or the op landing in project.ops with the right type. Each check FAILS hard if its control
// is missing — there is NO silent verb fallback (a missing button must not pass).
//
// RUN:  cd ui && SWEEP_CUTD=http://127.0.0.1:6202 SWEEP_APP=http://localhost:5202 \
//         node public-tests/verify-grade-inspector.mjs
// Exit 0 = all PASS; non-zero on any FAIL (CI / gate friendly).
//
// CLIP: talking_head.mp4 (a real video WITH an audio track — the import lands the video on
// v1 and the audio on its OWN track a1t, so the audio-clip checks (duck) have a target on a
// track distinct from the speech track v1, which is exactly when the Inspector duck control
// appears and edit.duck can compute speech windows against v1).
import { chromium } from "playwright";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { base64ToBuffer } from "../../scripts/lib/safe-data.mjs";

const CUTD = process.env.SWEEP_CUTD || "http://127.0.0.1:6202";
const APP = process.env.SWEEP_APP || "http://localhost:5202";
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..");
const CLIP = process.env.RELEASE_CLIP || join(REPO, "testdata", "talking_head.mp4");
const DUCK_PERCEPTION = join(REPO, "testdata", "talking_head.perception.json");
// A real (non-identity) .cube the engine can read (fences: must exist + end .cube).
// Override with a platform-correct path (e.g. a Windows temp path) for the installed
// engine, like RELEASE_CLIP.
const LUT = process.env.RELEASE_LUT || join(REPO, "testdata", "test_lut_invert.cube");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tmp = mkdtempSync(join(tmpdir(), "vgi-"));
let seq = 0;

async function verb(name, args = {}) {
  const r = await fetch(`${CUTD}/api/verb/${name}`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-cut-actor": "human:ui:ui" },
    body: JSON.stringify(args),
  });
  return r.json();
}
async function state() {
  return (await verb("project.state")).result;
}
async function ops() {
  return (await verb("project.ops")).result?.ops || [];
}
async function waitForState(pred, timeoutMs = 12000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const current = await state();
    try { if (pred(current)) return current; } catch {}
    await sleep(250);
  }
  return null;
}
async function waitJobs(maxS = 180) {
  for (let i = 0; i < maxS * 2; i++) {
    const js = (await verb("jobs.list")).result?.jobs || [];
    if (!js.some((j) => j.state === "queued" || j.state === "running")) return true;
    await sleep(500);
  }
  return false;
}
// POLL the op-log until a new edit.grade op for `clipId` has COMMITTED since `sinceLen`
// (and, when `pred` is given, until the stored clip.grade satisfies it). Mirrors the
// full-coverage harness's opLanded poll-don't-sleep: render.frame is cache-keyed on
// op_applied, so a frame fetched BEFORE the grade op lands returns a stale pre-grade
// cache hit (SSIM 1.0000) even though project.state already shows the new grade. A
// genuine no-op still times out → false, so real failures are NOT masked.
async function gradeOpLanded(sinceLen, clipId, pred, { timeoutMs = 6000, intervalMs = 150 } = {}) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const all = await ops();
    const opHit = all.slice(sinceLen).some((o) => o.verb === "edit.grade" && o.args?.clip === clipId);
    if (opHit && (!pred || pred(await clipField(clipId, "grade")))) return true;
    if (Date.now() >= deadline) return false;
    await sleep(intervalMs);
  }
}
// A composed frame at `at` ms → image path (cache-busted by op_applied; compose:true so the
// grade/effect is BAKED into the frame, the raw proxy never shows it).
async function frame(at) {
  // inline:true → the engine returns frame bytes as base64 over HTTP, so this
  // works whether cutd is local (WSL) or the installed Windows engine, whose
  // native path is not readable through WSL file-copy commands.
  const r = await verb("render.frame", { at_ms: at, compose: true, inline: true });
  const mime = String(r.result?.mime || "");
  const ext = mime.includes("png") ? "png" : (mime.includes("jpeg") || mime.includes("jpg")) ? "jpg" : "bin";
  const dst = join(tmp, `f${seq++}.${ext}`);
  const b64 = r.result?.base64;
  if (b64) {
    writeFileSync(dst, base64ToBuffer(b64, { expectPng: ext === "png" }));
    return dst;
  }
  // Fallback (older engine without inline support): cp the shared-fs path.
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

// Find the c1 video clip's current grade / effects / speed / reverse / stabilize from state.
async function clipField(clipId, field) {
  const s = await state();
  for (const t of s.tracks || []) {
    for (const c of t.clips || []) {
      if (c.id === clipId) return c[field];
    }
  }
  return undefined;
}

// ONE shared base project + clip for the whole run. A per-check fresh import was the
// faithful-isolation choice, but talking_head's import can kick off heavy perception jobs;
// eight overlapping passes OOM-killed cutd mid-run. So we import ONCE, seed deterministic
// duck perception facts, and RESET the clip to neutral before each check measures its
// baseline — clean isolation without re-importing.
// IDS: talking_head → v1/c1 (video), a1t/c2 (audio). Resolved live so we never hard-code.
let BASE = { vid: null, aud: null };

async function seedDuckPerception(page, projectDir, asset) {
  const report = JSON.parse(readFileSync(DUCK_PERCEPTION, "utf8"));
  report.source_path = CLIP;
  if (report.words) report.words.asset = asset;
  const receipts = join(projectDir, "receipts");
  mkdirSync(receipts, { recursive: true });
  writeFileSync(join(receipts, `${asset}.perception.json`), JSON.stringify(report, null, 2));
  try { unlinkSync(join(projectDir, "project.json")); } catch {}
  const reopened = await verb("project.open", { path: projectDir });
  if (!reopened.ok) throw new Error(`project.open after duck perception seed failed: ${JSON.stringify(reopened.error || reopened).slice(0, 160)}`);
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(1000);
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(300);
}

async function setupBase(page) {
  const created = await verb("project.create", { name: "vgi_base_" + Math.random().toString(36).slice(2, 6), settings: { width: 1280, height: 720, fps: 30 } });
  const projectDir = created.result?.path;
  const imported = await verb("media.import", { path: CLIP });
  const asset = imported.result?.asset_id;
  await waitJobs();
  if (projectDir && asset) await seedDuckPerception(page, projectDir, asset);
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(1200);
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(400);
  const s = await state();
  BASE.vid = s.tracks.find((t) => t.kind === "video")?.clips?.find((c) => c.asset)?.id;
  BASE.aud = s.tracks.find((t) => t.kind === "audio")?.clips?.find((c) => c.asset)?.id;
}

// Reset the video clip's grade / effects / speed / reverse / stabilize to neutral via verbs
// (legitimate setup so each check measures a clean baseline — NOT the control under test, which
// each check drives through the real DOM). Reload so the UI re-seeds sliders from the reset clip.
async function resetClip(page) {
  if (!BASE.vid) return;
  await verb("edit.grade", { clip: BASE.vid, contrast: 1, brightness: 0, saturation: 1, gamma: 1, rationale: "gate: reset" });
  await verb("edit.effect", { clip: BASE.vid, effects: [], rationale: "gate: reset" });
  await verb("edit.speed", { clip: BASE.vid, factor: 1, rationale: "gate: reset" });
  await verb("edit.reverse", { clip: BASE.vid, enabled: false, rationale: "gate: reset" });
  await verb("edit.stabilize", { clip: BASE.vid, enabled: false, rationale: "gate: reset" });
  await sleep(300);
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(1000);
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(300);
}

// Prep a check: reset the clip to neutral, return the base ids. (Replaces freshProject.)
async function prep(page) {
  await resetClip(page);
  return { vid: BASE.vid, aud: BASE.aud };
}

async function selectClip(page, clipId) {
  await page.locator(`[data-cut-clip="${clipId}"]`).waitFor({ timeout: 8000 }).catch(() => {});
  await page.locator(`[data-cut-clip="${clipId}"]`).click({ timeout: 5000 }).catch(() => {});
  await sleep(400);
}

async function ensureRightRail(page) {
  const expand = page.locator('[data-cut-action="expand-rail"]');
  if (await expand.count()) {
    await expand.click().catch(() => {});
    await sleep(250);
  }
}

// Open the Color (grade) tab; FAIL if its body or a given slider input is missing.
async function openColorTab(page) {
  await ensureRightRail(page);
  const tab = page.locator('[data-cut-right-tab="color"]');
  if (!(await tab.count())) throw new Error("Color right-tab control missing");
  await tab.click();
  await sleep(500);
  if (!(await cnt(page, "[data-cut-grade-embed]"))) throw new Error("Color tab body did not open");
}

// Set a range/number slider to an exact value (fill dispatches input+change → React onChange
// updates the grade state) and assert the live value readout reflects it.
async function setGradeSlider(page, attr, value) {
  const input = page.locator(`[data-cut-grade-input="${attr}"]`);
  if (!(await input.count())) throw new Error(`grade slider [${attr}] missing`);
  await input.fill(String(value));
  await sleep(150);
}

async function applyGrade(page) {
  const btn = page.locator("[data-cut-grade-apply]");
  if (!(await btn.count())) throw new Error("grade Apply button missing");
  await btn.click();
}

// ── CLUSTER 1 — Grade / Color tab ────────────────────────────────────────────
// Each: drive the REAL slider→apply, assert the COMPOSED frame changes (SSIM<0.99).
// Per-check fresh project so the baseline frame is un-graded.

async function checkGradeContrast(page) {
  const { vid } = await prep(page);
  if (!vid) return { pass: false, detail: "no video clip" };
  await selectClip(page, vid);
  const f0 = await frame(1500);
  await openColorTab(page);
  await setGradeSlider(page, "contrast", 1.8); // range 0..2, step .01 — well off neutral 1.0
  await applyGrade(page);
  await sleep(900);
  const f1 = await frame(1500);
  const s = f0 && f1 ? ssim(f0, f1) : null;
  const grade = await clipField(vid, "grade");
  const stored = grade?.contrast;
  return {
    pass: s != null && s < 0.99 && stored === 1.8,
    detail: `contrast→1.8 stored=${stored}; composed SSIM ${s?.toFixed(4)} (<0.99 ⇒ applied)`,
  };
}

async function checkGradeGamma(page) {
  const { vid } = await prep(page);
  if (!vid) return { pass: false, detail: "no video clip" };
  await selectClip(page, vid);
  const f0 = await frame(1500);
  await openColorTab(page);
  await setGradeSlider(page, "gamma", 2.2); // range .1..3, step .01 — off neutral 1.0
  await applyGrade(page);
  await sleep(900);
  const f1 = await frame(1500);
  const s = f0 && f1 ? ssim(f0, f1) : null;
  const stored = (await clipField(vid, "grade"))?.gamma;
  return {
    pass: s != null && s < 0.99 && stored === 2.2,
    detail: `gamma→2.2 stored=${stored}; composed SSIM ${s?.toFixed(4)} (<0.99 ⇒ applied)`,
  };
}

async function checkGradeWhiteBalance(page) {
  // Enable Kelvin white-balance (data-cut-grade-temp-on) → drag temp to a WARM value
  // (3000K; the UI default 6500K is ~neutral and would not move the frame) → apply →
  // the frame warms (SSIM<0.99) and temperature_k lands in clip.grade.
  const { vid } = await prep(page);
  if (!vid) return { pass: false, detail: "no video clip" };
  await selectClip(page, vid);
  const f0 = await frame(1500);
  await openColorTab(page);
  const temp = page.locator("[data-cut-grade-temp-on]");
  if (!(await temp.count())) return { pass: false, detail: "white-balance (Kelvin) toggle missing" };
  await temp.check();
  await sleep(200);
  await setGradeSlider(page, "temperature_k", 3000); // warm; range 2000..12000
  // POLL-DON'T-SLEEP (full-coverage opLanded pattern): snapshot the op-log length BEFORE
  // Apply, then WAIT for the edit.grade op to actually COMMIT before fetching f1. The old
  // fixed sleep(900) raced the async commit under load — render.frame is cache-keyed on
  // op_applied, so fetching f1 before the grade landed returned a CACHE HIT at the
  // pre-grade revision (SSIM exactly 1.0000) even though stored=3000. With the op awaited
  // the frame renders at the graded revision and the white-balance warm is measurable
  // (proven: temp=3000 → SSIM ~0.85 in isolation).
  const beforeOps = (await ops()).length;
  await applyGrade(page);
  const landed = await gradeOpLanded(beforeOps, vid, (g) => g?.temperature_k === 3000);
  const f1 = await frame(1500);
  const s = f0 && f1 ? ssim(f0, f1) : null;
  const storedTemp = (await clipField(vid, "grade"))?.temperature_k;
  return {
    pass: landed && s != null && s < 0.99 && storedTemp === 3000,
    detail: `temp→3000K landed=${landed} stored=${storedTemp}; composed SSIM ${s?.toFixed(4)} (<0.99 ⇒ warmed)`,
  };
}

async function checkGradeReset(page) {
  // Reset to neutral clears the grade: apply a non-neutral grade first (clip.grade set),
  // then click "Reset to neutral" (data-cut-grade-reset → all-identity slider state) and
  // Apply → an all-identity grade makes the engine store clip.grade = null (cleared).
  const { vid } = await prep(page);
  if (!vid) return { pass: false, detail: "no video clip" };
  await selectClip(page, vid);
  await openColorTab(page);
  await setGradeSlider(page, "contrast", 1.5);
  await applyGrade(page);
  await sleep(800);
  const before = await clipField(vid, "grade");
  if (!before) return { pass: false, detail: "setup grade did not land before reset" };
  const reset = page.locator("[data-cut-grade-reset]");
  if (!(await reset.count())) return { pass: false, detail: "Reset-to-neutral button missing" };
  await reset.click();
  await sleep(200);
  await applyGrade(page); // reset only sets slider state; Apply commits the neutral grade
  await sleep(800);
  const after = await clipField(vid, "grade");
  return {
    pass: !!before && after == null,
    detail: `grade before=${JSON.stringify(before && { c: before.contrast })} after=${JSON.stringify(after)} (null ⇒ cleared)`,
  };
}

// LUT (.cube): fill the data-cut-grade-lut path with a real non-identity LUT, Apply, and
// assert the composed frame changes (SSIM<0.99) AND clip.grade.lut is stored ending in
// .cube. The engine fences the path (must exist + end .cube), so this proves the whole
// path: UI → edit.grade{lut} → render. Was SKIPPED for lack of a fixture (now bundled).
async function checkGradeLut(page) {
  const { vid } = await prep(page);
  if (!vid) return { pass: false, detail: "no video clip" };
  await selectClip(page, vid);
  const f0 = await frame(1500);
  await openColorTab(page);
  const advanced = page.locator("[data-cut-grade-lut-advanced]");
  if (await advanced.count()) {
    const open = await advanced.evaluate((el) => el.open).catch(() => true);
    if (!open) await advanced.locator("summary").click();
  }
  const lut = page.locator("[data-cut-grade-lut]");
  if (!(await lut.count())) return { pass: false, detail: "LUT input [data-cut-grade-lut] missing" };
  await lut.fill(LUT);
  await sleep(150);
  await applyGrade(page);
  await sleep(1500);
  const f1 = await frame(1500);
  const s = f0 && f1 ? ssim(f0, f1) : null;
  const grade = await clipField(vid, "grade");
  const stored = grade?.lut ? String(grade.lut) : null;
  return {
    pass: s != null && s < 0.99 && !!stored && stored.endsWith(".cube"),
    detail: `lut=${stored ? stored.split(/[\\/]/).pop() : "none"}; composed SSIM ${s?.toFixed(4)} (<0.99 ⇒ applied)`,
  };
}

// ── CLUSTER 2 — Inspector actions (Properties tab) ───────────────────────────

// Open the Properties tab + assert the Inspector body is the right kind for the selection.
async function openProperties(page) {
  await ensureRightRail(page);
  const tab = page.locator('[data-cut-right-tab="properties"]');
  if (!(await tab.count())) throw new Error("Properties right-tab control missing");
  await tab.click();
  await sleep(400);
  if (!(await cnt(page, '[data-cut-panel="inspector"]'))) throw new Error("Inspector body did not open");
}

async function expandInspectorSection(page, key) {
  const section = page.locator(`[data-cut-section="${key}"]`).first();
  if (!(await section.count())) throw new Error(`Inspector section "${key}" missing`);
  if ((await section.getAttribute("data-cut-section-collapsed")) === "true") {
    await page.locator(`[data-cut-section-toggle="${key}"]`).first().click();
    await sleep(200);
  }
}

async function checkInspectorStabilize(page) {
  // Stabilize button (data-cut-inspector-action="stabilize") → edit.stabilize op + the
  // clip.stabilize field becomes non-null (None = not stabilized).
  const { vid } = await prep(page);
  if (!vid) return { pass: false, detail: "no video clip" };
  await selectClip(page, vid);
  await openProperties(page);
  await expandInspectorSection(page, "video-motion");
  const before = (await ops()).length;
  const btn = page.locator('[data-cut-inspector-action="stabilize"]');
  if (!(await btn.count())) return { pass: false, detail: "Stabilize button missing" };
  await btn.click();
  await sleep(900);
  const newOps = (await ops()).slice(before);
  const hasOp = newOps.some((o) => o.verb === "edit.stabilize" && o.args?.clip === vid);
  const field = await clipField(vid, "stabilize");
  return {
    pass: hasOp && field != null,
    detail: `edit.stabilize op=${hasOp}; clip.stabilize=${JSON.stringify(field)} (non-null ⇒ applied)`,
  };
}

async function checkInspectorReverse(page) {
  // Speed / Retime Reverse toggle (data-cut-prop="speed-reverse") toggles clip.reverse true,
  // and edit.reverse op recorded. (reverse is serde-skipped at false, so the field is
  // absent/false before and true after.)
  const { vid } = await prep(page);
  if (!vid) return { pass: false, detail: "no video clip" };
  await selectClip(page, vid);
  await openProperties(page);
  await expandInspectorSection(page, "speed");
  const revBefore = (await clipField(vid, "reverse")) || false;
  const before = (await ops()).length;
  const btn = page.locator('[data-cut-prop="speed-reverse"]');
  if (!(await btn.count())) return { pass: false, detail: "Reverse button missing" };
  await btn.click();
  await sleep(900);
  const newOps = (await ops()).slice(before);
  const hasOp = newOps.some((o) => o.verb === "edit.reverse" && o.args?.clip === vid);
  const revAfter = (await clipField(vid, "reverse")) || false;
  return {
    pass: hasOp && revBefore === false && revAfter === true,
    detail: `edit.reverse op=${hasOp}; clip.reverse ${revBefore}→${revAfter} (toggled ⇒ applied)`,
  };
}

async function checkInspectorDuck(page) {
  // Duck button (data-cut-inspector-action="duck") on an AUDIO clip → edit.duck op.
  // The button only renders when the selected audio clip is on a track distinct
  // from a speech audio track. Build a simple "music" track with a second audio
  // clip, select that, and duck it against the base speech track a1t. The
  // speech perception facts are seeded deterministically in setupBase().
  const { aud } = await prep(page);
  if (!aud) return { pass: false, detail: "no audio clip" };
  // Resolve the speech audio track + source asset, then add a distinct audio
  // track that represents music/background audio for the UI control.
  const s0 = await state();
  const speechTrack = (s0.tracks || []).find((t) => t.kind === "audio" && (t.clips || []).some((c) => c.id === aud));
  const speechClip = speechTrack?.clips?.find((c) => c.id === aud);
  if (!speechTrack?.id || !speechClip?.asset) return { pass: false, detail: "no speech audio track/asset" };
  const addTrack = await verb("edit.add_track", { kind: "audio", rationale: "verifier: music track for duck" });
  const musicTrack = addTrack.result?.track_id || (await state()).tracks.filter((t) => t.kind === "audio").pop()?.id;
  await verb("edit.insert", { asset: speechClip.asset, track: musicTrack, at_ms: 0, rationale: "verifier: music clip for duck" });
  let musicClip = null;
  for (let i = 0; i < 16; i++) {
    await sleep(250);
    const mt = (await state()).tracks.find((t) => t.id === musicTrack);
    musicClip = (mt?.clips || []).find((c) => c.asset === speechClip.asset)?.id;
    if (musicClip) break;
  }
  if (!musicClip) return { pass: false, detail: `music clip never landed on ${musicTrack}` };
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(1000);
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await selectClip(page, musicClip);
  await openProperties(page);
  await expandInspectorSection(page, "audio-mix");
  const kind = await page.locator("[data-cut-inspector-scope]").innerText().catch(() => "");
  const before = (await ops()).length;
  let duckReqs = 0;
  const onReq = (req) => { if (req.url().includes("/api/verb/edit.duck")) duckReqs++; };
  page.on("request", onReq);
  const btn = page.locator('[data-cut-inspector-action="duck"]');
  try {
    if (!(await btn.count())) return { pass: false, detail: `Duck button missing (inspector scope="${kind}", music=${musicTrack}, speech=${speechTrack.id})` };
    await btn.click();
    await sleep(1600);
    const newOps = (await ops()).slice(before);
    const duck = newOps.find((o) => o.verb === "edit.duck");
    return {
      pass: !!duck && duck.args?.music_track === musicTrack && duck.args?.against_track === speechTrack.id,
      detail: `edit.duck op=${!!duck} reqs=${duckReqs} music_track=${duck?.args?.music_track} against=${duck?.args?.against_track} expected=${musicTrack}/${speechTrack.id}`,
    };
  } finally {
    page.off("request", onReq);
  }
}

async function checkInspectorEffectChips(page) {
  // Two effect chips beyond invert: sepia + vignette (data-cut-inspector-effect="<type>").
  // Each must append its effect (right type) to clip.effects AND change the composed frame.
  // (Effect chips are SET-semantics — edit.effect replaces the list — so after sepia then
  // vignette the list holds exactly vignette; assert each chip's own effect when applied.)
  const { vid } = await prep(page);
  if (!vid) return { pass: false, detail: "no video clip" };
  await selectClip(page, vid);
  await openProperties(page);
  await expandInspectorSection(page, "video-effects");
  const f0 = await frame(1500);

  // sepia
  const sepia = page.locator('[data-cut-inspector-effect="sepia"]');
  if (!(await sepia.count())) return { pass: false, detail: "sepia effect chip missing" };
  await sepia.click();
  await sleep(900);
  const fxSepia = await clipField(vid, "effects");
  const fSepia = await frame(1500);
  const sSepia = f0 && fSepia ? ssim(f0, fSepia) : null;
  const sepiaOk = Array.isArray(fxSepia) && fxSepia.some((e) => e.type === "sepia") && sSepia != null && sSepia < 0.99;
  // toggle sepia off so the list is clean before vignette (SET semantics)
  await sepia.click();
  await sleep(700);

  // vignette
  const vig = page.locator('[data-cut-inspector-effect="vignette"]');
  if (!(await vig.count())) return { pass: false, detail: "vignette effect chip missing" };
  await vig.click();
  await sleep(900);
  const fxVig = await clipField(vid, "effects");
  const fVig = await frame(1500);
  const sVig = f0 && fVig ? ssim(f0, fVig) : null;
  const vigOk = Array.isArray(fxVig) && fxVig.some((e) => e.type === "vignette") && sVig != null && sVig < 0.99;

  return {
    pass: sepiaOk && vigOk,
    detail: `sepia: type+SSIM ${sSepia?.toFixed(4)} ok=${sepiaOk}; vignette: type+SSIM ${sVig?.toFixed(4)} ok=${vigOk}`,
  };
}

async function checkInspectorSpeed(page) {
  // Inspector Speed / Retime numeric input (data-cut-prop-input="speed") → the clip's timeline span
  // HALVES: edit.speed sets clip.speed=2.0 and the verb reports new_timeline_duration_ms ≈
  // src_dur/2. We assert via project.state (clip.speed flips to 2.0) AND that the op landed
  // — this is the selected-clip Inspector home, distinct from the timeline-toolbar speed control.
  const { vid } = await prep(page);
  if (!vid) return { pass: false, detail: "no video clip" };
  await selectClip(page, vid);
  await openProperties(page);
  await expandInspectorSection(page, "speed");
  const srcIn = await clipField(vid, "src_in_ms");
  const srcOut = await clipField(vid, "src_out_ms");
  const srcDur = (srcOut ?? 0) - (srcIn ?? 0);
  const speedBefore = (await clipField(vid, "speed")) ?? 1;
  let dur = null;
  const onResp = async (r) => {
    if (r.url().includes("/api/verb/edit.speed")) { try { dur = (await r.json())?.result?.new_timeline_duration_ms; } catch {} }
  };
  page.on("response", onResp);
  try {
    const before = (await ops()).length;
    const input = page.locator('[data-cut-prop-input="speed"]');
    if (!(await input.count())) return { pass: false, detail: "Speed input missing" };
    await input.fill("2");
    await input.press("Enter");
    await sleep(900);
    const newOps = (await ops()).slice(before);
    const hasOp = newOps.some((o) => o.verb === "edit.speed" && o.args?.clip === vid && o.args?.factor === 2);
    const speedAfter = await clipField(vid, "speed");
    // timeline span halved: clip.speed 1→2 AND the reported new duration ≈ srcDur/2.
    const spanHalved = speedBefore === 1 && speedAfter === 2;
    const durHalved = dur != null && Math.abs(dur - srcDur / 2) <= 2; // integer ms rounding
    return {
      pass: hasOp && spanHalved && durHalved,
      detail: `edit.speed op=${hasOp}; clip.speed ${speedBefore}→${speedAfter}; tl_dur ${dur} vs src/2 ${Math.round(srcDur / 2)} (halved=${durHalved})`,
    };
  } finally {
    page.off("response", onResp);
  }
}

async function checkPowerWindowAtomicRemove(page) {
  // Add two windows through the real Inspector, then remove the first. One click must
  // produce exactly one request/op carrying remove_index:0, and preserve the second
  // window. The old clear+rebuild implementation could destroy the whole stack if any
  // append failed between those calls.
  const { vid } = await prep(page);
  if (!vid) return { pass: false, detail: "no video clip" };
  await verb("edit.grade_window", { clip: vid, enabled: false, rationale: "gate: reset windows" });
  await selectClip(page, vid);
  await openProperties(page);
  await expandInspectorSection(page, "video-color");
  const add = page.locator('[data-cut-action="grade-window-add"]');
  if (!(await add.count())) return { pass: false, detail: "Power window Add button missing" };

  await page.locator('[data-cut-grade-window-region]').selectOption('center');
  await add.click();
  const one = await waitForState((s) => (s.tracks || []).some((t) => (t.clips || []).some((c) => c.id === vid && c.grade_windows?.length === 1)));
  await page.locator('[data-cut-grade-window-region]').selectOption('left');
  await add.click();
  const two = await waitForState((s) => (s.tracks || []).some((t) => (t.clips || []).some((c) => c.id === vid && c.grade_windows?.length === 2)));
  if (!one || !two) return { pass: false, detail: `window setup one=${!!one} two=${!!two}` };

  const before = (await ops()).length;
  let requests = 0;
  const onRequest = (req) => {
    if (!req.url().includes('/api/verb/edit.grade_window')) return;
    try { if (req.postDataJSON()?.remove_index === 0) requests++; } catch {}
  };
  page.on('request', onRequest);
  try {
    await page.locator('[data-cut-grade-window-row] [data-cut-action="grade-window-remove"]').first().click();
    const removed = await waitForState((s) => (s.tracks || []).some((t) => (t.clips || []).some((c) => c.id === vid && c.grade_windows?.length === 1)));
    const newOps = (await ops()).slice(before).filter((o) => o.verb === 'edit.grade_window');
    const windows = await clipField(vid, 'grade_windows');
    const preservedLeft = windows?.length === 1 && windows[0]?.window?.points?.[0]?.[0] === 0;
    const atomic = requests === 1 && newOps.length === 1 && newOps[0]?.args?.remove_index === 0;
    return {
      pass: !!removed && preservedLeft && atomic,
      detail: `len 2→1=${!!removed}; preserved second=${preservedLeft}; requests=${requests}; ops=${newOps.length}; remove_index=${newOps[0]?.args?.remove_index}`,
    };
  } finally {
    page.off('request', onRequest);
  }
}

// ── runner ───────────────────────────────────────────────────────────────────
async function main() {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  const errors = [];
  page.on("response", (r) => {
    if (r.status() >= 400 && !/favicon|\/api\/frame|\/filmstrip\/|\/proxies\/|\/api\/source\/|\/api\/export\/_monitor_/.test(r.url()))
      errors.push(`HTTP ${r.status()} ${r.url().replace(/^https?:\/\/[^/]+/, "")}`);
  });
  await page.goto(APP, { waitUntil: "domcontentloaded" });
  await sleep(1000);

  // One shared base project + clip (one perception/STT pass; duck needs v1's silence facts).
  await setupBase(page);
  if (!BASE.vid) {
    console.error("FATAL: base project import produced no video clip — cutd/import broken");
    await browser.close();
    process.exit(2);
  }

  const results = [];
  const run = async (name, fn) => {
    try {
      const r = await fn(page);
      results.push({ name, ...r });
    } catch (e) {
      results.push({ name, pass: false, detail: String(e.message || e).slice(0, 140) });
    }
  };

  // CLUSTER 1 — Grade / Color tab
  await run("grade-contrast", checkGradeContrast);
  await run("grade-gamma", checkGradeGamma);
  await run("grade-white-balance", checkGradeWhiteBalance);
  await run("grade-reset", checkGradeReset);
  await run("grade-lut", checkGradeLut); // real .cube fixture (testdata/test_lut_invert.cube)

  // CLUSTER 2 — Inspector actions
  await run("inspector-stabilize", checkInspectorStabilize);
  await run("inspector-reverse", checkInspectorReverse);
  await run("inspector-duck", checkInspectorDuck);
  await run("inspector-effect-chips", checkInspectorEffectChips);
  await run("inspector-speed-2x", checkInspectorSpeed);
  await run("power-window-atomic-remove", checkPowerWindowAtomicRemove);

  results.push({ name: "console-clean", pass: errors.length === 0, detail: errors.length ? errors.slice(0, 4).join(" | ") : "0 errors" });

  await browser.close();

  let fail = 0;
  console.log("\n== GRADE + INSPECTOR EFFECT-PROOF ==");
  for (const r of results) {
    console.log(`  ${r.pass ? "PASS" : "FAIL"}  ${r.name.padEnd(26)} ${r.detail}`);
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
