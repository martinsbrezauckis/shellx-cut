// A job that fails and cannot be retried is shown in the status bar to the
// person, whoever started it. This pins which job records become notices, how
// they are ordered and capped, what the row says, and how it is wired.

import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { JobRecord } from '../src/lib/client'
import { FAILED_JOB_NOTICE_LIMIT, failedJobNotices } from '../src/lib/failedJobNotices'
import { jobFailureText } from '../src/lib/jobFailureText'
import { jobTaskName } from '../src/lib/jobPresentation'
import { failedJobRetryViews } from '../src/panels/Environment/jobRetryRecoveryModel'
import { FailedJobRows } from '../src/statusbar/FailedJobRows'
import { interruptedJobNotices, interruptedKey, readInterruptedJobs } from '../src/lib/interruptedJobNotices'

const T0 = Date.parse('2026-01-01T10:00:00Z')
const REASON = "this asset's audio is too long to label speakers (about 2h 01m 40s); the limit is 2h 00m 00s of audio per run."
const ACTION = 'Trim or split the asset and run speaker labelling on the parts.'

function failed(id: string, at: string, over: Partial<JobRecord> = {}): JobRecord {
  return {
    job_id: id, kind: 'diarize', state: 'failed', outcome: 'failed', outcome_reason: 'true_failure', progress: 0,
    created_ts: at, updated_ts: at,
    error: { code: 'invalid_args', message: REASON, cause: 'too long', suggested_action: ACTION },
    ...over,
  } as JobRecord
}
const none = new Set<string>()
const ids = (records: JobRecord[], dismissed: ReadonlySet<string> = none) => failedJobNotices(records, T0, dismissed).shown.map((n) => n.job_id)

// Text: the reason and the next step, under the name of what failed.
const own = failed('job_a', '2026-01-01T10:05:00Z')
const [notice] = failedJobNotices([own], T0, none).shown
assert.equal(notice.title, 'Speaker labelling failed')
assert.ok(notice.text.startsWith('This asset') && notice.text.includes('too long to label speakers'), notice.text)
assert.ok(notice.text.endsWith(ACTION), notice.text)
assert.equal(notice.text, jobFailureText(own, ''), 'the status bar and every other surface use one text')
assert.equal(
  failedJobNotices([failed('x', '2026-01-01T10:05:00Z', { error: undefined, kind: 'transcribe' })], T0, none).shown[0].text,
  'Transcription did not finish.', 'a failure without an error still says what did not finish',
)

// Who started it does not matter: any kind that failed is shown.
for (const kind of ['render', 'proxy', 'transcribe', 'import_chain', 'asset_generate', 'judge', 'fetch_tool', 'made_up_kind']) {
  assert.deepEqual(ids([failed('k', '2026-01-01T10:05:00Z', { kind })]), ['k'], kind + ' is shown')
}

// Not shown: retry-eligible (own row), cancelled, replaced, interrupted, still running, done, old.
assert.deepEqual(ids([failed('r', '2026-01-01T10:05:00Z', { retry: { eligible: true, root_job_id: 'r', attempt: 1 } })]), [], 'the retry row owns retryable failures')
assert.deepEqual(ids([failed('r', '2026-01-01T10:05:00Z', { retry: { eligible: false, reason: 'no recipe', root_job_id: 'r', attempt: 1 } })]), ['r'], 'a failure the engine will not retry is shown')
for (const outcome of ['cancelled', 'superseded', 'interrupted'] as const) {
  assert.deepEqual(ids([failed('c', '2026-01-01T10:05:00Z', { outcome })]), [], outcome + ' is not a failure')
}
for (const outcome_reason of ['user_cancelled', 'project_switch_cancelled', 'superseded', 'restart_interrupted'] as const) {
  assert.deepEqual(ids([failed('c', '2026-01-01T10:05:00Z', { outcome: undefined, outcome_reason })]), [], outcome_reason + ' is not a failure')
}
for (const code of ['job_cancelled', 'render_cancelled', 'job_superseded']) {
  assert.deepEqual(ids([failed('c', '2026-01-01T10:05:00Z', { outcome: undefined, outcome_reason: undefined, error: { code, message: 'x', cause: 'y' } })]), [], code + ' on an older record is not a failure')
}
assert.deepEqual(ids([failed('r', '2026-01-01T10:05:00Z', { state: 'running' }), failed('d', '2026-01-01T10:05:00Z', { state: 'done' })]), [], 'only failed jobs')
assert.deepEqual(ids([failed('old', '2025-12-31T10:00:00Z')]), [], 'a failure from before the project was opened is not replayed')

// Dismissal, order and the cap.
assert.deepEqual(ids([own], new Set(['job_a'])), [], 'a dismissed failure is gone')
const five = [1, 2, 3, 4, 5].map((n) => failed('j' + n, `2026-01-01T10:0${n}:00Z`))
const capped = failedJobNotices(five, T0, none)
assert.equal(FAILED_JOB_NOTICE_LIMIT, 3)
assert.deepEqual(capped.shown.map((n) => n.job_id), ['j3', 'j4', 'j5'], 'the newest few are shown, oldest of them first')
assert.equal(capped.hidden, 2, 'older failures are counted, not dropped silently')
const afterDismiss = failedJobNotices(five, T0, new Set(['j5']))
assert.deepEqual(afterDismiss.shown.map((n) => n.job_id), ['j2', 'j3', 'j4'], 'dismissing one brings the next older one back')
assert.equal(afterDismiss.hidden, 1)

