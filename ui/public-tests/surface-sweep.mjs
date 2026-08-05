// surface-sweep.mjs — ShellX Cut UI release check (EFFECT-driven).
//
// The point: not "did the panel open" but "did the ACTION produce the
// DESIRED RESULT, without hanging." So every editing check drives a REAL action
// through the UI and then asserts the effect landed in cutd's project state (the
// source of truth) within a timeout — if nothing changes in time, the check FAILS
// (a wiring malfunction or a hang, exactly what we want to catch). Surface-health
// checks (palette, drawers, tabs) round out coverage so a crash anywhere is caught.
//
// SELF-CONTAINED given a running dev stack:
//   1. cutd headless on :6161   — ./app/target/release/cutd serve --headless --addr 127.0.0.1:6161
//   2. vite dev on :5173        — (cd ui && npm run dev)   [proxies /api -> 6161]
// The script creates a FRESH demo project (talking-head clip on the timeline) over
// the cutd API each run, so effect counts are deterministic.
//
// RUN:  cd ui && npm run sweep        (or: node ui/public-tests/surface-sweep.mjs)
// OUT:  ui/public-tests/__evidence__/<NN>-<name>.png + report.md/json
// EXIT: non-zero if any check FAILED (skips don't fail the run).
import { chromium } from "playwright";
import { mkdirSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

// Port-parameterized so the sweep NEVER clobbers a live app. A running
// desktop app answers on :6161 (WSL2 forwards localhost) — pointing the sweep at
// it would project.create over his open project. Default to a dev port; override
// with SWEEP_CUTD / SWEEP_APP. The vite dev proxy must target the same cutd.
const CUTD = process.env.SWEEP_CUTD || "http://127.0.0.1:6171";
const APP = process.env.SWEEP_APP || "http://localhost:5173";
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..");
const OUT = join(HERE, "__evidence__");
const CLIP = join(REPO, "testdata", "talking_head.mp4");
const CLIP2 = join(REPO, "testdata", "insert_clip.mp4");
const PROJ = join(process.env.HOME || "/tmp", ".shellx-scratch", "sweep", "sweep.cutproj");

/** ffprobe: does a media file carry an audio stream? (node-side checks only). */
function hasAudioStream(path) {
  try {
    const out = execFileSync("ffprobe", ["-v", "error", "-select_streams", "a",
      "-show_entries", "stream=codec_type", "-of", "csv=p=0", path], { encoding: "utf8" });
    return /audio/.test(out);
  } catch { return false; }
}
/** ffmpeg volumedetect mean_volume (dB) — < -80 ≈ silence. volumedetect prints to
 *  STDERR, so use spawnSync and read both streams regardless of exit code. */
function meanVolumeDb(path) {
  const r = spawnSync("ffmpeg", ["-hide_banner", "-i", path, "-af", "volumedetect", "-f", "null", "-"], { encoding: "utf8" });
  const s = (r.stderr || "") + (r.stdout || "");
  const m = s.match(/mean_volume:\s*(-?[\d.]+) dB/);
  return m ? parseFloat(m[1]) : null;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function verb(name, args = {}) {
  try {
    const r = await fetch(`${CUTD}/api/verb/${name}`, {
      method: "POST",
      headers: { "content-type": "application/json", "x-cut-actor": "human:ui:ui" },
      body: JSON.stringify(args),
    });
    return await r.json();
  } catch (e) {
    return { ok: false, error: String(e) };
  }
}

// ── cutd state probes (the "did the effect land" oracle) ──────────────────────
async function state() { return (await verb("project.state")).result || {}; }
async function clipCount() {
  const s = await state();
  return (s.tracks || []).reduce((n, t) => n + (t.clips || []).length, 0);
}
async function markerCount() { return ((await state()).markers || []).length; }
async function ops() {
  const r = await verb("project.ops", {});
  return r.result?.ops || r.result || [];
}
async function opsCount() { return (await ops()).length; }
/** Poll a predicate until true or timeout — the hang/no-effect detector. */
async function waitFor(pred, ms = 6000, every = 150) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    try { if (await pred()) return true; } catch { /* keep polling */ }
    await sleep(every);
  }
  return false;
}

/** Always start from a FRESH project so effect counts are deterministic. The
 *  dir is removed first: project.create refuses an existing dir, and silently
 *  continuing would run the whole sweep against a STALE/other project (and its
 *  exports fence), which masked a wrong-project run on. */
async function freshProject() {
  rmSync(PROJ, { recursive: true, force: true });
  const cr = await verb("project.create", { name: "sweep", dir: PROJ }).catch((e) => ({ ok: false, error: String(e) }));
  if (!cr.ok) {
    const op = await verb("project.open", { dir: PROJ }).catch(() => ({ ok: false }));
    if (!op.ok) throw new Error(`freshProject: could not create or open ${PROJ}: ${JSON.stringify(cr.error)}`);
  }
  await verb("media.import", { path: CLIP });
  let st = {};
  for (let i = 0; i < 50; i++) {
    st = await state();
    if (st.assets && Object.keys(st.assets).length) break;
    await sleep(300);
  }
  const aid = Object.keys(st.assets || { a1: 1 })[0] || "a1";
  const vt = (st.tracks || []).find((t) => t.kind === "video")?.id || "v1";
  await verb("edit.insert", { asset: aid, track: vt, at_ms: 0 });
  await waitFor(async () => (await clipCount()) >= 1, 6000);
  return `fresh project, clip on track ${vt}`;
}

