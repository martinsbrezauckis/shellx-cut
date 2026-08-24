import assert from 'node:assert/strict'
import { test } from 'node:test'

import { reapStatusbarCancellation } from './lib/fullCoverageStatusbarActions.mjs'

test('statusbar cancellation recovery cancels and drains a queued render after a row failure', async () => {
  const primaryError = new Error('selector timed out')
  const calls = []
  const terminal = await reapStatusbarCancellation({
    jobId: 'job_render_1',
    terminal: null,
    primaryError,
    cancelAndAwaitTerminal: async (jobId, options) => {
      calls.push({ jobId, options })
      return {
        cancellation: { ok: true },
        terminal: { state: 'failed', error: { code: 'job_cancelled' } },
      }
    },
  })

  assert.deepEqual(calls, [{ jobId: 'job_render_1', options: { timeoutMs: 45_000 } }])
  assert.equal(terminal.state, 'failed')
  assert.equal(terminal.error.code, 'job_cancelled')
})

test('statusbar cancellation recovery preserves the row failure when cleanup cannot reap', async () => {
  const primaryError = new Error('running state timed out')
  await assert.rejects(
    reapStatusbarCancellation({
      jobId: 'job_render_2',
      terminal: null,
      primaryError,
      cancelAndAwaitTerminal: async () => ({
        cancellation: { ok: true },
        terminal: null,
        lastStatus: { state: 'running' },
      }),
    }),
    (error) => {
      assert.ok(error instanceof AggregateError)
      assert.equal(error.errors[0], primaryError)
      assert.match(error.errors[1].message, /did not reap job_render_2/)
      return true
    },
  )
})

test('statusbar cancellation recovery preserves the row failure when cleanup rejects', async () => {
  const primaryError = new Error('cancel selector disappeared')
  const cleanupError = new Error('jobs.cancel transport failed')
  await assert.rejects(
    reapStatusbarCancellation({
      jobId: 'job_render_3',
      terminal: null,
      primaryError,
      cancelAndAwaitTerminal: async () => {
        throw cleanupError
      },
    }),
    (error) => {
      assert.ok(error instanceof AggregateError)
      assert.deepEqual(error.errors, [primaryError, cleanupError])
      return true
    },
  )
})
