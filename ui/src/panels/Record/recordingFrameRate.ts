/**
 * `screen_record.start` accepts rates in this range. The Record workspace
 * limits manual entry to whole numbers so an accidental fractional draft
 * cannot silently alter the next capture.
 */
export const RECORDING_FRAME_RATE_MIN = 1
export const RECORDING_FRAME_RATE_MAX = 240
/** Common capture rates with one-click controls in the Record workspace. */
export const RECORDING_FRAME_RATE_PRESETS = [24, 25, 30, 50, 60] as const

export function parseRecordingFrameRate(value: string | number): number | null {
  if (typeof value === 'string' && !/^\d+$/.test(value.trim())) return null
  const frameRate = typeof value === 'number' ? value : Number(value.trim())
  if (!Number.isInteger(frameRate) || frameRate < RECORDING_FRAME_RATE_MIN || frameRate > RECORDING_FRAME_RATE_MAX) return null
  return frameRate
}

export function recordingFrameRateReason(value: string | number): string | null {
  return parseRecordingFrameRate(value) === null
    ? `Enter a whole-number frame rate from ${RECORDING_FRAME_RATE_MIN} to ${RECORDING_FRAME_RATE_MAX} FPS.`
    : null
}

/** A typed draft never silently becomes the rate sent to the native recorder. */
export function recordingFrameRateDraftError(customValue: string, appliedFrameRate: number): string | null {
  if (!customValue.trim()) return null
  const reason = recordingFrameRateReason(customValue)
  if (reason) return reason
  return parseRecordingFrameRate(customValue) === appliedFrameRate
    ? null
    : 'Apply the custom frame rate before recording.'
}
