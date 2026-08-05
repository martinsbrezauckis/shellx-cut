// panels/Preview/meterMath — pure level-meter math (Audio Monitoring v2a).
//
// Extracted from MasterMeter so the dBFS conversion + scale mapping are unit-
// testable without a DOM / Web Audio context. No side effects, no imports.

/** Bottom of the visible meter scale, in dBFS (≤ FLOOR reads as empty). */
export const FLOOR_DB = -60

/** Peak at/above this linear amplitude (~0 dBFS) counts as clipping. */
export const CLIP_AMP = 0.999

/**
 * RMS + true-peak of a block of time-domain samples (each in [-1, 1]), as dBFS.
 * Silence → -Infinity (the meter floor). `clip` = the block hit full scale.
 */
export function blockLevels(samples: Float32Array | number[]): {
  rmsDb: number
  peakDb: number
  clip: boolean
} {
  let sumSq = 0
  let peak = 0
  const n = samples.length
  for (let i = 0; i < n; i++) {
    const v = samples[i]
    sumSq += v * v
    const a = v < 0 ? -v : v
    if (a > peak) peak = a
  }
  const rms = n > 0 ? Math.sqrt(sumSq / n) : 0
  return {
    rmsDb: rms > 0 ? 20 * Math.log10(rms) : -Infinity,
    peakDb: peak > 0 ? 20 * Math.log10(peak) : -Infinity,
    clip: peak >= CLIP_AMP,
  }
}

/** Map a dBFS value to a 0..100% fill against the FLOOR_DB..0 dB scale. */
export function dbToPct(db: number): number {
  if (!isFinite(db)) return 0
  return Math.max(0, Math.min(100, ((db - FLOOR_DB) / (0 - FLOOR_DB)) * 100))
}

/**
 * Per-track meter level (v2b): RMS + true-peak over a short window of a decoded
 * stem buffer, at timeline time `tMs`. Used by the mixer's per-track meters, which
 * sample each track's STEM (export.audio{track}) at the playhead — no extra audio
 * playback, so it works headless (decode + array math only). Takes the buffer's
 * per-channel Float32Array data; reports the LOUDEST channel (standard for a meter).
 * Off the end of the buffer (a gap / past the end) → -Infinity (silence).
 */
export function sampleBufferLevels(
  channels: Float32Array[],
  sampleRate: number,
  tMs: number,
  windowMs = 50,
): { rmsDb: number; peakDb: number; clip: boolean } {
  const start = Math.max(0, Math.floor((tMs / 1000) * sampleRate))
  const len = Math.max(1, Math.floor((windowMs / 1000) * sampleRate))
  let rmsDb = -Infinity
  let peakDb = -Infinity
  let clip = false
  for (const ch of channels) {
    const end = Math.min(ch.length, start + len)
    if (end <= start) continue
    const lv = blockLevels(ch.subarray(start, end))
    if (lv.rmsDb > rmsDb) rmsDb = lv.rmsDb
    if (lv.peakDb > peakDb) peakDb = lv.peakDb
    clip = clip || lv.clip
  }
  return { rmsDb, peakDb, clip }
}
