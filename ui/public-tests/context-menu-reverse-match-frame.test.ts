import assert from 'node:assert/strict'
import {
  isReverseMatchFrameFixtureAnchored,
  isReverseMatchFrameFixtureRestored,
} from './lib/contextMenuReverseMatchFrame.mjs'

assert.equal(
  isReverseMatchFrameFixtureAnchored({ ok: true }, { ok: true, result: { playhead_ms: 0 } }),
  true,
  'an applied seek is anchored only after authoritative state confirms it',
)
assert.equal(
  isReverseMatchFrameFixtureAnchored(
    { ok: false, error: { code: 'conflict' } },
    { ok: true, result: { playhead_ms: 0 } },
  ),
  true,
  'an already-at-start conflict is an anchored no-op when authoritative state agrees',
)
assert.equal(
  isReverseMatchFrameFixtureAnchored({ ok: true }, { ok: true, result: { playhead_ms: 1 } }),
  false,
  'an applied response does not hide a mismatched playhead state',
)
assert.equal(
  isReverseMatchFrameFixtureAnchored({ ok: true }, { ok: false, result: { playhead_ms: 0 } }),
  false,
  'an unavailable ui.state transport is not authoritative',
)
assert.equal(
  isReverseMatchFrameFixtureAnchored(
    { ok: false, error: { code: 'unavailable' } },
    { ok: true, result: { playhead_ms: 0 } },
  ),
  false,
  'an unrelated playhead transport failure is not a valid no-op',
)

assert.equal(
  isReverseMatchFrameFixtureRestored(true, { id: 'clip' }, { playhead_ms: 0 }),
  true,
  'a serialized normal clip omits reverse: false',
)
assert.equal(
  isReverseMatchFrameFixtureRestored(true, { id: 'clip', reverse: true }, { playhead_ms: 0 }),
  false,
  'a reversed clip is not restored',
)
assert.equal(
  isReverseMatchFrameFixtureRestored(true, { id: 'clip' }, { playhead_ms: 1 }), false, 'a moved playhead is not restored')

console.log('context-menu reverse Match Frame setup and restoration tests passed')
