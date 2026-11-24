import { test } from 'node:test'
import assert from 'node:assert/strict'
import { exportRefusalMessage } from '../src/topbar/exportRefusal'
const message = 'This recording has unapplied zoom changes. Update video first (screen_record.take_refresh), or explicitly pass allow_stale_recordings:true to export the old video.'
test('stale recording refusal explains why export did not start and the next action', () => {
  assert.equal(exportRefusalMessage({code:'invalid_args',message}), 'Export did not start: this recording has zoom changes that are not in the video yet. Press Update video, then export again.')
})
test('ordinary export errors retain their own text; marker requires invalid_args', () => {
  assert.equal(exportRefusalMessage({code:'conflict',message:'Output folder is read-only.'}), 'Export failed: the export folder is read-only. Choose a writable export folder, then try again.')
  assert.equal(exportRefusalMessage({code:'conflict',message}), 'Export did not start: this recording has zoom changes that are not in the video yet. Press Update video, then export again.')
})
