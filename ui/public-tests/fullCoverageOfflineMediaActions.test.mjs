import assert from 'node:assert/strict'
import {
  RELINK_FIXTURE_UNLINK_MAX_ATTEMPTS,
  RELINK_FIXTURE_UNLINK_RETRY_MS,
  unlinkRelinkFixture,
} from './lib/fullCoverageOfflineMediaActions.mjs'

function fixtureError(code) {
  const error = new Error(`fixture ${code}`)
  error.code = code
  return error
}

{
  let attempts = 0
  const delays = []
  await unlinkRelinkFixture('C:\\fixture\\relink.mp4', {
    unlinkFn: async () => {
      attempts++
      if (attempts < 3) throw fixtureError('EBUSY')
    },
    sleepFn: async (delayMs) => { delays.push(delayMs) },
  })
  assert.equal(attempts, 3, 'retries a transient Windows media lock')
  assert.deepEqual(delays, [RELINK_FIXTURE_UNLINK_RETRY_MS, RELINK_FIXTURE_UNLINK_RETRY_MS],
    'uses the bounded retry delay for each transient lock')
}

{
  let attempts = 0
  let delayed = false
  const original = fixtureError('ENOENT')
  await assert.rejects(
    () => unlinkRelinkFixture('/fixture/missing.mp4', {
      unlinkFn: async () => {
        attempts++
        throw original
      },
      sleepFn: async () => { delayed = true },
    }),
    (error) => error.code === 'ENOENT' && error.cause === original,
    'does not retry a non-lock fixture failure',
  )
  assert.equal(attempts, 1, 'non-lock failures remain immediate')
  assert.equal(delayed, false, 'non-lock failures do not wait')
}

{
  let attempts = 0
  const original = fixtureError('EBUSY')
  await assert.rejects(
    () => unlinkRelinkFixture('C:\\fixture\\still-locked.mp4', {
      unlinkFn: async () => {
        attempts++
        throw original
      },
      sleepFn: async () => {},
    }),
    (error) => error.code === 'EBUSY'
      && error.cause === original
      && error.message.includes(`after ${RELINK_FIXTURE_UNLINK_MAX_ATTEMPTS} attempts`),
    'reports a persistent Windows lock after the bounded retry budget',
  )
  assert.equal(attempts, RELINK_FIXTURE_UNLINK_MAX_ATTEMPTS,
    'persistent locks cannot retry indefinitely')
}

console.log('PASS fullCoverageOfflineMediaActions bounded Windows fixture unlink')
