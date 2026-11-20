// What the person is being told right now is readable by an agent:
// `ui.state.notices` lists the shared action notice, the odd project size
// notice, the recording recovery rows and the failed-job rows, each as a
// structured, path-free entry that names the verb or record behind it.

import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { createUiObservableState, type UiStateSource } from '../src/app/uiControlState'
import { recoveryNotices, type Row } from '../src/app/RecordingRecoveryNotice'
import { pendingNotices } from '../src/app/recordingRecoveryModel'
import { actionFeedbackNotice } from '../src/components/UserActionFeedback'
import { oddSizeNotice } from '../src/components/OddProjectSizeNotice'
import { oddProjectSize } from '../src/lib/oddProjectSize'
import { getUiNotices, setUiNotices, subscribeUiNotices } from '../src/lib/uiNotices'
import { userActionFailureDetail } from '../src/lib/userActionFeedback'
import { failedJobUiNotices } from '../src/statusbar/FailedJobRows'

function stateWith(extra: Partial<UiStateSource>) {
  return createUiObservableState({
    revision: 1,
    layout: {
      workspaceMode: 'edit', leftCollapsed: false, leftTab: 'assets', findSurface: 'find-media',
      railCollapsed: false, railPinned: false, rightTab: 'properties',
    },
    generateTab: 'templates', wizardOpen: false, envOpen: false, envCategory: 'about', commentsOpen: false,
    activeDrawer: null, highlight: null, playheadMs: 0, selectedClipIds: [], exportRange: null, project: null,
    dom: { activeReviewTab: null, rightBodyTab: null, rightBodyStatus: null, rightRecoverySelector: null, dialogs: [], aboutVersion: null, uiSourceContentManifestSha256: null },
    ...extra,
  } as unknown as UiStateSource)
}

// The store: owners publish and clear their own entries; no change, no notification.
{
  let heard = 0
  const off = subscribeUiNotices(() => { heard += 1 })
  assert.deepEqual(getUiNotices(), [])
  setUiNotices('failed-jobs', [{ kind: 'failed_job', job_id: 'j7', job_kind: 'import_chain' }])
  setUiNotices('odd-project-size', [oddSizeNotice('open', oddProjectSize({ width: 641, height: 361 })!)])
  assert.equal(heard, 2)
  setUiNotices('failed-jobs', [{ kind: 'failed_job', job_id: 'j7', job_kind: 'import_chain' }])
  assert.equal(heard, 2, 'an unchanged list does not republish ui.state')
  assert.deepEqual(getUiNotices().map((entry) => entry.kind), ['failed_job', 'odd_project_size'])
  assert.deepEqual(
    stateWith({ notices: getUiNotices() }).notices,
    [
      { kind: 'failed_job', job_id: 'j7', job_kind: 'import_chain' },
      { kind: 'odd_project_size', state: 'open', width: 641, height: 361, even_width: 640, even_height: 360 },
    ],
    'ui.state carries the entries as published',
  )
  setUiNotices('failed-jobs', [])
  setUiNotices('odd-project-size', [])
  assert.deepEqual(getUiNotices(), [], 'cleared when the notices go')
  assert.equal(heard, 4)
  off()
}

// An older client or a state without notices still has the field.
assert.deepEqual(stateWith({}).notices, [])

// The shared action notice names the verb and the code, never the text.
{
  const refused = {
    ok: false as const,
    error: { code: 'tool_missing', message: 'ffprobe is not installed', cause: '/opt/x/ffprobe: not found', suggested_action: 'system.set_ffmpeg' },
  }
  const detail = userActionFailureDetail('media.import', refused, 'Import failed')!
  assert.deepEqual(detail, { message: 'ffprobe is not installed. Set up video processing to import, preview and export.', setupSurface: 'settings-video-performance' },
    'the detail returned to callers keeps its shape')
  assert.deepEqual(actionFeedbackNotice({ ...detail, source: 'media.import', code: 'tool_missing' }), [{
    kind: 'action_feedback', source: 'media.import', code: 'tool_missing', setup_surface: 'settings-video-performance',
  }])
  assert.deepEqual(
    actionFeedbackNotice({ message: 'Not imported: “Holiday”. Folders are not imported…', source: 'drop', code: 'not_imported', count: 4 }),
    [{ kind: 'action_feedback', source: 'drop', code: 'not_imported', setup_surface: null, count: 4 }],
  )
  assert.deepEqual(actionFeedbackNotice({ message: 'Could not paste the copied clip.' }),
    [{ kind: 'action_feedback', source: 'app', code: null, setup_surface: null }])
  assert.deepEqual(actionFeedbackNotice(null), [])
  const entry = JSON.stringify(actionFeedbackNotice({ ...detail, source: 'media.import', code: 'tool_missing' }))
  assert.doesNotMatch(entry, /\/opt\/|ffprobe is not installed/, 'no message text or path reaches ui.state')
}

