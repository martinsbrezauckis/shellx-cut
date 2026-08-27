import { strict as assert } from 'node:assert'
import {
  parseRecordingFrameRate,
  recordingFrameRateReason,
  RECORDING_FRAME_RATE_MAX,
  RECORDING_FRAME_RATE_MIN,
  RECORDING_FRAME_RATE_PRESETS,
} from '../src/panels/Record/recordingFrameRate'

assert.deepEqual(RECORDING_FRAME_RATE_PRESETS, [24, 25, 30, 50, 60], 'common recording rates include the requested 25 and 50 FPS choices')
assert.equal(parseRecordingFrameRate(RECORDING_FRAME_RATE_MIN), 1, 'the engine minimum remains selectable')
assert.equal(parseRecordingFrameRate(RECORDING_FRAME_RATE_MAX), 240, 'the engine maximum remains selectable')
assert.equal(parseRecordingFrameRate('47.5'), 47.5, 'custom frame rates preserve valid precision')

for (const invalid of ['', '0', '240.01', 'not a number', Number.POSITIVE_INFINITY]) {
  assert.equal(parseRecordingFrameRate(invalid), null, `invalid frame rate is rejected: ${String(invalid)}`)
  assert.match(recordingFrameRateReason(invalid) || '', /1 to 240 FPS/, `invalid frame rate has a correction message: ${String(invalid)}`)
}

assert.equal(recordingFrameRateReason('25'), null, 'a preset rate is accepted')
console.log('PASS recording frame-rate choices and custom range validation')
