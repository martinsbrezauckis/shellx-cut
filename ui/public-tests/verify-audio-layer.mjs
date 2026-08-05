// verify-audio-layer.mjs — effect-proof gate for Mixer and Layer human-edit
// controls. Standalone from the shared interaction and release gates.
//
// Both clusters drive the REAL UI control (Playwright click/drag/select on the
// actual data-cut hooks) AND assert the ENGINE EFFECT (the op landed with the
// right args / the project.state field changed), not merely that "an op recorded".
// Each check FAILS HARD if its UI control is missing — it never falls back to
// firing the verb directly, because that would hide broken wiring (the whole point
// of the gate). Where a control reaches the engine but the engine state does NOT
// change, that's reported as a real BROKEN-wiring bug.
//
// CLUSTER 1 — Mixer levels (panels/Mixer, right-sidebar "Audio" tab):
//   fader → track gain_db changes · mute → Track.muted flag toggles without
//   changing gain_db · solo → Track.solo flag isolates audibility without changing
//   any gain_db · add-audio-track → new audio track appears in project.state.
// CLUSTER 2 — Layer transform (panels/Layer, scrim drawer on an OVERLAY clip):
//   apply transform → clip.transform set · apply crop → clip.crop set · reverse /
//   freeze checkboxes → clip.reverse / clip.freeze set · add keyframe → clip.keyframes.
//
// FIXTURE: talking_head.mp4 (has BOTH video + audio) auto-places as the timeline
// wedge on the FIRST import → a v1 video track + an a1t audio track, both with a
// clip. The Mixer therefore shows a real audio track to fade/mute/solo. The Layer
// overlay is a SECOND import (silent_screen.mp4) on a fresh video track via
// edit.add_track + edit.insert (a 2nd import is NOT auto-placed).
//
// RUN:  cd ui && SWEEP_CUTD=http://127.0.0.1:6201 SWEEP_APP=http://localhost:5201 \
//         node public-tests/verify-audio-layer.mjs
// Exit 0 = all PASS; non-zero on any FAIL (CI/gate friendly).
import { chromium } from "playwright";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const CUTD = process.env.SWEEP_CUTD || "http://127.0.0.1:6201";
const APP = process.env.SWEEP_APP || "http://localhost:5201";
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..");
const CLIP = process.env.RELEASE_CLIP || join(REPO, "testdata", "talking_head.mp4");
const CLIP2 = process.env.RELEASE_CLIP2 || join(REPO, "testdata", "silent_screen.mp4");
const ONLY = new Set((process.env.CUT_AUDIO_LAYER_ONLY || "").split(",").filter(Boolean));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ── engine helpers (same shape as interaction-verify.mjs) ────────────────────
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
const cnt = async (page, sel) => await page.locator(sel).count();
const vis = async (page, sel) => await page.locator(sel).first().isVisible().catch(() => false);

async function ensureRightRail(page) {
  const expand = page.locator('[data-cut-action="expand-rail"]');
  if (await expand.count()) {
    await expand.click().catch(() => {});
    await sleep(250);
  }
}

async function resetRightTab(page) {
  await ensureRightRail(page);
  await page.locator('[data-cut-right-tab="properties"]').click({ timeout: 1500 }).catch(() => {});
  await sleep(250);
}

/** Spin up a clean project + auto-placed talking-head wedge, reload the page into
 *  a clean Edit-mode editor. Returns {videoTrackId, audioTrackId} from the wedge. */
async function freshWedge(page, tag) {
  const projectName = `al_${tag}_` + Math.random().toString(36).slice(2, 6);
  await verb("project.create", {
    name: projectName,
    settings: { width: 1280, height: 720, fps: 30 },
  });
  await verb("media.import", { path: CLIP });
  // Auto-place is async (import chain → edit.insert on v1 + a1t). Poll for the
  // audio track to carry a clip before we drive the UI.
  let s = null;
  for (let i = 0; i < 20; i++) {
    await sleep(400);
    s = await state();
    const a = (s.tracks || []).find((t) => t.kind === "audio" && (t.clips || []).some((c) => c.asset));
    if (a) break;
  }
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(1200);
  await page.locator('[data-cut-mode="edit"]').click({ timeout: 1500 }).catch(() => {});
  await sleep(400);
  const videoTrackId = (s.tracks || []).find((t) => t.kind === "video" && (t.clips || []).some((c) => c.asset))?.id;
  const audioTrackId = (s.tracks || []).find((t) => t.kind === "audio" && (t.clips || []).some((c) => c.asset))?.id;
  const listed = (await verb("project.list", { sort: "recent" })).result?.projects || [];
  const projectPath = listed.find((entry) => entry.name === projectName)?.path;
  return { videoTrackId, audioTrackId, projectName, projectPath };
}

