import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import {
  recordingWorkspaceAdmission,
  recordingWorkspaceTransitionAllowed,
} from '../src/panels/Record/recordingWorkspaceAdmission'

for (const phase of ['countdown', 'starting', 'recording', 'finalizing'] as const) {
  const admission = recordingWorkspaceAdmission(phase)
  assert.equal(admission.blocked, true, `${phase} retains the mounted Record owner`)
  assert.match(admission.reason ?? '', /Recording|recording|countdown/, `${phase} exposes an exit reason`)
  assert.equal(recordingWorkspaceTransitionAllowed('record', 'edit', admission), false,
    `${phase} cannot unmount Record for Edit`)
  assert.equal(recordingWorkspaceTransitionAllowed('record', 'library', admission), false,
    `${phase} cannot unmount Record for Library`)
}

const idle = recordingWorkspaceAdmission('idle')
assert.equal(recordingWorkspaceTransitionAllowed('record', 'edit', idle), true,
  'a terminal recorder permits the ordinary Back to Edit route')
assert.equal(recordingWorkspaceTransitionAllowed('edit', 'library', recordingWorkspaceAdmission('recording')), true,
  'the admission only governs exits from an actually mounted Record workspace')

const root = fileURLToPath(new URL('../src/', import.meta.url))
const read = (path: string) => readFileSync(new URL(path, `file://${root}`), 'utf8')
const app = read('App.tsx')
const topBar = read('topbar/RecordingTopBar.tsx')
const surfaceEvents = read('app/useAppSurfaceEvents.ts')
const sourceNavigation = read('app/useSourceNavigationController.ts')
const keyboard = read('app/useAppKeyboardController.ts')

assert.match(app, /useRecordingWorkspaceNavigation\(layout, setRawLayout\)/,
  'App uses one guarded layout entry point instead of an independent Back-only flag')
assert.match(app, /onRecordWorkspaceAdmission=\{reportRecordingWorkspaceAdmission\}/,
  'the mounted Record surface reports its owned lifecycle to App without lifting recorder state')
assert.match(app, /response\?\.mode === 'no_project'[\s\S]*?recordingWorkspaceAdmission\.blocked[\s\S]*?deferredProjectChange\.current = true[\s\S]*?return finish\(nextProject\)/,
  'a confirmed no-project response defers the provider reset while Record owns a capture')
assert.match(app, /if \(recordingWorkspaceAdmission\.blocked \|\| !deferredProjectChange\.current\) return[\s\S]*?void onProjectSwitched\(\)/,
  'the deferred project reset resumes after the recorder reaches a safe terminal state')
assert.match(app, /backDisabled=\{recordingWorkspaceAdmission\.blocked\}[\s\S]*?backReason=\{recordingWorkspaceAdmission\.reason\}/,
  'Recording chrome receives the current admission and reason')
assert.match(topBar, /data-cut-record-back-blocked[\s\S]*?disabled=\{backDisabled\}[\s\S]*?data-cut-record-back-reason/,
  'Back visibly exposes both blocked state and its reason')
assert.match(surfaceEvents, /if \(!showEditor\(\)\) return false/,
  'agent and document surface routes refuse before they claim to open Edit')
assert.match(sourceNavigation, /const moved = setLayout[\s\S]*?if \(!moved\) return/,
  'source-reveal navigation does not publish a destination after a refused exit')
assert.match(keyboard, /if \(!setLayout\([\s\S]*?workspaceMode: 'edit'[\s\S]*?\)\) return/,
  'global editor shortcuts do not add follow-up effects after a refused exit')

console.log('PASS recording workspace admission retains live ownership across app navigation routes')
