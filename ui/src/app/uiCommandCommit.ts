import type { MutableRefObject } from 'react'
import type { UiObservableState } from './uiControlState'

export interface UiCommitScheduler {
  requestAnimationFrame: (callback: FrameRequestCallback) => number
  cancelAnimationFrame: (handle: number) => void
  setTimeout: (callback: () => void, delayMs: number) => number
  clearTimeout: (handle: number) => void
}

export const UI_COMMIT_WALL_CLOCK_FALLBACK_MS = 50
export const UI_COMMAND_CONFIRM_TIMEOUT_MS = 6_000

const DEFAULT_SCHEDULER: UiCommitScheduler = {
  requestAnimationFrame: (callback: FrameRequestCallback) =>
    typeof window !== 'undefined' && typeof window.requestAnimationFrame === 'function'
      ? window.requestAnimationFrame(callback)
      : globalThis.setTimeout(() => callback(performance.now()), 16) as unknown as number,
  cancelAnimationFrame: (handle: number) => {
    if (typeof window !== 'undefined' && typeof window.cancelAnimationFrame === 'function') {
      window.cancelAnimationFrame(handle)
    } else {
      globalThis.clearTimeout(handle)
    }
  },
  setTimeout: (callback: () => void, delayMs: number) => globalThis.setTimeout(callback, delayMs) as unknown as number,
  clearTimeout: (handle: number) => globalThis.clearTimeout(handle),
}

const commitListeners = new WeakMap<MutableRefObject<UiObservableState>, Set<() => void>>()

/** Wake any pending UI-command confirmation waiter immediately when
 * useUiStatePublisher commits a new UiObservableState revision. */
export function notifyUiStateCommitted(stateRef: MutableRefObject<UiObservableState>): void {
  const listeners = commitListeners.get(stateRef)
  if (!listeners) return
  for (const listener of [...listeners]) listener()
}

export function waitForUiCommitTick(scheduler: UiCommitScheduler = DEFAULT_SCHEDULER): Promise<void> {
  return new Promise((resolve) => {
    let settled = false
    let frame = 0
    let timer = 0
    const finish = () => {
      if (settled) return
      settled = true
      scheduler.cancelAnimationFrame(frame)
      scheduler.clearTimeout(timer)
      resolve()
    }
    timer = scheduler.setTimeout(finish, UI_COMMIT_WALL_CLOCK_FALLBACK_MS)
    frame = scheduler.requestAnimationFrame(() => finish())
  })
}

export async function waitForCommittedState(
  stateRef: MutableRefObject<UiObservableState>,
  previousRevision: number,
  predicate: (state: UiObservableState) => boolean,
  timeoutMs = UI_COMMAND_CONFIRM_TIMEOUT_MS,
  stopWhen?: (state: UiObservableState) => boolean,
  scheduler: UiCommitScheduler = DEFAULT_SCHEDULER,
): Promise<UiObservableState | null> {
  const check = () => {
    const state = stateRef.current
    if (state.state_revision > previousRevision) {
      if (predicate(state)) return { done: true as const, state }
      if (stopWhen?.(state)) return { done: true as const, state: null }
    }
    return null
  }
  const immediate = check()
  if (immediate) return immediate.state

  let listeners = commitListeners.get(stateRef)
  if (!listeners) {
    listeners = new Set()
    commitListeners.set(stateRef, listeners)
  }

  const deadline = performance.now() + timeoutMs
  while (true) {
    const remaining = deadline - performance.now()
    if (remaining <= 0) break
    let onCommit: (() => void) | undefined
    const commitSignal = new Promise<void>((resolve) => {
      onCommit = () => resolve()
      listeners!.add(onCommit)
    })
    let timer = 0
    const timeoutSignal = new Promise<void>((resolve) => {
      timer = scheduler.setTimeout(resolve, Math.min(remaining, UI_COMMIT_WALL_CLOCK_FALLBACK_MS))
    })
    let raf = 0
    const rafSignal = new Promise<void>((resolve) => {
      raf = scheduler.requestAnimationFrame(() => resolve())
    })
    try {
      await Promise.race([commitSignal, timeoutSignal, rafSignal])
    } finally {
      if (onCommit) listeners.delete(onCommit)
      scheduler.clearTimeout(timer)
      scheduler.cancelAnimationFrame(raf)
    }
    const resolved = check()
    if (resolved) return resolved.state
  }
  return null
}
