import assert from 'node:assert/strict'
import test from 'node:test'

import { createJobWaiters } from '../../ui/public-tests/lib/fullCoverageJobs.mjs'

test('import waiters follow import and enrichment jobs to terminal success', async () => {
  const states = new Map([
    ['import-1', [
      { state: 'running' },
      { state: 'done', result: { enrich_job: 'enrich-1' } },
    ]],
    ['enrich-1', [
      { state: 'running' },
      { state: 'done', result: { report: 'ready' } },
    ]],
  ])
  const verb = async (_name, { job_id: jobId }) => {
    const queue = states.get(jobId) || []
    return { result: queue.length > 1 ? queue.shift() : queue[0] }
  }
  const { awaitImportJobs } = createJobWaiters({
    verb,
    sleep: async () => {},
  })

  const terminal = await awaitImportJobs(
    { result: { job_id: 'import-1' } },
    1_000,
  )
  assert.equal(terminal.state, 'done')
  assert.equal(terminal.result.report, 'ready')
})

test('import waiters fail loudly when an enrichment job fails', async () => {
  const verb = async (_name, { job_id: jobId }) => ({
    result: jobId === 'import-2'
      ? { state: 'done', result: { enrich_job: 'enrich-2' } }
      : { state: 'failed', error: { code: 'fixture_failure' } },
  })
  const { awaitImportJobs } = createJobWaiters({
    verb,
    sleep: async () => {},
  })

  await assert.rejects(
    awaitImportJobs({ result: { job_id: 'import-2' } }, 1_000),
    /media enrichment job enrich-2 failed.*fixture_failure/,
  )
})

test('import waiters fail loudly instead of leaking a timed-out job into later sections', async () => {
  const { awaitImportJobs } = createJobWaiters({
    verb: async () => ({ result: { state: 'running' } }),
    sleep: async () => {},
  })

  await assert.rejects(
    awaitImportJobs({ result: { job_id: 'import-3' } }, 0),
    /media import job import-3 timed out after 0ms/,
  )
})
