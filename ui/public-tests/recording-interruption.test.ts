// A recording is never lost or misreported without the person knowing:
// the timer starts at the first frame, closing the app saves the recording
// first, a recording started elsewhere is shown, and interrupted recordings
// are offered for recovery in plain words.
import assert from 'node:assert/strict'
import { classifyFirstFrame, waitForFirstFrame } from '../src/app/recordingFirstFrame'
import { handleCloseRequest, SAVING_BEFORE_CLOSE } from '../src/app/useRecordingCloseRequest'
import { activeRecording } from '../src/app/useActiveRecordingAdoption'
import { pendingNotices, projectCaptureNotices, recoverOutcome } from '../src/app/recordingRecoveryModel'
import { stopArgs } from '../src/app/recordingSessionModel'
import { healthRecoveryRows } from '../src/panels/Environment/healthRecoveryModel'
import type { CaptureRecoveryInventory } from '../src/panels/Environment/captureRecoveryModel'
import { readFileSync } from 'node:fs'
import { KEEP_WORKING, QUIT_AND_STOP, agentListing, answerForKey, liveFromStatus, liveListing, questionJobs, quitQuestionView } from '../src/app/quitQuestionModel'
import { polishPercent } from '../src/app/polishProgress'
import { readQuitQuestion } from '../src/app/uiControlState'
import { jobTaskName } from '../src/lib/jobPresentation'

// --- The timer counts from the first captured frame -----------------------
assert.deepEqual(classifyFirstFrame('cap_1', { ok: true, result: { capture_id: 'cap_1', ready: false, terminal: false } }), { state: 'waiting' })
assert.deepEqual(
  classifyFirstFrame('cap_1', { ok: true, result: { capture_id: 'cap_1', ready: true, terminal: false, since_first_screen_frame_ms: 240 } }),
  { state: 'ready', sinceFirstFrameMs: 240 },
)
assert.deepEqual(classifyFirstFrame('cap_1', { ok: true, result: { capture_id: 'cap_1', ready: false, terminal: true } }), { state: 'ended' })
assert.deepEqual(classifyFirstFrame('cap_1', { ok: false, code: 'not_found' }), { state: 'ended' })
assert.deepEqual(classifyFirstFrame('cap_1', { ok: true, result: { capture_id: 'cap_2', ready: true } }), { state: 'waiting' })

{
  let reads = 0
  const outcome = await waitForFirstFrame('cap_1', () => true, 2_000, 1, async () => {
    reads += 1
    return { ok: true, result: { capture_id: 'cap_1', ready: reads >= 3, terminal: false, since_first_screen_frame_ms: 5 } }
  })
  assert.deepEqual(outcome, { state: 'ready', sinceFirstFrameMs: 5 }, 'Starting… stays until the first frame is reported')
  assert.equal(reads, 3)
  let current = true
  const cancelled = await waitForFirstFrame('cap_1', () => current, 2_000, 1, async () => {
    current = false
    return { ok: true, result: { capture_id: 'cap_1', ready: false } }
  })
  assert.deepEqual(cancelled, { state: 'cancelled' }, 'a Stop during Starting… ends the wait')
}

// --- Closing the app saves the recording first ----------------------------
{
  const calls: string[] = []
  const shell = {
    acknowledge: async () => { calls.push('acknowledge'); return true },
    finish: async () => { calls.push('finish'); return true },
  }
  await handleCloseRequest({
    isRecording: () => true,
    announce: (message) => calls.push(`announce:${message}`),
    stopForClose: async () => { calls.push('stop') },
  }, shell)
  assert.deepEqual(calls, ['acknowledge', `announce:${SAVING_BEFORE_CLOSE}`, 'stop', 'finish'])
  calls.length = 0
  await handleCloseRequest({ isRecording: () => false, announce: () => calls.push('announce'), stopForClose: async () => { calls.push('stop') } }, shell)
  assert.deepEqual(calls, ['acknowledge', 'finish'], 'nothing to save: close at once')
  calls.length = 0
  await assert.rejects(handleCloseRequest({
    isRecording: () => true,
    announce: () => {},
    stopForClose: async () => { calls.push('stop'); throw new Error('engine gone') },
  }, shell))
  assert.deepEqual(calls, ['acknowledge', 'stop', 'finish'], 'a failed save still hands the close back to the shell')
}
assert.deepEqual(stopArgs('cap_1', false, 'app_exit'), { capture_id: 'cap_1', autoedit: true, mux_raw: true, reason: 'app_exit' })
assert.deepEqual(stopArgs('cap_1', true), { capture_id: 'cap_1', autoedit: false, mux_raw: true })

