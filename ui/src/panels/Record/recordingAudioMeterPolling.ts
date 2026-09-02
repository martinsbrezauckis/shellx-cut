import type { ScreenRecordStatusResult } from '../../lib/clientResults'

/** A fixed, modest cadence keeps live level UI responsive without a new stream. */
export const RECORDING_AUDIO_METER_POLL_MS = 250

export interface RecordingAudioMeterPollToken {
  captureId: string
  generation: number
}

/**
 * Associates asynchronous status reads with one active capture. Replacing or
 * cancelling the token makes late responses inert before they reach React.
 */
export class RecordingAudioMeterPollGuard {
  private current: RecordingAudioMeterPollToken | null = null
  private generation = 0

  begin(captureId: string): RecordingAudioMeterPollToken {
    const token = { captureId, generation: ++this.generation }
    this.current = token
    return token
  }

  isCurrent(token: RecordingAudioMeterPollToken): boolean {
    return this.current?.captureId === token.captureId && this.current.generation === token.generation
  }

  cancel(token: RecordingAudioMeterPollToken): void {
    if (this.isCurrent(token)) this.current = null
  }
}

export function recordingAudioMeterStatusIsTerminal(status: ScreenRecordStatusResult): boolean {
  return status.terminal
}

export function recordingAudioMeterStatusIsNotFound(error: { code?: string } | undefined): boolean {
  return error?.code === 'not_found'
}

/** A terminal status is capture lifecycle evidence, not merely a dead meter. */
export function recordingAudioMeterCaptureEndDetail(status: ScreenRecordStatusResult): string | null {
  if (!status.terminal) return null
  if (status.source_lifecycle?.state === 'source_lost') {
    return 'The selected capture source closed. Checking recording recovery…'
  }
  return 'The native capture ended. Checking recording recovery…'
}

/** `not_found` is likewise terminal for this exact process-local capture id. */
export function recordingAudioMeterNotFoundEndDetail(error: { code?: string } | undefined): string | null {
  return recordingAudioMeterStatusIsNotFound(error)
    ? 'The native capture is no longer active. Checking recording recovery…'
    : null
}

/** Keep a polling failure factual; it never offers to reopen an audio input. */
export function recordingAudioMeterStatusError(error: unknown): string {
  if (error instanceof TypeError) return 'Audio meter status is unavailable: recorder unreachable.'
  if (typeof error === 'object' && error !== null) {
    const value = error as { code?: unknown; message?: unknown }
    if (value.code === 'not_found') return 'Audio meters stopped because this recording is no longer active.'
    if (typeof value.message === 'string' && value.message.trim()) {
      return `Audio meter status is unavailable: ${value.message.trim()}`
    }
  }
  return 'Audio meter status is unavailable. The recorder did not return a current reading.'
}