/** Open the right-sidebar "Audio" tab (the embedded Mixer). The right tabs only
 *  render after a clip is selected — select the wedge's video clip first, then
 *  click the Audio tab and assert the mixer body is visible. Throws on a missing
 *  control so a broken tab can't pass silently. */
async function openMixer(page, videoTrackId) {
  const s = await state();
  const clip = (s.tracks || []).find((t) => t.id === videoTrackId)?.clips?.find((c) => c.asset)?.id;
  if (clip) {
    await page.locator(`[data-cut-clip="${clip}"]`).click({ timeout: 5000 }).catch(() => {});
    await sleep(400);
  }
  await ensureRightRail(page);
  const tab = page.locator('[data-cut-right-tab="audio"]');
  if (!(await tab.count())) throw new Error("right-sidebar Audio tab not rendered (clip not selected?)");
  await tab.click();
  await sleep(600);
  if (!(await vis(page, "[data-cut-mixer-embed]"))) throw new Error("Mixer body did not mount on the Audio tab");
}

/** Read a track's server gain_db from project.state. */
async function gainOf(trackId) {
  const s = await state();
  const t = (s.tracks || []).find((x) => x.id === trackId);
  return t?.gain_db ?? 0;
}

/** Read the persistent track mute/solo flags and derived mix audibility. */
async function trackFlags(trackId) {
  const s = await state();
  const tracks = s.tracks || [];
  const anySolo = tracks.some((x) => x.kind === "audio" && x.solo === true);
  const track = tracks.find((x) => x.id === trackId) || {};
  return {
    muted: track.muted === true,
    solo: track.solo === true,
    gain_db: track.gain_db ?? 0,
    audible: track.kind === "audio" && track.muted !== true && (!anySolo || track.solo === true),
  };
}

// ── CLUSTER 1 — Mixer levels ─────────────────────────────────────────────────

async function checkMixerFader(page) {
  // Drag the audio track's fader → assert that track's gain_db in project.state
  // changed (the fader commits one edit.gain on pointer-up; onChange only drafts).
  const { videoTrackId, audioTrackId } = await freshWedge(page, "fader");
  if (!audioTrackId) return { pass: false, detail: "no auto-placed audio track" };
  await openMixer(page, videoTrackId);
  const fader = page.locator(`[data-cut-mixer-fader="${audioTrackId}"]`);
  if (!(await fader.count())) return { pass: false, detail: `no fader for ${audioTrackId} (control missing — broken wiring)` };
  const before = await gainOf(audioTrackId);
  // The fader is an <input type=range> that COMMITS on pointer-up (onPointerUp →
  // edit.gain), not on a bare value set. Set a new value, fire input (updates the
  // draft), then dispatch a real pointerup so the commit handler runs.
  const target = before > -20 ? before - 9 : before + 9; // a clear, in-range move
  await fader.evaluate((el, v) => {
    const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
    set.call(el, String(v));
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
  }, target);
  // Poll project.state until the server gain reflects the commit.
  let after = before;
  for (let i = 0; i < 16; i++) {
    await sleep(250);
    after = await gainOf(audioTrackId);
    if (Math.abs(after - before) > 1) break;
  }
  const changed = Math.abs(after - target) < 1.5; // committed to ~the dialed value
  return {
    pass: changed,
    detail: `gain_db ${before.toFixed(1)} → ${after.toFixed(1)} dB (target ${target.toFixed(1)}; fader committed=${changed})`,
  };
}

async function checkMixerMute(page) {
  // Click Mute → assert the track's persistent Track.muted flag toggles and the
  // derived mix audibility goes false. Gain is non-destructive and must stay at
  // the dialed value across mute/unmute.
  const { videoTrackId, audioTrackId } = await freshWedge(page, "mute");
  if (!audioTrackId) return { pass: false, detail: "no auto-placed audio track" };
  await openMixer(page, videoTrackId);
  const muteBtn = page.locator(`[data-cut-mixer-mute="${audioTrackId}"]`);
  if (!(await muteBtn.count())) return { pass: false, detail: `no Mute button for ${audioTrackId} (broken wiring)` };
  const before = await trackFlags(audioTrackId);
  await muteBtn.click();
  let track = before;
  for (let i = 0; i < 16; i++) {
    await sleep(250);
    track = await trackFlags(audioTrackId);
    if (track.muted === true) break;
  }
  const muteOk = track.muted === true && track.audible === false;
  const gainPreserved = Math.abs(track.gain_db - before.gain_db) < 0.01;
  // Unmute → flag clears and the same gain remains.
  await muteBtn.click();
  let clear = track;
  for (let i = 0; i < 16; i++) {
    await sleep(250);
    clear = await trackFlags(audioTrackId);
    if (clear.muted === false) break;
  }
  const unmuteOk = clear.muted === false && clear.audible === true;
  const unmuteGainPreserved = Math.abs(clear.gain_db - before.gain_db) < 0.01;
  return {
    pass: muteOk && unmuteOk && gainPreserved && unmuteGainPreserved,
    detail: `muted ${before.muted}→${track.muted}→${clear.muted}; audible ${before.audible}→${track.audible}→${clear.audible}; gain ${before.gain_db.toFixed(1)}→${track.gain_db.toFixed(1)}→${clear.gain_db.toFixed(1)} preserved=${gainPreserved && unmuteGainPreserved}`,
  };
}

