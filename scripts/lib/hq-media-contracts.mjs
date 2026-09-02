import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const DEFAULT_HQ_REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const DEFAULT_ADDR = "127.0.0.1:6219";
const DEFAULT_TIMEOUT_MS = 7_200_000;
const MIN_DURATION_MS = 5_000;
const MAX_DURATION_MS = 600_000;
const INPUT_CODECS = ["h264", "hevc", "prores", "av1"];

// These profiles intentionally name the actual release-admission geometry and
// fps. They are not synthetic media generators: each run requires a supplied,
// independently governed, real fixture at the matching geometry.
export const HQ_MEDIA_PROFILES = Object.freeze({
  "hq-4k-uhd-60": Object.freeze({
    id: "hq-4k-uhd-60",
    label: "HQ 4K UHD 60",
    input: Object.freeze({
      width: 3840,
      height: 2160,
      minDurationMs: MIN_DURATION_MS,
      maxDurationMs: MAX_DURATION_MS,
      minFps: 59.9,
      maxFps: 60.1,
      codecs: INPUT_CODECS,
    }),
    output: Object.freeze({ width: 3840, height: 2160, fps: 60, codecs: ["h264"] }),
    render: Object.freeze({ preset: "high", format: "h264", hardware: "auto", fit: "contain", profile: "talking_head" }),
  }),
  "hq-8k-uhd-60": Object.freeze({
    id: "hq-8k-uhd-60",
    label: "HQ 8K UHD 60",
    input: Object.freeze({
      width: 7680,
      height: 4320,
      minDurationMs: MIN_DURATION_MS,
      maxDurationMs: MAX_DURATION_MS,
      minFps: 59.9,
      maxFps: 60.1,
      codecs: INPUT_CODECS,
    }),
    output: Object.freeze({ width: 7680, height: 4320, fps: 60, codecs: ["h264"] }),
    render: Object.freeze({ preset: "high", format: "h264", hardware: "auto", fit: "contain", profile: "talking_head" }),
  }),
});

function requireValue(argv, index, name) {
  const value = argv[index + 1];
  if (!value || value.startsWith("--")) throw new Error(`${name} requires a value`);
  return value;
}

function positiveInt(value, name) {
  const number = Number(value);
  if (!Number.isInteger(number) || number <= 0) throw new Error(`${name} must be a positive integer`);
  return number;
}

export function parseHqMediaGateArgs(argv = process.argv.slice(2)) {
  const out = {
    profile: "",
    fixture: "",
    hostBinding: "",
    addr: DEFAULT_ADDR,
    daemon: "",
    out: "",
    timeoutMs: DEFAULT_TIMEOUT_MS,
    allowNonLoopback: false,
    help: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--profile") {
      out.profile = requireValue(argv, index, arg);
      index += 1;
    } else if (arg === "--fixture") {
      out.fixture = requireValue(argv, index, arg);
      index += 1;
    } else if (arg === "--host-binding") {
      out.hostBinding = requireValue(argv, index, arg);
      index += 1;
    } else if (arg === "--addr") {
      out.addr = requireValue(argv, index, arg);
      index += 1;
    } else if (arg === "--daemon") {
      out.daemon = requireValue(argv, index, arg);
      index += 1;
    } else if (arg === "--out") {
      out.out = requireValue(argv, index, arg);
      index += 1;
    } else if (arg === "--timeout-ms") {
      out.timeoutMs = positiveInt(requireValue(argv, index, arg), arg);
      index += 1;
    } else if (arg === "--allow-non-loopback") {
      out.allowNonLoopback = true;
    } else if (arg === "--help" || arg === "-h") {
      out.help = true;
    } else {
      throw new Error(`unknown option ${arg}`);
    }
  }
  return out;
}

export function selectHqMediaProfile(id) {
  const profile = HQ_MEDIA_PROFILES[id];
  if (!profile) throw new Error(`unknown HQ media profile '${id}'; choose ${Object.keys(HQ_MEDIA_PROFILES).join(", ")}`);
  return profile;
}

