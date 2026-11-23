import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RecordingLiveControls } from '../src/panels/Record/RecordingLiveControls'
import { clockAfterPauseChange, limitSecondsLeft, recordedMs, startRecordingClock } from '../src/panels/Record/recordingClock'
import { RecordingPauseControlLifetime, boundaryAfterStatus, recordingPauseAcknowledgedLine, recordingTakePause } from '../src/panels/Record/recordingPause'

const engineStatus = { capture_id: 'capture-a', terminal: false,
  pause: { enabled: true, state: 'paused', reason: null } }
assert.deepEqual(recordingTakePause(engineStatus, 'capture-a'),
  { enabled: true, state: 'paused', message: 'Paused. Resume to continue recording.' })
assert.equal(recordingTakePause(engineStatus, 'capture-b'), null, 'another take cannot provide live pause facts')
assert.equal(recordingTakePause({ ...engineStatus, terminal: true }, 'capture-a'), null)
assert.equal(recordingTakePause({ ...engineStatus, pause: { enabled: true, state: 'unknown' } }, 'capture-a'), null)
assert.equal(recordingTakePause({ capture_id: 'capture-a' }, 'capture-a'), null, 'missing engine facts are never guessed from a form')
const reason = 'The native recorder cannot pause this take.'
assert.deepEqual(recordingTakePause({ ...engineStatus, pause: { enabled: false, state: 'unavailable', reason } }, 'capture-a'),
  { enabled: false, state: 'idle', message: 'The recorder cannot pause this take.' })

// A paused take says the time as minutes:seconds and what to do next, in words a
// person uses: no milliseconds and no word about the engine's journal.
const pausedLine = recordingPauseAcknowledgedLine('paused', 47_021)
assert.equal(pausedLine, 'Paused at 0:47. Resume to continue recording.')
assert.equal(recordingPauseAcknowledgedLine('paused', 5_000_000), 'Paused at 83:20. Resume to continue recording.')
assert.equal(recordingPauseAcknowledgedLine('paused', 0), 'Paused at 0:00. Resume to continue recording.')
assert.equal(recordingPauseAcknowledgedLine('paused', -5), 'Paused at 0:00. Resume to continue recording.')
assert.equal(recordingPauseAcknowledgedLine('recording', 47_021), 'Recording resumed at 0:47.')
for (const sentence of [pausedLine, recordingPauseAcknowledgedLine('recording', 125_999),
  recordingTakePause(engineStatus, 'capture-a')!.message]) {
  assert.doesNotMatch(sentence, /journal|durable|acknowledg|\bms\b|millisecond/i, `plain words: ${sentence}`)
}
assert.match(pausedLine, /\d+:\d{2}\./, 'the time is minutes:seconds')

// The timer counts the recorded time, which stands still while the take is paused.
{
  const T0 = 1_000_000
  let clock = startRecordingClock(T0)
  assert.equal(recordedMs(clock, T0 + 5_000), 5_000)
  // Pause confirmed by the recorder at 5.02 s of the take: the clock stands still on that figure.
  const paused = { state: 'paused' as const, ms: 5_020, at: T0 + 5_050 }
  clock = clockAfterPauseChange(clock, 'pausing', null, T0 + 5_000)
  assert.equal(recordedMs(clock, T0 + 5_030), 5_030, 'still counting while the pause is being sealed')
  clock = clockAfterPauseChange(clock, 'paused', paused, T0 + 5_050)
  const first = recordedMs(clock, T0 + 6_000)
  const second = recordedMs(clock, T0 + 600_000)
  assert.equal(first, 5_020, 'the engine\'s recorded time at the pause')
  assert.equal(second, first, 'ten minutes of pause add nothing')
  assert.equal(recordedMs(clockAfterPauseChange(clock, 'paused', paused, T0 + 9_000), T0 + 9_500), 5_020, 'applying the same state again changes nothing')
  assert.equal(recordedMs(clockAfterPauseChange(clock, 'resuming', paused, T0 + 70_000), T0 + 71_000), 5_020, 'stays still until Resume is confirmed')
  // Resume confirmed at the same recorded time: it counts on from there, not from the wall-clock start.
  const resumed = { state: 'recording' as const, ms: 5_020, at: T0 + 600_000 }
  clock = clockAfterPauseChange(clock, 'recording', resumed, T0 + 600_000)
  assert.equal(recordedMs(clock, T0 + 603_000), 8_020)
  // A status poll alone (reload, or another client paused): stands still on the count so far.
  let polled = clockAfterPauseChange(startRecordingClock(T0), 'paused', null, T0 + 7_000)
  assert.equal(recordedMs(polled, T0 + 7_000), 7_000)
  assert.equal(recordedMs(polled, T0 + 90_000), 7_000)
  polled = clockAfterPauseChange(polled, 'recording', null, T0 + 90_000)
  assert.equal(recordedMs(polled, T0 + 92_000), 9_000)
  // Without pause facts the clock keeps counting.
  assert.equal(recordedMs(clockAfterPauseChange(startRecordingClock(T0), 'idle', null, T0 + 3_000), T0 + 4_000), 4_000)
  // The time limit is a wall-clock deadline in the recorder (a pause does not move it), so the seconds left run
  // on through a pause while the timer stands still: 30 s limit, paused at 5.02 s recorded, 20 s of wall time in.
  assert.equal(limitSecondsLeft(30_000, T0, T0 + 4_100), 26)
  assert.equal(limitSecondsLeft(30_000, T0, T0 + 20_000), 10, 'the pause does not give the limit its time back')
  const pausedAt5 = clockAfterPauseChange(startRecordingClock(T0), 'paused', paused, T0 + 5_050)
  assert.equal(recordedMs(pausedAt5, T0 + 20_000), 5_020, 'while the recorded time stands still')
  assert.equal(limitSecondsLeft(10_000, T0, T0 + 12_000), 0)
  assert.equal(limitSecondsLeft(10_000, T0, T0 - 500), 10, 'a clock step back never adds seconds')
  // The last acknowledgement is outdated by a poll that shows the opposite state.
  assert.equal(boundaryAfterStatus(paused, 'recording'), null)
  assert.equal(boundaryAfterStatus(resumed, 'paused'), null)
  assert.equal(boundaryAfterStatus(paused, 'paused'), paused)
  assert.equal(boundaryAfterStatus(paused, 'resuming'), paused)
  assert.equal(boundaryAfterStatus(null, 'paused'), null)
}

