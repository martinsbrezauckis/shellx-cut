import assert from 'node:assert/strict'

import { createUiObservableState, type UiStateSource } from '../src/app/uiControlState'

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

// The recording shortcut state rides in ui.state, including why it failed.
assert.deepEqual(stateWith({}).record_hotkey, { state: 'unknown', scope: 'unknown', reason: null })
assert.deepEqual(stateWith({
  recordHotkey: {
    schema: 'shellx-cut/record-hotkey-capability@1', state: 'unavailable', scope: 'focused_only',
    backend: 'native_global_shortcut', enabled: false, can_enable: false,
    reason: 'Global F9 could not be registered: already registered.',
  },
}).record_hotkey, {
  state: 'unavailable', scope: 'focused_only', reason: 'Global F9 could not be registered: already registered.',
})

console.log('record hotkey state: ok')