async function checkMixerSolo(page) {
  // Solo one real audio track → assert a SECOND real audio track becomes
  // non-audible. Video tracks contribute pixels only and must expose no audio
  // controls in either Mixer or timeline.
  const { videoTrackId, audioTrackId } = await freshWedge(page, "solo");
  if (!audioTrackId) return { pass: false, detail: "no auto-placed audio track" };
  const s = await state();
  const assetId = (s.tracks || [])
    .find((t) => t.id === audioTrackId)?.clips?.find((c) => c.asset)?.asset;
  if (!assetId) return { pass: false, detail: `no source asset on ${audioTrackId}` };
  const added = await verb("edit.add_track", { kind: "audio", rationale: "solo isolation proof" });
  const otherId = added.result?.track_id;
  if (!otherId) return { pass: false, detail: "could not create second audio track" };
  const inserted = await verb("edit.insert", {
    asset: assetId,
    track: otherId,
    at_ms: 0,
    src_range_ms: [0, 5000],
    ripple: false,
    rationale: "solo isolation proof",
  });
  if (!inserted.ok) return { pass: false, detail: `could not seed ${otherId}: ${inserted.error?.message}` };
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(1200);
  await openMixer(page, videoTrackId);
  const deadVideoControls = await page.locator([
    `[data-cut-mixer-fader="${videoTrackId}"]`,
    `[data-cut-mixer-mute="${videoTrackId}"]`,
    `[data-cut-mixer-solo="${videoTrackId}"]`,
    `[data-cut-mixer-listen="${videoTrackId}"]`,
    `[data-cut-mute-track="${videoTrackId}"]`,
    `[data-cut-solo-track="${videoTrackId}"]`,
  ].join(",")).count();
  if (deadVideoControls !== 0) {
    return { pass: false, detail: `${deadVideoControls} dead audio control(s) exposed for video track ${videoTrackId}` };
  }
  if (!(await page.locator(`[data-cut-mixer-fader="${otherId}"]`).count())) {
    return { pass: false, detail: `no mixer strip for second audio track ${otherId}` };
  }
  const soloBtn = page.locator(`[data-cut-mixer-solo="${audioTrackId}"]`);
  if (!(await soloBtn.count())) return { pass: false, detail: `no Solo button for ${audioTrackId} (broken wiring)` };
  const before = await trackFlags(audioTrackId);
  const otherBefore = await trackFlags(otherId);
  await soloBtn.click();
  let track = before;
  let otherAfter = otherBefore;
  for (let i = 0; i < 16; i++) {
    await sleep(250);
    track = await trackFlags(audioTrackId);
    otherAfter = await trackFlags(otherId);
    if (track.solo === true) break;
  }
  const soloOk = track.solo === true && track.audible === true && otherAfter.solo === false && otherAfter.audible === false;
  const gainPreserved =
    Math.abs(track.gain_db - before.gain_db) < 0.01 &&
    Math.abs(otherAfter.gain_db - otherBefore.gain_db) < 0.01;
  // un-solo → clear the flag and restore the other track's derived audibility.
  await soloBtn.click();
  let clear = track;
  let otherRestored = otherAfter;
  for (let i = 0; i < 16; i++) {
    await sleep(250);
    clear = await trackFlags(audioTrackId);
    otherRestored = await trackFlags(otherId);
    if (clear.solo === false) break;
  }
  const unsoloOk = clear.solo === false && otherRestored.audible === true;
  const unsoloGainPreserved =
    Math.abs(clear.gain_db - before.gain_db) < 0.01 &&
    Math.abs(otherRestored.gain_db - otherBefore.gain_db) < 0.01;
  return {
    pass: soloOk && unsoloOk && gainPreserved && unsoloGainPreserved,
    detail: `solo ${before.solo}→${track.solo}→${clear.solo}; audio-only controls=true; other(${otherId}) audible ${otherBefore.audible}→${otherAfter.audible}→${otherRestored.audible}; gain preserved=${gainPreserved && unsoloGainPreserved}`,
  };
}

