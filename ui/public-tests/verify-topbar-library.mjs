// verify-topbar-library.mjs — EFFECT-PROOF gate for three previously-untested
// human surfaces coverage audit). Companion to interaction-verify.mjs
// and release-verify.mjs; this file is STANDALONE (it does not touch them).
//
// It drives the REAL UI control and asserts the INTENDED EFFECT (not "op recorded"):
//
//   CLUSTER 1 — Timeline global tools (TimelineGlobalTools.tsx; data-cut-tool="…"):
//     • Trim dead air  (trim_edges)   → timeline duration DROPS.
//     • Split at scenes(split_at_scenes)→ video clip count GROWS.
//     • Mark scenes    (mark_scenes)  → project.markers GROWS.
//   Each Tools verb is perception-driven, so each check builds its OWN project
//   with footage engineered to make the effect real (hard scene cuts / speech
//   wrapped in silence) and waits for the enrichment battery before clicking.
//
//   CLUSTER 2 — Export ▾ menu formats (data-cut-export-option="…"): only Video
//     was proven before. Here EACH of gif/srt/vtt/ass/fcpxml/premiere/resolve/
//     otio/chapters/transcript/frame is clicked through the REAL menu; the gate
//     captures the verb's response `path`, then reads the file off disk and
//     asserts it exists, is non-empty, and is the RIGHT type (magic bytes /
//     header / valid JSON). Caption + chapter data is seeded first (captions.import
//     + edit.add_marker) so every format produces real output.
//
//   CLUSTER 3 — dedicated Library workspace (src/panels/Library/index.tsx; the whole
//     surface was untested): an asset is seeded, then the per-card + folder UI
//     controls — favorite, new-folder, move-to-folder, tag, remove — are each
//     driven and asserted against library.list (the change must be reflected).
//
// ISOLATION: point the throwaway cutd at a temp HOME so the GLOBAL library
//   (~/.shellx-cut/library) and projects index are sandboxed — this gate never
//   pollutes the real library. Run cutd with HOME=<tmp>.
//
// RUN:  cd ui && SWEEP_CUTD=http://127.0.0.1:6203 SWEEP_APP=http://localhost:5203 \
//         node public-tests/verify-topbar-library.mjs
// Exit 0 = no FAIL (PASS/SKIP only); non-zero on any FAIL. SKIP(reason) is used
// only for legitimately-absent DATA (e.g. STT produced no transcript), never to
// hide a missing control — a missing control is always a FAIL.
import { chromium } from "playwright";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdtempSync, existsSync, statSync, readFileSync, writeFileSync, mkdirSync, rmSync, unlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const CUTD = process.env.SWEEP_CUTD || "http://127.0.0.1:6203";
const APP = process.env.SWEEP_APP || "http://localhost:5203";
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const tmp = mkdtempSync(join(tmpdir(), "tlv-"));
let activeBrowser = null;
const rnd = () => Math.random().toString(36).slice(2, 6);

async function resetViewportScroll(page) {
  await page.evaluate(() => {
    window.scrollTo(0, 0);
    document.documentElement.scrollLeft = 0;
    document.body.scrollLeft = 0;
    const root = document.getElementById("root");
    if (root) root.scrollLeft = 0;
  });
}

// Synthetic footage built by the harness (see buildFixtures). Engineered so the
// perception-driven Tools verbs have a real effect to produce.
const FX = {
  // 3 hard-cut colour segments (red|blue|green, 2s each) → ContentDetector fires
  // 2 scene cuts. Drives split_at_scenes / mark_scenes.
  scene: join(tmp, "scene_cut.mp4"),
  // talking_head speech (10s) wrapped in 2s black-silence lead + 2s trail. The
  // gate seeds deterministic transcript words over the speech region so
  // trim_edges can prove it actually trims the dead edges without depending on
  // live STT availability.
  speech: join(tmp, "th_padded.mp4"),
};
const SPEECH_SRC = join(REPO, "testdata", "talking_head.mp4");
const SCENE_PERCEPTION_FIXTURE = "scene_cut.perception.json";

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
// Total timeline duration of the video clips (sum of clip source windows).
function videoClips(s) {
  return (s.tracks || []).filter((t) => t.kind === "video").flatMap((t) => t.clips || []);
}
function videoDurMs(s) {
  return videoClips(s).reduce((acc, c) => acc + ((c.src_out_ms ?? 0) - (c.src_in_ms ?? 0)), 0);
}
// Poll until cutd reports no queued/running jobs (import enrichment / perception).
async function waitJobs(maxS = 240) {
  for (let i = 0; i < maxS * 2; i++) {
    const js = (await verb("jobs.list")).result?.jobs || [];
    if (!js.some((j) => j.state === "queued" || j.state === "running")) return i * 0.5;
    await sleep(500);
  }
  return -1;
}
// Create a fresh project + import a fixture + wait for enrichment. Reload the page
// so the UI binds to the new project (the topbar reads `project` from server truth).
async function freshProject(page, name, fixture, settings) {
  const projectName = `tlv_${name}_${rnd()}`;
  const projectDir = join(tmp, `${projectName}.cutproj`);
  await verb("project.create", { name: projectName, dir: projectDir, settings });
  const imp = await verb("media.import", { path: fixture });
  const asset = imp.result?.asset_id;
  await waitJobs();
  if (fixture === FX.scene && asset) seedScenePerception(projectDir, asset);
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(1200);
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
  await sleep(300);
  return { asset, projectDir };
}

