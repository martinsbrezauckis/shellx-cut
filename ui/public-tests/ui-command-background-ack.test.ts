import assert from 'node:assert/strict'

import {
  waitForUiCommitTick,
  type UiCommitScheduler,
} from '../src/app/useUiCommandController'
import {
  notifyUiStateCommitted,
  waitForCommittedState,
} from '../src/app/uiCommandCommit'
import type { UiObservableState } from '../src/app/uiControlState'

// A background or occluded WKWebView can suspend requestAnimationFrame. The
// UI-command acknowledgement loop must still advance on a wall-clock timer.
{
  const cancelledFrames: number[] = []
  const clearedTimers: number[] = []
  const scheduler: UiCommitScheduler = {
    requestAnimationFrame: () => 41,
    cancelAnimationFrame: (handle) => cancelledFrames.push(handle),
    setTimeout: (callback) => {
      queueMicrotask(callback)
      return 73
    },
    clearTimeout: (handle) => clearedTimers.push(handle),
  }
  await waitForUiCommitTick(scheduler)
  assert.deepEqual(cancelledFrames, [41])
  assert.deepEqual(clearedTimers, [73])
}

{
  const cancelledFrames: number[] = []
  const clearedTimers: number[] = []
  const scheduler: UiCommitScheduler = {
    requestAnimationFrame: (callback) => {
      queueMicrotask(() => callback(0))
      return 42
    },
    cancelAnimationFrame: (handle) => cancelledFrames.push(handle),
    setTimeout: () => 74,
    clearTimeout: (handle) => clearedTimers.push(handle),
  }
  await waitForUiCommitTick(scheduler)
  assert.deepEqual(cancelledFrames, [42])
  assert.deepEqual(clearedTimers, [74])
}

// Confirmation under load: a surface commit arriving after 1.5 s (1.8 s here)
// must still be accepted inside the bounded confirmation window (~6 s), while
// returning null promptly once the caller's deadline expires.
{
  const stateRef = {
    current: { state_revision: 1, open_surface_ids: ['preview'] } as unknown as UiObservableState,
  }
  const timer = setTimeout(() => {
    stateRef.current = { state_revision: 2, open_surface_ids: ['preview', 'color'] } as unknown as UiObservableState
  }, 1_800)
  try {
    const confirmed = await waitForCommittedState(
      stateRef,
      1,
      (state) => state.open_surface_ids.includes('color'),
    )
    assert.ok(confirmed, 'delayed confirmation at 1.8 s must succeed within bounded window')
    assert.equal(confirmed.state_revision, 2)
  } finally {
    clearTimeout(timer)
  }

  const unconfirmed = await waitForCommittedState(
    stateRef,
    2,
    (state) => state.open_surface_ids.includes('never-opens'),
    120,
  )
  assert.equal(unconfirmed, null, 'uncommitted state must still resolve null at the deadline')

  // Explicit commit notification wakes concurrent waiters promptly without waiting for fallback ticks.
  const firstWait = waitForCommittedState(stateRef, 2, (state) => state.open_surface_ids.includes('assets'))
  const secondWait = waitForCommittedState(stateRef, 2, (state) => state.open_surface_ids.includes('audio'))
  const commitStart = performance.now()
  stateRef.current = { state_revision: 3, open_surface_ids: ['preview', 'assets'] } as unknown as UiObservableState
  notifyUiStateCommitted(stateRef)
  const firstResolved = await firstWait
  assert.equal(firstResolved?.state_revision, 3)
  stateRef.current = { state_revision: 4, open_surface_ids: ['preview', 'assets', 'audio'] } as unknown as UiObservableState
  notifyUiStateCommitted(stateRef)
  const secondResolved = await secondWait
  assert.equal(secondResolved?.state_revision, 4)
  assert.ok(performance.now() - commitStart < 250, 'notifyUiStateCommitted wakes concurrent waiters immediately')
}

console.log('PASS ui commands retain bounded acknowledgements when animation frames are suspended')
