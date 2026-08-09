/**
 * `edit.speed.factor` bounds from schema/verbs.json. Keep every native speed
 * entry point on this one contract so the UI never narrows or exceeds the
 * engine's accepted factor window. Pitch remains engine-default/preserved.
 */
export const SPEED_FACTOR_MIN = 0.25
export const SPEED_FACTOR_MAX = 4
export const SPEED_FACTOR_STEP = 0.05

export function parseSpeedFactor(value: string | number): number | null {
  const factor = typeof value === 'number' ? value : Number(value)
  if (!Number.isFinite(factor) || factor < SPEED_FACTOR_MIN || factor > SPEED_FACTOR_MAX) return null
  return factor
}

export function speedFactorReason(value: string | number): string | null {
  return parseSpeedFactor(value) === null
    ? `Enter a speed from ${SPEED_FACTOR_MIN}× to ${SPEED_FACTOR_MAX}×.`
    : null
}
