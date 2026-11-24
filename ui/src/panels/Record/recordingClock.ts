import { recordingStandsStill, type RecordingPauseBoundary, type RecordingPauseState } from './recordingPause'

/**
 * The recorded time of a take, which stands still while the take is paused.
 * `baseMs` is the recorded time at `since` (a wall-clock instant); the clock
 * counts up from there, or does not count at all while `since` is null.
 */
export interface RecordingClock {
  readonly baseMs: number
  readonly since: number | null
}

/** Restore only this take's clock. A different or malformed snapshot is never reused. */
export function restoreRecordingClock(saved: string | null, captureId: string, startedAt: number): RecordingClock {
  try {
    const value = JSON.parse(saved ?? 'null')
    const clock = value?.clock
    if (value?.captureId === captureId && clock && Number.isFinite(clock.baseMs) && clock.baseMs >= 0
      && (clock.since === null || (Number.isFinite(clock.since) && clock.since >= 0))) return clock
  } catch { /* A corrupt snapshot cannot supply recorded time. */ }
  return startRecordingClock(startedAt)
}

/** A take that started at `startedAt` (wall clock) and has not been paused yet. */
export function startRecordingClock(startedAt: number): RecordingClock {
  return { baseMs: 0, since: startedAt }
}

export function recordedMs(clock: RecordingClock, now: number): number {
  return Math.max(0, clock.baseMs + (clock.since === null ? 0 : now - clock.since))
}

/**
 * The clock after the pause state or the recorder's last acknowledgement changed.
 *
 * While the take is paused (or a Resume is not yet confirmed) the clock stands
 * still on the recorder's own recorded time when the Pause was confirmed; when
 * only a status poll said "paused" it stands still on the count so far. After a
 * confirmed Resume it counts on from the recorder's time. Calling it again with
 * the same inputs changes nothing.
 */
export function clockAfterPauseChange(
  clock: RecordingClock,
  state: RecordingPauseState,
  boundary: RecordingPauseBoundary | null,
  now: number,
): RecordingClock {
  if (recordingStandsStill(state)) {
    if (boundary?.state === 'paused') return { baseMs: boundary.ms, since: null }
    return clock.since === null ? clock : { baseMs: recordedMs(clock, now), since: null }
  }
  if (state === 'recording' || state === 'pausing') {
    if (boundary?.state === 'recording') return { baseMs: boundary.ms, since: boundary.at }
    return clock.since === null ? { baseMs: clock.baseMs, since: now } : clock
  }
  return clock
}

/**
 * Whole seconds left of the take's time limit; never below zero.
 *
 * The limit is not counted in recorded time: the recorder stops a take when the
 * wall clock passes its start plus the limit, and a pause does not move that
 * deadline (`macos_pause_capture.rs`, the `deadline` of the capture loop). So
 * this counts wall-clock time since the start of the take, through any pause,
 * while the timer beside it shows the recorded time.
 */
export function limitSecondsLeft(capMs: number, clock: RecordingClock, now: number): number {
  return Math.max(0, Math.ceil((capMs - recordedMs(clock, now)) / 1000))
}
