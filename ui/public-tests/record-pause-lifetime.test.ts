import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { RecordingPauseControlLifetime } from '../src/panels/Record/recordingPause'

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

console.log('PASS recording pause capture lifetime rejects delayed A after capture B')
