// The Health and Recovery buttons that start work: "Check screen capture" runs
// the full capture check, "Recover recordings" opens the recording list and
// goes to Record. Each button is rendered, its real click handler is called,
// and what the click does is asserted.

import { strict as assert } from 'node:assert'
import { isValidElement, type ReactElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { createElement } from 'react'
import type { VerbResult } from '../src/lib/client'
import { OPEN_RECORDING_RECOVERY_EVENT } from '../src/app/RecordingRecoveryNotice'
import { ActionButton, readCaptureCheck, runHealthAction } from '../src/panels/Environment/HealthRecovery'

// The recovery banner listens on window; a plain event target stands in for it.
const events = new EventTarget()
;(globalThis as unknown as { window: EventTarget }).window = events
const opened: string[] = []
events.addEventListener(OPEN_RECORDING_RECOVERY_EVENT, () => opened.push('banner'))

const log: string[] = []
const to = {
  checkCapture: () => { log.push('check-capture') },
  openAssets: () => { log.push('assets') },
  openRecording: () => { log.push('recording') },
  openToolchain: () => { log.push('toolchain') },
}

function click(action: 'check-capture' | 'recover-recordings'): void {
  const button = ActionButton({ action, onRun: () => runHealthAction(action, to) })
  assert.ok(isValidElement(button))
  ;(button as ReactElement<{ onClick: () => void }>).props.onClick()
}

// Both buttons are real, labelled buttons.
const check = renderToStaticMarkup(createElement(ActionButton, { action: 'check-capture', onRun: () => {} }))
assert.match(check, /<button type="button"[^>]*data-cut-health-check-capture[^>]*>Check screen capture<\/button>/)
const recover = renderToStaticMarkup(createElement(ActionButton, { action: 'recover-recordings', onRun: () => {} }))
assert.match(recover, /<button type="button"[^>]*data-cut-health-recover-recordings[^>]*>Recover recordings<\/button>/)

// Check screen capture starts the check and goes nowhere else.
click('check-capture')
assert.deepEqual(log, ['check-capture'])
assert.deepEqual(opened, [])

// Recover recordings first asks the banner to list the recordings, then opens Record.
log.length = 0
click('recover-recordings')
assert.deepEqual(opened, ['banner'])
assert.deepEqual(log, ['recording'])

// The other buttons still go to their owners.
runHealthAction('assets', to)
runHealthAction('recording', to)
runHealthAction('toolchain', to)
assert.deepEqual(log.slice(1), ['assets', 'recording', 'toolchain'])
assert.deepEqual(opened, ['banner'], 'only Recover recordings opens the banner')

// The check itself asks for the full capture check (not the status read) and returns the engine's report.
const sent: Array<{ verb: string; args: unknown }> = []
const report = { ready: false, screen_check: 'permission_needed', cards: [] }
const answer = (reply: VerbResult) => (async (verb: string, args: unknown) => { sent.push({ verb, args }); return reply }) as never
assert.deepEqual(await readCaptureCheck(answer({ ok: true, result: report } as VerbResult)), report)
assert.deepEqual(sent, [{ verb: 'screen_record.doctor', args: { mode: 'check' } }])

// An engine that refuses or does not answer leaves no report, so the row can say the check did not finish.
assert.equal(await readCaptureCheck(answer({ ok: false, error: { code: 'busy', message: 'capture is busy' } } as VerbResult)), null)
assert.equal(await readCaptureCheck((async () => { throw new Error('offline') }) as never), null)

console.log('PASS Health and Recovery: Check screen capture and Recover recordings act on the engine and the app')
