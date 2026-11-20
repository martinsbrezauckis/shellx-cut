import { strict as assert } from 'node:assert'
import { assetReadiness, decodeWarning, type MediaReadinessAsset } from '../src/panels/Assets/mediaReadiness'

const warning =
  'Only the first 1.7 s of 7.6 s of picture can be read: the file is cut off or damaged. '
  + 'An export will stop early where it is missing. Import a complete copy of the file if you have one.'

const asset = (probe: MediaReadinessAsset['probe'], offline = false): MediaReadinessAsset => ({
  id: 'a1',
  path: '/media/cut-off.mp4',
  proxy: 'proxies/a1.mp4',
  probe,
  offline,
  used: 1,
})

// A cut-off source shows a red badge with the engine's sentence and asks for attention.
const cutOff = assetReadiness(asset({ kind: 'video', duration_ms: 7615, decode_check: { warning } }))
assert.equal(cutOff.badges[0].label, 'File cut off')
assert.equal(cutOff.badges[0].tone, 'bad')
assert.equal(cutOff.badges[0].title, warning)
assert.equal(cutOff.hint, warning)
assert.equal(cutOff.needsAction, true)
// The usual readiness facts stay next to it.
assert.ok(cutOff.badges.some((b) => b.label === 'Proxy ready'))

// A checked, whole source says nothing extra.
const whole = assetReadiness(asset({ kind: 'video', duration_ms: 7615, decode_check: {} }))
assert.ok(!whole.badges.some((b) => b.label === 'File cut off'))
assert.equal(whole.needsAction, false)
assert.equal(decodeWarning(asset({ kind: 'video', decode_check: { warning: '  ' } })), null)

// A missing source is about relinking first; the old file's warning is not repeated.
const offline = assetReadiness(asset({ kind: 'video', decode_check: { warning } }, true))
assert.equal(offline.level, 'missing')
assert.ok(!offline.badges.some((b) => b.label === 'File cut off'))

console.log('asset decode warning: ok')