// The live box under the preview: a paused take says "Paused" with a pause mark and not the red recording dot.
{
  const live = (pauseState: 'recording' | 'paused' | 'resuming' | 'pausing') => renderToStaticMarkup(createElement(RecordingLiveControls, {
    elapsed: '0:05', sceneName: 'Screen', audioMeters: null, captureSafety: null,
    pauseCapability: { supported: true, detail: '', incompatible: [] }, pauseEnabled: true, pauseState, pauseMessage: '',
    markerPending: false, stopRetry: false, onMarker: () => undefined, onPauseControl: () => undefined, onStop: () => undefined,
  }))
  const running = live('recording')
  assert.match(running, /rec__hud-dot/)
  assert.match(running, /<small>Recording · Screen<\/small>/)
  assert.doesNotMatch(running, /Paused|rec__hud-pause/)
  for (const state of ['paused', 'resuming'] as const) {
    const html = live(state)
    assert.match(html, /<small>Paused · Screen<\/small>/, `${state}: the box says it is paused`)
    assert.match(html, /rec__hud-pause/, `${state}: a pause mark`)
    assert.doesNotMatch(html, /rec__hud-dot|Recording · Screen/, `${state}: no red recording dot or recording words`)
    assert.match(html, /data-cut-rec-hud="true"/, 'machine-readable attribute kept')
    assert.match(html, /<strong data-cut-rec-elapsed="true">0:05<\/strong>/, 'machine-readable attribute kept; the time shown is the one given')
  }
  assert.match(live('pausing'), /<small>Recording · Screen<\/small>/, 'still recording until the pause is confirmed')
}

const lifetime = new RecordingPauseControlLifetime()
lifetime.replaceCapture('capture-a')
const delayedA = lifetime.begin('capture-a')
assert.ok(delayedA, 'A may issue one control while it is the acknowledged capture')

// B starts before A's response arrives. Its state is authoritative, and A must
// never be allowed to write Pause/Resume acknowledgement text over it.
lifetime.replaceCapture('capture-b')
const controlB = lifetime.begin('capture-b')
assert.ok(controlB, 'B receives its own control generation')
assert.equal(lifetime.isCurrent(delayedA), false, 'a delayed A response is stale after B replaces capture ownership')
assert.equal(lifetime.isCurrent(controlB), true, 'the current B response remains eligible to update the UI')

lifetime.clearCapture()
assert.equal(lifetime.isCurrent(controlB), false, 'finalization clears B before a delayed acknowledgement can write')
lifetime.replaceCapture('capture-c')
const controlC = lifetime.begin('capture-c')
assert.ok(controlC)
lifetime.unmount()
assert.equal(lifetime.isCurrent(controlC), false, 'unmount invalidates an outstanding response before React state writes')

const hook = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/useRecordingPause.ts', import.meta.url)),
  'utf8',
)
assert.match(hook, /controlLifetimeRef\.current\.begin\(captureId\)/,
  'each Pause or Resume request captures the current recording identity before awaiting')
assert.match(hook, /await callVerb\(`screen_record\.\$\{action\}`, \{ capture_id: captureId \}\)[\s\S]*?isCurrent\(controlLease\)/,
  'every asynchronous response proves its original capture lease before state or message writes')
assert.match(hook, /acknowledgeStart[\s\S]*?replaceCapture\(id\)/,
  'a replacement start invalidates the prior capture lifetime')
assert.match(hook, /clearCapture[\s\S]*?clearCapture\(\)/,
  'ordinary finalization invalidates an outstanding pause request')
assert.match(hook, /return \(\) => lifetime\.unmount\(\)/,
  'unmount invalidates an outstanding pause request')
assert.match(hook, /useLayoutEffect\(\(\) => \{[\s\S]*?return \(\) => lifetime\.unmount\(\)/,
  'unmount revocation is committed before a queued response can write')

// The visible line comes from the shared helper; the hook carries no engine wording of its own.
assert.doesNotMatch(hook, /durable journal|logical_media_time_ms\} ms/, 'the hook shows no engine words and no raw milliseconds')
assert.match(hook, /recordingPauseAcknowledgedLine\(acknowledgement\.state, acknowledgement\.logical_media_time_ms\)/)
assert.match(hook, /pausedLineRef\.current/, 'a status poll keeps the line that says when the pause happened')

console.log('PASS recording pause capture lifetime rejects delayed A after capture B')
