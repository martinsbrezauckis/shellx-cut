// The recording timer starts at the first captured screen frame, not at the
// Start reply. Between the two the person sees "Starting…": the native capture
// opens its source in that time, and a timer that already ran would promise
// seconds that are not in the file.

import { callVerb } from '../lib/client'

export type FirstFrameOutcome =
  | { state: 'ready'; sinceFirstFrameMs: number }
  | { state: 'ended' }
  | { state: 'cancelled' }
  | { state: 'waiting' }

export interface FirstFrameStatus {
  ok: boolean
  code?: string
  result?: { capture_id?: string; ready?: boolean; terminal?: boolean; since_first_screen_frame_ms?: number | null }
}

/** One status reading, classified. `waiting` means ask again. */
export function classifyFirstFrame(captureId: string, status: FirstFrameStatus): FirstFrameOutcome {
  if (!status.ok) return status.code === 'not_found' ? { state: 'ended' } : { state: 'waiting' }
  const result = status.result
  if (!result || result.capture_id !== captureId) return { state: 'waiting' }
  if (result.terminal === true) return { state: 'ended' }
  if (result.ready === true) {
    const since = typeof result.since_first_screen_frame_ms === 'number' && result.since_first_screen_frame_ms >= 0
      ? result.since_first_screen_frame_ms : 0
    return { state: 'ready', sinceFirstFrameMs: since }
  }
  return { state: 'waiting' }
}

/**
 * Poll `screen_record.status` until the first frame arrives, the capture ends,
 * `isCurrent()` turns false (the person stopped), or `limitMs` passes
 * (`waiting`). Never throws.
 */
export async function waitForFirstFrame(
  captureId: string,
  isCurrent: () => boolean,
  limitMs = 20_000,
  intervalMs = 100,
  read: (id: string) => Promise<FirstFrameStatus> = async (id) => {
    const reply = await callVerb('screen_record.status', { capture_id: id })
    return { ok: reply.ok, code: reply.error?.code, result: reply.result as FirstFrameStatus['result'] }
  },
): Promise<FirstFrameOutcome> {
  const deadline = Date.now() + limitMs
  while (Date.now() < deadline) {
    if (!isCurrent()) return { state: 'cancelled' }
    let status: FirstFrameStatus
    try { status = await read(captureId) } catch { status = { ok: false } }
    if (!isCurrent()) return { state: 'cancelled' }
    const outcome = classifyFirstFrame(captureId, status)
    if (outcome.state !== 'waiting') return outcome
    await new Promise((resolve) => setTimeout(resolve, intervalMs))
  }
  return { state: 'waiting' }
}
