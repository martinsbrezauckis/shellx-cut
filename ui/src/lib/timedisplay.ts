// lib/timedisplay — app-wide time-readout mode (ms ↔ frames ↔ SMPTE).
// Role: a tiny shared store so every clock readout (timeline ruler + tc chip,
// preview transport, statusbar) shows the SAME format — a timeline in frames
// while the transport stays in ms reads as half-built. Persisted to localStorage
// and broadcast via a DOM CustomEvent so components stay in sync live without
// prop-drilling. The pure formatters live in panels/Timeline/layout (formatClock,
// rulerTicks, framesOf, timecodeSmpte); this module only owns the MODE + subscription.
// Receipt/rationale strings keep using the raw ms `timecode()` — they are machine
// evidence, not display, and must not change with this UI toggle.
// Callers: Timeline (toggle + ruler + chip), Preview (transport), statusbar.

import { useEffect, useState } from 'react'
import type { TimeDisplayMode } from '../panels/Timeline/layout'

const KEY = 'cut.timeDisplay'
const EVT = 'cut:timedisplay'
const ORDER: TimeDisplayMode[] = ['ms', 'frames', 'smpte']
const isTimeDisplayMode = (v: unknown): v is TimeDisplayMode => v === 'ms' || v === 'frames' || v === 'smpte'

/** Short badge shown next to the readout so the active mode is legible. */
export const TIME_DISPLAY_LABEL: Record<TimeDisplayMode, string> = {
  ms: 'MS',
  frames: 'FR',
  smpte: 'TC',
}

/** Human title for the toggle control. */
export const TIME_DISPLAY_TITLE: Record<TimeDisplayMode, string> = {
  ms: 'Time: milliseconds (HH:MM:SS.mmm) — click for frames',
  frames: 'Time: frames (absolute frame number) — click for SMPTE timecode',
  smpte: 'Time: SMPTE timecode (HH:MM:SS:FF) — click for milliseconds',
}

export function getTimeDisplay(): TimeDisplayMode {
  try {
    const v = localStorage.getItem(KEY)
    if (isTimeDisplayMode(v) && ORDER.includes(v)) return v
  } catch { /* localStorage unavailable — fall through to default */ }
  return 'ms'
}

export function setTimeDisplay(mode: TimeDisplayMode): void {
  try { localStorage.setItem(KEY, mode) } catch { /* ignore quota/denied */ }
  document.dispatchEvent(new CustomEvent(EVT, { detail: mode }))
}

/** Advance ms → frames → smpte → ms and broadcast; returns the new mode. */
export function cycleTimeDisplay(): TimeDisplayMode {
  const next = ORDER[(ORDER.indexOf(getTimeDisplay()) + 1) % ORDER.length]
  setTimeDisplay(next)
  return next
}

/** Subscribe a component to the shared time-display mode (re-renders on change). */
export function useTimeDisplay(): TimeDisplayMode {
  const [mode, setMode] = useState<TimeDisplayMode>(getTimeDisplay)
  useEffect(() => {
    const on = (e: Event) => {
      if (e instanceof CustomEvent && isTimeDisplayMode(e.detail)) setMode(e.detail)
    }
    document.addEventListener(EVT, on)
    return () => document.removeEventListener(EVT, on)
  }, [])
  return mode
}
