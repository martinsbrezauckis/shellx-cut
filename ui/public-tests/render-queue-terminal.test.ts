import assert from 'node:assert/strict'
import { renderQueueTerminalError } from '../src/lib/renderQueueTerminal.ts'

assert.equal(
  renderQueueTerminalError({ count: 2, succeeded: 2, failed: 0 }),
  null,
  'only a complete zero-failure queue reaches the completed presentation',
)
assert.equal(
  renderQueueTerminalError({ count: 2, succeeded: 1, failed: 1 }),
  'Render queue completed with 1 failed delivery out of 2.',
  'a durable partial queue result remains an explicit error despite terminal done state',
)
assert.equal(
  renderQueueTerminalError({ count: 2, succeeded: 2, failed: 1 }),
  'Render queue finished without a valid delivery summary.',
  'contradictory accounting cannot be shown as completion',
)
assert.equal(
  renderQueueTerminalError({ count: 2, succeeded: 2, failed: 0, jobs: [{ ok: true }, { ok: false }] }),
  'Render queue finished without a valid delivery summary.',
  'a successful aggregate cannot conceal a failed durable child row',
)
assert.equal(
  renderQueueTerminalError({ count: 1, succeeded: 1 }),
  'Render queue finished without a valid delivery summary.',
  'a legacy or malformed terminal record cannot imply success',
)