async function seedTrimDeadAirTranscript(page, projectDir, asset) {
  const receipts = join(projectDir, "receipts");
  mkdirSync(receipts, { recursive: true });
  const words = [
    ["Opening", 2200, 2600],
    ["speech", 2850, 3350],
    ["keeps", 5100, 5480],
    ["the", 7350, 7580],
    ["middle", 7700, 8200],
    ["active", 10900, 11550],
  ].map(([word, start_ms, end_ms], idx) => ({
    idx,
    word,
    start_ms,
    end_ms,
    confidence: 1,
  }));
  writeFileSync(join(receipts, `${asset}.words.json`), JSON.stringify({
    asset,
    model: "fixture@topbar-library-trim-dead-air",
    language: "en",
    words,
  }, null, 2));
  try { unlinkSync(join(projectDir, "project.json")); } catch {}
  const reopened = await verb("project.open", { path: projectDir });
  if (!reopened.ok) throw new Error(`reopen after seeded trim transcript failed: ${JSON.stringify(reopened.error || reopened).slice(0, 160)}`);
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(900);
  await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
}

function seedScenePerception(projectDir, asset) {
  const receipts = join(projectDir, "receipts");
  mkdirSync(receipts, { recursive: true });
  const report = JSON.stringify({
    schema: "shellx-cut/perception/1",
    asset_hash: "fixture@topbar-library",
    source_path: SCENE_PERCEPTION_FIXTURE,
    instruments_run: ["scenes"],
    silences: [],
    scenes: [{ at_ms: 2000 }, { at_ms: 4000 }],
    black_spans: [],
    frozen_spans: [],
    content_bbox: null,
  }, null, 2);
  writeFileSync(join(receipts, `${asset}.perception.json`), report);
  writeFileSync(join(tmp, SCENE_PERCEPTION_FIXTURE), report);
}
// Click one global timeline tool by its data-cut-tool id. These used to live in
// the topbar; the release-facing UI now keeps them beside the timeline tools.
async function clickTool(page, tool) {
  if (await page.locator("[data-cut-tools-btn]").count()) throw new Error("old topbar Tools button still present");
  const trigger = page.locator("[data-cut-timeline-automation-trigger]").first();
  if (!(await trigger.count())) throw new Error("Timeline Automate menu trigger missing");
  if ((await trigger.getAttribute("aria-expanded")) !== "true") {
    await trigger.click();
    await sleep(150);
  }
  const group = page.locator("[data-cut-timeline-tools]");
  if (!(await group.count())) throw new Error("Timeline global tools group missing");
  const item = group.locator(`[data-cut-tool="${tool}"]`);
  if (!(await item.count())) throw new Error(`Timeline tool data-cut-tool="${tool}" missing`);
  await item.click();
  await sleep(1500); // the verb (and any ripple/split) commits
}

// ── CLUSTER 1 — Timeline Global Tools ──────────────────────────────────────
async function checkTrimDeadAir(page) {
  // Drive timeline tools → "Trim dead air" (edit.trim_edges) and assert the timeline
  // duration drops. trim_edges anchors on the SPEECH transcript. A missing
  // control is always a FAIL (clickTool throws); transcript words are seeded so
  // the check is not dependent on local STT/model state.
  if (!existsSync(FX.speech)) {
    return { skip: true, detail: `speech fixture not built (source ${SPEECH_SRC} absent) — cannot exercise trim_edges (needs-data)` };
  }
  const { asset, projectDir } = await freshProject(page, "trim", FX.speech, { width: 1280, height: 720, fps: 30 });
  if (!asset) return { pass: false, detail: "media.import did not return an asset id for trim fixture" };
  await seedTrimDeadAirTranscript(page, projectDir, asset);
  const tg = await verb("transcript.get", { asset });
  const words = (tg.result?.words || []).length;
  const before = videoDurMs(await state());
  await clickTool(page, "trim_edges");
  const after = videoDurMs(await state());
  const dropped = before - after;
  if (words === 0) {
    return {
      pass: false,
      detail: `seeded trim transcript did not link; dur ${before}→${after}`,
    };
  }
  return {
    pass: dropped > 0,
    detail: `transcript words=${words}; timeline dur ${before}→${after}ms (dropped ${dropped}; >0 ⇒ dead air trimmed)`,
  };
}

