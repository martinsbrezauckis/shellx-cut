import assert from 'node:assert/strict'

import { createUiObservableState, type UiStateSource } from '../src/app/uiControlState'
import { outputFolderStatusFrom } from '../src/lib/outputFolderStatus'

// An agent reading `ui.state` sees where exports go as the status bar shows it,
// without a second verb: the destination, whether it is usable, and the folder's
// NAME (never a path: ui.state is path-safe).
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

assert.equal(stateWith({}).export_folder, null, 'not yet answered by the engine')
const chosen = outputFolderStatusFrom({ dir: '/Users/me/Deliveries/', destination: 'chosen', usable: true, problem: null, suggested_action: null })!
assert.deepEqual(stateWith({ exportFolder: chosen }).export_folder, { destination: 'chosen', usable: true, folder: 'Deliveries' })
const gone = outputFolderStatusFrom({ dir: 'C:\\Deliveries', destination: 'chosen', usable: false, problem: 'export folder not found', suggested_action: 'choose another' })!
assert.deepEqual(stateWith({ exportFolder: gone }).export_folder, { destination: 'chosen', usable: false, folder: 'Deliveries' })
const project = outputFolderStatusFrom({ dir: null, destination: 'project_exports', usable: true, problem: null, suggested_action: null })!
assert.deepEqual(stateWith({ exportFolder: project }).export_folder, { destination: 'project_exports', usable: true, folder: null })
assert.doesNotMatch(JSON.stringify(stateWith({ exportFolder: chosen })), /Users|me\//, 'no path leaks into ui.state')
console.log('PASS ui.state carries the export folder (name only)')
