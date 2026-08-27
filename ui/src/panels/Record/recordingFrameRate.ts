/**
 * `screen_record.start` accepts a numeric capture rate in this exact window.
 * Keep the Record workspace on the engine contract instead of maintaining a
 * narrower UI-only limit that would make an otherwise supported capture rate
 * unreachable.
 */
export const RECORDING_FRAME_RATE_MIN = 1
export const RECORDING_FRAME_RATE_MAX = 240
/** Common capture rates with one-click controls in the Record workspace. */
export const RECORDING_FRAME_RATE_PRESETS = [24, 25, 30, 50, 60] as const

export function parseRecordingFrameRate(value: string | number): number | null {
  const frameRate = typeof value === 'number' ? value : Number(value)
  if (!Number.isFinite(frameRate) || frameRate < RECORDING_FRAME_RATE_MIN || frameRate > RECORDING_FRAME_RATE_MAX) return null
  return frameRate
}

export function recordingFrameRateReason(value: string | number): string | null {
  return parseRecordingFrameRate(value) === null
    ? `Enter a frame rate from ${RECORDING_FRAME_RATE_MIN} to ${RECORDING_FRAME_RATE_MAX} FPS.`
    : null
}