async function checkSplitAtScenes(page) {
  // Timeline tools → "Split at scenes" (edit.split_at_scenes). The scene fixture has 2
  // hard cuts → 1 video clip must become 3. A missing control FAILs (clickTool throws).
  await freshProject(page, "split", FX.scene, { width: 640, height: 360, fps: 30 });
  const before = videoClips(await state()).length;
  await clickTool(page, "split_scenes");
  // Poll for the split to land — scene detection can run >1500ms under aggregate load,
  // and a fixed wait raced it → false-FAIL "clips 1→1" (the flake). Wait up to ~10s.
  let after = before;
  for (let i = 0; i < 20 && after <= before; i++) { after = videoClips(await state()).length; if (after > before) break; await sleep(500); }
  return { pass: after > before, detail: `video clips ${before}→${after} (grew ⇒ split at scene cuts)` };
}

async function checkMarkScenes(page) {
  // Timeline tools → "Mark scenes" (edit.mark_scenes). The scene fixture's 2 cuts must
  // add markers to project.markers.
  await freshProject(page, "mark", FX.scene, { width: 640, height: 360, fps: 30 });
  const before = ((await state()).markers || []).length;
  await clickTool(page, "mark_scenes");
  // Poll for the markers to land (scene detection can run >1500ms under load).
  let after = before;
  for (let i = 0; i < 20 && after <= before; i++) { after = ((await state()).markers || []).length; if (after > before) break; await sleep(500); }
  return { pass: after > before, detail: `markers ${before}→${after} (grew ⇒ scene markers added)` };
}

