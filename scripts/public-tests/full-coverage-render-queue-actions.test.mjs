import assert from 'node:assert/strict'
import test from 'node:test'
import { sameRenderQueueJobs } from '../../ui/public-tests/lib/fullCoverageRenderQueueActions.mjs'

const expected = [
  { preset: 'high', output: '/fixture/master.mp4' },
  { preset: 'standard', aspect: '9:16', output: '/fixture/vertical.mp4' },
]

test('render queue job comparison ignores object key insertion order', () => {
  assert.equal(sameRenderQueueJobs([
    { output: '/fixture/master.mp4', preset: 'high' },
    { aspect: '9:16', output: '/fixture/vertical.mp4', preset: 'standard' },
  ], expected), true)
})

test('render queue job comparison still rejects changed or extra fields', () => {
  assert.equal(sameRenderQueueJobs([
    { output: '/fixture/master.mp4', preset: 'standard' },
    { aspect: '9:16', output: '/fixture/vertical.mp4', preset: 'standard' },
  ], expected), false)
  assert.equal(sameRenderQueueJobs([
    { output: '/fixture/master.mp4', preset: 'high', format: 'h264' },
    { aspect: '9:16', output: '/fixture/vertical.mp4', preset: 'standard' },
  ], expected), false)
})
