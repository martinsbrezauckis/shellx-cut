import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { basename, join, resolve } from "node:path";

import { resolveDriverPath } from "./cross-host-media.mjs";

const DEFAULT_ADDR = "127.0.0.1:6219";
const DEFAULT_TIMEOUT_MS = 900_000;
const DEFAULT_FRAME_HEIGHT = 540;
const DEFAULT_RANGE_MS = [0, 1000];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function requireValue(argv, i, name) {
  const value = argv[i + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function parsePositiveInt(value, name) {
  const n = Number(value);
  if (!Number.isInteger(n) || n <= 0) throw new Error(`${name} must be a positive integer`);
  return n;
}

function parseRange(value) {
  const [startRaw, endRaw, extra] = String(value).split(":");
  if (extra !== undefined || startRaw === undefined || endRaw === undefined) {
    throw new Error("--range-ms must be START:END");
  }
  const start = Number(startRaw);
  const end = Number(endRaw);
  if (!Number.isInteger(start) || !Number.isInteger(end) || start < 0 || end <= start) {
    throw new Error("--range-ms must be non-negative START:END with END greater than START");
  }
  return [start, end];
}

export function parseLargeMediaGateArgs(argv = process.argv.slice(2)) {
  const out = {
    addr: DEFAULT_ADDR,
    media: [],
    out: undefined,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    frameHeight: DEFAULT_FRAME_HEIGHT,
    rangeMs: [...DEFAULT_RANGE_MS],
    projectName: undefined,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      out.help = true;
    } else if (arg === "--addr") {
      out.addr = requireValue(argv, i, arg);
      i++;
    } else if (arg === "--media") {
      out.media.push(requireValue(argv, i, arg));
      i++;
    } else if (arg === "--out") {
      out.out = requireValue(argv, i, arg);
      i++;
    } else if (arg === "--timeout-ms") {
      out.timeoutMs = parsePositiveInt(requireValue(argv, i, arg), arg);
      i++;
    } else if (arg === "--frame-height") {
      out.frameHeight = parsePositiveInt(requireValue(argv, i, arg), arg);
      i++;
    } else if (arg === "--range-ms") {
      out.rangeMs = parseRange(requireValue(argv, i, arg));
      i++;
    } else if (arg === "--project-name") {
      out.projectName = requireValue(argv, i, arg);
      i++;
    } else if (arg.startsWith("--")) {
      throw new Error(`unknown option ${arg}`);
    } else {
      out.media.push(arg);
    }
  }
  return out;
}

export function resolveReceiptDir({
  platform = process.platform,
  date = new Date(),
  home = homedir(),
} = {}) {
  const day = date.toISOString().slice(0, 10);
  return join(home, ".shellx-scratch", "shellx-cut", `qualification-${day}`, platform);
}

export function buildGatePlan(media) {
  if (!Array.isArray(media) || media.length === 0) {
    throw new Error("at least one --media path is required");
  }
  const [base, second, third] = media;
  return {
    base,
    overlays: [
      second
        ? { path: second, reusedBase: false, atMs: 0, srcRangeMs: undefined }
        : { path: base, reusedBase: true, atMs: 0, srcRangeMs: [1000, 5000] },
      third
        ? { path: third, reusedBase: false, atMs: 1500, srcRangeMs: undefined }
        : { path: second || base, reusedBase: !second, atMs: 1500, srcRangeMs: second ? undefined : [3000, 7000] },
    ],
  };
}

export function summarizeJobs(jobs = []) {
  const summary = { done: 0, failed: 0, active: 0, byKind: {} };
  for (const job of jobs) {
    const state = job.state || "unknown";
    const kind = job.kind || "unknown";
    if (state === "done") summary.done++;
    if (state === "failed") summary.failed++;
    if (state === "queued" || state === "running") summary.active++;
    summary.byKind[kind] ||= {};
    summary.byKind[kind][state] = (summary.byKind[kind][state] || 0) + 1;
  }
  return summary;
}

export function toHashRows(rows) {
  return rows.map((row) => {
    const mode = row.compose ? "composed" : "raw";
    return {
      ...row,
      compose: !!row.compose,
      key: `${row.label} ${mode} ${row.atMs}ms`,
    };
  });
}

export function classifyFrameHashes(rows) {
  const byKey = new Map();
  for (const row of rows) {
    const key = row.key || `${row.label} ${row.compose ? "composed" : "raw"} ${row.atMs}ms`;
    if (!byKey.has(key)) byKey.set(key, []);
    byKey.get(key).push(row.sha256);
  }
  const unstableRepeats = [];
  for (const [key, hashes] of byKey) {
    if (hashes.length > 1 && new Set(hashes).size > 1) unstableRepeats.push(key);
  }

  let composedDiffersOnOverlap = false;
  const byPosition = new Map();
  for (const row of rows) {
    const key = `${row.label}:${row.atMs}`;
    byPosition.set(key, [...(byPosition.get(key) || []), row]);
  }
  for (const group of byPosition.values()) {
    const raw = group.filter((r) => !r.compose).map((r) => r.sha256);
    const composed = group.filter((r) => r.compose).map((r) => r.sha256);
    if (raw.length && composed.length && raw.some((r) => composed.some((c) => c !== r))) {
      composedDiffersOnOverlap = true;
      break;
    }
  }

  return {
    repeatedStable: unstableRepeats.length === 0,
    composedDiffersOnOverlap,
    unstableRepeats,
  };
}

function baseUrl(addr) {
  return /^https?:\/\//i.test(addr) ? addr.replace(/\/+$/, "") : `http://${addr}`;
}

async function postVerb({ addr, timeoutMs }, name, args = {}) {
  const url = `${baseUrl(addr)}/api/verb/${name}`;
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(timeoutMs),
  });
  const text = await response.text();
  let body;
  try {
    body = JSON.parse(text);
  } catch {
    throw new Error(`${name} returned non-JSON HTTP ${response.status}: ${text.slice(0, 400)}`);
  }
  if (!response.ok) throw new Error(`${name} HTTP ${response.status}: ${text.slice(0, 400)}`);
  return body;
}