// ── CLUSTER 2 — Export ▾ menu formats ──────────────────────────────────────
// Seed caption + chapter data so EVERY caption/marker-driven format produces real
// output, then drive each menu item and validate the written file.
async function checkExportFormats(page) {
  // One project for all formats. The small scene fixture imports fast.
  await freshProject(page, "export", FX.scene, { width: 640, height: 360, fps: 30 });

  // Seed captions via captions.import (a written SRT) — deterministic, no STT.
  const srtPath = join(tmp, `seed_${rnd()}.srt`);
  writeFileSync(
    srtPath,
    "1\n00:00:00,200 --> 00:00:01,000\nHello world\n\n2\n00:00:01,200 --> 00:00:02,000\nSecond cue here\n",
  );
  const ci = await verb("captions.import", { path: srtPath });
  const captionsSeeded = (ci.result?.caption_count || 0) >= 1;
  // Seed two markers so export.chapters has real chapters.
  await verb("edit.add_marker", { at_ms: 500, label: "Intro" });
  await verb("edit.add_marker", { at_ms: 3000, label: "Part 2" });

  // Each spec: the menu option id, the response field that carries the file path,
  // and a validator over the on-disk bytes. resolve + fcpxml share an extension
  // (resolve XML is the fcpxml dialect) — capturing the per-call response path
  // (not a guessed filename) keeps them independent even when they overwrite.
  const isXml = (b) => b.slice(0, 64).toString("utf8").includes("<?xml");
  // `verb` lets the response listener match THIS option's export verb specifically.
  // Was a cross-wiring flake: a slow exporter's response (e.g. export.otio) arrived
  // during the NEXT option's window, so "match any export.*" captured the wrong file
  // and it cascaded (chapters↔otio, transcript↔chapters, frame↔transcript). fcpxml/
  // premiere/resolve all use export.xml — harmless, they all validate as XML.
  const specs = [
    { id: "gif", verb: "export.gif", needs: null, ok: (b) => b.slice(0, 6).toString("ascii") === "GIF87a" || b.slice(0, 6).toString("ascii") === "GIF89a", kind: "GIF magic" },
    { id: "srt", verb: "export.srt", needs: "captions", ok: (b) => /^\d+\r?\n\d{2}:\d{2}:\d{2},\d{3} -->/.test(b.toString("utf8").slice(0, 64)), kind: "SRT cue (comma timing)" },
    { id: "vtt", verb: "export.vtt", needs: "captions", ok: (b) => b.toString("utf8").startsWith("WEBVTT"), kind: "WEBVTT header" },
    { id: "ass", verb: "export.ass", needs: "captions", ok: (b) => b.toString("utf8").startsWith("[Script Info]"), kind: "ASS [Script Info]" },
    { id: "fcpxml", verb: "export.xml", needs: null, ok: isXml, kind: "XML" },
    { id: "premiere", verb: "export.xml", needs: null, ok: isXml, kind: "XML" },
    { id: "resolve", verb: "export.xml", needs: null, ok: isXml, kind: "XML" },
    { id: "otio", verb: "export.otio", needs: null, ok: (b) => { try { return typeof JSON.parse(b.toString("utf8")).OTIO_SCHEMA === "string"; } catch { return false; } }, kind: "valid OTIO JSON" },
    { id: "chapters", verb: "export.chapters", needs: "markers", ok: (b) => /^\d+:\d{2}\s+\S/.test(b.toString("utf8")), kind: "chapter list" },
    { id: "transcript", verb: "export.transcript", needs: "captions", ok: (b) => b.toString("utf8").startsWith("# Transcript"), kind: "transcript md" },
    { id: "frame", verb: "export.frame", needs: null, ok: (b) => b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff, kind: "JPEG magic" },
  ];

  // Capture the path from each export verb's RESPONSE (the menu dispatches via
  // /api/verb/export.*); match response → option by sequencing one click at a time.
  const lines = [];
  let fail = 0;
  for (const spec of specs) {
    // Listen for the very next export.* response while we click this one item.
    let captured = null;
    const onResp = async (r) => {
      const u = r.url();
      // Match THIS option's specific verb (not any export.*) so a slow prior export's
      // late response can't bleed into this capture (the cross-wiring flake).
      if (u.includes(`/api/verb/${spec.verb}`) && captured == null) {
        try { captured = await r.json(); } catch { captured = { __unparsed: true }; }
      }
    };
    page.on("response", onResp);
    // Open the Export ▾ menu and click the option.
    const eb = page.locator("[data-cut-export-btn]");
    if (!(await eb.count())) { page.off("response", onResp); throw new Error("Export ▾ button missing"); }
    await eb.click();
    await sleep(250);
    const opt = page.locator(`[data-cut-export-option="${spec.id}"]`);
    if (!(await opt.count())) {
      page.off("response", onResp);
      lines.push(`FAIL  export:${spec.id.padEnd(10)} menu option data-cut-export-option="${spec.id}" MISSING`);
      fail++;
      await page.keyboard.press("Escape").catch(() => {});
      continue;
    }
    await opt.click();
    // FFmpeg-backed exports may first open the same preflight warning a user sees.
    // Continue non-blocking preflight warnings, then wait long enough for GIF
    // render+import on slower CI/local machines.
    let preflight = null;
    for (let i = 0; i < 120 && captured == null; i++) {
      const warning = page.locator("[data-cut-pregate-warning]");
      if ((await warning.count()) > 0 && preflight == null) {
        preflight = await page.evaluate(() => {
          const warning = document.querySelector("[data-cut-pregate-warning]");
          if (!warning) return null;
          return {
            blocked: warning.getAttribute("data-cut-pregate-blocked"),
            risks: Array.from(warning.querySelectorAll("[data-cut-pregate-risk]")).map((el) => ({
              kind: el.getAttribute("data-cut-pregate-risk-kind") || "uninstrumented",
              severity: el.getAttribute("data-severity"),
            })),
          };
        });
        const cont = page.locator("[data-cut-pregate-continue]");
        if ((await cont.count()) > 0 && await cont.isEnabled()) {
          await cont.click();
        }
      }
      await sleep(250);
    }
    page.off("response", onResp);

    const path = captured?.result?.path;
    const verbOk = captured?.ok === true;
    if (!verbOk || !path) {
      // A caption/marker-needing format with no data would error NOT_FOUND — but
      // we SEEDED the data, so this is a genuine failure to report.
      const code = captured?.error?.code ?? (captured == null ? "no-response" : "—");
      const detail = preflight ? ` preflight=${JSON.stringify(preflight)}` : "";
      lines.push(`FAIL  export:${spec.id.padEnd(10)} verb ok=${verbOk} path=${path ?? "—"} err=${code}${detail}`);
      fail++;
      continue;
    }
    if (!existsSync(path)) {
      lines.push(`FAIL  export:${spec.id.padEnd(10)} response path does not exist on disk: ${path}`);
      fail++;
      continue;
    }
    const sz = statSync(path).size;
    if (sz === 0) {
      lines.push(`FAIL  export:${spec.id.padEnd(10)} wrote an EMPTY file: ${path}`);
      fail++;
      continue;
    }
    const bytes = readFileSync(path);
    const typeOk = spec.ok(bytes);
    if (!typeOk) {
      lines.push(`FAIL  export:${spec.id.padEnd(10)} ${sz}B but NOT ${spec.kind} (head=${JSON.stringify(bytes.slice(0, 24).toString("latin1"))})`);
      fail++;
      continue;
    }
    lines.push(`PASS  export:${spec.id.padEnd(10)} ${sz}B ${spec.kind} → …${path.slice(-32)}`);
  }
  // Print the per-format breakdown (this single check fans out into all formats).
  for (const l of lines) console.log(`      ${l}`);
  return {
    pass: fail === 0,
    detail: `${specs.length - fail}/${specs.length} formats produced valid files (captionsSeeded=${captionsSeeded})`,
  };
}

