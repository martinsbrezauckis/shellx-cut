import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import type { JobRecord } from '../src/lib/client'
import {
  failedJobRetryViews,
  retryAdmissionLine,
} from '../src/panels/Environment/jobRetryRecoveryModel'

function job(overrides: Partial<JobRecord>): JobRecord {
  return {
    job_id: 'job_001',
    kind: 'screen_record_export',
    state: 'failed',
    progress: 1,
    created_ts: '2026-09-02T10:00:00Z',
    updated_ts: '2026-09-02T10:00:00Z',
    ...overrides,
  }
}

const eligible = job({
  job_id: 'job_010',
  retry: { eligible: true, root_job_id: 'job_001', attempt: 2, retry_of: 'job_001' },
})
const ineligible = job({
  job_id: 'job_011',
  retry: {
    eligible: false,
    reason: 'recording source changed since the failed export',
    root_job_id: 'job_011',
    attempt: 1,
  },
})
const cancelled = job({
  job_id: 'job_012',
  outcome: 'cancelled',
  outcome_reason: 'project_switch_cancelled',
  retry: { eligible: true, root_job_id: 'job_012', attempt: 1 },
})
const superseded = job({
  job_id: 'job_013',
  outcome: 'superseded',
  outcome_reason: 'superseded',
  retry: { eligible: false, root_job_id: 'job_013', attempt: 1 },
})
const interrupted = job({
  job_id: 'job_014',
  outcome: 'interrupted',
  outcome_reason: 'restart_interrupted',
  retry: { eligible: true, root_job_id: 'job_014', attempt: 1 },
})
const queuedProjection = job({
  job_id: 'job_015',
  state: 'queued',
  retry: { eligible: true, root_job_id: 'job_014', attempt: 1 },
})

const views = failedJobRetryViews([queuedProjection, eligible, ineligible, cancelled, superseded, interrupted])
assert.deepEqual(views.map((view) => view.jobId), ['job_014', 'job_013', 'job_012', 'job_011', 'job_010'], 'only failed records appear in the recovery list, newest first')
assert.equal(views.find((view) => view.jobId === 'job_010')?.canRetry, true, 'only an engine-eligible failure can retry')
assert.equal(views.find((view) => view.jobId === 'job_011')?.canRetry, false, 'ineligible projection never exposes a retry action')
assert.match(views.find((view) => view.jobId === 'job_011')?.unavailableReason ?? '', /source changed/, 'engine ineligible reason is visible')
assert.equal(views.find((view) => view.jobId === 'job_012')?.terminalLabel, 'Cancelled', 'cancelled state stays distinct from a true failure')
assert.equal(views.find((view) => view.jobId === 'job_012')?.canRetry, false, 'a cancelled record stays unavailable even if a malformed projection says eligible')
assert.equal(views.find((view) => view.jobId === 'job_013')?.terminalLabel, 'Superseded', 'superseded state stays distinct from a true failure')
assert.equal(views.find((view) => view.jobId === 'job_014')?.terminalLabel, 'Interrupted', 'restart interruption stays distinct from a true failure')
assert.equal(views.find((view) => view.jobId === 'job_014')?.canRetry, true, 'the engine may safely retry a restart-interrupted job')
assert.equal(views.find((view) => view.jobId === 'job_010')?.lineage, 'Attempt 2 · root job_001 · retry of job_001', 'retry lineage is explicit before admission')
assert.equal(
  retryAdmissionLine({ job_id: 'job_015', retry_of: 'job_010', root_job_id: 'job_001', attempt: 3 }),
  'Retry queued · attempt 3 · retry of job_010 · root job_001',
  'admission reports the new attempt and durable lineage',
)

const component = readFileSync(new URL('../src/panels/Environment/JobRetryRecovery.tsx', import.meta.url), 'utf8')
const statusbar = readFileSync(new URL('../src/statusbar/index.tsx', import.meta.url), 'utf8')
assert.match(component, /if \(!job[.]canRetry \|\| retrying[.]current[.]has\(job[.]jobId\)/, 'a synchronous guard rejects duplicate retry clicks')
assert.match(component, /retrying[.]current[.]add\(job[.]jobId\)/, 'duplicate guard is armed before the request')
assert.match(component, /callVerb\('jobs[.]retry'/, 'the sole retry control invokes jobs.retry')
assert.match(component, /data-cut-job-retry-unavailable=/, 'ineligible reason has a stable UI selector')
assert.match(component, /data-cut-job-retry-admitted=/, 'admitted retry lineage has a stable UI selector')
assert.match(statusbar, /onOpenEnvironment\('health-recovery'\)/, 'status bar delegates retry review to the single Health owner')
assert.doesNotMatch(statusbar, /callVerb\('jobs[.]retry'/, 'status bar cannot race the Health retry action')

console.log('PASS durable job retry controls preserve eligibility, terminal distinctions, and admitted lineage')
