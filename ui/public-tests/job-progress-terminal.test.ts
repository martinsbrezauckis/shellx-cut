import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import { cutEventFrom } from '../src/lib/events'

// An ended job keeps the progress it reached, so a reader learns that it
// ENDED from the event's `terminal` flag, never from progress >= 1.
const running = cutEventFrom({ type: 'job_progress', job_id: 'j1', kind: 'render', progress: 1, message: 'almost' })
assert.deepEqual(running, { type: 'job_progress', job_id: 'j1', kind: 'render', progress: 1, message: 'almost' },
  'progress 1 on a non-terminal event is not an ending')
const cancelled = cutEventFrom({ type: 'job_progress', job_id: 'j2', kind: 'render', progress: 0.4, message: 'cancelled', terminal: true })
assert.deepEqual(cancelled, { type: 'job_progress', job_id: 'j2', kind: 'render', progress: 0.4, message: 'cancelled', terminal: true })
const bogus = cutEventFrom({ type: 'job_progress', job_id: 'j3', kind: 'render', progress: 0.1, terminal: 'yes' })
assert.equal(bogus && 'terminal' in bogus, false, 'only a real boolean true counts')

const topbar = readFileSync(new URL('../src/topbar/useTopbarJobs.ts', import.meta.url), 'utf8')
const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8')
assert.doesNotMatch(topbar, /ev\.progress\s*>=\s*1/, 'the job tracker must not read progress as an ending')
assert.doesNotMatch(app, /ev\.progress\s*>=\s*1/, 'the project refresh must not read progress as an ending')
assert.match(topbar, /if \(ev\.terminal\)/)
console.log('PASS job progress events: ending comes from terminal, not progress')

// The Preview monitor's "proxy building" dot ends with the job, not with the
// wording of its last message (a cancelled or failed proxy says neither "done").
const preview = readFileSync(new URL('../src/panels/Preview/index.tsx', import.meta.url), 'utf8')
assert.match(preview, /if \(ev\.terminal && \(ev\.kind === 'import_chain' \|\| ev\.kind === 'proxy'\)\) setProxyBuilding\(false\)/)
console.log('PASS the proxy dot ends on the job ending')
