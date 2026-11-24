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
import { healthRecoveryRows } from '../src/panels/Environment/healthRecoveryModel'

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

// Toolchain card shows plain person guidance, never engine verb names or JSON args.
const missingToolchainReport = {
  ready: true,
  checked_at: '2026-11-24T00:00:00Z',
  cards: [
    { id: 'ffmpeg', kind: 'ffmpeg', status: 'ok' as const, summary: 'Ready', required: true, required_for: [] },
    {
      id: 'perception',
      kind: 'perception',
      status: 'missing' as const,
      summary: 'Captions not set up',
      hint: 'Install Python + faster-whisper + open-clip (call system.setup_perception)',
      required: false,
      required_for: ['captions'],
    },
    {
      id: 'matte',
      kind: 'matte',
      status: 'missing' as const,
      summary: 'Standard matte not installed',
      hint: 'Run system.setup_matte or open Settings -> Environment',
      required: false,
      required_for: ['matte'],
    },
    {
      id: 'matte_premium',
      kind: 'matte',
      status: 'missing' as const,
      summary: 'Premium matte not installed',
      hint: 'Run system.setup_matte with {model:"matanyone", accept_noncommercial:true} (NVIDIA GPU + CUDA required)',
      required: false,
      required_for: ['matte'],
    },
  ],
}
const missingToolchainRow = healthRecoveryRows({
  hasProject: true,
  projectHealth: null,
  jobs: null,
  toolchain: missingToolchainReport,
  captureDoctor: null,
  captureRecovery: { captures: [], complete: true as const },
}).find((candidate) => candidate.id === 'toolchain')
assert.ok(missingToolchainRow, 'toolchain row is present when optional tools are missing')
const missingText = `${missingToolchainRow.summary} ${missingToolchainRow.detail ?? ''}`
assert.ok(!missingText.includes('system.'), `toolchain missing text must not contain "system.": ${missingText}`)
assert.ok(!missingText.includes('{'), `toolchain missing text must not contain "{": ${missingText}`)
assert.ok(!/\b(?:call|run)\s+[a-z0-9_]+\.[a-z0-9_]+/i.test(missingText), `toolchain missing text must not contain agent commands: ${missingText}`)
assert.match(missingText, /Settings > AI & transcription/, 'toolchain missing text directs person to Settings page')

const nonHealthyToolchainReport = {
  ready: true,
  checked_at: '2026-11-24T00:00:00Z',
  cards: [
    {
      id: 'gpu-encode',
      kind: 'hardware',
      status: 'degraded' as const,
      summary: 'Hardware encode ready',
      hint: 'A faster, hardware-accelerated ffmpeg is installed at /opt/ffmpeg/bin/ffmpeg, separate from your main tool.',
      required: false,
      required_for: ['export'],
    },
    {
      id: 'judge.transcript',
      kind: 'judge',
      status: 'unknown' as const,
      summary: 'Transcript judge unverified',
      hint: 'Perception environment missing — call system.setup_perception',
      required: false,
      required_for: ['review'],
    },
    {
      id: 'dub',
      kind: 'service',
      status: 'missing' as const,
      summary: 'Dubbing not configured',
      hint: 'No dubbing provider registered. Set audio.dub connector in Settings.',
      required: false,
      required_for: ['dub'],
    },
    {
      id: 'custom_plugin',
      kind: 'custom',
      status: 'degraded' as const,
      summary: 'Custom check failed',
      hint: 'Call system.check_plugin with {"force":true}',
      required: false,
      required_for: [],
    },
  ],
}
const nonHealthyToolchainRow = healthRecoveryRows({
  hasProject: true,
  projectHealth: null,
  jobs: null,
  toolchain: nonHealthyToolchainReport,
  captureDoctor: null,
  captureRecovery: { captures: [], complete: true as const },
}).find((candidate) => candidate.id === 'toolchain')
assert.ok(nonHealthyToolchainRow, 'toolchain row is present for non-healthy optional cards')
const nonHealthyText = `${nonHealthyToolchainRow.summary} ${nonHealthyToolchainRow.detail ?? ''}`
assert.ok(!nonHealthyText.includes('system.'), `toolchain non-healthy text must not contain "system.": ${nonHealthyText}`)
assert.ok(!nonHealthyText.includes('{'), `toolchain non-healthy text must not contain "{": ${nonHealthyText}`)
assert.ok(!/\baudio\.dub\b|\bverify\.judge\b/.test(nonHealthyText), `toolchain non-healthy text must not expose verb names: ${nonHealthyText}`)

console.log('PASS Health and Recovery: Check screen capture and Recover recordings act on the engine and the app')