async function checkMixerAddAudioTrack(page) {
  // Click "+ Add audio track" → assert a NEW audio track appears in project.state
  // (one more audio track than before).
  const { videoTrackId } = await freshWedge(page, "addaudio");
  await openMixer(page, videoTrackId);
  const before = (await state()).tracks.filter((t) => t.kind === "audio").length;
  const addBtn = page.locator("[data-cut-mixer-add-audio]");
  if (!(await addBtn.count())) return { pass: false, detail: "no '+ Add audio track' button (broken wiring)" };
  await addBtn.click();
  let after = before;
  for (let i = 0; i < 16; i++) { await sleep(250); after = (await state()).tracks.filter((t) => t.kind === "audio").length; if (after > before) break; }
  return { pass: after === before + 1, detail: `audio tracks ${before} → ${after} (expected +1)` };
}

// ── CLUSTER 2 — Layer transform (overlay clip) ───────────────────────────────

/** Build an OVERLAY clip on a non-base video track + open the Layer drawer on it.
 *  Returns the overlay clip id. Throws if the overlay never lands or the Layer
 *  drawer / open-layer control is missing (so broken wiring fails hard). */
async function overlayAndOpenLayer(page, tag, speedFactor = 1) {
  const wedge = await freshWedge(page, tag); // talking-head wedge = base v1/a1t
  const imp = await verb("media.import", { path: CLIP2 }); // 2nd import → NOT auto-placed
  const a2 = imp.result?.asset_id;
  await sleep(1500); // let the probe finish (insert + crop need source dims)
  const at = await verb("edit.add_track", { kind: "video", rationale: "overlay" });
  const ovTrackId = at.result?.track_id || (await state()).tracks.filter((t) => t.kind === "video").pop()?.id;
  await verb("edit.insert", { asset: a2, track: ovTrackId, at_ms: 0 });
  // Poll for the inserted overlay clip (don't assume an id).
  let ovClip;
  for (let i = 0; i < 14; i++) {
    await sleep(400);
    const ot = (await state()).tracks.find((t) => t.id === ovTrackId);
    ovClip = (ot?.clips || []).find((c) => c.asset === a2)?.id;
    if (ovClip) break;
  }
  if (!ovClip) throw new Error("overlay clip never landed");
  if (speedFactor !== 1) {
    const retimed = await verb("edit.speed", { clip: ovClip, factor: speedFactor, rationale: "layer verifier retime" });
    if (!retimed.ok) throw new Error(`overlay retime failed: ${retimed.error?.message || 'unknown error'}`);
    await page.reload({ waitUntil: "domcontentloaded" });
    await sleep(900);
    await page.locator('[data-cut-mode="edit"]').click({ timeout: 1500 }).catch(() => {});
  }
  await openLayerForClip(page, ovClip);
  return { ovClip, ovTrackId, ...wedge };
}

/** Select an existing video clip and open its bound Layer drawer. */
async function openLayerForClip(page, clipId) {
  await page.locator(`[data-cut-clip="${clipId}"]`).waitFor({ timeout: 8000 }).catch(() => {});
  await page.locator(`[data-cut-clip="${clipId}"]`).click({ timeout: 5000 }).catch(() => {});
  await sleep(400);
  const openBtn = page.locator('[data-cut-action="open-layer"]');
  if (!(await openBtn.count())) throw new Error("no open-layer control on the timeline toolbar");
  await openBtn.click();
  await sleep(600);
  if (!(await vis(page, "[data-cut-layer]"))) throw new Error("Layer drawer did not open");
  // The drawer must have BOUND to the clip (not the 'select a clip' empty state).
  if (await vis(page, "[data-cut-layer-empty]")) throw new Error("Layer drawer opened on empty state (clip not bound)");
}

/** Look up the live clip object from project.state by id. */
async function clipById(id) {
  const s = await state();
  for (const t of s.tracks || []) for (const c of t.clips || []) if (c.id === id) return c;
  return null;
}