async function mustVerb(ctx, name, args = {}) {
  const body = await postVerb(ctx, name, args);
  if (body.ok !== true) {
    throw new Error(`${name} failed: ${JSON.stringify(body.error || body).slice(0, 800)}`);
  }
  return body.result ?? {};
}

async function tryVerb(ctx, name, args = {}) {
  return postVerb(ctx, name, args).catch((error) => ({ ok: false, error: String(error) }));
}

async function waitJob(ctx, jobId, timeoutMs = ctx.timeoutMs) {
  const started = Date.now();
  let last = null;
  while (Date.now() - started < timeoutMs) {
    const body = await postVerb(ctx, "jobs.status", { job_id: jobId });
    last = body.result;
    const state = body.result?.state;
    if (state === "done") return body.result;
    if (state === "failed") {
      throw new Error(`job ${jobId} failed: ${JSON.stringify(body.result?.error || body.result)}`);
    }
    await sleep(1000);
  }
  throw new Error(`job ${jobId} timed out after ${timeoutMs}ms; last=${JSON.stringify(last)}`);
}

async function jobSnapshot(ctx) {
  const body = await postVerb(ctx, "jobs.list", {});
  return body.result?.jobs || [];
}

async function waitImportProxy(ctx, assetId) {
  const started = Date.now();
  let lastJobs = [];
  let lastAsset = null;
  while (Date.now() - started < ctx.timeoutMs) {
    const state = await mustVerb(ctx, "project.state", {});
    lastAsset = state.assets?.[assetId];
    lastJobs = await jobSnapshot(ctx);
    const target = lastJobs.filter((j) => j.kind === "import" || j.kind === "proxy");
    const active = target.filter((j) => j.state === "queued" || j.state === "running");
    const failed = target.filter((j) => j.state === "failed");
    if (failed.length) throw new Error(`import/proxy job failed: ${JSON.stringify(failed)}`);
    if (lastAsset?.probe?.duration_ms && lastAsset?.proxy && active.length === 0) {
      return { state, asset: lastAsset, jobs: lastJobs, elapsedMs: Date.now() - started };
    }
    await sleep(1500);
  }
  throw new Error(
    `asset ${assetId} did not reach probe+proxy within ${ctx.timeoutMs}ms; asset=${JSON.stringify(lastAsset)} jobs=${JSON.stringify(summarizeJobs(lastJobs))}`,
  );
}