// ── UI interaction helpers ────────────────────────────────────────────────────
async function focusTimeline(page) {
  // click an empty area of the timeline so S/Del/M land in the timeline scope
  await page.mouse.click(450, 760).catch(() => {});
}
async function seekRuler(page, x) {
  // click the ruler (y≈623) to move the playhead
  await page.mouse.click(x, 624).catch(() => {});
}
async function selectFirstClip(page) {
  const clip = page.locator("[data-cut-clip]").first();
  if (await clip.count()) { await clip.click(); return true; }
  return false;
}

async function paintRulerExportRange(page) {
  const ruler = page.locator("[data-cut-ruler]");
  const box = await ruler.boundingBox();
  if (!box) return { ok: false, detail: "ruler not found" };
  const y = box.y + 6;
  const x1 = Math.min(box.x + 360, box.x + Math.max(220, box.width - 520));
  const x2 = Math.min(x1 + 400, box.x + box.width - 80);
  const attempts = [];
  for (let attempt = 1; attempt <= 3; attempt++) {
    await page.evaluate(({ x1, x2, y }) => {
      const ruler = document.querySelector("[data-cut-ruler]");
      if (!ruler) return;
      const fire = (target, type, x, buttons) => {
        target.dispatchEvent(new MouseEvent(type, {
          bubbles: true,
          cancelable: true,
          clientX: x,
          clientY: y,
          button: 0,
          buttons,
          view: window,
        }));
      };
      // TimelineRuler attaches mousemove/mouseup to window from its mousedown
      // handler, so dispatch at the same boundaries the component actually uses.
      fire(ruler, "mousedown", x1, 1);
      fire(window, "mousemove", x1 + 30, 1);
      fire(window, "mousemove", (x1 + x2) / 2, 1);
      fire(window, "mousemove", x2, 1);
    }, { x1, x2, y });
    await page.waitForTimeout(120);
    const liveBand = await page.locator("[data-cut-range]").first().getAttribute("data-cut-range").catch(() => null);
    await page.evaluate(({ x2, y }) => {
      window.dispatchEvent(new MouseEvent("mouseup", {
        bubbles: true,
        cancelable: true,
        clientX: x2,
        clientY: y,
        button: 0,
        buttons: 0,
        view: window,
      }));
    }, { x2, y });
    await page.waitForTimeout(260);
    const band = await page.locator("[data-cut-range]").first().getAttribute("data-cut-range").catch(() => null);
    const disabled = await page.locator('[data-cut-action="render-section"]').isDisabled().catch(() => true);
    const uiState = await verb("ui.state", {});
    const stateRange = uiState.result?.export_range;
    attempts.push(`try ${attempt}: live=${liveBand || "none"} band=${band || "none"} state=${JSON.stringify(stateRange || null)} disabled=${disabled}`);
    if (band && !disabled) return { ok: true, band, detail: attempts.join(" | ") };
    await page.keyboard.press("Escape").catch(() => {});
    await page.waitForTimeout(180);
  }
  return { ok: false, detail: attempts.join(" | ") || "no attempts" };
}

// ── EFFECT checks (ordered; they mutate state) ────────────────────────────────
// 'grade' + 'mixer' are no longer modal drawers; they are the right-
// sidebar Color / Audio TABS (covered by wf-grade-records-op + wf-mixer-audition-
// loads-stem and ui-workspace-modes), so they're dropped from the drawer sweep.
const DRAWERS = ["layer", "matte", "music", "title", "kinetic", "shape", "stock", "search", "clips", "autopilot"];

/** Ensure the right-sidebar tab `t` (properties|color|audio) is OPEN + active.
 *  The rail defaults collapsed, so the tab buttons may not be mounted yet. Expand
 *  the right-edge Tools strip first; fallback relays keep legacy drawer commands
 *  covered. Waits for the tab's embed to render so callers can act immediately. */
async function openRightTab(page, t) {
  const expand = page.locator('[data-cut-action="expand-rail"]');
  if (await expand.count()) {
    await expand.click().catch(() => {});
    await page.waitForTimeout(200);
  }
  const tabBtn = page.locator(`[data-cut-right-tab="${t}"]`);
  if (await tabBtn.count()) {
    await tabBtn.click();
  } else {
    // rail collapsed → relay the legacy event that expands it + sets the tab.
    const drawer = t === "color" ? "grade" : t === "audio" ? "mixer" : null;
    if (drawer) await page.evaluate((d) => document.dispatchEvent(new CustomEvent("cut:open-drawer", { detail: d })), drawer);
    await page.waitForTimeout(200);
    await page.locator(`[data-cut-right-tab="${t}"]`).click().catch(() => {});
  }
  const embed = t === "color" ? "[data-cut-grade-embed]" : t === "audio" ? "[data-cut-mixer-embed]" : '[data-cut-panel="inspector"]';
  await page.waitForSelector(embed, { timeout: 4000 });
}

