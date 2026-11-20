import { useCallback, useEffect, useRef, useState } from 'react'
import { frameUrl } from '../../lib/client'
import { previewFrameMs } from './model'

/** The playhead has to rest this long before the exact frame is requested:
 * scrubbing and stepping stay on the instant live preview and never queue
 * engine renders. */
export const COMPOSED_REST_MS = 150
/** A request still running after this long shows the small busy mark. */
const BUSY_MARK_MS = 400

export type ComposedFrameState = 'off' | 'waiting' | 'pending' | 'shown' | 'failed'

export interface ComposedFrameStatus {
  state: ComposedFrameState
  atMs: number | null
  reason: string | null
  /** Rest -> picture on screen, for the last shown frame. */
  latencyMs: number | null
  /** Requests started since the monitor mounted (debug evidence for scrubbing cost). */
  requests: number
}

interface Request { key: string; atMs: number; url: string; startedAt: number }

const keyOf = (atMs: number, op: string | undefined) => `${atMs}|${op ?? ''}`

/** Read the engine's own error words for a failed frame (one extra request,
 * only after a failure). */
async function failureReason(url: string): Promise<string> {
  try {
    const response = await fetch(url)
    if (response.ok) return 'the frame loaded on a second try; move the playhead to refresh it'
    const body = await response.json().catch(() => null) as { error?: { message?: string } } | null
    return body?.error?.message || `the engine answered ${response.status}`
  } catch {
    // fetch rejects only when no answer came at all ("Load failed", "Failed to fetch").
    return 'the engine could not be reached'
  }
}

/** The exact composed frame, drawn ON TOP of the still-mounted live preview
 * while the playhead rests (COMPOSED on, paused). It is removed the moment
 * play, a seek or an edit starts, so it never shows a picture from another
 * position or from before the edit; the live preview underneath stays mounted,
 * so starting play never re-creates media elements. One request at a time; a
 * failure is said on the picture. State is published on the element for
 * ui.state (preview.composed_frame). */
export function ComposedFrameOverlay({
  enabled, playheadMs, headOpId, durationMs, frameMs, onShownChange,
}: {
  enabled: boolean
  playheadMs: number
  headOpId?: string
  durationMs: number
  frameMs: number
  /** The exact frame is (true) or is not (false) what the person sees now. */
  onShownChange?: (shown: boolean) => void
}) {
  const atMs = previewFrameMs(playheadMs, durationMs, frameMs)
  const wantKey = enabled ? keyOf(atMs, headOpId) : null
  const [request, setRequest] = useState<Request | null>(null)
  const [shownKey, setShownKey] = useState<string | null>(null)
  const [failed, setFailed] = useState<{ key: string; reason: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [latencyMs, setLatencyMs] = useState<number | null>(null)
  const requests = useRef(0)
  // The key whose rest timer has fired, and when: a request for it may start.
  const rested = useRef<{ key: string | null; at: number }>({ key: null, at: 0 })
  const inFlight = useRef<Request | null>(null)
  const want = useRef<{ key: string | null; atMs: number; op?: string }>({ key: null, atMs: 0 })
  want.current = { key: wantKey, atMs, op: headOpId }

  const start = useCallback(() => {
    const target = want.current
    if (!target.key || inFlight.current) return
    const next = { key: target.key, atMs: target.atMs, url: frameUrl(target.atMs, target.op, true), startedAt: performance.now() }
    inFlight.current = next
    requests.current += 1
    setRequest(next)
  }, [])

  // A new position, edit or mode: drop the old picture at once, then wait for rest.
  useEffect(() => {
    setShownKey(null)
    setFailed(null)
    setBusy(false)
    if (!wantKey) return undefined
    const rest = window.setTimeout(() => {
      rested.current = { key: wantKey, at: performance.now() }
      start()
    }, COMPOSED_REST_MS)
    const mark = window.setTimeout(() => setBusy(true), COMPOSED_REST_MS + BUSY_MARK_MS)
    return () => {
      window.clearTimeout(rest)
      window.clearTimeout(mark)
    }
  }, [wantKey, start])

  const settle = (done: Request, ok: boolean) => {
    if (inFlight.current?.key !== done.key) return
    inFlight.current = null
    if (done.key === want.current.key) {
      setBusy(false)
      if (ok) {
        setShownKey(done.key)
        const restAt = rested.current.key === done.key ? rested.current.at : done.startedAt
        setLatencyMs(Math.round(performance.now() - restAt))
      } else {
        void failureReason(done.url).then((reason) => {
          if (want.current.key === done.key) setFailed({ key: done.key, reason })
        })
      }
    } else if (want.current.key && rested.current.key === want.current.key) {
      // The playhead moved on and came to rest while this one rendered: now
      // ask for the latest position (never more than one request at a time).
      start()
    }
  }

  const visible = !!wantKey && shownKey === wantKey
  useEffect(() => { onShownChange?.(visible) }, [visible, onShownChange])
  const failedHere = failed && failed.key === wantKey ? failed.reason : null
  const state: ComposedFrameState = !wantKey ? 'off' : visible ? 'shown' : failedHere ? 'failed'
    : inFlight.current?.key === wantKey ? 'pending' : 'waiting'
  return (
    <div
      className="pv-composed-frame"
      data-cut-composed-frame-state={state}
      data-cut-composed-frame-at={wantKey ? atMs : undefined}
      data-cut-composed-frame-error={failedHere ?? undefined}
      data-cut-composed-frame-latency={latencyMs ?? undefined}
      data-cut-composed-frame-requests={requests.current}
    >
      {request && (
        <img
          // A new element per request: asking again for the same position and
          // edit gives the same URL, and an <img> whose src does not change
          // never fires load again (it stayed "pending" for ever).
          key={request.startedAt}
          src={request.url}
          alt=""
          data-cut-poster
          data-cut-composed-frame
          style={{ opacity: visible && request.key === wantKey ? 1 : 0 }}
          onLoad={() => settle(request, true)}
          onError={() => settle(request, false)}
        />
      )}
      {busy && state === 'pending' && <div className="pv-spinner" data-cut-spinner />}
      {failedHere && (
        <div className="pv-chip pv-chip--warn pv-composed-frame-error" role="status" data-cut-composed-frame-failure>
          Exact frame not available: {failedHere}. Showing the playback preview.
        </div>
      )}
    </div>
  )
}