// The recovery rows: the first three with their state, the rest counted.
{
  const row = (id: string, over: Partial<Row> = {}): Row => ({
    captureId: id, projectPath: '/home/p/Trip', projectName: 'Trip', situation: 'interrupted',
    canRecover: true, message: 'A recording was interrupted.', filesFolder: '/home/p/Trip/captures/x', ...over,
  })
  const notices = recoveryNotices([row('c1'), row('c2', { busy: true }), row('c3', { result: 'Added.' }), row('c4'), row('c5')])
  assert.deepEqual(notices, [
    { kind: 'recording_recovery', capture_id: 'c1', situation: 'interrupted', can_recover: true, state: 'offered' },
    { kind: 'recording_recovery', capture_id: 'c2', situation: 'interrupted', can_recover: true, state: 'busy' },
    { kind: 'recording_recovery', capture_id: 'c3', situation: 'interrupted', can_recover: true, state: 'done' },
    { kind: 'recording_recovery_more', count: 2 },
  ])
  assert.doesNotMatch(JSON.stringify(notices), /\/home\//, 'no project path or files folder reaches ui.state')
  assert.deepEqual(recoveryNotices([]), [])
}

// The failed-job rows of the status bar and the "+N more failed" count.
assert.deepEqual(
  failedJobUiNotices({ shown: [{ job_id: 'j1', kind: 'render', title: 'Render failed', text: 'x' }], hidden: 2 }),
  [{ kind: 'failed_job', job_id: 'j1', job_kind: 'render' }, { kind: 'failed_jobs_more', count: 2 }],
)
assert.deepEqual(failedJobUiNotices({ shown: [], hidden: 0 }), [])

// The odd-size notice states.
{
  const size = oddProjectSize({ width: 641, height: 361 })!
  assert.deepEqual(oddSizeNotice('refused', size), { kind: 'odd_project_size', state: 'refused', width: 641, height: 361, even_width: 640, even_height: 360 })
  assert.equal(oddSizeNotice('done', size).state, 'done')
}

// Wiring: every notice component publishes, and the publisher reads the store.
for (const [file, owner] of [
  ['../src/components/UserActionFeedback.tsx', 'action-feedback'],
  ['../src/components/OddProjectSizeNotice.tsx', 'odd-project-size'],
  ['../src/app/RecordingRecoveryNotice.tsx', 'recording-recovery'],
  ['../src/statusbar/FailedJobRows.tsx', 'failed-jobs'],
] as const) {
  const source = readFileSync(new URL(file, import.meta.url), 'utf8')
  assert.match(source, new RegExp(`usePublishedUiNotices\\('${owner}'`), `${file} publishes its notices`)
}
assert.match(readFileSync(new URL('../src/app/useUiStatePublisher.ts', import.meta.url), 'utf8'),
  /useSyncExternalStore\(subscribeUiNotices, getUiNotices\)/)
assert.match(readFileSync(new URL('../src/DropZone.tsx', import.meta.url), 'utf8'),
  /publishUserActionMessage\(skippedNotice, \{ source: 'drop', code: 'not_imported'/)

// The recovery notice says how much can be recovered of what was recorded; an
// agent reads the numbers behind the sentence from ui.state.notices.
{
  const rows = pendingNotices({ recordings: [{
    capture_id: 'cap_9', project_path: '/p/a.cutproj', project_name: 'a', situation: 'interrupted', can_recover: true,
    message: 'ShellX Cut closed while recording in “a”. What was saved can be added to the timeline. About 15 s of about 20 s can be recovered; the last 5 s were not saved before the recording stopped.',
    files_folder: '/p/a.cutproj/cache/screen_record/cap_9', recovered_ms: 15000, recorded_ms_at_least: 19800, recorded_longer_unknown: false,
  }] })
  assert.equal(rows.length, 1)
  assert.match(rows[0].message, /About 15 s of about 20 s can be recovered/)
  assert.deepEqual(
    [rows[0].recoveredMs, rows[0].recordedMsAtLeast, rows[0].recordedLongerUnknown],
    [15000, 19800, false],
  )
  assert.deepEqual(recoveryNotices([rows[0]]), [{
    kind: 'recording_recovery', capture_id: 'cap_9', situation: 'interrupted', can_recover: true,
    recovered_ms: 15000, recorded_ms_at_least: 19800, recorded_longer_unknown: false, state: 'offered',
  }])
  // No measurement: nothing is claimed.
  const bare = pendingNotices({ recordings: [{ capture_id: 'cap_8', situation: 'interrupted', can_recover: true, message: 'x' }] })
  assert.deepEqual(Object.keys(recoveryNotices(bare)[0]).includes('recovered_ms'), false)
}

console.log('ui notices state: ok')