function ensureDir(path) {
  mkdirSync(path, { recursive: true });
  return path;
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function readablePath(path) {
  return resolveDriverPath(path);
}

function fileInfo(path) {
  const local = readablePath(path);
  const exists = existsSync(local);
  return {
    path,
    local,
    exists,
    bytes: exists ? statSync(local).size : 0,
    sha256: exists ? sha256File(local) : null,
  };
}

function sha256File(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function ffprobeJson(path) {
  const local = readablePath(path);
  const result = spawnSync(
    "ffprobe",
    ["-v", "error", "-print_format", "json", "-show_format", "-show_streams", local],
    { encoding: "utf8" },
  );
  if (result.status !== 0) {
    return {
      ok: false,
      path,
      local,
      error: result.stderr || result.stdout || `ffprobe exited ${result.status}`,
    };
  }
  try {
    return { ok: true, path, local, ...JSON.parse(result.stdout) };
  } catch (error) {
    return { ok: false, path, local, error: `ffprobe JSON parse failed: ${error.message}` };
  }
}

function mediaStreams(probe) {
  const streams = probe?.streams || [];
  return {
    video: streams.find((s) => s.codec_type === "video") || null,
    audio: streams.find((s) => s.codec_type === "audio") || null,
  };
}

function copyFrameResult(result, framesDir, label, seq) {
  const ext = String(result.mime || "").includes("png") ? "png" : "jpg";
  const dst = join(framesDir, `${String(seq).padStart(2, "0")}-${label}.${ext}`);
  if (result.base64) {
    writeFileSync(dst, Buffer.from(result.base64, "base64"));
  } else if (result.path) {
    copyFileSync(readablePath(result.path), dst);
  } else {
    throw new Error(`render.frame returned no path/base64 for ${label}`);
  }
  return dst;
}

function videoTracks(state) {
  return (state.tracks || []).filter((track) => track.kind === "video");
}

function mediaClips(state) {
  return (state.tracks || []).flatMap((track) =>
    (track.clips || [])
      .filter((clip) => clip.asset)
      .map((clip) => ({ ...clip, track: track.id, trackKind: track.kind })),
  );
}

function clipById(state, id) {
  return mediaClips(state).find((clip) => clip.id === id) || null;
}

function firstVideoTrackId(state) {
  return videoTracks(state)[0]?.id || "v1";
}

function firstClipForAsset(state, assetId) {
  return mediaClips(state).find((clip) => clip.asset === assetId && clip.trackKind === "video") || null;
}

async function ensureBaseClip(ctx, assetId) {
  let state = await mustVerb(ctx, "project.state", {});
  let clip = firstClipForAsset(state, assetId);
  if (clip) return { state, clip };
  const track = firstVideoTrackId(state);
  const result = await mustVerb(ctx, "edit.insert", {
    asset: assetId,
    track,
    at_ms: 0,
    ripple: false,
    rationale: "large-media gate: seed base clip",
  });
  state = await mustVerb(ctx, "project.state", {});
  clip = result.clip_id ? clipById(state, result.clip_id) : firstClipForAsset(state, assetId);
  if (!clip) throw new Error("base clip was not present after insert");
  return { state, clip };
}

async function createOverlayTrack(ctx, id) {
  const state = await mustVerb(ctx, "project.state", {});
  if ((state.tracks || []).some((track) => track.id === id)) return id;
  await mustVerb(ctx, "edit.add_track", {
    kind: "video",
    id,
    rationale: "large-media gate: overlay track",
  });
  return id;
}

async function waitExportJobIfAny(ctx, result) {
  if (result.job_id) {
    const job = await waitJob(ctx, result.job_id, ctx.timeoutMs);
    return job.result || result;
  }
  return result;
}

async function renderAndHashFrames(ctx, framesDir, requests, seqStart = 1) {
  const rows = [];
  for (const [index, req] of requests.entries()) {
    const started = Date.now();
    const frame = await mustVerb(ctx, "render.frame", {
      at_ms: req.atMs,
      h: ctx.frameHeight,
      compose: req.compose,
    });
    const path = copyFrameResult(
      frame,
      framesDir,
      `${req.label}-${req.compose ? "composed" : "raw"}-${req.atMs}ms`,
      seqStart + index,
    );
    rows.push({
      ...req,
      path,
      width: frame.width,
      height: frame.height,
      fast: frame.fast,
      elapsedMs: Date.now() - started,
      sha256: sha256File(path),
    });
  }
  return toHashRows(rows);
}

export async function runLargeMediaGate(options) {
  if (!options.media?.length) throw new Error("at least one --media path is required");
  const receiptDir = resolve(options.out || resolveReceiptDir());
  const framesDir = ensureDir(join(receiptDir, "frames"));
  ensureDir(receiptDir);

  const ctx = {
    addr: options.addr || DEFAULT_ADDR,
    timeoutMs: options.timeoutMs || DEFAULT_TIMEOUT_MS,
    frameHeight: options.frameHeight || DEFAULT_FRAME_HEIGHT,
  };
  const plan = buildGatePlan(options.media.map((p) => resolve(p)));
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  const projectName = options.projectName || `large-media-gate-${stamp}`;
  const projectDir = join(receiptDir, `${projectName}.cutproj`);
  const receipt = {
    schema: "shellx-cut/large-media-gate/1",
    startedAt: new Date().toISOString(),
    addr: ctx.addr,
    receiptDir,
    projectName,
    projectDir,
    plan,
    checks: {},
    artifacts: {},
  };

  try {
    writeJson(join(receiptDir, "plan.json"), receipt);
    await mustVerb(ctx, "project.create", {
      name: projectName,
      dir: projectDir,
      settings: { width: 3840, height: 2160, fps: 60 },
    });

    const originalProbe = ffprobeJson(plan.base);
    writeJson(join(receiptDir, "ffprobe-original-0.json"), originalProbe);
    receipt.artifacts.original = fileInfo(plan.base);

    const baseImport = await mustVerb(ctx, "media.import", {
      path: plan.base,
      proxy: true,
      rationale: "large-media gate: base proxy import",
    });
    receipt.baseAsset = baseImport.asset_id;
    const baseReady = await waitImportProxy(ctx, baseImport.asset_id);
    receipt.importProxyElapsedMs = baseReady.elapsedMs;
    await ensureBaseClip(ctx, baseImport.asset_id);

    const baselineFrameRows = await renderAndHashFrames(ctx, framesDir, [
      { label: "early", atMs: 500, compose: false },
      { label: "early", atMs: 500, compose: false },
      { label: "overlap", atMs: 1500, compose: false },
    ]);

    const assetByPath = new Map([[plan.base, baseImport.asset_id]]);
    for (const overlay of plan.overlays) {
      if (!overlay.reusedBase && !assetByPath.has(overlay.path)) {
        const imported = await mustVerb(ctx, "media.import", {
          path: overlay.path,
          proxy: true,
          rationale: "large-media gate: overlay proxy import",
        });
        assetByPath.set(overlay.path, imported.asset_id);
        await waitImportProxy(ctx, imported.asset_id);
      }
    }

    let state = await mustVerb(ctx, "project.state", {});
    const baseAsset = state.assets?.[baseImport.asset_id];
    const proxyProbe = baseAsset?.proxy ? ffprobeJson(join(projectDir, baseAsset.proxy)) : { ok: false, error: "asset.proxy missing" };
    writeJson(join(receiptDir, "ffprobe-proxy-base.json"), proxyProbe);
    receipt.artifacts.proxy = baseAsset?.proxy ? fileInfo(join(projectDir, baseAsset.proxy)) : null;

    const overlayClips = [];
    for (const [idx, overlay] of plan.overlays.entries()) {
      const track = await createOverlayTrack(ctx, `v_gate_overlay_${idx + 1}`);
      const asset = assetByPath.get(overlay.path) || baseImport.asset_id;
      const insertArgs = {
        asset,
        track,
        at_ms: overlay.atMs,
        ripple: false,
        rationale: "large-media gate: overlay insert",
      };
      if (overlay.srcRangeMs) insertArgs.src_range_ms = overlay.srcRangeMs;
      const inserted = await mustVerb(ctx, "edit.insert", insertArgs);
      const clipId = inserted.clip_id;
      await mustVerb(ctx, "edit.transform", {
        clip: clipId,
        x: idx === 0 ? 0.24 : 0.70,
        y: idx === 0 ? 0.24 : 0.64,
        scale: idx === 0 ? 0.55 : 0.42,
        opacity: idx === 0 ? 0.72 : 0.68,
        rationale: "large-media gate: overlay transform",
      });
      if (idx === 1) {
        await mustVerb(ctx, "edit.move", {
          clip: clipId,
          to_track: track,
          at_ms: 2500,
          ripple: false,
          rationale: "large-media gate: overlay move",
        });
      }
      overlayClips.push({ ...overlay, asset, track, clipId });
    }

    state = await mustVerb(ctx, "project.state", {});
    writeJson(join(receiptDir, "project-state-after-overlays.json"), state);
    receipt.overlayClips = overlayClips.map((clip) => ({ ...clip, state: clipById(state, clip.clipId) }));
    receipt.checks.overlaySemantics =
      receipt.overlayClips.length === 2 &&
      receipt.overlayClips.every((clip) => clip.state?.track === clip.track && clip.state?.transform);

    const composedFrameRows = await renderAndHashFrames(
      ctx,
      framesDir,
      [
        { label: "overlap", atMs: 1500, compose: true },
        { label: "overlap", atMs: 1500, compose: true },
        { label: "late", atMs: 3000, compose: true },
        { label: "late", atMs: 3000, compose: true },
      ],
      baselineFrameRows.length + 1,
    );
    const frameRows = [...baselineFrameRows, ...composedFrameRows];
    const frameClassification = classifyFrameHashes(frameRows);
    writeJson(join(receiptDir, "frame-hashes.json"), { frames: frameRows, classification: frameClassification });
    receipt.checks.previewConsistency =
      frameClassification.repeatedStable && frameClassification.composedDiffersOnOverlap;

    const audioResult = await waitExportJobIfAny(
      ctx,
      await mustVerb(ctx, "export.audio", {
        format: "mp3",
        rationale: "large-media gate: audio consistency",
      }),
    );
    const audioPath = audioResult.path;
    const audioProbe = audioPath ? ffprobeJson(audioPath) : { ok: false, error: "export.audio returned no path" };
    writeJson(join(receiptDir, "ffprobe-export-audio.json"), audioProbe);
    receipt.artifacts.audio = audioPath ? fileInfo(audioPath) : null;
    receipt.checks.audioConsistency = !!mediaStreams(audioProbe).audio;

    const rangeResult = await waitExportJobIfAny(
      ctx,
      await mustVerb(ctx, "export.range", {
        range_ms: options.rangeMs || DEFAULT_RANGE_MS,
        preset: "draft",
        to_asset: false,
      }),
    );
    const rangePath = rangeResult.path || rangeResult.out;
    const rangeProbe = rangePath ? ffprobeJson(rangePath) : { ok: false, error: "export.range returned no path" };
    writeJson(join(receiptDir, "ffprobe-export-range.json"), rangeProbe);
    receipt.artifacts.range = rangePath ? fileInfo(rangePath) : null;
    const rangeStreams = mediaStreams(rangeProbe);
    receipt.checks.rangeExport = !!rangeStreams.video && !!rangeStreams.audio;

    const jobs = await jobSnapshot(ctx);
    writeJson(join(receiptDir, "jobs.json"), { jobs, summary: summarizeJobs(jobs) });
    receipt.jobs = summarizeJobs(jobs);
    receipt.checks.jobsNoImportProxyFailure = !jobs.some(
      (job) => (job.kind === "import" || job.kind === "proxy") && job.state === "failed",
    );

    receipt.completedAt = new Date().toISOString();
    receipt.pass = Object.values(receipt.checks).every(Boolean);
    writeJson(join(receiptDir, "large-media-gate-receipt.json"), receipt);
    return receipt;
  } catch (error) {
    receipt.completedAt = new Date().toISOString();
    receipt.pass = false;
    receipt.error = String(error?.stack || error?.message || error);
    try {
      writeJson(join(receiptDir, "large-media-gate-receipt.json"), receipt);
    } catch {
      // Keep the original failure.
    }
    throw error;
  }
}

export function largeMediaGateUsage() {
  return [
    "Usage: node scripts/large-media-gate.mjs --media /path/to/4k.mp4 [--media overlay.mp4] [options]",
    "",
    "Requires a running cutd, for example:",
    "  ./scripts/dev.sh --headless --addr 127.0.0.1:6219",
    "",
    "Options:",
    "  --addr HOST:PORT              cutd REST address (default 127.0.0.1:6219)",
    "  --media PATH                  media path, repeat up to 3 times",
    "  --out DIR                     receipt directory",
    "  --timeout-ms MS               job/HTTP timeout (default 900000)",
    "  --frame-height PX             render.frame preview height (default 540)",
    "  --range-ms START:END          export.range span (default 0:1000)",
    "  --project-name NAME           project name",
  ].join("\n");
}
