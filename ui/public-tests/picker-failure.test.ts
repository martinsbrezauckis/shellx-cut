import assert from 'node:assert/strict'

import { USER_ACTION_FEEDBACK_EVENT, type UserActionFeedbackDetail } from '../src/lib/userActionFeedback'

// A native file dialog that fails to open must reach the person and the agent
// (the shared action notice behind `ui.state.notices`); a real cancel stays
// silent. The dialog plugin talks to the shell through __TAURI_INTERNALS__.
const events: UserActionFeedbackDetail[] = []
const target = new EventTarget()
target.addEventListener(USER_ACTION_FEEDBACK_EVENT, (event) => {
  events.push((event as CustomEvent<UserActionFeedbackDetail>).detail)
})
let reply: () => Promise<unknown> = async () => null
const g = globalThis as Record<string, unknown>
g.document = target
g.window = {
  __TAURI__: { core: { invoke: async () => null }, event: { listen: async () => () => {} } },
  __TAURI_INTERNALS__: { invoke: () => reply(), transformCallback: () => 1 },
}

const { pickMedia, pickFolder, pickExportOutput, pickCube } = await import('../src/lib/tauri')

// Cancel: the dialog resolves with no selection.
reply = async () => null
assert.deepEqual(await pickMedia(), [])
assert.equal(await pickFolder(), null)
assert.equal(await pickExportOutput(), null)
assert.equal(events.length, 0, 'a cancel gives no notice')

// A real selection is returned and gives no notice either.
reply = async () => '/tmp/lut.cube'
assert.equal(await pickCube(), '/tmp/lut.cube')
assert.equal(events.length, 0)

// Failure: the shell refuses to open the dialog.
reply = async () => { throw 'dialog backend unavailable' }
assert.deepEqual(await pickMedia(), [], 'a failed picker still returns no selection')
assert.equal(events.length, 1, 'but the person is told')
assert.equal(events[0].source, 'app')
assert.equal(events[0].code, 'picker_failed')
assert.match(events[0].message, /could not open \(pickMedia\): dialog backend unavailable/)
assert.match(events[0].message, /Try again/)
assert.equal(await pickFolder(), null)
assert.equal(await pickExportOutput(), null)
assert.equal(events.length, 3)
console.log('PASS native picker failures are shown, cancels stay silent')
