import type { RecordingCadence } from './recordingCadence'

export const CAPTURE_QUALITY_SCHEMA = 'shellx-record/capture-quality/1'
export const OUTPUT_SIZES = ['source', '1080p', '720p'] as const
export const QUALITY_PROFILES = ['standard', 'high'] as const

export type RecordingOutputSize = (typeof OUTPUT_SIZES)[number]
export type RecordingQualityProfile = (typeof QUALITY_PROFILES)[number]
export interface RecordingQualityRequest {
  output_size: RecordingOutputSize
  profile: RecordingQualityProfile
}
export interface RecordingQualityCapability {
  supported: true
  output_sizes: RecordingOutputSize[]
  profiles: RecordingQualityProfile[]
}
export interface RecordingQualityResolution {
  schema: string
  requested: RecordingQualityRequest
  width: number
  height: number
  encoder: string
}

const isRecord = (value: unknown): value is Record<string, unknown> => Boolean(value) && typeof value === 'object'
const isOutputSize = (value: unknown): value is RecordingOutputSize => typeof value === 'string' && OUTPUT_SIZES.includes(value as RecordingOutputSize)
const isProfile = (value: unknown): value is RecordingQualityProfile => typeof value === 'string' && QUALITY_PROFILES.includes(value as RecordingQualityProfile)
const positiveInt = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) > 0

function choices<T extends string>(value: unknown, accepts: (candidate: unknown) => candidate is T): T[] {
  return Array.isArray(value) ? [...new Set(value.filter(accepts))] : []
}

/** Reject an incomplete Doctor response so an older/stale backend never grows a fake picker. */
export function qualityCapability(value: unknown): RecordingQualityCapability | null {
  if (!isRecord(value) || value.supported !== true) return null
  const outputSizes = choices(value.output_sizes, isOutputSize)
  const profiles = choices(value.profiles, isProfile)
  return outputSizes.includes('source') && profiles.includes('standard')
    ? { supported: true, output_sizes: outputSizes, profiles }
    : null
}

/** Final facts are shown only when the versioned, verifier-backed shape is complete. */
export function qualityResolution(value: unknown): RecordingQualityResolution | null {
  if (!isRecord(value) || value.schema !== CAPTURE_QUALITY_SCHEMA || !isRecord(value.requested)) return null
  if (!isOutputSize(value.requested.output_size) || !isProfile(value.requested.profile)) return null
  if (!positiveInt(value.width) || !positiveInt(value.height) || typeof value.encoder !== 'string' || !value.encoder.trim()) return null
  return {
    schema: value.schema,
    requested: { output_size: value.requested.output_size, profile: value.requested.profile },
    width: value.width,
    height: value.height,
    encoder: value.encoder.trim(),
  }
}

export function finalQualityLabel(resolution: RecordingQualityResolution, cadence: RecordingCadence | null): string {
  const rate = cadence?.probed_media?.avg_frame_rate
  const fps = rate && Number.isSafeInteger(rate.num) && Number.isSafeInteger(rate.den) && rate.num > 0 && rate.den > 0
    ? rate.den === 1 ? String(rate.num) : (rate.num / rate.den).toFixed(2).replace(/\.00$/, '')
    : null
  return `Resolved ${resolution.width} × ${resolution.height}${fps ? ` · ${fps} FPS` : ''}`
}