// --- A recording started elsewhere is shown -------------------------------
assert.deepEqual(
  activeRecording({ active: true, kind: 'recording', capture_id: 'cap_9', ready: true, since_first_screen_frame_ms: 1200 }),
  { captureId: 'cap_9', ready: true, sinceFirstFrameMs: 1200 },
)
assert.equal(activeRecording({ active: true, kind: 'rehearsal', capture_id: 'cap_9' }), null, 'a rehearsal is not a recording')
assert.equal(activeRecording({ active: false, capture_id: null }), null)

// --- Interrupted recordings are offered in plain words --------------------
const notices = pendingNotices({ recordings: [
  { capture_id: 'cap_a', project_path: '/p/Talk.cutproj', project_name: 'Talk', situation: 'interrupted', can_recover: true,
    message: 'ShellX Cut closed while recording in “Talk”. What was saved can be added to the timeline.', files_folder: '/p/Talk.cutproj/cache/screen_record/cap_a' },
  { capture_id: 'cap_b', situation: 'unknown', message: 'x' },
] })
assert.equal(notices.length, 1, 'malformed rows are dropped')
assert.equal(notices[0].canRecover, true)
assert.equal(notices[0].projectPath, '/p/Talk.cutproj')
const local = projectCaptureNotices({ captures: [
  { capture_id: 'cap_a', state: 'owner_ambiguous', recoverable: true },
  { capture_id: 'cap_c', state: 'owner_ambiguous', recoverable: true },
  { capture_id: 'cap_d', state: 'owner_ambiguous', recoverable: false },
  { capture_id: 'cap_e', state: 'complete', recoverable: true },
] }, new Set(['cap_a']))
assert.deepEqual(local.map((notice) => notice.captureId), ['cap_c'], 'only unnoted captures with something to recover')
assert.doesNotMatch(local[0].message, /owner|ambiguous/i)
assert.equal(
  recoverOutcome({ ok: false, error: { message: 'nothing can be recovered from this recording', suggested_action: 'dismiss the notice with screen_record.pending_dismiss' } }),
  'Nothing can be recovered from this recording. Dismiss the notice with screen_record.pending_dismiss.',
)
assert.match(recoverOutcome({ ok: true, result: { message: 'The recovered recording was added to the end of the timeline.', skipped_files: ['mic.wav is a link or not an ordinary file; it was not used'] } }),
  /added to the end of the timeline\. Some files were not used: mic\.wav/)

