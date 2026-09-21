import type { Project } from '../../lib/clientModel'
import { sourceFrameMatch } from '../Timeline/layout'

/** Translate the drawer's normalized source point and timeline clock to the API. */
export function matteSubjectSeed(project: Project | null, clipId: string, playheadMs: number, x: number, y: number) {
  if (![x, y].every((v) => Number.isFinite(v) && v >= 0 && v <= 1)) {
    throw new Error('Choose a subject point between 0 and 1 on each axis.')
  }
  const match = sourceFrameMatch(project, playheadMs, { clipId })
  if (!match.source) throw new Error(match.reason)
  const probe = project?.assets[match.source.asset]?.probe as { width?: number; height?: number } | undefined
  const width = probe?.width
  const height = probe?.height
  if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width! <= 0 || height! <= 0) {
    throw new Error('The source dimensions are unavailable. Re-check this clip in Library before picking a subject.')
  }
  return {
    at_ms: Math.round(match.source.srcMs),
    point: [Math.round(x * (width! - 1)), Math.round(y * (height! - 1))] as [number, number],
  }
}

type JsonObject = Record<string, unknown>
function object(value: unknown): JsonObject | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as JsonObject : null
}

export interface MatteResult {
  clip: string
  enabled: boolean
}

/** The committed effect is authoritative for both Apply and Clear feedback. */
export function matteResultFromReceipt(value: unknown, clipId: string, enabled: boolean): MatteResult {
  const op = object(object(value)?.op)
  const args = object(op?.args)
  const effects = Array.isArray(op?.effects) ? op.effects : []
  // OpEffect serializes its detail with serde(flatten), beside track.
  const effect = effects.map(object).find((entry) => entry?.clip === clipId)
  const hasMatte = effect && Object.hasOwn(effect, 'new_matte')
  const applied = hasMatte && object(effect.new_matte) !== null
  if (op?.verb !== 'edit.matte' || op.status !== 'applied' || typeof op.op_id !== 'string' || !op.op_id
    || args?.clip !== clipId || (args.enabled !== false) !== enabled || !hasMatte
    || (enabled ? !applied : effect.new_matte !== null)) {
    throw new Error('Cut did not return a matching background-removal result. Check the selected clip before trying again.')
  }
  return { clip: clipId, enabled }
}
