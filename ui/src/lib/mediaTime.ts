export interface TimelineSourceWindow {
  startMs: number
  srcInMs: number
  srcOutMs: number
  speed?: number
  reverse?: boolean
}

function playbackSpeed(window: TimelineSourceWindow): number {
  return window.speed && window.speed > 0 ? window.speed : 1
}

/** Map a timeline position inside a media clip to its source-media timestamp. */
export function sourceMsAtTimelinePosition(window: TimelineSourceWindow, atMs: number): number {
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
