/** API shape for optional CaptureCadence@1 evidence returned by recording verbs. */
export interface RecordingFrameRate {
  num: number
  den: number
}

export interface RecordingCadence {
  schema: string
  requested: RecordingFrameRate
  backend_requested: RecordingFrameRate
  probed_media?: {
    avg_frame_rate?: RecordingFrameRate
    r_frame_rate?: RecordingFrameRate
    decoded_video_frames?: number
    duration_ms?: number
  }
}

export const CAPTURE_CADENCE_SCHEMA = 'shellx-record/capture-cadence/1'

function validRate(rate: RecordingFrameRate | undefined): rate is RecordingFrameRate {
  if (!rate) return false
  return Number.isSafeInteger(rate.num)
    && Number.isSafeInteger(rate.den)
    && rate.num > 0
    && rate.den > 0
}

function validCadence(cadence: RecordingCadence | null): cadence is RecordingCadence {
  return cadence?.schema === CAPTURE_CADENCE_SCHEMA
    && validRate(cadence.requested)
    && validRate(cadence.backend_requested)
}

/** Preserve terminating decimal requests in editor-friendly form; keep all
 * other measured rates as their exact numerator/denominator evidence. */
export function formatRecordingFrameRate(rate: RecordingFrameRate | undefined): string | null {
  if (!validRate(rate)) return null
  let remaining = rate.den
  let decimalPlaces = 0
  while (remaining % 2 === 0) { remaining /= 2; decimalPlaces += 1 }
  while (remaining % 5 === 0) { remaining /= 5; decimalPlaces += 1 }
  if (remaining !== 1) return `${rate.num}/${rate.den}`
  return (rate.num / rate.den).toFixed(decimalPlaces).replace(/\.?0+$/, '')
}

export function requestedCadenceLabel(cadence: RecordingCadence | null, fallbackFps: number): string {
  const requested = validCadence(cadence) ? cadence.requested : undefined
  return `Requested ${formatRecordingFrameRate(requested) ?? String(fallbackFps)} FPS`
}

export function probedAverageCadenceLabel(cadence: RecordingCadence | null): string {
  const average = validCadence(cadence)
    ? formatRecordingFrameRate(cadence.probed_media?.avg_frame_rate)
    : null
  return average ? `Measured average: ${average} FPS` : 'Measured average: Not measured.'
}
