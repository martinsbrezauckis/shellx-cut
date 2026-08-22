import assert from 'node:assert/strict'
import test from 'node:test'

import { runWithCleanup } from '../../ui/public-tests/lib/fullCoverageCleanup.mjs'

test('coverage cleanup runs after success and preserves its value', async () => {
  const events = []
  const value = await runWithCleanup(
    async () => { events.push('action'); return 42 },
    async () => { events.push('cleanup') },
  )
  assert.equal(value, 42)
  assert.deepEqual(events, ['action', 'cleanup'])
})

test('coverage cleanup preserves the primary action failure', async () => {
  const primary = new Error('primary')
  await assert.rejects(
    runWithCleanup(async () => { throw primary }, async () => {}),
    (error) => error === primary,
  )
})

test('coverage cleanup reports its own failure after a successful action', async () => {
  const cleanup = new Error('cleanup')
  await assert.rejects(
    runWithCleanup(async () => 'ok', async () => { throw cleanup }),
    (error) => error === cleanup,
  )
})

test('coverage cleanup retains both failures in action-first order', async () => {
  const primary = new Error('primary')
  const cleanup = new Error('cleanup')
  await assert.rejects(
    runWithCleanup(async () => { throw primary }, async () => { throw cleanup }, 'Library'),
    (error) => error instanceof AggregateError
      && error.message === 'Library and cleanup both failed'
      && error.errors[0] === primary
      && error.errors[1] === cleanup,
  )
})
