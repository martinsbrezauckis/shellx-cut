//! Public DTO guards for the bounded macOS pause-safe recorder.

import { fmtElapsed } from './recordingUiModel'

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

/** True while the take is paused, including the moment between pressing Resume and the recorder confirming it. */
export function recordingStandsStill(state: RecordingPauseState): boolean {
  return state === 'paused' || state === 'resuming'
}

/**
 * The recorder's own recorded time (`logical_media_time_ms` of its last Pause or
 * Resume acknowledgement) and the wall-clock instant the acknowledgement arrived.
 * That is the only recorded-time figure the engine gives the page.
 */
export interface RecordingPauseBoundary {
  state: 'paused' | 'recording'
  ms: number
  at: number
}

/** A status poll that shows the opposite state outdates the last acknowledgement; transitions keep it. */
export function boundaryAfterStatus(boundary: RecordingPauseBoundary | null, state: RecordingPauseState): RecordingPauseBoundary | null {
  if (!boundary) return null
  if (state === 'paused' && boundary.state !== 'paused') return null
  if (state === 'recording' && boundary.state !== 'recording') return null
  return boundary
}

/** What a paused take says when the recorder's status is all that is known (no pause time). */
export const PAUSED_STATUS_LINE = 'Paused. Resume to continue recording.'

/** minutes:seconds, the same clock as the live timer; never raw milliseconds. */
const clock = (ms: number): string => fmtElapsed(Math.max(0, Math.floor(ms / 1000)))

/** The line shown once the recorder confirms a Pause or Resume at `ms` of the recording. */
export function recordingPauseAcknowledgedLine(state: 'paused' | 'recording', ms: number): string {
  return state === 'paused'
    ? `Paused at ${clock(ms)}. Resume to continue recording.`
    : `Recording resumed at ${clock(ms)}.`
}

export interface RecordingTakePause {
  enabled: boolean
  state: RecordingPauseState
  message: string
}

/** Live facts belong to the exact capture, independently of the next Start form. */
export function recordingTakePause(value: unknown, captureId: string): RecordingTakePause | null {
  if (!value || typeof value !== 'object') return null
  const status = value as Record<string, unknown>
  if (status.capture_id !== captureId || !status.pause || typeof status.pause !== 'object') return null
  const pause = status.pause as Record<string, unknown>
  if (pause.enabled === false && typeof pause.reason === 'string' && pause.reason.trim()) {
    return { enabled: false, state: 'idle', message: pause.reason }
  }
  if (pause.enabled !== true || status.terminal === true) return null
  const state = pause.state
  if (state !== 'recording' && state !== 'paused' && state !== 'pausing' && state !== 'resuming') return null
  return { enabled: true, state, message: state === 'paused' ? PAUSED_STATUS_LINE
    : state === 'pausing' ? 'Waiting for the recorder to seal the pause…'
      : state === 'resuming' ? 'Waiting for the recorder to seal the resume…' : 'Recording.' }
}

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