async function checkLayerTransform(page) {
  // Move the X/Y/Scale/Opacity sliders, click "Apply layer" → assert the clip's
  // transform fields are SET in project.state (the intended effect). The focused
  // stack contract below separately proves the composed pixel result.
  const { ovClip } = await overlayAndOpenLayer(page, "xform");
  // Set each transform slider to a non-identity value via the real range inputs.
  const setSlider = async (attr, value) => {
    const el = page.locator(`[data-cut-layer-input="${attr}"]`);
    if (!(await el.count())) throw new Error(`missing layer slider ${attr}`);
    await el.evaluate((node, v) => {
      const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
      set.call(node, String(v));
      node.dispatchEvent(new Event("input", { bubbles: true }));
      node.dispatchEvent(new Event("change", { bubbles: true }));
    }, value);
    await sleep(80);
  };
  await setSlider("scale", 0.5);
  await setSlider("x", 0.2);
  await setSlider("y", 0.15);
  await setSlider("opacity", 0.7);
  const applyBtn = page.locator("[data-cut-layer-apply]");
  if (!(await applyBtn.count())) return { pass: false, detail: "no 'Apply layer' button (broken wiring)" };
  await applyBtn.click();
  let tf = null;
  for (let i = 0; i < 16; i++) {
    await sleep(250);
    tf = (await clipById(ovClip))?.transform;
    if (tf && (tf.scale ?? 1) !== 1) break;
  }
  // The transform must be non-identity and reflect the dialed values (~tolerance).
  const ok =
    !!tf &&
    Math.abs((tf.scale ?? 1) - 0.5) < 0.05 &&
    Math.abs((tf.x ?? 0) - 0.2) < 0.05 &&
    Math.abs((tf.y ?? 0) - 0.15) < 0.05 &&
    Math.abs((tf.opacity ?? 1) - 0.7) < 0.05;
  return { pass: ok, detail: `clip.transform=${JSON.stringify(tf)} (expected ~scale0.5 x0.2 y0.15 op0.7)` };
}

async function checkLayerCrop(page) {
  // Set the crop sliders + "Apply crop" → assert clip.crop is SET in project.state.
  const { ovClip } = await overlayAndOpenLayer(page, "crop");
  // The crop section only renders sliders once the asset probe gives dims. If the
  // pending hint shows, the clip isn't probed yet — wait + re-open is overkill;
  // the probe completed in overlayAndOpenLayer's 1.5s wait (silent_screen is tiny).
  if (await vis(page, "[data-cut-layer-crop-pending]")) {
    return { pass: false, detail: "crop section stuck on 'probe pending' (no source dims)" };
  }
  const setSlider = async (attr, value) => {
    const el = page.locator(`[data-cut-layer-input="${attr}"]`);
    if (!(await el.count())) throw new Error(`missing crop slider ${attr}`);
    await el.evaluate((node, v) => {
      const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
      set.call(node, String(v));
      node.dispatchEvent(new Event("input", { bubbles: true }));
      node.dispatchEvent(new Event("change", { bubbles: true }));
    }, value);
    await sleep(80);
  };
  // A non-identity crop (inset rectangle) so the engine stores it (identity clears).
  await setSlider("crop_x", 40);
  await setSlider("crop_y", 30);
  await setSlider("crop_w", 320);
  await setSlider("crop_h", 240);
  const cropBtn = page.locator("[data-cut-layer-crop-apply]");
  if (!(await cropBtn.count())) return { pass: false, detail: "no 'Apply crop' button (broken wiring)" };
  await cropBtn.click();
  let crop = null;
  for (let i = 0; i < 16; i++) {
    await sleep(250);
    crop = (await clipById(ovClip))?.crop;
    if (crop) break;
  }
  const ok = !!crop && crop.w > 0 && crop.h > 0 && (crop.x > 0 || crop.y > 0);
  return { pass: ok, detail: `clip.crop=${JSON.stringify(crop)} (expected an inset rect)` };
}

async function checkLayerReverse(page) {
  // Tick the Reverse checkbox → assert clip.reverse === true; untick → it clears.
  // Use click() not check(): the checkbox is CONTROLLED (checked={reverse}, set only
  // AFTER the async edit.reverse round-trips), so Playwright's check()/uncheck()
  // strict post-assertion races the verb and false-fails. click() fires the same
  // onChange; the engine-state assertion below is the real proof (and still fails
  // hard if the verb never lands — broken wiring can't slip through).
  const { ovClip } = await overlayAndOpenLayer(page, "reverse");
  const box = page.locator("[data-cut-layer-reverse]");
  if (!(await box.count())) return { pass: false, detail: "no Reverse checkbox (broken wiring)" };
  await box.click(); // tick
  let on = false;
  for (let i = 0; i < 16; i++) { await sleep(250); on = (await clipById(ovClip))?.reverse === true; if (on) break; }
  await box.click(); // untick
  let off = on;
  for (let i = 0; i < 16; i++) { await sleep(250); off = !(await clipById(ovClip))?.reverse; if (off) break; }
  return { pass: on && off, detail: `clip.reverse on=${on} → cleared=${off}` };
}

