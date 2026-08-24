import assert from 'node:assert/strict'

import { waitForUiCommitTick, type UiCommitScheduler } from '../src/app/useUiCommandController'

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

console.log('PASS ui commands retain bounded acknowledgements when animation frames are suspended')
