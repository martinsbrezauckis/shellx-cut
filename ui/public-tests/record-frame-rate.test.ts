import { strict as assert } from 'node:assert'
import {
  parseRecordingFrameRate,
  recordingFrameRateDraftError,
  recordingFrameRateReason,
  RECORDING_FRAME_RATE_MAX,
  RECORDING_FRAME_RATE_MIN,
  RECORDING_FRAME_RATE_PRESETS,
} from '../src/panels/Record/recordingFrameRate'

assert.deepEqual(RECORDING_FRAME_RATE_PRESETS, [24, 25, 30, 50, 60], 'common recording rates include the requested 25 and 50 FPS choices')
assert.equal(parseRecordingFrameRate(RECORDING_FRAME_RATE_MIN), 1, 'the engine minimum remains selectable')
assert.equal(parseRecordingFrameRate(RECORDING_FRAME_RATE_MAX), 240, 'the engine maximum remains selectable')
assert.equal(parseRecordingFrameRate('47'), 47, 'whole-number custom frame rates remain available')

for (const invalid of ['', '0', '47.5', '240.01', 'not a number', Number.POSITIVE_INFINITY]) {
  assert.equal(parseRecordingFrameRate(invalid), null, `invalid frame rate is rejected: ${String(invalid)}`)
  assert.match(recordingFrameRateReason(invalid) || '', /1 to 240 FPS/, `invalid frame rate has a correction message: ${String(invalid)}`)
}

assert.equal(recordingFrameRateReason('25'), null, 'a preset rate is accepted')
assert.equal(recordingFrameRateDraftError('', 60), null, 'an empty draft leaves the selected preset ready')
assert.match(recordingFrameRateDraftError('241', 60) || '', /1 to 240 FPS/, 'an invalid draft blocks Start')
assert.equal(recordingFrameRateDraftError('47', 60), 'Apply the custom frame rate before recording.', 'an unapplied valid draft blocks Start')
assert.equal(recordingFrameRateDraftError('47', 47), null, 'the applied custom rate is ready')
assert.equal(recordingFrameRateDraftError('60', 60), null, 'typing the already-selected rate does not change the request')
console.log('PASS recording frame-rate choices and custom range validation')