function inventory(rows: Array<{ state: 'owner_ambiguous' | 'interrupted' | 'corrupt'; recoverable?: boolean; checkpoints?: number }>): CaptureRecoveryInventory {
  return {
    complete: true,
    captures: rows.map((row, index) => ({
      capture_id: `cap-${index}`,
      state: row.state,
      checkpoints: row.checkpoints ?? 1,
      has_open_segment: true,
      receipt: row.state === 'interrupted'
        ? { state: 'interrupted', recovered_segments: 0, lost_tail_ms: null, lost_tail_lower_bound_ms: 0, lost_tail_upper_bound_ms: null, audio_first_packet_offset_ms: null, source: null }
        : null,
      ...(row.recoverable === undefined ? {} : { recoverable: row.recoverable }),
    })),
  }
}
function captureRow(captureRecovery: CaptureRecoveryInventory) {
  return healthRecoveryRows({
    hasProject: true,
    projectHealth: null,
    jobs: null,
    captureDoctor: { ready: true, cards: [] },
    captureRecovery,
    toolchain: null,
  }).find((row) => row.id === 'capture')
}
const interrupted = captureRow(inventory([{ state: 'owner_ambiguous', recoverable: true }]))
assert.equal(interrupted?.state, 'recoverable')
assert.equal(interrupted?.summary, '1 recording was interrupted.')
assert.equal(interrupted?.action, 'recover-recordings', 'the row offers Recover recordings, not Open Record')
assert.doesNotMatch(`${interrupted?.summary} ${interrupted?.detail}`, /owner|ambiguous/i, 'no internal state words for the person')
assert.equal(captureRow(inventory([{ state: 'owner_ambiguous', recoverable: false }]))?.state, 'healthy', 'an early stop with nothing saved is not a problem to report')
const damaged = captureRow(inventory([{ state: 'corrupt' }]))
assert.equal(damaged?.state, 'attention')
assert.match(damaged?.detail ?? '', /recovery record cannot be read/)
assert.doesNotMatch(damaged?.detail ?? '', /torn|journal|corrupt/)

// --- Closing during an export asks, keeping the export is the default -----
// A job message is free text: this one holds ';' and ',' (an installed
// Windows run showed half of such a sentence as a second running job).
const longMessage = 'encoding (Software encoding uses 32 of 96 threads; it will take longer. Close other programs for full speed, or choose HEVC to encode in hardware.)'
const exporting = {
  message: 'quitting ShellX Cut would cancel 2 running job(s) that were asked for (exports, renders, reviews, analysis, downloads) and nothing has been stopped',
  cause: `running: job_002 (render, 40% done, ${longMessage}); job_003 (verify-rerun, 0% done)`,
  suggested_action: 'wait for them to finish',
  question: '',
  running: [
    { job_id: 'job_002', kind: 'render', progress: 0.4, message: longMessage },
    { job_id: 'job_003', kind: 'verify-rerun', progress: 0, message: null },
  ],
}
assert.deepEqual(questionJobs(exporting).map((job) => job.job_id), ['job_002', 'job_003'], 'each running job is named once, from its record')
const quitView = quitQuestionView(exporting)
assert.equal(quitView.running.length, 2, 'two jobs are two lines, whatever their messages hold')
assert.ok(quitView.running.every((line) => !/take longer|Close other programs/.test(line)), 'no part of a job message becomes a line')
assert.equal(quitView.title, 'Work is still running', 'a re-check is not called an export')
assert.equal(quitQuestionView({ ...exporting, running: [exporting.running[0]] }).title, 'An export is still running')
assert.match(quitView.consequence, /Quit anyway\?/)
assert.match(quitView.consequence, /lost/)
assert.deepEqual(quitQuestionView(exporting, (id) => (id === 'job_002' ? 68 : null)).running,
  [`${jobTaskName('render')}, 68% done`, 'Output check'], 'live progress, and no number when nothing current is known')
