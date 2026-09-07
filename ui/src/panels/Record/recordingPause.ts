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

/** One live control response may update only the capture that issued it. */
export interface RecordingPauseControlLease {
  captureId: string
  generation: number
}

/**
 * Tracks capture replacement, request replacement, and unmount as one local
 * lifetime. A delayed acknowledgement must prove its lease before React may
 * expose it as the state of the current recording.
 */
export class RecordingPauseControlLifetime {
  private captureId: string | null = null
  private generation = 0
  private mounted = true

  mount(): void {
    this.mounted = true
  }

  replaceCapture(captureId: string | null): void {
    this.generation += 1
    this.captureId = captureId
  }

  clearCapture(): void {
    this.replaceCapture(null)
  }

  unmount(): void {
    this.mounted = false
    this.clearCapture()
  }

  begin(captureId: string): RecordingPauseControlLease | null {
    if (!this.mounted || this.captureId !== captureId) return null
    this.generation += 1
    return { captureId, generation: this.generation }
  }

  isCurrent(lease: RecordingPauseControlLease): boolean {
    return this.mounted
      && this.captureId === lease.captureId
      && this.generation === lease.generation
  }
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
