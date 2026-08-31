import type { VolumeAutomationPoint } from '../Inspector/volumeAutomationModel'
import { clampAutomationTime, replaceVolumeAutomationPoint } from '../Inspector/volumeAutomationModel'

export const VOLUME_AUTOMATION_MIN_DB = -60
export const VOLUME_AUTOMATION_MAX_DB = 12

/** The visible Add button is deliberately safer than a curve click: a novice
 * must first place the playhead on the selected clip instead of silently
 * receiving an endpoint keyframe from another clip. */
export function playheadIsOverSelectedClip(playheadMs: number, clipStartMs: number, durationMs: number): boolean {
  return durationMs > 0 && playheadMs >= clipStartMs && playheadMs <= clipStartMs + durationMs
}

/** A pointer gesture may last across a project refresh. Never turn its stale
 * local preview into a complete SET after either the revision or volume track
 * has changed underneath it. */
export function volumeAutomationDragStillCurrent(
  startedRevision: string | null,
  startedTrack: string,
  currentRevision: string | null,
  currentTrack: string,
): boolean {
  return startedRevision !== null
    && startedRevision === currentRevision
    && startedTrack === currentTrack
}

export function linearVolumeToDb(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return VOLUME_AUTOMATION_MIN_DB
  return Math.min(VOLUME_AUTOMATION_MAX_DB, Math.max(VOLUME_AUTOMATION_MIN_DB, 20 * Math.log10(value)))
}
export function dbToLinearVolume(value: number): number {
  const db = Math.min(VOLUME_AUTOMATION_MAX_DB, Math.max(VOLUME_AUTOMATION_MIN_DB, Number.isFinite(value) ? value : 0))
  return 10 ** (db / 20)
}
export function formatAutomationDb(value: number): string {
  const db = linearVolumeToDb(value)
  return `${db > 0 ? '+' : ''}${db.toFixed(Math.abs(db) < 10 ? 1 : 0)} dB`
}
export function seedVolumeAutomationPoints(points: VolumeAutomationPoint[], durationMs: number, staticGainDb: number): VolumeAutomationPoint[] {
  if (points.length) return points.slice().sort((a, b) => a.t_ms - b.t_ms)
  const duration = clampAutomationTime(durationMs, durationMs); const value = dbToLinearVolume(staticGainDb)
  return duration === 0 ? [{ t_ms: 0, value }] : [{ t_ms: 0, value }, { t_ms: duration, value }]
}
function valueAt(points: VolumeAutomationPoint[], tMs: number, gainDb: number): number {
  const sorted = points.slice().sort((a, b) => a.t_ms - b.t_ms)
  if (!sorted.length) return dbToLinearVolume(gainDb)
  if (tMs <= sorted[0].t_ms) return sorted[0].value
  const last = sorted[sorted.length - 1]; if (tMs >= last.t_ms) return last.value
  for (let i = 1; i < sorted.length; i += 1) if (tMs <= sorted[i].t_ms) {
    const left = sorted[i - 1]; const right = sorted[i]
    return left.value + (right.value - left.value) * ((tMs - left.t_ms) / Math.max(1, right.t_ms - left.t_ms))
  }
  return last.value
}
export function addVolumeAutomationPoint(points: VolumeAutomationPoint[], durationMs: number, tMs: number, gainDb: number): VolumeAutomationPoint[] {
  const seeded = seedVolumeAutomationPoints(points, durationMs, gainDb); const at = clampAutomationTime(tMs, durationMs)
  return replaceVolumeAutomationPoint(seeded, at, valueAt(seeded, at, gainDb) * 100)
}
export function clampAutomationPointBetweenNeighbours(points: VolumeAutomationPoint[], original: number, requested: number, durationMs: number): number {
  const others = points.filter((point) => point.t_ms !== original).sort((a, b) => a.t_ms - b.t_ms)
  const previous = others.filter((point) => point.t_ms < original).pop(); const next = others.find((point) => point.t_ms > original)
  const lower = previous ? previous.t_ms + 1 : 0; const upper = next ? next.t_ms - 1 : clampAutomationTime(durationMs, durationMs)
  return Math.min(Math.max(lower, clampAutomationTime(requested, durationMs)), Math.max(lower, upper))
}
export function moveVolumeAutomationPoint(points: VolumeAutomationPoint[], original: number, requested: number, db: number, durationMs: number): VolumeAutomationPoint[] {
  const t_ms = clampAutomationPointBetweenNeighbours(points, original, requested, durationMs)
  return [...points.filter((point) => point.t_ms !== original), { t_ms, value: dbToLinearVolume(db) }].sort((a, b) => a.t_ms - b.t_ms)
}