// ── CLUSTER 3 — dedicated Library workspace ────────────────────────────────
async function checkLibrarySurface(page) {
  await resetViewportScroll(page);
  // Seed ONE asset directly (the UI "Browse" picker is Tauri-only / native, so
  // the headless gate seeds via library.add, then drives the per-card + folder UI
  // controls — favorite, new-folder, move, tag, remove — asserting each mutation
  // is reflected by library.list). SAFE-BY-DEFAULT: this check only ever touches
  // the item + folder IT creates — it never wipes pre-existing library entries —
  // so it does no damage even if (against the documented isolation) it runs
  // against a cutd using the real ~/.shellx-cut/library.
  const add = await verb("library.add", { path: FX.scene, source: "user" });
  const id = add.result?.item?.id;
  if (!id) return { pass: false, detail: `library.add seed failed: ${JSON.stringify(add.error || add)}` };

  // Open the top-level Library workspace → the panel activates + loads library.list.
  const launcher = page.locator('[data-cut-library-btn]');
  if (!(await launcher.count())) return { pass: false, detail: 'Library launcher [data-cut-library-btn] missing' };
  await launcher.click();
  const workspace = page.locator('[data-cut-library-workspace]');
  const opened = await workspace.waitFor({ state: 'visible', timeout: 5_000 })
    .then(() => true)
    .catch(() => false);
  if (!opened) {
    return { pass: false, detail: 'dedicated Library workspace did not open' };
  }
  await sleep(800);
  const card = page.locator(`[data-cut-library-card="${id}"]`);
  if (!(await card.count())) {
    // The panel may need a beat to render the seeded item; retry once.
    await sleep(800);
  }
  if (!(await page.locator(`[data-cut-library-card="${id}"]`).count())) {
    return { pass: false, detail: `seeded library card ${id} did not render in the Library workspace` };
  }

  const results = [];
  const get = async () => ((await verb("library.list", { ids: [id], limit: 1 })).result || { items: [], folders: [] });

  // Layout proof: the Library owns the middle workspace, so its primary item
  // actions must be visible without hover/scroll gymnastics. Also guard the
  // selection affordance: unselected rows show an empty box, not a checkmark.
  {
    const layout = await page.evaluate(() => {
      const panel = document.querySelector("[data-cut-library-workspace]");
      const panelBox = panel?.getBoundingClientRect();
      const firstCard = document.querySelector("[data-cut-library-card]");
      const actionSelectors = [
        "[data-cut-library-toproject]",
        "[data-cut-library-insert]",
        "[data-cut-library-move]",
        "[data-cut-library-tagbtn]",
        "[data-cut-library-portable]",
        "[data-cut-library-remove]",
      ];
      const actions = firstCard ? Array.from(firstCard.querySelectorAll(actionSelectors.join(","))) : [];
      const actionMetrics = actions.map((el) => {
        const b = el.getBoundingClientRect();
        const cs = getComputedStyle(el);
        const visible = b.width > 0 && b.height > 0 && cs.visibility !== "hidden" && cs.display !== "none";
        const inPanel = !!panelBox && b.top >= panelBox.top && b.bottom <= panelBox.bottom && b.left >= panelBox.left && b.right <= panelBox.right;
        const inViewport = b.top >= 0 && b.left >= 0 && b.bottom <= innerHeight && b.right <= innerWidth;
        const hook = actionSelectors.find((selector) => el.matches(selector)) ?? el.tagName;
        return { hook, visible, inPanel, inViewport, rect: { top: b.top, bottom: b.bottom, left: b.left, right: b.right, width: b.width, height: b.height } };
      });
      const unselectedSelects = Array.from(document.querySelectorAll('[data-cut-library-select][aria-pressed="false"]'));
      return {
        listMode: !!document.querySelector(".lb-list"),
        browseHasIcon: !!document.querySelector("[data-cut-library-browse] svg"),
        actionMetrics,
        visibleActionCount: actionMetrics.filter((a) => a.visible && a.inPanel && a.inViewport).length,
        hiddenActionCount: actionMetrics.filter((a) => !a.visible || !a.inPanel || !a.inViewport).length,
        unselectedSelectCount: unselectedSelects.length,
        unselectedSelectsWithCheckSvg: unselectedSelects.filter((el) => !!el.querySelector("svg")).length,
        unselectedSelectsWithEmptyBox: unselectedSelects.filter((el) => !!el.querySelector(".lb-select-empty")).length,
      };
    });
    results.push(layout.listMode && layout.browseHasIcon && layout.visibleActionCount >= 5 && layout.hiddenActionCount === 0
      ? "PASS library-actions-visible-in-first-viewport"
      : `FAIL library-actions-visible-in-first-viewport (${JSON.stringify(layout)})`);
    results.push(layout.unselectedSelectCount > 0 && layout.unselectedSelectsWithCheckSvg === 0 && layout.unselectedSelectsWithEmptyBox === layout.unselectedSelectCount
      ? "PASS library-unselected-selects-are-empty-boxes"
      : `FAIL library-unselected-selects-are-empty-boxes (${JSON.stringify(layout)})`);
  }

  // One-item boundary still proves the persistent paging surface and its
  // keyboard-native disabled states; the 1k/10k gate exercises both directions.
  {
    const previous = page.locator("[data-cut-library-page-prev]");
    const next = page.locator("[data-cut-library-page-next]");
    const status = page.locator("[data-cut-library-page-status]");
    const pagingOk = (await previous.count()) === 1
      && (await next.count()) === 1
      && (await status.count()) === 1
      && await previous.isDisabled()
      && await next.isDisabled()
      && /1–1 of 1/.test((await status.textContent()) || "");
    results.push(pagingOk
      ? "PASS library-pagination-boundary"
      : `FAIL library-pagination-boundary (status=${JSON.stringify(await status.textContent().catch(() => null))})`);
  }

  // Conditional missing-source recovery: move the linked fixture, prove Relink
  // becomes visible, then repair it through the same public verb the native
  // picker dispatches. The final installed matrix owns the OS-dialog click.
  {
    const moved = join(tmp, `moved_${rnd()}.mp4`);
    const replacement = join(tmp, `replacement_${rnd()}.mp4`);
    copyFileSync(FX.scene, moved);
    copyFileSync(FX.scene, replacement);
    await verb("library.add", { path: moved, source: "user" });
    unlinkSync(moved);
    await page.locator('[data-cut-library-close]').click();
    await page.locator('[data-cut-library-btn]').click();
    await sleep(500);
    await page.locator('[data-cut-library-collection="missing"]').click();
    await sleep(400);
    const relinkButton = page.locator(`[data-cut-library-relink="${id}"]`);
    const relinkVisible = (await relinkButton.count()) === 1 && await relinkButton.isVisible();
    const relink = await verb("library.relink", { id, path: replacement });
    const repaired = await get();
    const mediaOk = repaired.items.find((item) => item.id === id)?.media_ok === true;
    results.push(relinkVisible && relink.ok && mediaOk
      ? "PASS library-relink-missing-source"
      : `FAIL library-relink-missing-source (visible=${relinkVisible}, ok=${relink.ok}, media_ok=${mediaOk})`);
    await page.locator('[data-cut-library-collection="all"]').click();
    await sleep(350);
  }

  // (1) FAVORITE — click the card's star, assert library.list shows favorite=true.
  {
    const favBtn = page.locator(`[data-cut-library-fav="${id}"]`);
    if (!(await favBtn.count())) results.push("FAIL favorite: button missing");
    else {
      await favBtn.click();
      await sleep(700);
      const it = (await get()).items.find((x) => x.id === id);
      results.push(it?.favorite === true ? "PASS favorite" : `FAIL favorite (favorite=${it?.favorite})`);
    }
  }

  // Collection navigation is a first-class workspace surface, not a decorative
  // rail: exercise every built-in destination and return to All before mutations.
  for (const collection of ["favorites", "missing", "recent", "all"]) {
    const button = page.locator(`[data-cut-library-collection="${collection}"]`);
    if (!(await button.count())) {
      results.push(`FAIL collection-${collection}: button missing`);
      continue;
    }
    await button.click();
    await sleep(350);
    const active = (await button.getAttribute("aria-pressed")) === "true";
    const cardVisible = (await page.locator(`[data-cut-library-card="${id}"]`).count()) > 0;
    const expectedVisible = collection !== "missing";
    results.push(active && cardVisible === expectedVisible
      ? `PASS collection-${collection}`
      : `FAIL collection-${collection} (active=${active}, cardVisible=${cardVisible})`);
  }

  // (2) NEW FOLDER — type into the new-folder input + Enter; assert it appears in
  // library.list folders.
  const folderName = "TLV_" + rnd().toUpperCase();
  {
    const nf = page.locator("[data-cut-library-newfolder]");
    if (!(await nf.count())) results.push("FAIL folder-create: input missing");
    else {
      await nf.fill(folderName);
      await nf.press("Enter");
      await sleep(700);
      const folders = (await get()).folders;
      results.push(folders.includes(folderName) ? "PASS folder-create" : `FAIL folder-create (folders=${JSON.stringify(folders)})`);
    }
  }

  // (3) MOVE — the card's move <select> now lists the new folder; pick it and
  // assert the item's folder updates in library.list.
  {
    const mv = page.locator(`[data-cut-library-move="${id}"]`);
    if (!(await mv.count())) results.push("FAIL move: select missing");
    else {
      await mv.selectOption(folderName).catch(() => {});
      await sleep(700);
      const it = (await get()).items.find((x) => x.id === id);
      results.push(it?.folder === folderName ? "PASS move" : `FAIL move (folder=${it?.folder})`);
    }
  }

  // (4) TAG — open the tag editor, type comma-separated tags, Enter; assert the
  // tag set in library.list.
  {
    const tagBtn = page.locator(`[data-cut-library-tagbtn="${id}"]`);
    if (!(await tagBtn.count())) results.push("FAIL tag: button missing");
    else {
      await tagBtn.click();
      await sleep(300);
      const input = page.locator("[data-cut-library-taginput]");
      if (!(await input.count())) results.push("FAIL tag: input did not open");
      else {
        const tagVal = "alpha, beta";
        await input.fill(tagVal);
        await input.press("Enter");
        await sleep(700);
        const it = (await get()).items.find((x) => x.id === id);
        const got = (it?.tags || []).slice().sort().join(",");
        results.push(got === "alpha,beta" ? "PASS tag" : `FAIL tag (tags=${JSON.stringify(it?.tags)})`);
        const tagFacet = page.locator('[data-cut-library-collection-tag="alpha"]');
        if (!(await tagFacet.count())) {
          results.push("FAIL collection-tag: alpha facet missing");
        } else {
          await tagFacet.click();
          await sleep(350);
          const active = (await tagFacet.getAttribute("aria-pressed")) === "true";
          const filteredCard = (await page.locator(`[data-cut-library-card="${id}"]`).count()) > 0;
          results.push(active && filteredCard
            ? "PASS collection-tag"
            : `FAIL collection-tag (active=${active}, cardVisible=${filteredCard})`);
          await page.locator('[data-cut-library-collection="all"]').click();
          await sleep(350);
        }
      }
    }
  }

  // (5) REMOVE — click the card's ✕, assert the item leaves library.list.
  {
    const rm = page.locator(`[data-cut-library-remove="${id}"]`);
    if (!(await rm.count())) results.push("FAIL remove: button missing");
    else {
      await rm.click();
      await sleep(700);
      const gone = !(await get()).items.some((x) => x.id === id);
      results.push(gone ? "PASS remove" : "FAIL remove (item still listed)");
    }
  }

  // Clean up the folder we created (leave the isolated library tidy).
  await verb("library.folder_remove", { name: folderName }).catch(() => {});
  for (const l of results) console.log(`      ${l}`);
  const fails = results.filter((r) => r.startsWith("FAIL")).length;
  return { pass: fails === 0, detail: `${results.length - fails}/${results.length} library actions reflected in library.list` };
}