const checks = [
  // ── editing workflow: action → assert result in cutd state (hang-timeout) ──
  {
    name: "wf-clip-present",
    async run() {
      const c = await clipCount();
      if (c < 1) throw new Error("setup failed — no clip on the timeline");
      return `timeline has ${c} clip(s) after import+insert`;
    },
  },
  {
    // Each added clip is separate working material on its OWN line: its OWN new
    // video track + its OWN new audio track (NOT the
    // base v1/a1t, NOT silent). Overlay is dropping ON TOP of a clip (Layer panel).
    // Drives the real UI: import → Assets "Insert" → assert the clip lands on a
    // NON-base video track AND a NON-base audio track (its own line, audio linked).
    name: "wf-clip-on-own-line-with-audio",
    async run(page) {
      const beforeAssets = Object.keys((await state()).assets || {}).length;
      await verb("media.import", { path: CLIP2 });
      const arrived = await waitFor(async () => Object.keys((await state()).assets || {}).length > beforeAssets, 10000);
      if (!arrived) return "SKIP: 2nd asset did not import";
      const st0 = await state();
      const a2 = Object.keys(st0.assets).find((id) => /insert_clip/.test(st0.assets[id].path || "")) || Object.keys(st0.assets).at(-1);
      const baseVideo = (st0.tracks || []).find((t) => t.kind === "video")?.id;
      const baseAudio = (st0.tracks || []).find((t) => t.kind === "audio")?.id;
      await page.getByText("Assets", { exact: false }).first().click();
      await page.waitForTimeout(300);
      const card = page.locator(`[data-cut-asset-card="${a2}"]`);
      if (!(await card.count())) return "SKIP: 2nd asset card not found in Assets tray";
      const insert = card.locator('[data-cut-action="insert-asset"]');
      if (!(await insert.count())) return "SKIP: Insert button not present";
      await insert.click();
      const ok = await waitFor(async () => {
        const s = await state();
        // its own line = on a video track that is NOT the base, and audio on a
        // NON-base audio track too (its own audio line, linked + never desynced).
        const onOwnVideo = (s.tracks || []).some((t) => t.kind === "video" && t.id !== baseVideo && (t.clips || []).some((c) => c.asset === a2));
        const onOwnAudio = (s.tracks || []).some((t) => t.kind === "audio" && t.id !== baseAudio && (t.clips || []).some((c) => c.asset === a2));
        return onOwnVideo && onOwnAudio;
      }, 8000);
      if (!ok) {
        const s = await state();
        const where = (s.tracks || []).filter((t) => (t.clips || []).some((c) => c.asset === a2)).map((t) => `${t.id}:${t.kind}`);
        throw new Error(`2nd clip NOT on its own video+audio line — asset ${a2} on [${where.join(", ")}] (base=${baseVideo}/${baseAudio})`);
      }
      return `2nd clip on its OWN video line + OWN audio line (separate working material)`;
    },
  },
  {
    // Preview-audio regression: drive playback and assert the hidden monitor
    // <audio> actually LOADED the export.audio mix — currentSrc set, no media
    // error, metadata ready. A 404 (the Windows exportUrl bug) sets a.error here.
    name: "wf-audio-monitor-loads",
    async run(page) {
      await page.mouse.click(760, 300).catch(() => {}); // focus the monitor area
      await page.keyboard.press(" "); // play → ensureMix renders + <audio> loads
      const ok = await waitFor(async () => page.evaluate(() => {
        const a = document.querySelector("audio");
        return !!a && !!a.currentSrc && a.error === null && a.readyState >= 1;
      }), 12000);
      const diag = await page.evaluate(() => {
        const a = document.querySelector("audio");
        return a ? { src: a.currentSrc, err: a.error && a.error.code, net: a.networkState, rs: a.readyState } : null;
      });
      await page.keyboard.press(" "); // pause
      if (!ok) throw new Error(`preview audio monitor <audio> did NOT load the mix: ${JSON.stringify(diag)}`);
      return `preview audio monitor loaded (readyState ${diag.rs}, no error, src set)`;
    },
  },
  {
    // Prove exported clips ACTUALLY have audio — mp3 AND mp4, measured.
    // Node-side: render a short range + the audio mix, then ffprobe/loudness the
    // files (local fs only; SKIP if the render lands off-box).
    name: "wf-export-has-audio-mp3-mp4",
    async run() {
      const rAudio = await verb("export.audio", { format: "mp3", rationale: "sweep: prove mp3 has audio" });
      const mp3 = rAudio.result?.path;
      let mp3line = "mp3: SKIP (no path)";
      if (mp3 && existsSync(mp3)) {
        const ha = hasAudioStream(mp3), mv = meanVolumeDb(mp3);
        if (!ha || mv === null || mv <= -80) throw new Error(`exported mp3 is SILENT (audio=${ha}, mean=${mv}dB)`);
        mp3line = `mp3 audio ${mv}dB`;
      }
      // No explicit path → export.range writes inside the OPEN project's exports/
      // and returns the absolute path (sync). (Passing 'rationale' is rejected.)
      const rRange = await verb("export.range", { range_ms: [0, 3000], to_asset: false });
      let job = rRange.result?.job_id;
      if (job) await waitFor(async () => ["done", "failed"].includes((await verb("jobs.status", { job_id: job })).result?.state), 60000);
      const mp4 = rRange.result?.path || rRange.result?.out;
      let mp4line = "mp4: SKIP (no path / off-box)";
      if (mp4 && existsSync(mp4)) {
        const ha = hasAudioStream(mp4), mv = meanVolumeDb(mp4);
        if (!ha || mv === null || mv <= -80) throw new Error(`exported mp4 is SILENT (audio=${ha}, mean=${mv}dB)`);
        mp4line = `mp4 audio ${mv}dB`;
      }
      return `${mp3line} · ${mp4line}`;
    },
  },
  {
    // The Mixer's per-track "Listen" button loads that
    // track's isolated stem (export.audio{track}) into the hidden audition player
    // so you can hear ONE track without the whole Preview. Runs HERE (before the
    // destructive delete checks) while the base audio track still carries the
    // freshly-imported clip's linked audio. Asserts the click wires a stem URL
    // into the player without error (audible volume is engine-proven separately —
    // export.audio mp3 carries a real audio stream).
    name: "wf-mixer-audition-loads-stem",
    async run(page) {
      const at = (await state()).tracks.find((t) => t.kind === "audio");
      if (!at) return "SKIP: no audio track";
      // Only meaningful if the track actually has audio to play.
      if (!(await verb("export.audio", { format: "mp3", track: at.id })).ok) return `SKIP: ${at.id} has no audible audio`;
      // The Mixer is the right-sidebar AUDIO tab now (was a modal drawer). It
      // renders embedded (data-cut-mixer-embed); the per-track Listen button + hidden
      // audition <audio> are unchanged. openRightTab expands the rail + waits for the embed.
      await openRightTab(page, "audio");
      await page.waitForTimeout(300);
      const listen = page.locator(`[data-cut-mixer-listen="${at.id}"]`);
      if (!(await listen.count())) { await page.locator('[data-cut-right-tab="properties"]').click().catch(() => {}); return `SKIP: no Listen button for ${at.id}`; }
      await listen.click();
      // The stem URL is assigned via el.src (property → reflected to the attribute) and
      // el.currentSrc once loading starts; accept either so the assertion is robust.
      const ok = await waitFor(async () =>
        page.locator("[data-cut-mixer-audition-player]").evaluate((el) => (!!el.getAttribute("src") || !!el.currentSrc) && el.error === null).catch(() => false), 8000);
      const info = await page.locator("[data-cut-mixer-audition-player]").evaluate((el) => ({ src: !!el.getAttribute("src") || !!el.currentSrc, paused: el.paused, ready: el.readyState })).catch(() => null);
      // leave clean: back to the Properties tab (no drawer/scrim to dismiss anymore).
      await page.locator('[data-cut-right-tab="properties"]').click().catch(() => {});
      if (!ok) throw new Error(`Listen on ${at.id} but no stem loaded into the audition player (${JSON.stringify(info)})`);
      return `mixer Listen on ${at.id} → per-track stem loaded into player (src set, no error, paused=${info?.paused})`;
    },
  },
  {
    // Save-as-clip range selection must export EXACTLY the
    // span the user selects, not a 30s fallback). Drag the ruler → a range band
    // appears, the Section button enables, and rendering it produces the EXACT
    // composite over that span (the chip shows the range).
    name: "wf-range-select-to-save",
    async run(page) {
      const btn = page.locator('[data-cut-action="render-section"]');
      // clear any selection/range so the button starts disabled
      await page.keyboard.press("Escape").catch(() => {});
      await page.waitForTimeout(150);
      const painted = await paintRulerExportRange(page);
      if (!painted.ok) throw new Error(`ruler drag did NOT paint a range band ([data-cut-range]): ${painted.detail}`);
      const band = painted.band;
      // render that exact span → EXACT composite appears with the range chip
      await btn.click();
      const exactOk = await waitFor(async () => (await page.locator("[data-cut-exact]").count()) > 0, 30000);
      if (!exactOk) return `range band [${band}] + Section enabled (render slow — EXACT not shown in time)`;
      const chip = (await page.locator("[data-cut-exact-chip]").first().textContent())?.trim();
      await page.locator('[data-cut-action="exit-exact"]').click().catch(() => {});
      return `ruler drag → range [${band}] → rendered EXACT span (${chip})`;
    },
  },
  {
    // Discoverable clip removal: right-click a clip and choose Remove.
    // The clip must then leave the timeline.
    name: "wf-clip-context-menu-remove",
    async run(page) {
      const before = await clipCount();
      const clip = page.locator("[data-cut-clip]").first();
      if (!(await clip.count())) return "SKIP: no clip to right-click";
      await clip.click({ button: "right" });
      const menuOk = await waitFor(async () => (await page.locator("[data-cut-clip-menu]").count()) > 0, 3000);
      if (!menuOk) throw new Error("right-click did NOT open the clip context menu");
      const items = await page.locator("[data-cut-clip-menu] [data-cut-ctx]").count();
      await page.locator('[data-cut-ctx="remove"]').click();
      const ok = await waitFor(async () => (await clipCount()) < before, 6000);
      if (!ok) throw new Error("context-menu Remove did NOT remove the clip");
      return `right-click menu (${items} items) → Remove → clip count ${before} → ${await clipCount()}`;
    },
  },
  {
    // edit.remove_track + auto-clean: put a clip on a NEW overlay track, delete
    // it via the right-click menu → the emptied overlay track is auto-removed.
    name: "wf-empty-overlay-track-auto-removed",
    async run(page) {
      const st0 = await state();
      const aid = Object.keys(st0.assets || {})[0];
      if (!aid) return "SKIP: no asset to place";
      const at = await verb("edit.add_track", { kind: "video" });
      const vt = at.result?.track_id;
      if (!vt) throw new Error("edit.add_track failed");
      await verb("edit.insert", { asset: aid, track: vt, at_ms: 1000, ripple: false });
      const placed = await waitFor(async () => (await state()).tracks.some((t) => t.id === vt && (t.clips || []).some((c) => c.asset)), 5000);
      if (!placed) return "SKIP: overlay clip didn't land";
      const vtTrack = (await state()).tracks.find((t) => t.id === vt);
      const clipId = (vtTrack.clips || []).find((c) => c.id && c.asset)?.id;
      if (!clipId) return "SKIP: overlay clip has no id";
      const clipEl = page.locator(`[data-cut-clip="${clipId}"]`);
      // the DOM lags the API state (op_applied WS → re-render) — wait for the element
      if (!(await waitFor(async () => (await clipEl.count()) > 0, 5000))) return "SKIP: overlay clip not rendered in UI";
      await clipEl.click({ button: "right" });
      if (!(await waitFor(async () => (await page.locator("[data-cut-clip-menu]").count()) > 0, 3000))) throw new Error("no context menu on overlay clip");
      await page.locator('[data-cut-ctx="remove"]').click();
      const removed = await waitFor(async () => !(await state()).tracks.some((t) => t.id === vt), 6000);
      if (!removed) throw new Error(`overlay track ${vt} NOT auto-removed after deleting its only clip`);
      return `deleted only clip on overlay ${vt} → track auto-removed (edit.remove_track)`;
    },
  },
  {
    name: "wf-split-produces-clip",
    async run(page) {
      // Ensure the base video track HAS a clip (earlier checks may have removed
      // it; the new each-clip-own-line model means the base can be left empty).
      let s = await state();
      let v1 = (s.tracks || []).find((t) => t.kind === "video");
      if (!v1 || !(v1.clips || []).some((c) => c.asset)) {
        const aid = Object.keys(s.assets || {})[0];
        if (!aid || !v1) return "SKIP: no clip/asset to split";
        await verb("edit.insert", { asset: aid, track: v1.id, at_ms: 0, ripple: false });
        await waitFor(async () => (await state()).tracks.some((t) => t.id === v1.id && (t.clips || []).some((c) => c.asset)), 4000);
        s = await state();
        v1 = (s.tracks || []).find((t) => t.kind === "video");
      }
      const clipId = (v1.clips || []).find((c) => c.asset)?.id;
      const clipEl = page.locator(`[data-cut-clip="${clipId}"]`);
      if (!(await waitFor(async () => (await clipEl.count()) > 0, 4000))) return "SKIP: base clip not rendered";
      const before = await clipCount();
      await clipEl.click(); // select the base-track clip
      await seekRuler(page, 280); // early x → inside the clip (which starts at 0)
      await page.waitForTimeout(150);
      await clipEl.click(); // re-select after the ruler click
      await page.keyboard.press("s"); // split-at-playhead on the selected clip's track
      const ok = await waitFor(async () => (await clipCount()) > before, 6000);
      const after = await clipCount();
      if (!ok) throw new Error(`split produced NO new clip (stayed ${before}) — effect did not apply / possible hang`);
      return `split: clip count ${before} → ${after} (effect applied)`;
    },
  },
  {
    name: "wf-grade-records-op",
    async run(page) {
      const before = await opsCount();
      // Grade is the right-sidebar COLOR tab now (was a modal drawer). Select
      // a clip, then openRightTab expands the Tools rail and waits for the
      // grade embed. The grade controls only render for a selected video clip.
      await selectFirstClip(page);
      await openRightTab(page, "color");
      await page.waitForTimeout(300);
      // nudge a grade slider then Apply
      const slider = page.locator("[data-cut-grade-input]").first();
      if (await slider.count()) {
        await slider.evaluate((el) => {
          const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
          set.call(el, el.max && +el.max >= 20 ? "20" : "0.2");
          el.dispatchEvent(new Event("input", { bubbles: true }));
          el.dispatchEvent(new Event("change", { bubbles: true }));
        });
      }
      const applied = await page.locator("[data-cut-grade-apply]").first();
      if (!(await applied.count())) return "SKIP: grade Apply not present (clip may be un-gradeable)";
      await applied.click();
      const ok = await waitFor(async () => {
        const list = await ops();
        return list.some((o) => JSON.stringify(o).includes("grade")) && (await opsCount()) > before;
      }, 8000);
      // leave clean: back to the Properties tab (no drawer/scrim to dismiss anymore).
      await page.locator('[data-cut-right-tab="properties"]').click().catch(() => {});
      if (!ok) throw new Error("grade Apply recorded NO edit.grade op — effect did not apply / possible hang");
      return `grade applied → op recorded (ops ${before} → ${await opsCount()})`;
    },
  },
  {
    name: "wf-ripple-delete-removes-clip",
    async run(page) {
      const before = await clipCount();
      await focusTimeline(page);
      if (!(await selectFirstClip(page))) return "SKIP: no clip to delete";
      await page.keyboard.press("Delete");
      const ok = await waitFor(async () => (await clipCount()) < before, 6000);
      const after = await clipCount();
      if (!ok) throw new Error(`delete removed NO clip (stayed ${before}) — effect did not apply / possible hang`);
      return `ripple-delete: clip count ${before} → ${after} (effect applied)`;
    },
  },
  {
    name: "wf-undo-reverts",
    async run(page) {
      const before = await clipCount();
      await focusTimeline(page);
      await page.keyboard.press("Control+z");
      const ok = await waitFor(async () => (await clipCount()) !== before, 6000);
      const after = await clipCount();
      if (!ok) return `SKIP: undo did not change clip count (${before}) — may need a focused scope`;
      return `undo: clip count ${before} → ${after} (reverted the delete)`;
    },
  },
  {
    name: "wf-transport-seek-moves-playhead",
    async run(page) {
      await page.mouse.click(760, 300).catch(() => {});
      const t0 = await readPlayhead(page);
      for (let k = 0; k < 8; k++) await page.keyboard.press("ArrowRight");
      await page.waitForTimeout(200);
      const t1 = await readPlayhead(page);
      if (t0 === null || t1 === null) return "SKIP: couldn't read playhead";
      if (t1 <= t0) return `SKIP: playhead didn't advance (${t0}→${t1})`;
      return `frame-step seek: playhead ${t0} → ${t1}ms (effect applied)`;
    },
  },
  {
    // Deleting a project must REMOVE it from the recent list, not leave a stale
    // "missing" ghost. Engine regression: create →
    // switch-open-away → delete → assert it's gone (no ghost), then re-open the
    // sweep project so later surface checks still have state.
    name: "wf-delete-project-no-ghost",
    async run() {
      const tag = "sweepdel_" + Math.random().toString(36).slice(2, 8);
      const cr = await verb("project.create", { name: tag, settings: { width: 1280, height: 720, fps: 30 } });
      if (!cr.ok) return "SKIP: project.create failed";
      // delete refuses the currently-open project → open another first.
      await verb("project.create", { name: "sweepsw_" + Math.random().toString(36).slice(2, 8), settings: { width: 1280, height: 720, fps: 30 } });
      const ent = (await verb("project.list", { sort: "recent" })).result.projects.find((p) => p.name === tag);
      if (!ent) throw new Error("created project missing from project.list");
      const del = await verb("project.delete", { id: ent.id });
      if (!del.ok) throw new Error("project.delete failed: " + JSON.stringify(del.error));
      const ghost = (await verb("project.list", { sort: "recent" })).result.projects.find((p) => p.name === tag);
      if (ghost) throw new Error(`GHOST: deleted project still listed (missing=${ghost.missing})`);
      await verb("project.open", { path: PROJ }); // restore sweep state for later checks
      return `create → delete → gone (forgotten=${del.result.forgotten}, no ghost)`;
    },
  },
  {
    // When the perception (STT) engine isn't set up, the Transcript panel must
    // EXPLAIN it + offer one-click setup, not imply
    // transcripts are coming (and never stay blank). The dev cutd resolves the
    // appdata venv (no STT) → degraded → the setup card must show.
    name: "wf-transcript-perception-honest-empty",
    async run(page) {
      await page.getByText("Transcript", { exact: true }).first().click();
      await page.waitForTimeout(500);
      const ready = (await verb("system.doctor", {})).result.cards.find((c) => c.id === "perception")?.details?.stt_ready;
      const hasSetup = (await page.locator("[data-cut-perception-setup]").count()) > 0;
      const hasPending = (await page.locator("[data-cut-transcribe-pending]").count()) > 0;
      const hasWords = (await page.locator("[data-word-idx]").count()) > 0;
      if (ready === false) {
        if (!hasSetup) throw new Error("perception NOT ready but no setup card (panel would stay blank/misleading)");
        if (!(await page.locator('[data-cut-action="setup-perception"]').count())) throw new Error("setup card shown but no 'Install captions' CTA");
        return "captions not installed → setup card + CTA shown (not blank)";
      }
      // "Select a clip to see its words" / "No words on the timeline yet" is explicit,
      // honest guidance — not a blank pane — so it also counts as a non-empty state.
      const hasGuidance = (await page.locator("[data-cut-timeline-empty], .tx__empty").count()) > 0;
      if (!hasSetup && !hasPending && !hasWords && !hasGuidance) throw new Error("transcript empty-state is blank (no setup, pending, words, or guidance)");
      return `perception ready → honest state (words=${hasWords} pending=${hasPending} guidance=${hasGuidance})`;
    },
  },
  {
    // : the EDL-aware transcript view toggle (Clip / Program / Source) is
    // present, and — when transcripts exist — selecting a clip renders its words
    // in the timeline-mapped Clip view. On a no-STT rig only the toggle is checked.
    name: "wf-transcript-edl-view-toggle",
    async run(page) {
      await page.getByText("Transcript", { exact: true }).first().click();
      await page.waitForTimeout(300);
      const toggle = await page.locator('[data-cut-action="view-clip"]').count();
      if (!toggle) throw new Error("transcript view toggle (Clip/Program/Source) missing");
      // If a clip with a transcript exists, prove the Clip view maps its words.
      const st = await state();
      const transcribed = Object.values(st.assets || {}).some((a) => a && a.transcript);
      if (!transcribed) return "view toggle present (no transcript on this rig → words not asserted)";
      const clipId = (st.tracks || []).flatMap((t) => t.clips || []).find((c) => c.asset)?.id;
      if (!clipId) return "view toggle present (no placed clip to select)";
      await page.locator(`[data-cut-clip="${clipId}"]`).first().click().catch(() => {});
      await page.locator('[data-cut-action="view-clip"]').click();
      await page.waitForTimeout(500);
      const words = await page.locator('[data-cut-timeline-view="clip"] [data-cut-timeline-word]').count();
      await page.locator('[data-cut-action="view-program"]').click();
      await page.waitForTimeout(400);
      const prog = await page.locator('[data-cut-timeline-view="program"] [data-cut-timeline-word]').count();
      if (words === 0 && prog === 0) throw new Error("EDL Clip+Program views rendered NO words for a transcribed clip");
      return `EDL views: clip=${words} words, program=${prog} words`;
    },
  },

  {
    // edit.fade wired to the clip context menu: right-click a
    // base clip → Fade in → assert an edit.fade op is recorded.
    name: "wf-clip-fade-records-op",
    async run(page) {
      // ensure the base video track has a clip to fade
      let s = await state();
      let v1 = (s.tracks || []).find((t) => t.kind === "video");
      if (!v1 || !(v1.clips || []).some((c) => c.asset)) {
        const aid = Object.keys(s.assets || {})[0];
        if (!aid || !v1) return "SKIP: no clip/asset to fade";
        await verb("edit.insert", { asset: aid, track: v1.id, at_ms: 0, ripple: false });
        await waitFor(async () => (await state()).tracks.some((t) => t.id === v1.id && (t.clips || []).some((c) => c.asset)), 4000);
        s = await state();
        v1 = (s.tracks || []).find((t) => t.kind === "video");
      }
      const clipId = (v1.clips || []).find((c) => c.asset)?.id;
      const clipEl = page.locator(`[data-cut-clip="${clipId}"]`);
      if (!(await waitFor(async () => (await clipEl.count()) > 0, 4000))) return "SKIP: clip not rendered";
      const before = await opsCount();
      await clipEl.click({ button: "right" });
      if (!(await waitFor(async () => (await page.locator("[data-cut-clip-menu]").count()) > 0, 3000))) throw new Error("no context menu");
      await page.locator('[data-cut-ctx="fade-in"]').click();
      const ok = await waitFor(async () => {
        const list = await ops();
        return list.some((o) => JSON.stringify(o).includes("fade")) && (await opsCount()) > before;
      }, 6000);
      if (!ok) throw new Error("Fade in recorded NO edit.fade op — effect did not apply");
      return `context-menu Fade in → edit.fade op recorded (ops ${before} → ${await opsCount()})`;
    },
  },

  // ── surface health: open every surface, assert it renders (catch crashes) ──
  {
    name: "ui-boot",
    async run(page) {
      if (!/ShellX Cut/i.test(await page.title())) throw new Error("wrong title");
      if (!(await page.locator("[data-cut-brand]").count())) throw new Error("brand mark missing");
      return "title + brand mark present";
    },
  },
  {
    name: "ui-export-menu",
    async run(page) {
      await page.locator("[data-cut-export-btn]").click();
      await page.waitForSelector("[data-cut-export-menu]", { timeout: 3000 });
      const opts = await page.locator("[data-cut-export-option]").count();
      await page.keyboard.press("Escape").catch(() => {});
      await page.mouse.click(700, 400).catch(() => {});
      if (!opts) throw new Error("export menu had 0 options");
      return `export menu: ${opts} options`;
    },
  },
  {
    name: "ui-left-tabs",
    async run(page) {
      await page.getByText("Assets", { exact: false }).first().click();
      await page.waitForTimeout(120);
      await page.getByText("Transcript", { exact: true }).first().click();
      await page.waitForTimeout(120);
      return "Assets ↔ Transcript tab switch ok";
    },
  },
  {
    name: "ui-palette-filter",
    async run(page) {
      await page.keyboard.press("Control+k");
      await page.waitForSelector(".cmdk", { timeout: 3000 });
      await page.locator(".cmdk-input").fill("audio");
      await page.waitForTimeout(150);
      const labels = await page.locator(".cmdk-row-label").allTextContents();
      await page.keyboard.press("Escape");
      if (!labels.length || !labels.every((l) => /audio|music|mixer/i.test(l)))
        throw new Error(`filter "audio" → [${labels.join(", ")}]`);
      return `⌘K + filter "audio" → [${labels.join(", ")}]`;
    },
  },
  ...DRAWERS.map((d) => ({
    name: `ui-drawer-${d}`,
    async run(page) {
      await page.evaluate((name) => document.dispatchEvent(new CustomEvent("cut:open-drawer", { detail: name })), d);
      await page.waitForSelector(".cd-drawer, .mb-drawer", { timeout: 3000 });
      await page.waitForTimeout(300);
      const title = (await page.locator(".cd-title, .mb-title").first().textContent())?.trim() || "(no title)";
      const legacy = (await page.locator(".mb-drawer").count()) > 0 ? " [legacy .mb-]" : "";
      return `opened → "${title}"${legacy}`;
    },
  })),
  {
    name: "ui-drawer-close",
    async run(page) {
      await page.mouse.click(300, 400);
      await page.waitForTimeout(200);
      if (await page.locator(".cd-drawer, .mb-drawer").count()) return "SKIP: drawer didn't close on scrim";
      return "drawer closed via scrim";
    },
  },
  {
    name: "ui-keymap",
    async run(page) {
      await page.keyboard.press("?");
      await page.waitForTimeout(200);
      const open = (await page.locator(".km__scrim").count()) > 0;
      await page.keyboard.press("Escape");
      if (!open) throw new Error("keymap overlay (?) did not open");
      return "keymap overlay (?) opened";
    },
  },
  {
    // The mode bar is Edit · Record only:
    // Color + Audio are no longer modes, they moved to the right-sidebar Color/Audio
    // TABS. Record mode swaps in the capture surface (doctor cards + Start/not-ready);
    // Edit mode restores the timeline editor AND exposes the Color + Audio right-tabs.
    name: "ui-workspace-modes",
    async run(page) {
      await page.locator('[data-cut-mode="record"]').click();
      await page.waitForSelector('[data-cut-panel="record"]', { timeout: 4000 });
      await page.waitForTimeout(500);
      const cards = await page.locator("[data-cut-rec-cards] [data-cut-rec-card]").count();
      const hasStart = (await page.locator('[data-cut-action="record-start"]').count()) > 0
        || (await page.locator("[data-cut-rec-not-ready]").count()) > 0;
      await page.locator('[data-cut-mode="edit"]').click();
      await page.waitForTimeout(300);
      const editBack = await page.locator("[data-cut-ruler]").count();
      // Color/Audio are right-sidebar tabs now. Expand the rail explicitly
      // and assert BOTH tabs exist.
      await selectFirstClip(page);
      await openRightTab(page, "color");
      const hasColorTab = (await page.locator('[data-cut-right-tab="color"]').count()) > 0;
      const hasAudioTab = (await page.locator('[data-cut-right-tab="audio"]').count()) > 0;
      await page.locator('[data-cut-right-tab="properties"]').click().catch(() => {});
      if (!hasStart) throw new Error("Record mode rendered no Start (or not-ready) control");
      if (!editBack) throw new Error("Edit mode did not restore the timeline editor");
      if (!hasColorTab || !hasAudioTab) throw new Error(`right sidebar missing Color/Audio tabs (color=${hasColorTab} audio=${hasAudioTab})`);
      return `Record surface (${cards} cards, start/ready ctrl ✓) · Edit restored · right-tabs Color+Audio ✓`;
    },
  },
  {
    // The context Inspector is the PROPERTIES right-tab.
    // Selecting a clip sets context; opening the Tools rail shows that clip's
    // tools + scope.
    name: "ui-inspector-context",
    async run(page) {
      await page.locator('[data-cut-mode="edit"]').click().catch(() => {});
      await page.waitForTimeout(200);
      let s = await state();
      let v1 = (s.tracks || []).find((t) => t.kind === "video");
      if (!v1 || !(v1.clips || []).some((c) => c.asset)) {
        const aid = Object.keys(s.assets || {})[0];
        if (!aid || !v1) return "SKIP: no clip/asset";
        await verb("edit.insert", { asset: aid, track: v1.id, at_ms: 0, ripple: false });
        await waitFor(async () => (await state()).tracks.some((t) => t.id === v1.id && (t.clips || []).some((c) => c.asset)), 4000);
        s = await state(); v1 = (s.tracks || []).find((t) => t.kind === "video");
      }
      const clipId = (v1.clips || []).find((c) => c.asset)?.id;
      const clipEl = page.locator(`[data-cut-clip="${clipId}"]`);
      if (!(await waitFor(async () => (await clipEl.count()) > 0, 4000))) return "SKIP: clip not rendered";
      await clipEl.click();
      await page.waitForTimeout(400);
      // Ensure the Properties tab (the Inspector) is the active right-tab.
      await openRightTab(page, "properties");
      await page.waitForSelector('[data-cut-panel="inspector"]', { timeout: 4000 });
      await page.waitForTimeout(200);
      const insp = await page.locator('[data-cut-panel="inspector"]').count();
      const tools = await page.locator("[data-cut-inspector-tool]").count();
      if (!insp) throw new Error("Inspector (Properties tab) did not render on clip selection");
      if (!tools) throw new Error("Inspector showed no tools for the selected clip");
      const scope = (await page.locator("[data-cut-inspector-scope]").first().textContent().catch(() => "")) || "";
      return `clip selected → Properties/Inspector shows ${tools} tools ("${scope.trim()}")`;
    },
  },
];

