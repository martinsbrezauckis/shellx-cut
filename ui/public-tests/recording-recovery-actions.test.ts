// The two buttons of the interrupted-recording banner: "Add to timeline" opens
// the recording's project when needed and recovers the capture, "Dismiss"
// tells the engine the note was seen and hides the row. The row is rendered
// and its real click handlers are called; the engine is a scripted stand-in
// that records every verb it receives.

import { strict as assert } from 'node:assert'
import { Children, createElement, isValidElement, type ReactElement, type ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { pendingNotices } from '../src/app/recordingRecoveryModel'
import type { VerbResult } from '../src/lib/client'
import type { CutEvent } from '../src/lib/events'
import {
  dismissRecording,
  recoverRecording,
  RecoveryRowView,
  recoveryNotices,
  subscribeRecoveryPreparation,
  type Outcome,
  type Row,
} from '../src/app/RecordingRecoveryNotice'

function findAction(node: ReactNode, id: string): ReactElement<Record<string, unknown>> | null {
  if (!isValidElement(node)) return null
  const props = node.props as Record<string, unknown>
  if (props['data-cut-action'] === id) return node as ReactElement<Record<string, unknown>>
  for (const child of Children.toArray(props.children as ReactNode)) {
    const found = findAction(child, id)
    if (found) return found
  }
  return null
}

const interrupted: Row = {
  captureId: 'cap-001',
  projectPath: null,
  projectName: null,
  situation: 'interrupted',
  canRecover: true,
  message: 'A recording was interrupted.',
  filesFolder: null,
}

type Call = { verb: string; args: unknown }
function scriptedEngine(replies: Record<string, VerbResult>) {
  const calls: Call[] = []
  const call = (async (verb: string, args: unknown) => {
    calls.push({ verb, args })
    return replies[verb] ?? { ok: false, error: { code: 'not_found', message: `no reply for ${verb}` } }
  }) as never
  const patches: Outcome[] = []
  return { calls, patches, effects: { call, update: (_row: Row, patch: Outcome) => { patches.push(patch) } } }
}

// A recoverable note shows both buttons; clicking "Add to timeline" runs the
// row's recover handler and "Dismiss" the dismiss handler.
const clicks: string[] = []
const view = RecoveryRowView({ row: interrupted, onRecover: () => clicks.push('recover'), onDismiss: () => clicks.push('dismiss') })
const recoverButton = findAction(view, 'recording-recover')
const dismissButton = findAction(view, 'recording-recovery-dismiss')
assert.ok(recoverButton && dismissButton, 'a recoverable note offers both actions')
;(recoverButton.props.onClick as () => void)()
;(dismissButton.props.onClick as () => void)()
assert.deepEqual(clicks, ['recover', 'dismiss'])
const markup = renderToStaticMarkup(createElement(RecoveryRowView, { row: interrupted, onRecover: () => {}, onDismiss: () => {} }))
assert.match(markup, /data-cut-action="recording-recover"[^>]*>Add to timeline</)
assert.match(markup, /data-cut-action="recording-recovery-dismiss"[^>]*>Dismiss</)

// A note that cannot be recovered offers only the way to hide it, and says where the files are.
const notRecoverable = RecoveryRowView({ row: { ...interrupted, canRecover: false, filesFolder: '/rec/cap-001' }, onRecover: () => {}, onDismiss: () => {} })
assert.equal(findAction(notRecoverable, 'recording-recover'), null)
assert.ok(findAction(notRecoverable, 'recording-recovery-dismiss'))
assert.match(renderToStaticMarkup(notRecoverable), /The files stay in \/rec\/cap-001/)

// While a recovery runs the buttons are disabled; afterwards only "OK" remains.
const busy = renderToStaticMarkup(createElement(RecoveryRowView, { row: { ...interrupted, busy: true, result: 'Recovering…' }, onRecover: () => {}, onDismiss: () => {} }))
assert.match(busy, /data-cut-action="recording-recovery-dismiss"[^>]*disabled/)
assert.doesNotMatch(busy, /data-cut-action="recording-recover"/)
const done = renderToStaticMarkup(createElement(RecoveryRowView, { row: { ...interrupted, result: 'The recording was added to the end of the timeline.' }, onRecover: () => {}, onDismiss: () => {} }))
assert.match(done, /data-cut-recording-session="done"/)
assert.match(done, />OK</)

// Add to timeline in the open project: one recover request for that capture, then its message is shown.
{
  const engine = scriptedEngine({ 'screen_record.recover': { ok: true, result: { message: 'Added 2 segments to the end of the timeline.' } } as VerbResult })
  await recoverRecording(interrupted, engine.effects)
  assert.deepEqual(engine.calls, [{ verb: 'screen_record.recover', args: { capture_id: 'cap-001' } }])
  assert.deepEqual(engine.patches[0], { busy: true, result: 'Recovering…' })
  assert.deepEqual(engine.patches.at(-1), { busy: false, canRecover: false, result: 'Added 2 segments to the end of the timeline.' })
}

// Add to timeline for a recording of another project: that project opens first, in order.
{
  const other: Row = { ...interrupted, projectPath: '/projects/demo', projectName: 'Demo' }
  const engine = scriptedEngine({
    'project.open': { ok: true, result: {} } as VerbResult,
    'screen_record.recover': { ok: true, result: {} } as VerbResult,
  })
  await recoverRecording(other, engine.effects)
  assert.deepEqual(engine.calls.map((c) => c.verb), ['project.open', 'screen_record.recover'])
  assert.deepEqual(engine.calls[0].args, { path: '/projects/demo' })
  assert.equal(engine.patches[0].result, 'Opening “Demo”…')
  assert.equal(engine.patches.at(-1)?.result, 'The recording was added to the end of the timeline.')
}

// If that project cannot be opened nothing is recovered, and the person is told why.
{
  const other: Row = { ...interrupted, projectPath: '/projects/gone', projectName: 'Gone' }
  const engine = scriptedEngine({ 'project.open': { ok: false, error: { code: 'not_found', message: 'no such folder' } } as VerbResult })
  await recoverRecording(other, engine.effects)
  assert.deepEqual(engine.calls.map((c) => c.verb), ['project.open'], 'recover is never sent after a failed open')
  assert.deepEqual(engine.patches.at(-1), { busy: false, result: 'The project could not be opened: no such folder.' })
}

// A refused recovery keeps the button available for another try and shows the engine's reason.
{
  const engine = scriptedEngine({ 'screen_record.recover': { ok: false, error: { code: 'busy', message: 'another recovery is running', suggested_action: 'wait a moment' } } as VerbResult })
  await recoverRecording(interrupted, engine.effects)
  assert.deepEqual(engine.patches.at(-1), { busy: false, canRecover: true, result: 'Another recovery is running. Wait a moment.' })
}

// Dismiss on an app-wide note tells the engine, then forgets and hides the row.
{
  const engine = scriptedEngine({ 'screen_record.pending_dismiss': { ok: true, result: {} } as VerbResult })
  const forgotten: string[] = []
  const hidden: string[] = []
  await dismissRecording(interrupted, { ...engine.effects, forget: (id) => forgotten.push(id), hide: (id) => hidden.push(id) })
  assert.deepEqual(engine.calls, [{ verb: 'screen_record.pending_dismiss', args: { capture_id: 'cap-001' } }])
  assert.deepEqual([forgotten, hidden], [['cap-001'], ['cap-001']])
}

// "Later" on a capture found in the open project, and "OK" after an outcome, only hide the row.
for (const row of [{ ...interrupted, situation: 'project_capture' as const }, { ...interrupted, result: 'done' }]) {
  const engine = scriptedEngine({})
  const hidden: string[] = []
  await dismissRecording(row, { ...engine.effects, forget: () => {}, hide: (id) => hidden.push(id) })
  assert.deepEqual(engine.calls, [], 'no engine request for a row the engine never listed or already finished')
  assert.deepEqual(hidden, ['cap-001'])
}

// If the engine refuses the dismissal the row stays, with the reason; "not found" counts as already gone.
{
  const engine = scriptedEngine({ 'screen_record.pending_dismiss': { ok: false, error: { code: 'io', message: 'disk is read-only' } } as VerbResult })
  const hidden: string[] = []
  await dismissRecording(interrupted, { ...engine.effects, forget: () => {}, hide: (id) => hidden.push(id) })
  assert.deepEqual(hidden, [])
  assert.deepEqual(engine.patches, [{ result: 'Could not dismiss: disk is read-only.' }])
  const gone = scriptedEngine({ 'screen_record.pending_dismiss': { ok: false, error: { code: 'not_found', message: 'gone' } } as VerbResult })
  await dismissRecording(interrupted, { ...gone.effects, forget: () => {}, hide: (id) => hidden.push(id) })
  assert.deepEqual(hidden, ['cap-001'])
}


// Pending checkpoint preparation must remain unknown through decoding, the
// person's banner and the path-free debug notice; never turn it into zero.
{
  const message = 'ShellX Cut closed while recording in “Demo”. What was saved can be added to the timeline. How much of it was saved has not been measured yet.'
  const [row] = pendingNotices({ recordings: [{
    capture_id: 'cap_pending', situation: 'interrupted', can_recover: true,
    message, recovered_extent_state: 'not_measured',
  }] })
  const markup = renderToStaticMarkup(createElement(RecoveryRowView, { row, onRecover: () => {}, onDismiss: () => {} }))
  assert.ok(markup.includes(message), 'the banner displays the engine preparation sentence')
  assert.doesNotMatch(markup, /0 s|were not saved/)
  assert.deepEqual(recoveryNotices([row]), [{
    kind: 'recording_recovery', capture_id: 'cap_pending', situation: 'interrupted', can_recover: true,
    recovered_extent_state: 'not_measured', state: 'offered',
  }])
}

// Only a terminal preparation event refreshes the measured banner; its
// subscription is released with the component.
{
  let receive: ((event: CutEvent) => void) | undefined
  let refreshed = 0
  let released = false
  const off = subscribeRecoveryPreparation(() => { refreshed += 1 }, {
    subscribe: (listener) => { receive = listener; return () => { released = true } },
  })
  receive!({ type: 'job_progress', job_id: 'j1', kind: 'recording_recovery', progress: 0.05 })
  receive!({ type: 'job_progress', job_id: 'j2', kind: 'export', progress: 1, terminal: true })
  assert.equal(refreshed, 0)
  receive!({ type: 'job_progress', job_id: 'j1', kind: 'recording_recovery', progress: 1, terminal: true })
  assert.equal(refreshed, 1)
  off()
  assert.equal(released, true)
}

console.log('PASS interrupted-recording banner: actions, unmeasured extent and preparation refresh')