// ── fixtures ────────────────────────────────────────────────────────────────
function ff(args) {
  const r = spawnSync("ffmpeg", args, { encoding: "utf8" });
  if (r.status !== 0) throw new Error(`ffmpeg failed: ${(r.stderr || "").slice(-300)}`);
}
function buildFixtures() {
  // Scene-cut clip: red|blue|green 2s each, 30fps + a 6s 440Hz tone (so it has an
  // audio track). Three hard cuts → PySceneDetect ContentDetector emits 2 cuts.
  ff([
    "-y",
    "-f", "lavfi", "-i", "color=c=red:s=640x360:d=2,format=yuv420p",
    "-f", "lavfi", "-i", "color=c=blue:s=640x360:d=2,format=yuv420p",
    "-f", "lavfi", "-i", "color=c=green:s=640x360:d=2,format=yuv420p",
    "-f", "lavfi", "-t", "6", "-i", "sine=frequency=440:sample_rate=44100",
    "-filter_complex", "[0:v][1:v][2:v]concat=n=3:v=1:a=0[v]",
    "-map", "[v]", "-map", "3:a", "-r", "30", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", FX.scene,
  ]);
  // Speech clip: 2s black-silence + 10s of talking_head (real speech) + 2s black-
  // silence → the transcript anchors trim_edges' leading + trailing dead-air trim.
  // (talking_head.mp4 ships in the repo; if absent, the speech fixture is skipped
  // and checkTrimDeadAir degrades to SKIP.)
  if (existsSync(SPEECH_SRC)) {
    ff([
      "-y", "-ss", "0", "-t", "10", "-i", SPEECH_SRC,
      "-f", "lavfi", "-t", "2", "-i", "color=c=black:s=1280x720:r=30",
      "-f", "lavfi", "-t", "2", "-i", "anullsrc=r=44100:cl=stereo",
      "-f", "lavfi", "-t", "2", "-i", "color=c=black:s=1280x720:r=30",
      "-f", "lavfi", "-t", "2", "-i", "anullsrc=r=44100:cl=stereo",
      "-filter_complex",
      "[1:v]format=yuv420p,setsar=1[lv];[2:a]aformat=sample_rates=44100:channel_layouts=stereo[la];" +
        "[0:v]scale=1280:720,setsar=1,format=yuv420p[mv];[0:a]aformat=sample_rates=44100:channel_layouts=stereo[ma];" +
        "[3:v]format=yuv420p,setsar=1[tv];[4:a]aformat=sample_rates=44100:channel_layouts=stereo[ta];" +
        "[lv][la][mv][ma][tv][ta]concat=n=3:v=1:a=1[v][a]",
      "-map", "[v]", "-map", "[a]", "-r", "30", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", FX.speech,
    ]);
  }
}

