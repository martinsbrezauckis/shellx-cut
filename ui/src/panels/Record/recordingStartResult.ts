import type { RecordingCadence } from './recordingCadence'

/** The UI must not render a live transport until the recorder names its capture. */
export interface RecordingStartResult {
  captureId: string
  cadence: RecordingCadence | null
  scenes: unknown
  pause: unknown
}

/**
 * `ok` describes the verb envelope, not admission to a controllable capture.
 * Keep malformed/older successes on the setup surface so they cannot create a
 * false live controller with no capture to stop.
 */
export function recordingStartResult(value: unknown): RecordingStartResult | null {
  if (!value || typeof value !== 'object') return null
  const result = value as Record<string, unknown>
  const captureId = typeof result.capture_id === 'string' ? result.capture_id : null
  // Capture ids are opaque. Reject padding instead of silently directing later
  // controls at an identity the recorder never issued.
  if (!captureId || captureId.trim() !== captureId) return null
  return {
    captureId,
    cadence: (result.cadence as RecordingCadence | undefined) ?? null,
    scenes: result.scenes,
    pause: result.pause,
  }
}