function endpointUrl(addr) {
  const raw = /^https?:\/\//i.test(addr) ? addr : `http://${addr}`;
  let url;
  try {
    url = new URL(raw);
  } catch {
    throw new Error(`--addr is not a valid HTTP endpoint: ${addr}`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error(`--addr must use http or https, not ${url.protocol}`);
  }
  return url;
}

export function endpointScope(addr, { allowNonLoopback = false } = {}) {
  const url = endpointUrl(addr);
  const host = url.hostname.toLowerCase();
  const isLiteralLoopback = host === "127.0.0.1" || host === "::1" || host === "[::1]";
  if (!isLiteralLoopback && !allowNonLoopback) {
    throw new Error("HQ media qualification refuses a non-loopback --addr by default; use literal 127.0.0.1 or explicitly pass --allow-non-loopback with --daemon");
  }
  if (!isLiteralLoopback && allowNonLoopback) {
    return { url: url.toString().replace(/\/$/, ""), scope: "explicit-non-loopback-opt-in", host };
  }
  return { url: url.toString().replace(/\/$/, ""), scope: "literal-loopback", host };
}

export function resolveHqReceiptDir({
  repoRoot = DEFAULT_HQ_REPO_ROOT,
  profile,
  runId = randomUUID(),
} = {}) {
  const root = resolve(repoRoot);
  const name = String(profile || "hq-media").replace(/[^a-z0-9-]+/gi, "-");
  return resolve(root, ".scratch", "hq-media-gate", name, runId);
}

function pathIsInside(path, parent) {
  const child = resolve(path);
  const root = resolve(parent);
  const relation = relative(root, child);
  return relation !== "" && !relation.startsWith("..") && !isAbsolute(relation);
}

export function assertReceiptDir(path, repoRoot) {
  const hqScratch = resolve(repoRoot, ".scratch", "hq-media-gate");
  if (!pathIsInside(path, hqScratch)) {
    throw new Error("HQ evidence must be a new directory inside this checkout's .scratch/hq-media-gate root");
  }
  if (existsSync(path)) throw new Error(`HQ evidence directory already exists: ${path}`);
  return resolve(path);
}

export function assertProfileProjectFormat(state, profile) {
  const settings = state?.settings;
  const expected = profile.output;
  if (Number(settings?.width) !== expected.width || Number(settings?.height) !== expected.height || Number(settings?.fps) !== expected.fps) {
    throw new Error(`HQ project format must be ${expected.width}x${expected.height}@${expected.fps}; got ${settings?.width}x${settings?.height}@${settings?.fps}`);
  }
  return true;
}

export function firstVideoTrackId(state) {
  const track = (state?.tracks || []).find((item) => item?.kind === "video" && typeof item.id === "string" && item.id.length > 0);
  if (!track) throw new Error("HQ project state contains no usable video track");
  return track.id;
}

export function inspectImportedVideoPlacement(state, assetId, videoTrackId) {
  const asset = state?.assets?.[assetId];
  if (!asset?.probe?.duration_ms) {
    throw new Error(`completed media.import did not retain a probed video asset: ${assetId}`);
  }
  const occurrences = (state?.tracks || []).flatMap((track) =>
    track?.kind === "video"
      ? (track.clips || []).filter((clip) => clip?.asset === assetId).map((clip) => ({ clipId: clip.id, trackId: track.id }))
      : [],
  );
  if (occurrences.length > 1) {
    throw new Error(`completed media.import auto-placed ${assetId} more than once on video tracks`);
  }
  if (occurrences.length === 1) {
    const [placement] = occurrences;
    if (placement.trackId !== videoTrackId) {
      throw new Error(`completed media.import placed ${assetId} on unexpected video track ${placement.trackId}`);
    }
    if (typeof placement.clipId !== "string" || placement.clipId.length === 0) {
      throw new Error(`completed media.import auto-placement has no clip id for ${assetId}`);
    }
    return { mode: "already-placed", ...placement };
  }
  return { mode: "insert-required", trackId: videoTrackId };
}

export function buildRenderRequest(profile, outputPath) {
  return {
    path: outputPath,
    ...profile.render,
    // The target geometry belongs to project.create settings. Keeping the
    // render request in the published verb subset means a strict schema cannot
    // silently turn this candidate gate into a rejected pre-render request.
    rationale: `HQ media gate ${profile.id}: actual final render at profile project geometry`,
  };
}

export function assertRenderRequestSchema(request, verb) {
  const properties = verb?.args?.properties;
  if (!verb || verb.name !== "render.final" || verb?.args?.additionalProperties !== false || !properties) {
    throw new Error("render.final schema contract is unavailable or not strict");
  }
  const unknown = Object.keys(request).filter((key) => !(key in properties));
  if (unknown.length) throw new Error(`HQ render request has schema-unknown key(s): ${unknown.join(", ")}`);
  return true;
}

export function hqMediaGateUsage() {
  return [
    "Release use: node scripts/windows-installed-hq.mjs --source-receipt <source-receipt.json> --full-coverage-receipt <full-coverage-receipt.json> --fixture-4k <real-4k60> --fixture-8k <real-8k60> --host-binding <private-binding.json>",
    "Internal component diagnostic: node scripts/hq-candidate-chain.mjs --profile hq-4k-uhd-60|hq-8k-uhd-60 --fixture <real-media-path> --host-binding <private-binding.json>",
    "",
    "The standalone hq-media-gate entrypoint is disabled. The HQ workload is private to the installed and component orchestrators; a final receipt is evidence only, never an input authorizing another render.",
  ].join("\n");
}
