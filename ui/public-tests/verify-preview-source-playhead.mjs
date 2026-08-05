// verify-preview-source-playhead.mjs — focused playback clock regression.
//
// Repro target: high-bitrate phone/camera sources can be playable as a <video>
// source before a proxy exists or when proxies are disabled. If the browser
// exposes requestVideoFrameCallback but does not deliver callbacks for that
// source, Cut must still advance the timeline playhead while playback runs.
//
// RUN:
//   cd ui && SWEEP_CUTD=http://127.0.0.1:6211 SWEEP_APP=http://127.0.0.1:5211 \
//     RELEASE_CLIP=/path/to/4k-phone.mp4 node public-tests/verify-preview-source-playhead.mjs
import { chromium } from "playwright";
import { existsSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const CUTD = process.env.SWEEP_CUTD || "http://127.0.0.1:6211";
const APP = process.env.SWEEP_APP || "http://127.0.0.1:5211";
const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = join(HERE, "..", "..");
const CLIP = process.env.RELEASE_CLIP || join(REPO, "testdata", "talking_head.mp4");
const PROJ = join(process.env.HOME || "/tmp", ".shellx-scratch", "preview-source-playhead", "preview-source-playhead.cutproj");

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: "POST",
    headers: { "content-type": "application/json", "x-cut-actor": "human:ui:ui" },
    body: JSON.stringify(args),
  });
  return await response.json();
}

async function waitFor(pred, ms = 8000, every = 150) {
  const start = Date.now();
  while (Date.now() - start < ms) {
    if (await pred()) return true;
    await sleep(every);
  }
  return false;
}

async function setupProject() {
  if (!existsSync(CLIP)) {
    console.log(`SKIP: source clip not found: ${CLIP}`);
    process.exit(0);
  }
  rmSync(PROJ, { recursive: true, force: true });
  const created = await verb("project.create", { name: "preview-source-playhead", dir: PROJ });
  if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created)}`);
  const imported = await verb("media.import", {
    path: CLIP,
    proxy: false,
    rationale: "verify preview source playhead",
  });
  if (!imported.ok) throw new Error(`media.import failed: ${JSON.stringify(imported)}`);
  const ready = await waitFor(async () => {
    const project = (await verb("project.state")).result;
    const asset = Object.values(project.assets || {})[0];
    const hasClip = (project.tracks || []).some((track) => (track.clips || []).some((clip) => clip.asset));
    return !!asset?.probe?.duration_ms && hasClip && !asset.proxy;
  }, 15000);
  if (!ready) throw new Error("source-only project did not become ready");
}

await setupProject();

const browser = await chromium.launch({
  headless: true,
  args: ["--autoplay-policy=no-user-gesture-required"],
});
const page = await browser.newPage({ viewport: { width: 1440, height: 960 } });
try {
  await page.goto(APP, { waitUntil: "domcontentloaded" });
  await page.waitForSelector("[data-cut-video-kind='source']", { state: "attached", timeout: 15000 });
  await page.mouse.click(720, 320);
  await page.keyboard.press("Space");

  const advanced = await waitFor(async () => {
    const state = await verb("ui.state", {});
    return (state.result?.playhead_ms || 0) >= 700;
  }, 6000, 200);
  const state = await verb("ui.state", {});
  const media = await page.evaluate(() => {
    const video = document.querySelector("[data-cut-video-kind='source']");
    const audio = document.querySelector("[data-cut-timeline-audio]");
    return {
      playheadText: document.body.innerText.match(/\d\d:\d\d:\d\d\.\d{3}/)?.[0] || null,
      video: video ? { currentTime: video.currentTime, paused: video.paused, readyState: video.readyState, src: video.currentSrc } : null,
      audio: audio ? { currentTime: audio.currentTime, paused: audio.paused, readyState: audio.readyState, src: audio.currentSrc } : null,
    };
  });
  if (!advanced) {
    throw new Error(`source video played but Cut playhead did not advance: state=${JSON.stringify(state.result)} media=${JSON.stringify(media)}`);
  }
  console.log(`PASS: source preview advanced playhead to ${state.result.playhead_ms}ms (${CLIP})`);
} finally {
  await browser.close();
}