async function checkLayerFreeze(page) {
  // Tick the Freeze checkbox → assert clip.freeze is SET (an {at_ms} object).
  // click() not check() (controlled checkbox — same reason as Reverse above).
  const { ovClip } = await overlayAndOpenLayer(page, "freeze");
  const box = page.locator("[data-cut-layer-freeze]");
  if (!(await box.count())) return { pass: false, detail: "no Freeze checkbox (broken wiring)" };
  await box.click(); // tick
  let fz = null;
  for (let i = 0; i < 16; i++) { await sleep(250); fz = (await clipById(ovClip))?.freeze; if (fz) break; }
  const on = !!fz && typeof fz.at_ms === "number";
  await box.click(); // untick
  let off = on;
  for (let i = 0; i < 16; i++) { await sleep(250); off = !(await clipById(ovClip))?.freeze; if (off) break; }
  return { pass: on && off, detail: `clip.freeze=${JSON.stringify(fz)} on=${on} → cleared=${off}` };
}

async function checkLayerKeyframeAdd(page) {
  // Set the keyframe time + value, click "Add / update point" → assert a keyframe
  // track lands on the clip with at least one point at the chosen value.
  const { ovClip } = await overlayAndOpenLayer(page, "kf", 2);
  const retimed = await clipById(ovClip);
  const expectedMax = Math.round((retimed.src_out_ms - retimed.src_in_ms) / 2);
  const timeInput = page.locator('[data-cut-layer-input="kf_time"]');
  const actualMax = Number(await timeInput.getAttribute('max'));
  // Default param is 'opacity'. Set a value + a non-zero time, then add the point.
  const setSlider = async (attr, value) => {
    const el = page.locator(`[data-cut-layer-input="${attr}"]`);
    if (!(await el.count())) return false;
    await el.evaluate((node, v) => {
      const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
      set.call(node, String(v));
      node.dispatchEvent(new Event("input", { bubbles: true }));
      node.dispatchEvent(new Event("change", { bubbles: true }));
    }, value);
    await sleep(80);
    return true;
  };
  // kf_time only renders when srcSpanMs>0 (it does for a placed clip); kf_value always.
  await setSlider("kf_time", 200);
  const setVal = await setSlider("kf_value", 0.4);
  if (!setVal) return { pass: false, detail: "no kf_value slider in the keyframe editor (broken wiring)" };
  const addBtn = page.locator("[data-cut-layer-kf-add]");
  if (!(await addBtn.count())) return { pass: false, detail: "no 'Add / update point' button (broken wiring)" };
  await addBtn.click();
  let kf = null;
  for (let i = 0; i < 16; i++) {
    await sleep(250);
    const c = await clipById(ovClip);
    kf = (c?.keyframes || []).find((k) => k.param === "opacity");
    if (kf && (kf.points || []).length >= 1) break;
  }
  const ok = !!kf && (kf.points || []).length >= 1 && actualMax === expectedMax;
  return { pass: ok, detail: `2x slider max=${actualMax} expected=${expectedMax}; clip.keyframes opacity param=${kf?.param} points=${JSON.stringify(kf?.points)}` };
}

async function waitForTrack(trackId, predicate) {
  for (let i = 0; i < 24; i++) {
    const track = (await state()).tracks?.find((candidate) => candidate.id === trackId);
    if (track && predicate(track)) return track;
    await sleep(250);
  }
  return null;
}

async function exactFrameSamples(page, atMs = 500) {
  const points = [
    [0.18, 0.18], [0.42, 0.18], [0.78, 0.18],
    [0.18, 0.50], [0.50, 0.50], [0.78, 0.50],
    [0.18, 0.82], [0.50, 0.82], [0.78, 0.82],
  ];
  return page.evaluate(async ({ atMs, points, token }) => {
    const image = new Image();
    image.src = `/api/frame?at_ms=${atMs}&compose=1&v=${token}`;
    await new Promise((resolve, reject) => {
      image.onload = resolve;
      image.onerror = () => reject(new Error(`composed frame failed: ${image.src}`));
    });
    const canvas = document.createElement("canvas");
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    const ctx = canvas.getContext("2d", { willReadFrequently: true });
    ctx.drawImage(image, 0, 0);
    return points.map(([x, y]) => Array.from(ctx.getImageData(
      Math.min(canvas.width - 1, Math.round(canvas.width * x)),
      Math.min(canvas.height - 1, Math.round(canvas.height * y)),
      1,
      1,
    ).data.slice(0, 3)));
  }, { atMs, points, token: `${Date.now()}-${Math.random()}` });
}

function pixelMad(a, b) {
  if (!a?.length || a.length !== b?.length) return Number.POSITIVE_INFINITY;
  let total = 0;
  let count = 0;
  for (let i = 0; i < a.length; i++) {
    for (let channel = 0; channel < 3; channel++) {
      total += Math.abs(a[i][channel] - b[i][channel]);
      count++;
    }
  }
  return total / count;
}

async function setLayerSlider(page, attr, value) {
  const input = page.locator(`[data-cut-layer-input="${attr}"]`);
  if (!(await input.count())) throw new Error(`missing layer slider ${attr}`);
  await input.evaluate((node, next) => {
    const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
    set.call(node, String(next));
    node.dispatchEvent(new Event("input", { bubbles: true }));
    node.dispatchEvent(new Event("change", { bubbles: true }));
  }, value);
}

