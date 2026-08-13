export interface TimelineSourceWindow {
  startMs: number
  srcInMs: number
  srcOutMs: number
  speed?: number
  reverse?: boolean
  /** Offset into the visible source window of a held video frame. */
  freezeAtMs?: number | null
}

function playbackSpeed(window: TimelineSourceWindow): number {
  return window.speed && window.speed > 0 ? window.speed : 1
}

/** Map a timeline position inside a media clip to its source-media timestamp. */
export function sourceMsAtTimelinePosition(window: TimelineSourceWindow, atMs: number): number {
  // A freeze holds one source frame for the whole timeline slot. The engine
  // clamps the stored offset when the edit is committed; clamp again here so a
  // malformed or legacy project never resolves outside the half-open source
  // window.
  if (typeof window.freezeAtMs === 'number' && Number.isFinite(window.freezeAtMs)) {
    const maxSourceMs = Math.max(window.srcInMs, window.srcOutMs - 1)
    return Math.max(window.srcInMs, Math.min(maxSourceMs, window.srcInMs + Math.round(window.freezeAtMs)))
  }
  const sourceOffsetMs = Math.round((atMs - window.startMs) * playbackSpeed(window))
  return window.reverse
    ? window.srcOutMs - sourceOffsetMs
    : window.srcInMs + sourceOffsetMs
}

/** Inverse mapping used by the forward-playing media clock. */
export function timelineMsAtSourcePosition(window: TimelineSourceWindow, sourceMs: number): number {
  const sourceOffsetMs = window.reverse
    ? window.srcOutMs - sourceMs
    : sourceMs - window.srcInMs
  return window.startMs + sourceOffsetMs / playbackSpeed(window)
}
