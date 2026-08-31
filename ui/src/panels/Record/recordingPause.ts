//! Public DTO guards for the bounded macOS pause-safe recorder.

export interface RecordingPauseCapability {
  supported: boolean
  detail: string
  incompatible: Array<{ feature: string; reason: string }>
}

export const NO_RECORDING_PAUSE_CAPABILITY: RecordingPauseCapability = {
  supported: false,
  detail: 'Pause and resume are unavailable in this recorder build.',
  incompatible: [],
}

export type RecordingPauseState = 'idle' | 'recording' | 'pausing' | 'paused' | 'resuming' | 'error'

export interface RecordingPauseAcknowledgement {
  action: 'pause' | 'resume'
  saved: true
  state: 'paused' | 'recording'
  logical_media_time_ms: number
}

export function recordingPauseCapability(value: unknown): RecordingPauseCapability {
  if (!value || typeof value !== 'object') return NO_RECORDING_PAUSE_CAPABILITY
  const source = value as Record<string, unknown>
  const incompatible = Array.isArray(source.incompatible)
    ? source.incompatible.flatMap((entry) => {
      if (!entry || typeof entry !== 'object') return []
      const item = entry as Record<string, unknown>
      return typeof item.feature === 'string' && typeof item.reason === 'string'
        ? [{ feature: item.feature, reason: item.reason }]
        : []
    })
    : []
  return {
    supported: source.supported === true,
    detail: typeof source.detail === 'string' ? source.detail : NO_RECORDING_PAUSE_CAPABILITY.detail,
    incompatible,
  }
}

/** Reject an incomplete acknowledgement so the UI never flips state on dispatch alone. */
export function recordingPauseAcknowledged(
  value: unknown,
  action: 'pause' | 'resume',
): RecordingPauseAcknowledgement | null {
  if (!value || typeof value !== 'object') return null
  const source = value as Record<string, unknown>
  const expectedState = action === 'pause' ? 'paused' : 'recording'
  if (source.action !== action || source.saved !== true || source.state !== expectedState) return null
  return typeof source.logical_media_time_ms === 'number' && Number.isFinite(source.logical_media_time_ms)
    ? { action, saved: true, state: expectedState, logical_media_time_ms: source.logical_media_time_ms }
    : null
}
