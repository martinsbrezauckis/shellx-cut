import { spawnSync } from "node:child_process";

import { oneLine } from "./hq-media-identity.mjs";

// At a fixed 60 fps, 100 ms covers a few frame/mux timestamp quanta without
// allowing a short source or accidental timeline edit to masquerade as a match.
const OUTPUT_DURATION_TOLERANCE_MS = 100;

function parseRational(value) {
  const text = String(value || "").trim();
  const match = /^(\d+(?:\.\d+)?)\/(\d+(?:\.\d+)?)$/.exec(text);
  if (match) {
    const numerator = Number(match[1]);
    const denominator = Number(match[2]);
    return denominator > 0 ? numerator / denominator : null;
  }
  const parsed = Number(text);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
}

export function summarizeFfprobe(raw) {
  const streams = Array.isArray(raw?.streams) ? raw.streams : [];
  const video = streams.find((stream) => stream?.codec_type === "video" && stream?.disposition?.attached_pic !== 1);
  if (!video) throw new Error("ffprobe found no timed video stream");
  const width = Number(video.width);
  const height = Number(video.height);
  const fps = parseRational(video.avg_frame_rate) ?? parseRational(video.r_frame_rate);
  const durationSeconds = Number(raw?.format?.duration ?? video.duration);
  const durationMs = Math.round(durationSeconds * 1_000);
  const codec = String(video.codec_name || "").toLowerCase();
  if (!Number.isInteger(width) || width <= 0 || !Number.isInteger(height) || height <= 0) {
    throw new Error("ffprobe returned invalid video dimensions");
  }
  if (!fps) throw new Error("ffprobe returned no positive video frame rate");
  if (!Number.isSafeInteger(durationMs) || durationMs <= 0) throw new Error("ffprobe returned no positive media duration");
  if (!codec) throw new Error("ffprobe returned no video codec name");
  return { width, height, fps, durationMs, codec };
}

export function validateInputProbe(probe, profile) {
  const issues = [];
  const expected = profile.input;
  if (probe.width !== expected.width || probe.height !== expected.height) {
    issues.push(`expected ${expected.width}x${expected.height}, got ${probe.width}x${probe.height}`);
  }
  if (probe.fps < expected.minFps || probe.fps > expected.maxFps) {
    issues.push(`expected ${expected.minFps}-${expected.maxFps} fps, got ${probe.fps}`);
  }
  if (probe.durationMs < expected.minDurationMs || probe.durationMs > expected.maxDurationMs) {
    issues.push(`expected ${expected.minDurationMs}-${expected.maxDurationMs}ms duration, got ${probe.durationMs}ms`);
  }
  if (!expected.codecs.includes(probe.codec)) {
    issues.push(`expected input codec ${expected.codecs.join("|")}, got ${probe.codec}`);
  }
  if (issues.length) throw new Error(`fixture does not match ${profile.id}: ${issues.join("; ")}`);
  return true;
}

export function validateOutputProbe(probe, profile, expectedDurationMs) {
  const issues = [];
  const expected = profile.output;
  if (probe.width !== expected.width || probe.height !== expected.height) {
    issues.push(`expected ${expected.width}x${expected.height}, got ${probe.width}x${probe.height}`);
  }
  if (Math.abs(probe.fps - expected.fps) > 0.1) {
    issues.push(`expected ${expected.fps} fps, got ${probe.fps}`);
  }
  if (!expected.codecs.includes(probe.codec)) {
    issues.push(`expected output codec ${expected.codecs.join("|")}, got ${probe.codec}`);
  }
  if (Math.abs(probe.durationMs - expectedDurationMs) > OUTPUT_DURATION_TOLERANCE_MS) {
    issues.push(`expected duration within ${OUTPUT_DURATION_TOLERANCE_MS}ms of ${expectedDurationMs}ms, got ${probe.durationMs}ms`);
  }
  if (issues.length) throw new Error(`render output does not match ${profile.id}: ${issues.join("; ")}`);
  return true;
}

export function runFfprobe(path, executable) {
  const result = spawnSync(executable, [
    "-v", "error", "-show_entries",
    "format=duration,format_name:stream=codec_type,codec_name,width,height,avg_frame_rate,r_frame_rate,duration,disposition",
    "-of", "json", path,
  ], { encoding: "utf8", windowsHide: true });
  if (result.status !== 0) throw new Error(`ffprobe failed for ${path}: ${oneLine(result.stderr || result.stdout || result.error?.message)}`);
  try {
    return JSON.parse(result.stdout);
  } catch (error) {
    throw new Error(`ffprobe emitted invalid JSON for ${path}: ${error.message}`);
  }
}
