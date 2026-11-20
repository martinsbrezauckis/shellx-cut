import { strict as assert } from 'node:assert'
import { assetReadiness, variableRateBadge, type MediaReadinessAsset } from '../src/panels/Assets/mediaReadiness'

const asset = (probe: MediaReadinessAsset['probe']): MediaReadinessAsset => ({
  id: 'a1',
  path: '/media/20260618_172347.mp4',
  proxy: 'proxies/a1.mp4',
  probe,
  offline: false,
  used: 1,
})

// The HQ-run phone clip: nominal 60, average 58.59, variable.
const phone = asset({ kind: 'video', fps: 70200000 / 1198127, nominal_fps: 60, variable_frame_rate: true })
const badge = variableRateBadge(phone)
assert.ok(badge)
assert.equal(badge.label, 'Variable frame rate')
assert.equal(badge.tone, 'neutral')
assert.equal(
  badge.title,
  'This clip has a variable frame rate (58.59 fps on average, 60 fps nominal), as phones record. Exports place each frame at its real time.',
)
const readiness = assetReadiness(phone)
assert.ok(readiness.badges.some((b) => b.label === 'Variable frame rate'))
assert.equal(readiness.needsAction, false, 'a note, not a problem')

// A constant-rate clip has no such badge.
assert.equal(variableRateBadge(asset({ kind: 'video', fps: 25, nominal_fps: 25 })), null)

console.log('asset variable frame rate: ok')