assert.equal(liveFromStatus({ ok: true, result: { state: 'running', progress: 0.68 } }), 68)
assert.equal(liveListing('job_002 (render, 8% done, encoding)', 27.4), 'job_002 (render, 27% done, encoding)', 'ui.state carries the live percent')
assert.equal(liveListing('job_002 (render, 8% done, encoding)', null), 'job_002 (render, encoding)', 'or no number when nothing current is known')
assert.equal(liveListing('job_003 (verify-rerun, 0% done)', 'finished'), 'job_003 (verify-rerun, finished)')
assert.equal(liveListing('job_009 (proxy_check)', 40), 'job_009 (proxy_check)', 'a listing without a number stays as the engine said it')
assert.equal(liveFromStatus({ ok: true, result: { state: 'done', progress: 1 } }), 'finished')
assert.equal(liveFromStatus({ ok: false }), null)
assert.equal(answerForKey('Escape'), 'keep', 'Esc keeps working')
assert.equal(answerForKey('Enter'), null, 'Enter presses the focused button, which starts on Keep working')
assert.deepEqual(quitQuestionView({ ...exporting, running: [] }).running, ['work you started'], 'never an empty list, and the cause line is never split')
assert.deepEqual(quitView.running, [`${jobTaskName('render')}, 40% done`, 'Output check, 0% done'], 'the jobs in the person\'s words')
assert.ok(quitView.running.every((job) => !/job_\d/.test(job)), 'a job id is never the visible text')
assert.match(quitView.running[0], /, 40% done$/)
{
  const source = readFileSync(new URL('../src/app/QuitQuestion.tsx', import.meta.url), 'utf8')
  assert.match(source, /ref=\{keepRef\}/, 'Keep working is the focused (default) button')
  assert.match(source, /event\.target === event\.currentTarget\) answer\(false\)/, 'clicking outside keeps working')
  assert.match(source, /document\.addEventListener\('keydown', onKey\)/, 'Esc keeps working wherever the focus is')
  assert.match(source, /void acknowledgeQuitQuestion\(\)/, 'the window takes the question so the shell does not ask natively')
  assert.match(source, /data-cut-quitting/, 'after Quit and stop it the window says it is stopping, it does not look frozen')
  assert.equal(KEEP_WORKING, 'Keep working')
  assert.equal(QUIT_AND_STOP, 'Quit and stop it')
  const app = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8')
  assert.match(app, /<QuitQuestion \/>/, 'the question is mounted')
}
{
  // ui.state shows the open question to agents.
  // ui.state carries names and numbers only: a job's free-text message (a
  // setup step can print a full path) stays in the dialog.
  assert.equal(agentListing({ job_id: 'job_007', kind: 'setup', progress: 0.5, message: 'Installing C:\\Users\\someone\\AppData\\uv.exe' }, 50), 'job_007 (setup, 50% done)')
  // ui.state reads one entry per job, written as a JSON array from the records.
  const lines = exporting.running.map((job) => agentListing(job, job.job_id === 'job_002' ? 68 : null))
  assert.deepEqual(lines, ['job_002 (render, 68% done)', 'job_003 (verify-rerun)'], 'one listing per job with live progress; the message stays in the dialog')
  const dataset = { cutQuitQuestionMessage: exporting.message, cutQuitQuestionRunning: JSON.stringify(lines) }
  const documentRef = { querySelector: (selector: string) => (selector === '[data-cut-quit-question]' ? { dataset } : null) } as unknown as Document
  assert.deepEqual(readQuitQuestion(documentRef), {
    message: exporting.message,
    running: lines,
    choices: ['keep_working', 'quit_and_stop'],
  })
  const unreadable = { querySelector: () => ({ dataset: { cutQuitQuestionRunning: 'job_002 (render; x)' } }) } as unknown as Document
  assert.deepEqual(readQuitQuestion(unreadable)?.running, [], 'a value that is not a list is never split')
  assert.equal(readQuitQuestion({ querySelector: () => null } as unknown as Document), null)
}

console.log('recording-interruption tests passed')

// --- The polish bake is a visible job; its clip never lands in another project
assert.equal(polishPercent({ ok: true, result: { jobs: [{ kind: 'proxy', state: 'running', progress: 0.5 },
  { kind: 'screen_record_polish', state: 'running', progress: 0.374 }] } }), 37, 'the polish job\'s percent is shown')
assert.equal(polishPercent({ ok: true, result: { jobs: [{ kind: 'screen_record_polish', state: 'done', progress: 1 }] } }), null,
  'a finished polish shows no running percent')
assert.equal(polishPercent({ ok: false }), null)
{
  const source = readFileSync(new URL('../src/app/useRecordingSession.ts', import.meta.url), 'utf8')
  assert.match(source, /placement\?\.placed === false\) \{ publish\(\{ phase: 'error'/,
    'a clip that waits in its own project is not reported as added to the current project')
  assert.match(source, /watchPolishProgress\(/, 'the Record panel follows the polish job')
}