// ── runner ──────────────────────────────────────────────────────────────────
async function main() {
  buildFixtures();

  const browser = activeBrowser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  const httpErrors = [];
  page.on("response", (r) => {
    if (r.status() >= 400 && !/favicon|\/api\/frame|\/filmstrip\/|\/proxies\/|\/api\/source\/|\/api\/export\/_monitor_/.test(r.url())) {
      httpErrors.push(`HTTP ${r.status()} ${r.url().replace(/^https?:\/\/[^/]+/, "")}`);
    }
  });
  await page.goto(APP, { waitUntil: "domcontentloaded" });
  await sleep(1000);

  const results = [];
  const run = async (name, fn) => {
    try {
      const r = await fn(page);
      results.push({ name, ...r });
    } catch (e) {
      results.push({ name, pass: false, detail: String(e.message || e).slice(0, 160) });
    }
  };

  console.log("\n== VERIFY TOPBAR + LIBRARY ==");
  // trim-dead-air runs first because it is the highest-signal Tools proof:
  // the deterministic transcript should turn the padded-speech fixture into a
  // measurable duration drop.
  await run("trim-dead-air", checkTrimDeadAir);
  await run("split-at-scenes", checkSplitAtScenes);
  await run("mark-scenes", checkMarkScenes);
  await run("export-formats", checkExportFormats);
  await run("library-surface", checkLibrarySurface);

  await browser.close();
  activeBrowser = null;

  let fail = 0;
  let skip = 0;
  console.log("");
  for (const r of results) {
    const tag = r.skip ? "SKIP" : r.pass ? "PASS" : "FAIL";
    console.log(`  ${tag}  ${r.name.padEnd(18)} ${r.detail}`);
    if (r.skip) skip++;
    else if (!r.pass) fail++;
  }
  if (httpErrors.length) console.log(`  NOTE  console-http       ${httpErrors.slice(0, 4).join(" | ")}`);
  const pass = results.length - fail - skip;
  console.log(`\n${pass} PASS, ${fail} FAIL, ${skip} SKIP  (${results.length} checks)`);
  process.exitCode = fail ? 1 : 0;
}
main().catch((e) => {
  console.error(e);
  process.exitCode = 2;
}).finally(async () => {
  if (activeBrowser) await activeBrowser.close().catch(() => {});
  rmSync(tmp, { recursive: true, force: true });
});
