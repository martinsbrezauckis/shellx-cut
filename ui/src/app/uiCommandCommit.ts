import type { MutableRefObject } from 'react'
import type { UiObservableState } from './uiControlState'

export interface UiCommitScheduler {
  requestAnimationFrame: (callback: FrameRequestCallback) => number
  cancelAnimationFrame: (handle: number) => void
  setTimeout: (callback: () => void, delayMs: number) => number
  clearTimeout: (handle: number) => void
}

export const UI_COMMIT_WALL_CLOCK_FALLBACK_MS = 50

/** Wait for a paint opportunity without depending on one. WebKit may suspend
 * requestAnimationFrame while a native window is occluded or its desktop is
 * inactive, but agent UI commands still need a bounded acknowledgement. */
export function waitForUiCommitTick(
  scheduler?: UiCommitScheduler,
  fallbackMs = UI_COMMIT_WALL_CLOCK_FALLBACK_MS,
): Promise<void> {
  const active = scheduler ?? {
    requestAnimationFrame: (callback: FrameRequestCallback) => window.requestAnimationFrame(callback),
    cancelAnimationFrame: (handle: number) => window.cancelAnimationFrame(handle),
    setTimeout: (callback: () => void, delayMs: number) => window.setTimeout(callback, delayMs),
    clearTimeout: (handle: number) => window.clearTimeout(handle),
  }

  return new Promise<void>((resolve) => {
    let settled = false
    let frameHandle: number | null = null
    let timerHandle: number | null = null
    const finish = () => {
      if (settled) return
      settled = true
      if (frameHandle !== null) active.cancelAnimationFrame(frameHandle)
      if (timerHandle !== null) active.clearTimeout(timerHandle)
      resolve()
    }
    timerHandle = active.setTimeout(finish, fallbackMs)
    frameHandle = active.requestAnimationFrame(finish)
  })
}

export async function waitForCommittedState(
  stateRef: MutableRefObject<UiObservableState>,
  previousRevision: number,
  predicate: (state: UiObservableState) => boolean,
  timeoutMs = 1_500,
  stopWhen?: (state: UiObservableState) => boolean,
): Promise<UiObservableState | null> {
  const deadline = performance.now() + timeoutMs
  while (performance.now() < deadline) {
    await waitForUiCommitTick()
    const state = stateRef.current
    if (state.state_revision > previousRevision) {
      if (predicate(state)) return state
      if (stopWhen?.(state)) return null
    }
  }
  return null
}
