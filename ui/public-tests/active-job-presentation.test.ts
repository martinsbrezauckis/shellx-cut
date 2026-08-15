import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { activeJobLabel, activeJobProgress } from '../src/lib/jobPresentation'
import type { JobRecord } from '../src/lib/client'
import { activeJobViews, type JobView } from '../src/topbar/useTopbarJobs'

const job = (overrides: Partial<JobView> = {}): JobView => ({
  job_id: 'job_001',
  kind: 'render_queue',
  state: 'running',
  progress: 0.42,
  ...overrides,
})

assert.equal(activeJobLabel('render_queue'), 'Render queue')
assert.equal(activeJobLabel('reframe-portrait'), 'Reframing video')
assert.equal(activeJobLabel('custom_analysis'), 'Custom analysis')
assert.equal(activeJobProgress(job({ state: 'queued', progress: 0 })), 'waiting to start')
assert.equal(
  activeJobProgress(job({ state: 'queued', progress: 0, queue: { resource: 'analysis', max_running: 2 } })),
  'waiting for analysis capacity · 2 slots',
)
assert.equal(
  activeJobProgress(job({ state: 'queued', progress: 0, queue: { resource: 'screen_record.export', max_running: 1 } })),
  'waiting for screen record export capacity · 1 slot',
)
assert.equal(activeJobProgress(job()), '42% complete')
assert.equal(activeJobProgress(job({ progress: 2, message: 'finalizing output' })), '100% · finalizing output')

const record = (job_id: string, created_ts: string, state: JobRecord['state']): JobRecord => ({
  job_id,
  kind: 'render',
  state,
  progress: state === 'running' ? 0.2 : 0,
  created_ts,
  updated_ts: created_ts,
  ...(state === 'queued' ? { queue: { resource: 'render', max_running: 1 } } : {}),
})
const projected = activeJobViews([
  record('job_003', '2026-08-09T03:00:00Z', 'queued'),
  record('job_001', '2026-08-09T01:00:00Z', 'running'),
  record('job_002', '2026-08-09T02:00:00Z', 'queued'),
])
assert.deepEqual(projected.map((entry) => entry.job_id), ['job_001', 'job_002', 'job_003'])
assert.equal(activeJobProgress(projected[1]), '1 of 2 waiting for render capacity · 1 slot')
assert.equal(activeJobProgress(projected[2]), '2 of 2 waiting for render capacity · 1 slot')
const orchestrator = record('job_040', '2026-08-09T04:00:00Z', 'running')
orchestrator.kind = 'render_queue'
orchestrator.waiting_on = { job_id: 'job_042', kind: 'render' }
assert.deepEqual(activeJobViews([orchestrator])[0]?.waiting_on, orchestrator.waiting_on)
assert.equal(
  activeJobProgress(job({
    kind: 'render_queue',
    progress: 0.5,
    message: 'rendering delivery 2/4',
    waiting_on: { job_id: 'job_042', kind: 'render' },
  })),
  '50% · rendering delivery 2/4 · waiting on rendering video job_042',
)

const statusbarCss = readFileSync(new URL('../src/statusbar/statusbar.css', import.meta.url), 'utf8')
const cancelRule = statusbarCss.match(/[.]sb-job-cancel\s*\{([^}]*)\}/)?.[1] || ''
assert.match(cancelRule, /width:\s*24px/)
assert.match(cancelRule, /height:\s*24px/)
assert.match(cancelRule, /flex:\s*none/)

console.log('PASS active jobs use human labels and truthful queued/running progress')
