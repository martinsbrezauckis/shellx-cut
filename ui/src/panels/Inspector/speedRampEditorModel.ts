import { SPEED_FACTOR_MAX, SPEED_FACTOR_MIN } from '../Timeline/speedFactor'

export const SPEED_RAMP_MIN_POINTS = 2
export const SPEED_RAMP_MAX_POINTS = 12
export const SPEED_RAMP_DEFAULT_SEGMENTS = 24

export interface StoredSpeedRampPoint {
  at_ms: number
  factor: number
}

export interface StoredSpeedRamp {
  points?: StoredSpeedRampPoint[]
  segments?: number
  preferred_segments?: number | null
}

export interface SpeedRampDraftPoint {
  atSeconds: string
  factor: string
}

export interface SpeedRampDraft {
  points: SpeedRampDraftPoint[]
  segments: number
}

export interface SpeedRampValidation {
  points: StoredSpeedRampPoint[] | null
  reason: string | null
  invalidPoint: number | null
}

function compactNumber(value: number, digits: number): string {
  return value.toFixed(digits).replace(/\.?0+$/, '') || '0'
}

function pointDraft(atMs: number, factor: number): SpeedRampDraftPoint {
  return {
    atSeconds: compactNumber(atMs / 1000, 3),
    factor: compactNumber(factor, 3),
  }
}

function storedPoints(stored: StoredSpeedRamp | null | undefined): StoredSpeedRampPoint[] | null {
  if (!Array.isArray(stored?.points) || stored.points.length < SPEED_RAMP_MIN_POINTS) return null
  const points: StoredSpeedRampPoint[] = []
  for (const point of stored.points) {
    if (
      !point ||
      !Number.isInteger(point.at_ms) ||
      point.at_ms < 0 ||
      !Number.isFinite(point.factor)
    ) return null
    points.push({ at_ms: point.at_ms, factor: point.factor })
  }
  return points
}

function storedSegments(stored: StoredSpeedRamp | null | undefined): number {
  const value = stored?.preferred_segments ?? stored?.segments
  return Number.isInteger(value) && Number(value) >= 2 && Number(value) <= 120
    ? Number(value)
    : SPEED_RAMP_DEFAULT_SEGMENTS
}

export function createSpeedRampDraft(
  stored: StoredSpeedRamp | null | undefined,
  srcDurMs: number,
): SpeedRampDraft {
  const duration = Math.max(0, Math.round(srcDurMs))
  const points = storedPoints(stored) ?? [
    { at_ms: 0, factor: 1 },
    { at_ms: Math.round(duration / 2), factor: 2 },
    { at_ms: duration, factor: 1 },
  ]
  return {
    points: points.map((point) => pointDraft(point.at_ms, point.factor)),
    segments: storedSegments(stored),
  }
}

export function validateSpeedRampDraft(
  draft: SpeedRampDraft,
  srcDurMs: number,
): SpeedRampValidation {
  const duration = Math.max(0, Math.round(srcDurMs))
  if (draft.points.length < SPEED_RAMP_MIN_POINTS) {
    return { points: null, reason: 'Keep at least two control points.', invalidPoint: null }
  }
  if (draft.points.length > SPEED_RAMP_MAX_POINTS) {
    return { points: null, reason: `Use no more than ${SPEED_RAMP_MAX_POINTS} control points.`, invalidPoint: null }
  }

  const points: StoredSpeedRampPoint[] = []
  for (let index = 0; index < draft.points.length; index += 1) {
    const draftPoint = draft.points[index]
    const pointNumber = index + 1
    if (draftPoint.atSeconds.trim() === '') {
      return { points: null, reason: `Point ${pointNumber} needs a source time.`, invalidPoint: index }
    }
    const atSeconds = Number(draftPoint.atSeconds)
    if (!Number.isFinite(atSeconds)) {
      return { points: null, reason: `Point ${pointNumber} source time is not a number.`, invalidPoint: index }
    }
    const atMs = Math.round(atSeconds * 1000)
    if (atMs < 0 || atMs > duration) {
      return {
        points: null,
        reason: `Point ${pointNumber} source time must be between 0 and ${compactNumber(duration / 1000, 3)} s.`,
        invalidPoint: index,
      }
    }
    if (draftPoint.factor.trim() === '') {
      return { points: null, reason: `Point ${pointNumber} needs a speed.`, invalidPoint: index }
    }
    const factor = Number(draftPoint.factor)
    if (!Number.isFinite(factor) || factor < SPEED_FACTOR_MIN || factor > SPEED_FACTOR_MAX) {
      return {
        points: null,
        reason: `Point ${pointNumber} speed must be ${SPEED_FACTOR_MIN}×–${SPEED_FACTOR_MAX}×.`,
        invalidPoint: index,
      }
    }
    if (points.length > 0 && atMs <= points[points.length - 1].at_ms) {
      return { points: null, reason: `Point ${pointNumber} must come after point ${pointNumber - 1}.`, invalidPoint: index }
    }
    points.push({ at_ms: atMs, factor })
  }
  return { points, reason: null, invalidPoint: null }
}

export function insertSpeedRampPoint(
  draft: SpeedRampDraft,
  srcDurMs: number,
): SpeedRampDraft {
  const validation = validateSpeedRampDraft(draft, srcDurMs)
  if (!validation.points || validation.points.length >= SPEED_RAMP_MAX_POINTS) return draft

  let gapIndex = -1
  let gapMs = 0
  for (let index = 0; index < validation.points.length - 1; index += 1) {
    const gap = validation.points[index + 1].at_ms - validation.points[index].at_ms
    if (gap > gapMs) {
      gapMs = gap
      gapIndex = index
    }
  }
  if (gapIndex < 0 || gapMs < 2) return draft

  const left = validation.points[gapIndex]
  const right = validation.points[gapIndex + 1]
  const atMs = left.at_ms + Math.floor(gapMs / 2)
  const factor = left.factor + ((right.factor - left.factor) * (atMs - left.at_ms)) / gapMs
  const points = [...draft.points]
  points.splice(gapIndex + 1, 0, pointDraft(atMs, factor))
  return { ...draft, points }
}
