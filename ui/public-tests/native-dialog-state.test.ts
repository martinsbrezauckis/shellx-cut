import assert from 'node:assert/strict'

import { createUiObservableState, type UiStateSource } from '../src/app/uiControlState'
import {
  getOpenNativeDialogs,
  loadTrackedDialogs,
  subscribeNativeDialogs,
  trackNativeDialog,
} from '../src/lib/nativeDialogs'

type Plugin = Parameters<typeof loadTrackedDialogs>[1]

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

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}

let notifications = 0
const unsubscribe = subscribeNativeDialogs(() => { notifications += 1 })

// Set while the dialog is open, then cleared when the person chooses.
{
  const gate = deferred<string | null>()
  const plugin = { save: () => gate.promise } as unknown as Awaited<ReturnType<NonNullable<Plugin>>>
  const dialogs = await loadTrackedDialogs('pickExportOutput', async () => plugin)
  assert.deepEqual(getOpenNativeDialogs(), [])
  const pending = dialogs.save({})
  assert.deepEqual(getOpenNativeDialogs(), [{ kind: 'save', opened_by: 'pickExportOutput' }],
    'ui.state names the open dialog and the action that opened it')
  assert.deepEqual(stateWith({ nativeDialogs: getOpenNativeDialogs() }).overlays.native_dialogs,
    [{ kind: 'save', opened_by: 'pickExportOutput' }])
  gate.resolve('/tmp/out.mp4')
  assert.equal(await pending, '/tmp/out.mp4', 'the chosen path still reaches the caller')
  assert.deepEqual(getOpenNativeDialogs(), [], 'cleared when the dialog returns')
}

// Cleared on cancel (the plugin resolves null).
{
  const gate = deferred<null>()
  const plugin = { open: () => gate.promise } as unknown as Awaited<ReturnType<NonNullable<Plugin>>>
  const dialogs = await loadTrackedDialogs('pickMedia', async () => plugin)
  const pending = dialogs.open({})
  assert.equal(getOpenNativeDialogs().length, 1)
  gate.resolve(null)
  assert.equal(await pending, null)
  assert.deepEqual(getOpenNativeDialogs(), [], 'cleared on cancel')
}

// Cleared on error, and the error still reaches the caller.
{
  const gate = deferred<string>()
  const plugin = { open: () => gate.promise } as unknown as Awaited<ReturnType<NonNullable<Plugin>>>
  const dialogs = await loadTrackedDialogs('pickFfmpeg', async () => plugin)
  const pending = dialogs.open({})
  assert.equal(getOpenNativeDialogs().length, 1)
  gate.reject(new Error('portal unavailable'))
  await assert.rejects(pending, /portal unavailable/)
  assert.deepEqual(getOpenNativeDialogs(), [], 'cleared on error')
}

// A dialog that throws synchronously also clears.
await assert.rejects(trackNativeDialog('confirm', 'confirmAction', () => { throw new Error('sync failure') }), /sync failure/)
assert.deepEqual(getOpenNativeDialogs(), [])

// Two open at once are both listed, and closing one keeps the other.
{
  const first = deferred<null>()
  const second = deferred<null>()
  const a = trackNativeDialog('open', 'pickCube', () => first.promise)
  const b = trackNativeDialog('save', 'pickRenderOutput', () => second.promise)
  assert.deepEqual(getOpenNativeDialogs().map((entry) => entry.opened_by), ['pickCube', 'pickRenderOutput'])
  first.resolve(null)
  await a
  assert.deepEqual(getOpenNativeDialogs().map((entry) => entry.opened_by), ['pickRenderOutput'])
  second.resolve(null)
  await b
  assert.deepEqual(getOpenNativeDialogs(), [])
}

// The title says which dialog or question is open: the options object is the
// first argument of open/save and the second of confirm/message.
{
  const gate = deferred<boolean>()
  const saveGate = deferred<null>()
  const plugin = {
    confirm: () => gate.promise,
    save: () => saveGate.promise,
  } as unknown as Awaited<ReturnType<NonNullable<Plugin>>>
  const dialogs = await loadTrackedDialogs('confirmAction', async () => plugin)
  const question = dialogs.confirm('2 jobs would be cancelled.', { title: 'Work is still running' })
  const output = dialogs.save({ title: 'Choose output file — ShellX Cut' })
  assert.deepEqual(getOpenNativeDialogs(), [
    { kind: 'confirm', opened_by: 'confirmAction', title: 'Work is still running' },
    { kind: 'save', opened_by: 'confirmAction', title: 'Choose output file — ShellX Cut' },
  ], 'ui.state names which question is waiting for the person')
  gate.resolve(false)
  saveGate.resolve(null)
  await question
  await output
  assert.deepEqual(getOpenNativeDialogs(), [])
}

assert.ok(notifications >= 2 * 5, 'subscribers hear every open and close')
unsubscribe()

console.log('native dialog state: ok')
