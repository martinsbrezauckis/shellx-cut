import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  countdownRemainingSeconds,
  RecordingCountdownGuard,
} from '../src/panels/Record/recordingCountdown'

const guard = new RecordingCountdownGuard()
const first = guard.begin()
assert.equal(guard.isCurrent(first), true, 'a new countdown owns its timer generation')
guard.cancel()
assert.equal(guard.isCurrent(first), false, 'cancelling wins a timer/cancel race before capture starts')
let startsAfterCancel = 0
assert.equal(
  guard.handoff(first, () => { startsAfterCancel += 1 }),
  false,
  'a zero callback queued before Cancel cannot claim the start handoff',
)
assert.equal(startsAfterCancel, 0, 'Cancel before zero leaves onStart at zero calls')
const second = guard.begin()
assert.equal(guard.isCurrent(second), true, 'a replacement countdown receives a fresh generation')
let startsAtZero = 0
assert.equal(guard.handoff(second, () => { startsAtZero += 1 }), true, 'zero may hand off once')
assert.equal(guard.handoff(second, () => { startsAtZero += 1 }), false, 'a queued repeat cannot hand off twice')
assert.equal(startsAtZero, 1, 'the zero handoff calls onStart exactly once')
assert.equal(countdownRemainingSeconds(3_000, 1), 3, 'the first visible beat remains 3')
assert.equal(countdownRemainingSeconds(3_000, 2_001), 1, 'the final visible beat remains 1')
assert.equal(countdownRemainingSeconds(3_000, 3_001), 0, 'zero is the start handoff only')

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const panel = readFileSync(resolve(uiRoot, 'src/panels/Record/index.tsx'), 'utf8')
const control = readFileSync(resolve(uiRoot, 'src/panels/Record/RecordingCountdownControl.tsx'), 'utf8')
const overlay = readFileSync(resolve(uiRoot, 'src/panels/Record/RecordingCountdownOverlay.tsx'), 'utf8')
const model = readFileSync(resolve(uiRoot, 'src/panels/Record/recordingCountdown.ts'), 'utf8')
const hook = readFileSync(resolve(uiRoot, 'src/panels/Record/useRecordingCountdown.ts'), 'utf8')
assert.match(model, /RECORDING_COUNTDOWN_CHOICES = \[0, 3, 5\]/, 'all requested countdown choices remain available')
assert.match(control, /RECORDING_COUNTDOWN_CHOICES\.map/, 'the choices remain in one setup control')
assert.match(panel, /countdown\.active/, 'countdown is distinct from live recording')
assert.match(control, /data-cut-rec-countdown=/, 'each countdown choice has a stable action identity')
assert.match(overlay, /data-cut-action="record-countdown-cancel"/, 'a stable cancellation route is rendered')
assert.match(panel, /e\.key === 'Escape'/, 'Escape cancels setup before capture')
assert.match(panel, /if \(countdown\.active\)[\s\S]*countdown\.cancel\(\)/, 'the F9 toggle cancels setup before capture')
assert.match(hook, /onStart\(\)\.finally/, 'zero hands off exactly once to the start callback')
assert.match(hook, /tickRef\.current !== null/, 'a zero-valued interval handle is cleaned up')
assert.match(overlay, /Recording has not started yet/, 'the overlay does not imply an active recording')
assert.match(overlay, /aria-modal="true"/, 'the countdown is keyboard-focused while it is active')

console.log('PASS recording countdown choices, cancellation race, and keyboard contract')
