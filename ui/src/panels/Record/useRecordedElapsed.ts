import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { clockAfterPauseChange, limitSecondsLeft, recordedMs, restoreRecordingClock, type RecordingClock } from './recordingClock'
import type { RecordingPauseBoundary, RecordingPauseState } from './recordingPause'

interface RecordedElapsedInput {
  captureId: string | null
  /** True while a take is being recorded. */
  recording: boolean
  /** Wall-clock start of the take (first frame); null before there is one. */
  startedAt: number | null
  pauseState: RecordingPauseState
  pauseBoundary: RecordingPauseBoundary | null
  /** Optional time limit in milliseconds. */
  capMs: number | null
}

/**
 * Recorded seconds of the running take and, with a time limit, seconds left.
 * The elapsed count follows the recorder's recorded time, so it stands still
 * while the take is paused (a wall-clock count would keep running through the
 * pause and stay ahead of the take after Resume). The seconds left follow the
 * recorder's limit, which is a wall-clock deadline that a pause does not move.
 */
export function useRecordedElapsed({ captureId, recording, startedAt, pauseState, pauseBoundary, capMs }: RecordedElapsedInput) {
  const [elapsed, setElapsed] = useState(0)
  const [remaining, setRemaining] = useState(0)
  const clockRef = useRef<RecordingClock | null>(null)
  const takeRef = useRef<string | null>(null)
  const [clockWarning, setClockWarning] = useState<string | null>(null)
  const getRecordedMs = useCallback(() => clockRef.current ? recordedMs(clockRef.current, Date.now()) : 0, [])
  const tick = useCallback(() => {
    const clock = clockRef.current
    if (!clock) return
    const now = Date.now()
    setElapsed(Math.floor(recordedMs(clock, now) / 1000))
    setRemaining(capMs === null ? 0 : limitSecondsLeft(capMs, clock, now))
  }, [capMs])

  useLayoutEffect(() => {
    if (!recording || startedAt === null || !captureId) { clockRef.current = null; takeRef.current = null; return }
    const key = 'cut.recorded-clock'
    if (clockRef.current === null || takeRef.current !== captureId) {
      let saved: string | null = null
      try { saved = sessionStorage.getItem(key) }
      catch { setClockWarning('The recording timer cannot be kept after a reload in this window. Keep this page open until you stop.') }
      clockRef.current = restoreRecordingClock(saved, captureId, startedAt)
      takeRef.current = captureId
    }
    clockRef.current = clockAfterPauseChange(clockRef.current, pauseState, pauseBoundary, Date.now())
    try { sessionStorage.setItem(key, JSON.stringify({ captureId, clock: clockRef.current })) }
    catch { setClockWarning('The recording timer cannot be kept after a reload in this window. Keep this page open until you stop.') }
    tick()
  }, [captureId, recording, startedAt, pauseState, pauseBoundary, tick])

  useEffect(() => {
    if (!recording || !startedAt) return
    const timer = window.setInterval(tick, 250)
    return () => window.clearInterval(timer)
  }, [recording, startedAt, tick])

  return { elapsed, remaining, getRecordedMs, clockWarning }
}
