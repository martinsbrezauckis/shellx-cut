/** Coalesces native and DOM callbacks for one physical F9 and blocks a handoff. */
export class RecordingToggleGate {
  private lastAccepted = -Infinity
  private inFlight = false

  claim(now: number): boolean {
    if (this.inFlight || now - this.lastAccepted < 400) return false
    this.lastAccepted = now
    return true
  }

  setInFlight(value: boolean): void { this.inFlight = value }
}

export function stopArgs(captureId: string, raw: boolean) {
  return { capture_id: captureId, autoedit: !raw, mux_raw: true } as const
}

export function classifyStopFailure(
  captureId: string,
  status: { ok: boolean; code?: string; result?: { capture_id?: string; terminal?: boolean } },
): { state: 'live' | 'ended' | 'unknown'; detail: string } {
  if (status.ok && status.result?.capture_id === captureId) {
    return status.result.terminal === true
      ? { state: 'ended', detail: 'The native capture ended; inspect Recording Recovery for its output.' }
      : status.result.terminal === false
        ? { state: 'live', detail: 'The native capture is still active.' }
        : { state: 'unknown', detail: 'The recorder did not confirm whether capture is still active.' }
  }
  if (!status.ok && status.code === 'not_found') {
    return { state: 'ended', detail: 'The native capture is no longer active; inspect Recording Recovery for its output.' }
  }
  return { state: 'unknown', detail: 'Capture state could not be checked.' }
}
