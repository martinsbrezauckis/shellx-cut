export interface TimelineSourceWindow {
  startMs: number
  srcInMs: number
  srcOutMs: number
  speed?: number
  reverse?: boolean
  /** Offset into the visible source window of a held video frame. */
  freezeAtMs?: number | null
}

// Cut's editable/source-navigation clock is whole milliseconds: `src_*_ms`,
// timeline positions, and Source Monitor `at_ms` seeks are all integer ms.
// Project frame rate affects display and export quantization separately, so a
// source mapping must not guess a fractional frame duration here.
const SOURCE_TIMEBASE_MS = 1

function playbackSpeed(window: TimelineSourceWindow): number {
  return window.speed && window.speed > 0 ? window.speed : 1
}

/** The final addressable instant in Cut's half-open source-ms window. */
function lastSourceMs(window: TimelineSourceWindow): number {
  // The engine only emits positive source windows. Keep malformed legacy data
  // total here; source-navigation callers refuse a missing range before use.
  return Math.max(window.srcInMs, window.srcOutMs - SOURCE_TIMEBASE_MS)
}

function clampSourceMs(window: TimelineSourceWindow, sourceMs: number): number {
  return Math.max(window.srcInMs, Math.min(lastSourceMs(window), sourceMs))
}

/** Map a timeline position inside a media clip to its source-media timestamp. */
export function sourceMsAtTimelinePosition(window: TimelineSourceWindow, atMs: number): number {
  // A freeze holds one source frame for the whole timeline slot. The engine
  // clamps the stored offset when the edit is committed; clamp again here so a
  // malformed or legacy project never resolves outside the half-open source
  // window.
  if (typeof window.freezeAtMs === 'number' && Number.isFinite(window.freezeAtMs)) {
    return clampSourceMs(window, window.srcInMs + Math.round(window.freezeAtMs))
  }
  const sourceOffsetMs = Math.round((atMs - window.startMs) * playbackSpeed(window))
  // Reverse begins on the final *included* source instant. `srcOutMs` is the
  // exclusive boundary, so seeking it would open the following frame/window.
  const sourceMs = window.reverse
    ? window.srcOutMs - sourceOffsetMs
    : window.srcInMs + sourceOffsetMs
  return clampSourceMs(window, sourceMs)
}

/** Inverse mapping used by the forward-playing media clock. */
export function timelineMsAtSourcePosition(window: TimelineSourceWindow, sourceMs: number): number {
  const sourceOffsetMs = window.reverse
    ? window.srcOutMs - sourceMs
    : sourceMs - window.srcInMs
  return window.startMs + sourceOffsetMs / playbackSpeed(window)
}