// An export written but NOT passing its own checks (outcome_reason
// checks_failed, state done) is shown the same way, with the checks' words.
const SUMMARY = 'The export was written but did not pass its checks (duration_matches_edl, sources_decodable): The file is not as long as the timeline: part of the picture or sound is missing.'
const WAY = 'Import a complete copy of the cut-off or damaged source (or replace it on the timeline), then export again.'
const problem = failed('p1', '2026-01-01T10:06:00Z', {
  kind: 'render', state: 'done', completion: 'done_with_warnings', outcome: 'partial', outcome_reason: 'checks_failed', error: undefined,
  result: { path: '/x/final.mp4', checks: { verdict: 'problems', summary: SUMMARY, action: WAY } },
})
const [problemNotice] = failedJobNotices([problem], T0, none).shown
assert.equal(problemNotice.title, 'Render finished with problems')
assert.equal(problemNotice.text, `${SUMMARY} ${WAY}`, 'the row names the checks, what they mean and the way forward')
assert.deepEqual(ids([problem], new Set(['p1'])), [], 'it can be dismissed like a failure')
assert.deepEqual(ids([{ ...problem, updated_ts: '2025-12-31T10:00:00Z' }]), [], 'older problems are not replayed')
assert.deepEqual(ids([{ ...problem, outcome_reason: 'completed_with_warnings' }]), [], 'other warnings are not a problem row')
assert.deepEqual(ids([{ ...problem, outcome_reason: 'completed', completion: 'success' }]), [], 'a passing render is not shown')
assert.equal(failedJobNotices([{ ...problem, result: {} }], T0, none).shown[0].text,
  'The file was written but did not pass its checks. Open the receipt for details.', 'without a verdict it still says so')
const problemHtml = renderToStaticMarkup(createElement(FailedJobRows, { failed: failedJobNotices([problem], T0, none), onDismiss: () => {}, onOpenDetails: () => {} }))
assert.ok(problemHtml.includes('data-cut-job-failed-details="p1"') && problemHtml.includes('data-cut-job-failed-dismiss="p1"'), 'Details and dismiss on the problem row')
assert.ok(problemHtml.includes('Render finished with problems') && problemHtml.includes('duration_matches_edl'), problemHtml)

// A render queue that finished with some deliveries failed (items_failed) is
// shown too: "finished with failures", the count and each failed delivery.
const partial = failed('q1', '2026-01-01T10:07:00Z', {
  kind: 'render_queue', state: 'done', completion: 'done_with_warnings', outcome: 'partial', outcome_reason: 'items_failed',
  message: 'done, but 1 of 3 failed', error: undefined,
  result: { summary_line: 'render queue: 2/3 delivered, 1 failed', failures: ['delivery 3: the output folder is gone'] },
})
const [partialNotice] = failedJobNotices([partial], T0, none).shown
assert.equal(partialNotice.title, 'Render queue finished with failures')
assert.equal(partialNotice.text, 'render queue: 2/3 delivered, 1 failed. delivery 3: the output folder is gone.')
assert.equal(failedJobNotices([{ ...partial, result: {} }], T0, none).shown[0].text,
  'done, but 1 of 3 failed Open the job for details.', 'without a summary the job message still says it')
assert.deepEqual(ids([{ ...partial, outcome_reason: 'completed', completion: 'success' }]), [], 'a clean queue is not shown')
// All deliveries failed: the job itself failed, shown as a failure.
const allFailed = failed('q2', '2026-01-01T10:08:00Z', { kind: 'render_queue', error: { code: 'job_failed', message: 'None of the 1 deliveries was written: x', cause: 'y' } })
assert.equal(failedJobNotices([allFailed], T0, none).shown[0].title, 'Render queue failed')

// Names of what failed: every kind the engine creates reads as plain words.
for (const kind of [
  'render', 'render_queue', 'reframe', 'reframe-direct', 'reframe-qc', 'bundle', 'autopilot', 'proxy', 'proxy_check', 'title_remake', 'probe', 'transcribe',
  'perception', 'enrich', 'judge', 'import_chain', 'media_intelligence', 'diarize', 'recipe', 'asset_generate',
  'screen_record_export', 'screen_record_copy_raw', 'recording_recovery', 'motion_import_apply', 'fetch_tool', 'setup_perception', 'setup_matte',
  'cache_purge', 'cache_rebuild', 'portable_package', 'verify-rerun',
]) {
  const name = jobTaskName(kind)
  assert.ok(/^[A-Z][a-z]+( [a-z]+)*$/.test(name) && !name.includes('_') && !name.includes('-'), kind + ' -> ' + name)
}
assert.equal(jobTaskName('reframe-portrait'), 'Reframe')
assert.equal(jobTaskName('reframe-qc'), 'Reframe check')
assert.equal(jobTaskName('some_new_kind'), 'Some new kind')