/** Interspersed-track proof over the v1,a1t,v2 project layout. */
async function checkLayerStackContract(page) {
  const { ovClip, ovTrackId, videoTrackId, projectPath } = await overlayAndOpenLayer(page, "stack");
  if (!projectPath) return { pass: false, detail: "created project path was not discoverable for reopen proof" };

  await setLayerSlider(page, "scale", 0.5);
  await setLayerSlider(page, "x", 0);
  await setLayerSlider(page, "y", 0);
  await setLayerSlider(page, "opacity", 1);
  await page.locator("[data-cut-layer-apply]").click();
  for (let i = 0; i < 20; i++) {
    const transform = (await clipById(ovClip))?.transform;
    if (Math.abs((transform?.scale ?? 1) - 0.5) < 0.01) break;
    await sleep(250);
  }
  const layered = await exactFrameSamples(page);

  await page.locator("[data-cut-layer-close]").click();
  const visibility = page.locator(`[data-cut-visibility-track="${ovTrackId}"]`);
  if (!(await visibility.count())) return { pass: false, detail: `missing visibility control for ${ovTrackId}` };
  await visibility.click();
  if (!(await waitForTrack(ovTrackId, (track) => track.visible === false))) {
    return { pass: false, detail: "visibility button did not hide the overlay track" };
  }
  const baseOnly = await exactFrameSamples(page);
  await visibility.click();
  if (!(await waitForTrack(ovTrackId, (track) => track.visible !== false))) {
    return { pass: false, detail: "visibility button did not restore the overlay track" };
  }

  await openLayerForClip(page, ovClip);
  const sendBack = page.locator("[data-cut-layer-back]");
  if (await sendBack.isDisabled()) return { pass: false, detail: "Send back is disabled on the second video layer" };
  await sendBack.click();
  let reordered = false;
  for (let i = 0; i < 20; i++) {
    const videoIds = (await state()).tracks.filter((track) => track.kind === "video").map((track) => track.id);
    reordered = videoIds[0] === ovTrackId && videoIds[1] === videoTrackId;
    if (reordered) break;
    await sleep(250);
  }
  const sentBack = await exactFrameSamples(page);

  await page.keyboard.press("Control+z");
  let undoRestored = false;
  for (let i = 0; i < 20; i++) {
    const videoIds = (await state()).tracks.filter((track) => track.kind === "video").map((track) => track.id);
    undoRestored = videoIds[0] === videoTrackId && videoIds[1] === ovTrackId;
    if (undoRestored) break;
    await sleep(250);
  }
  const afterUndo = await exactFrameSamples(page);

  await page.locator("[data-cut-layer-close]").click();
  const lock = page.locator(`[data-cut-lock-track="${ovTrackId}"]`);
  if (!(await lock.count())) return { pass: false, detail: `missing lock control for ${ovTrackId}` };
  await lock.click();
  if (!(await waitForTrack(ovTrackId, (track) => track.locked === true))) {
    return { pass: false, detail: "lock button did not persist the overlay lock" };
  }
  let uiLockRefreshed = false;
  for (let i = 0; i < 24; i++) {
    uiLockRefreshed = await lock.getAttribute("data-cut-locked") === "true";
    if (uiLockRefreshed) break;
    await sleep(250);
  }
  await openLayerForClip(page, ovClip);
  const lockNote = await vis(page, "[data-cut-layer-locked-note]");
  const fieldsetLocked = await page.locator("[data-cut-layer-edit-fieldset]").getAttribute("disabled") !== null;
  const applyLocked = await page.locator("[data-cut-layer-apply]").isDisabled().catch(() => false);
  await page.locator("[data-cut-layer-close]").click();
  await resetRightTab(page);
  const inspectorLockNote = await vis(page, "[data-cut-inspector-locked-note]");
  const inspectorLocked = await page.locator("[data-cut-inspector-edit-fieldset]").getAttribute("disabled") !== null;
  const inspectorControlLocked = await page.locator('[data-cut-section-toggle="transform"]').isDisabled().catch(() => false);

  await verb("project.save");
  await verb("project.close");
  const reopened = await verb("project.open", { path: projectPath });
  await page.reload({ waitUntil: "domcontentloaded" });
  await sleep(900);
  const reopenedState = await state();
  const reopenedVideoIds = reopenedState.tracks.filter((track) => track.kind === "video").map((track) => track.id);
  const reopenedOverlay = reopenedState.tracks.find((track) => track.id === ovTrackId);
  const reopenedClip = reopenedOverlay?.clips?.find((clip) => clip.id === ovClip);
  const afterReopen = await exactFrameSamples(page);

  const layerVisibleDifference = pixelMad(layered, baseOnly);
  const reorderedMatchesBase = pixelMad(sentBack, baseOnly);
  const undoMatchesLayered = pixelMad(afterUndo, layered);
  const reopenMatchesLayered = pixelMad(afterReopen, layered);
  const pass =
    layerVisibleDifference > 6 &&
    reorderedMatchesBase < 12 &&
    reordered &&
    undoRestored &&
    undoMatchesLayered < 12 &&
    uiLockRefreshed && lockNote && fieldsetLocked && applyLocked && inspectorLockNote && inspectorLocked && inspectorControlLocked &&
    reopened.ok &&
    reopenedVideoIds[0] === videoTrackId && reopenedVideoIds[1] === ovTrackId &&
    reopenedOverlay?.locked === true && reopenedOverlay?.visible !== false &&
    Math.abs((reopenedClip?.transform?.scale ?? 1) - 0.5) < 0.01 &&
    reopenMatchesLayered < 12;
  return {
    pass,
    detail: `pixels layer/base Δ=${layerVisibleDifference.toFixed(1)}, sent-back/base Δ=${reorderedMatchesBase.toFixed(1)}, undo/layer Δ=${undoMatchesLayered.toFixed(1)}, reopen/layer Δ=${reopenMatchesLayered.toFixed(1)}; order=${reordered}; undo=${undoRestored}; ui-lock=${uiLockRefreshed}; layer-lock=${lockNote}/${fieldsetLocked}/${applyLocked}; inspector-lock=${inspectorLockNote}/${inspectorLocked}/${inspectorControlLocked}; reopen=${reopened.ok}`,
  };
}