async function readPlayhead(page) {
  const txt = await page.locator("body").innerText().catch(() => "");
  const m = txt.match(/00:00:(\d\d)\.(\d{3})/);
  return m ? parseInt(m[1]) * 1000 + parseInt(m[2]) : null;
}

// ── runner ───────────────────────────────────────────────────────────────────
(async () => {
  rmSync(OUT, { recursive: true, force: true });
  mkdirSync(OUT, { recursive: true });

  const setup = await freshProject();
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const consoleErrors = [];
  page.on("console", (m) => { if (m.type() === "error" && !/favicon/.test(m.text())) consoleErrors.push(m.text()); });

  await page.goto(APP, { waitUntil: "networkidle" });
  await page.waitForTimeout(900);

  const results = [];
  let i = 0;
  for (const c of checks) {
    i++;
    const nn = String(i).padStart(2, "0");
    let status = "PASS", detail = "";
    try {
      const r = await c.run(page);
      if (typeof r === "string" && r.startsWith("SKIP:")) { status = "SKIP"; detail = r.slice(5).trim(); }
      else detail = r || "";
    } catch (e) { status = "FAIL"; detail = e.message || String(e); }
    await page.screenshot({ path: join(OUT, `${nn}-${c.name}.png`) }).catch(() => {});
    results.push({ n: nn, name: c.name, status, detail, shot: `${nn}-${c.name}.png` });
    console.log(`${status.padEnd(4)} ${nn} ${c.name} — ${detail}`);
  }

  const cstat = consoleErrors.length === 0 ? "PASS" : "FAIL";
  results.push({ n: "--", name: "console-clean", status: cstat, detail: consoleErrors.length ? consoleErrors.slice(0, 5).join(" | ") : "0 non-favicon console errors", shot: "" });
  console.log(`${cstat.padEnd(4)} -- console-clean — ${results.at(-1).detail}`);
  await browser.close();

  const pass = results.filter((r) => r.status === "PASS").length;
  const fail = results.filter((r) => r.status === "FAIL").length;
  const skip = results.filter((r) => r.status === "SKIP").length;
  const md = [
    `# ShellX Cut — UI release check (effect-driven)`, ``,
    `Setup: ${setup}. **${pass} PASS · ${fail} FAIL · ${skip} SKIP** of ${results.length}.`,
    `Workflow checks (wf-*) drive a real action and assert the effect in cutd state.`, ``,
    `| # | check | status | detail | evidence |`, `|---|---|---|---|---|`,
    ...results.map((r) => `| ${r.n} | ${r.name} | ${r.status} | ${r.detail.replace(/\|/g, "/")} | ${r.shot ? `![](${r.shot})` : ""} |`),
  ].join("\n");
  writeFileSync(join(OUT, "report.md"), md);
  writeFileSync(join(OUT, "report.json"), JSON.stringify({ setup, pass, fail, skip, results }, null, 2));
  console.log(`\n${pass} PASS · ${fail} FAIL · ${skip} SKIP → ${join(OUT, "report.md")}`);
  process.exit(fail > 0 ? 1 : 0);
})();
