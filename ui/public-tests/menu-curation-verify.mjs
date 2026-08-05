// menu-curation-verify.mjs — verify right-click menu curation through the real UI.
// Each clip kind shows a compact applicable menu instead of one large disabled
// inventory. The check also covers clean voice on talking-head video and the
// intentionally short caption menu.
//   RUN: cd ui && SWEEP_CUTD=http://127.0.0.1:6193 SWEEP_APP=http://localhost:5173 node public-tests/menu-curation-verify.mjs
import { chromium } from "playwright";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const CUTD = process.env.SWEEP_CUTD || "http://127.0.0.1:6193";
const APP = process.env.SWEEP_APP || "http://localhost:5173";
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..");
const CLIP = join(REPO, "testdata", "talking_head.mp4");
const PROJ = process.env.HOME + "/.shellx-scratch/menucur/menucur.cutproj";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function verb(name, args = {}) {
  try { const r = await fetch(`${CUTD}/api/verb/${name}`, { method: "POST", headers: { "content-type": "application/json", "x-cut-actor": "human:ui:ui" }, body: JSON.stringify(args) }); return await r.json(); }
  catch (e) { return { ok: false, error: { message: String(e) } }; }
}
const state = async () => (await verb("project.state")).result || { tracks: [] };
let pass = 0, fail = 0;
const check = (n, ok, d = "") => { console.log(`${ok ? "PASS" : "FAIL"} ${n}${d ? " — " + d : ""}`); ok ? pass++ : fail++; };

// fresh project: a muxed clip (→ base video + LINKED audio) + a caption.
await verb("project.create", { name: "menucur", dir: PROJ });
await verb("media.import", { path: CLIP });
await sleep(800);
await verb("captions.add_text", { text: "hello world", range_ms: [500, 2000], position: "bottom" });
await sleep(500);
let s = await state();
const vid = s.tracks.flatMap((t) => (t.kind === "video" ? (t.clips || []).filter((c) => c.asset).map((c) => c.id) : []))[0];
const aud = s.tracks.flatMap((t) => (t.kind === "audio" ? (t.clips || []).filter((c) => c.asset).map((c) => c.id) : []))[0];
const cap = s.tracks.flatMap((t) => (t.kind === "caption" ? (t.clips || []).map((c) => c.id) : []))[0];
check("setup: video + linked audio + caption", !!vid && !!aud && !!cap, `video=${vid} audio=${aud} caption=${cap}`);

const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
await page.goto(APP, { waitUntil: "networkidle" });
await page.waitForTimeout(1000);
await verb("project.open", { path: PROJ });
await page.reload({ waitUntil: "networkidle" });
await page.waitForTimeout(1500);

// Open the context menu on a clip and return the set of ITEM keys present + which are enabled,
// plus whether each SECTION label is shown.
async function openMenu(id) {
  // dismiss any open menu
  await page.keyboard.press("Escape").catch(() => {});
  const el = page.locator(`[data-cut-clip="${id}"]`).first();
  if (!(await el.count())) return null;
  await el.scrollIntoViewIfNeeded().catch(() => {});
  // force: a thin caption clip can be partly overlapped by a sibling — we want the
  // right-click to land on the clip element regardless of z-order intercepts.
  await el.click({ button: "right", force: true });
  await page.waitForTimeout(350);
  const menu = page.locator("[data-cut-clip-menu]");
  if (!(await menu.count())) return null;
  const items = await menu.locator("[data-cut-ctx]").evaluateAll((els) =>
    els.map((e) => ({ key: e.getAttribute("data-cut-ctx"), disabled: e.hasAttribute("disabled") })));
  const labels = await menu.locator(".tl-ctx__label").evaluateAll((els) => els.map((e) => e.textContent?.trim()));
  const kind = await menu.getAttribute("data-cut-clip-kind");
  return { items, labels, kind, present: (k) => items.some((i) => i.key === k), enabled: (k) => items.some((i) => i.key === k && !i.disabled) };
}
async function shot(name) {
  const menu = page.locator("[data-cut-clip-menu]");
  if (await menu.count()) await menu.screenshot({ path: `/tmp/menu_${name}.png` }).catch(() => {});
}

// ── VIDEO clip (with linked audio): full menu, Clean voice ENABLED (clean-voice regression) ──
{
  const m = await openMenu(vid);
  if (!m) check("video menu opens", false);
  else {
    await shot("video");
    check("video: Picture section shown", m.labels.includes("Picture"), m.labels.join(","));
    check("video: Audio section shown", m.labels.includes("Audio"));
    check("video: Privacy (blur) shown", m.present("blur-faces"));
    check("video: Speed shown (not a still)", m.present("speed-half") && m.present("freeze"));
    check("clean-voice regression video: Clean voice ENABLED (linked audio)", m.enabled("clean-voice"), "was disabled pre-fix");
  }
}

// ── AUDIO clip: Picture + Privacy HIDDEN; Audio shown; no freeze ──
{
  const m = await openMenu(aud);
  if (!m) check("audio menu opens", false);
  else {
    await shot("audio");
    check("audio: Picture section HIDDEN", !m.labels.includes("Picture"), m.labels.join(","));
    check("audio: Privacy (blur) HIDDEN", !m.present("blur-faces"));
    check("audio: Freeze HIDDEN", !m.present("freeze"));
    check("audio: Audio section shown", m.labels.includes("Audio") && m.enabled("clean-voice"));
    check("audio: Speed (rate) shown", m.present("speed-half"));
  }
}

// ── CAPTION clip: SHORT curated menu (edit + seek + remove), NOT the media wall ──
{
  const m = await openMenu(cap);
  if (!m) check("caption menu opens", false);
  else {
    await shot("caption");
    check("caption: short menu (kind=caption)", m.kind === "caption", `kind=${m.kind}`);
    check("caption: Edit + Seek + Remove present", m.present("caption-edit") && m.present("caption-seek") && m.present("remove"));
    check("caption: NO media items (grade/mute/blur absent)", !m.present("color-grade") && !m.present("mute") && !m.present("blur-faces"));
    check("caption: ≤4 items (not the 25-row wall)", m.items.length <= 4, `items=${m.items.length}`);
  }
}

await browser.close();
console.log(`\n${fail === 0 ? "PASS" : "FAIL"}: menu curation — ${pass} pass / ${fail} fail`);
process.exit(fail === 0 ? 0 : 1);