// ── runner ───────────────────────────────────────────────────────────────────
async function main() {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 } });
  const errors = [];
  const staleStemErrors = [];
  page.on("response", (r) => {
    if (r.status() < 400) return;
    const detail = `HTTP ${r.status()} ${r.url().replace(/^https?:\/\/[^/]+/, "")}`;
    if (/\/api\/export\/audio_[^./]+\.(wav|mp3)/.test(r.url())) {
      staleStemErrors.push(detail);
    } else if (!/favicon|\/api\/frame|\/filmstrip\/|\/proxies\/|\/api\/source\//.test(r.url())) {
      errors.push(detail);
    }
  });
  await page.goto(APP, { waitUntil: "domcontentloaded" });
  await sleep(1000);

  const results = [];
  const run = async (name, fn) => {
    if (ONLY.size && !ONLY.has(name)) return;
    try {
      const r = await fn(page);
      results.push({ name, ...r });
    } catch (e) {
      results.push({ name, pass: false, detail: String(e.message || e).slice(0, 160) });
    } finally {
      await resetRightTab(page);
    }
  };

  // CLUSTER 1 — Mixer levels.
  await run("mixer-fader-sets-gain", checkMixerFader);
  await run("mixer-mute-sets-flag", checkMixerMute);
  await run("mixer-solo-sets-flag", checkMixerSolo);
  await run("mixer-add-audio-track", checkMixerAddAudioTrack);
  // CLUSTER 2 — Layer transform.
  await run("layer-transform-applies", checkLayerTransform);
  await run("layer-crop-applies", checkLayerCrop);
  await run("layer-reverse-sets-field", checkLayerReverse);
  await run("layer-freeze-sets-field", checkLayerFreeze);
  await run("layer-keyframe-add", checkLayerKeyframeAdd);
  await run("layer-stack-order-contract", checkLayerStackContract);
  if (!ONLY.size || ONLY.has("mixer-stale-stem-fetches")) {
    results.push({
      name: "mixer-stale-stem-fetches",
      pass: staleStemErrors.length === 0,
      detail: staleStemErrors.length
        ? `${staleStemErrors.length} stale stem fetch(es): ${staleStemErrors.slice(0, 4).join(" | ")}`
        : "0 stale Mixer stem fetches",
    });
  }

  await browser.close();

  let fail = 0;
  console.log("\n== VERIFY AUDIO + LAYER ==");
  for (const r of results) {
    console.log(`  ${r.pass ? "PASS" : "FAIL"}  ${r.name.padEnd(28)} ${r.detail}`);
    if (!r.pass) fail++;
  }
  const pass = results.length - fail;
  console.log(`\n${pass} PASS, ${fail} FAIL  (${results.length} checks)`);
  if (errors.length) console.log(`(note: ${errors.length} HTTP≥400 during run: ${errors.slice(0, 4).join(" | ")})`);
  process.exit(fail ? 1 : 0);
}
main().catch((e) => {
  console.error(e);
  process.exit(2);
});