// The row: what failed, why, the next step, Details and Dismiss, stable selectors.
const html = renderToStaticMarkup(createElement(FailedJobRows, { failed: failedJobNotices(five, T0, none), onDismiss: () => {}, onOpenDetails: () => {} }))
assert.ok(html.includes('data-cut-job-failed-row="j5"') && html.includes('data-cut-job-failed-kind="diarize"'), html)
assert.ok(html.includes('data-cut-job-failed-details="j5"') && html.includes('data-cut-job-failed-dismiss="j5"'), html)
assert.ok(html.includes('data-cut-job-failed-more="2"') && html.includes('+2 more failed'), 'the hidden count is a visible control')
assert.ok(html.includes('Speaker labelling failed'), 'the row names what failed')
assert.ok(html.includes('too long to label speakers') && html.includes('Trim or split the asset'), 'the row carries the reason and the next step')
assert.ok(!/>job_\d|>j\d</.test(html), 'a job id is never the visible text')
assert.equal(renderToStaticMarkup(createElement(FailedJobRows, { failed: { shown: [], hidden: 0 }, onDismiss: () => {}, onOpenDetails: () => {} })), '', 'nothing failed: nothing shown')

// Health and Recovery lists the same words, so Details leads to the full message.
const view = failedJobRetryViews([own])[0]
assert.equal(view.failureText, jobFailureText(own, ''))
assert.equal(view.label, 'Speaker labelling')
assert.equal(failedJobRetryViews([failed('c', '2026-01-01T10:05:00Z', { outcome: 'cancelled' })])[0].failureText, undefined, 'a cancelled job has no failure text')

// Wiring.
const bar = readFileSync(new URL('../src/statusbar/index.tsx', import.meta.url), 'utf8')
assert.match(bar, /<FailedJobRows[\s\S]*onOpenDetails=\{\(\) => onOpenEnvironment\('health-recovery'\)\}/, 'Details opens Health and Recovery')
const hook = readFileSync(new URL('../src/topbar/useTopbarJobs.ts', import.meta.url), 'utf8')
assert.match(hook, /failedJobNotices\(lastRecords\.current, watchSince\.current, dismissed\.current\)/, 'the notices come from the shared jobs.list seed')
assert.match(hook, /ev\.type === 'project_changed'\) \{\s*forgetFailures\(\)/, 'a project change clears the notices')
assert.equal((hook.match(/setInterval/g) ?? []).length, 1, 'no second poll: the existing seed carries failures')
const speech = readFileSync(new URL('../src/panels/Transcript/useSpeechServiceActions.ts', import.meta.url), 'utf8')
assert.ok(!speech.includes("'jobs.list'"), 'Transcript Tools does not poll the job list itself')

// An export a kill, crash or power cut stopped is told at the next start, in
// the same row, before any project is open.
const cutOff = readInterruptedJobs({ interrupted: [
  { key: '4411-a1b2c3-job_002', job_id: 'job_002', kind: 'render', project_name: 'Talk', leftover_files_removed: 2, message: 'x', suggested_action: 'y' },
  { bogus: true },
] })
assert.equal(cutOff.length, 1, 'malformed items are skipped')
const [cutRow] = interruptedJobNotices(cutOff)
assert.equal(cutRow.title, `${jobTaskName('render')} was interrupted`)
assert.match(cutRow.text, /did not finish/)
assert.match(cutRow.text, /“Talk”/)
assert.match(cutRow.text, /2 unfinished files were removed/)
assert.match(cutRow.text, /Start it again\./)
assert.doesNotMatch(cutRow.text, /job_\d/, 'a job id is never the visible text')
assert.equal(interruptedKey(cutRow.job_id), '4411-a1b2c3-job_002')
assert.doesNotMatch(cutRow.text, /still running/, 'nothing was stopped: not said')
{
  const [orphan] = interruptedJobNotices([{ ...cutOff[0], helpers_stopped: 1, leftover_files_removed: 1 }])
  assert.match(orphan.text, /still running in the background and was stopped\. Its 1 unfinished file was removed\./,
    'a surviving writer is named as stopped before the files are said to be removed')
}
assert.equal(interruptedKey('job_002'), null)
const cutHtml = renderToStaticMarkup(createElement(FailedJobRows, { failed: { shown: [cutRow], hidden: 0 }, onDismiss: () => {}, onOpenDetails: () => {} }))
assert.ok(cutHtml.includes('data-cut-job-failed-dismiss="interrupted:4411-a1b2c3-job_002"'), cutHtml)
assert.ok(!cutHtml.includes('data-cut-job-failed-details'), 'no Details: Health lists project jobs only, the row holds the whole text')
assert.match(hook, /callVerb\('jobs\.interrupted', \{\}\)/, 'the start asks the engine for interrupted work')
assert.match(hook, /callVerb\('jobs\.interrupted_dismiss', \{ key \}\)/, 'dismissing forgets the note in the engine')

console.log('PASS failed jobs are visible to the person, whoever started them')
