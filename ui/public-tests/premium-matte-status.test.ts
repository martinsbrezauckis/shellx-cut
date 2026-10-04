import assert from 'node:assert/strict'
import { hasMatteSetupAction, premiumMatteAvailability, type DoctorCard } from '../src/lib/doctor'

const card = (status: DoctorCard['status'], details: Record<string, unknown> = {}, id = 'matte_premium'): DoctorCard =>
  ({ id, kind: 'matte', status, details })

for (const [input, expected, setup] of [
  [card('missing'), 'missing', true],
  [card('ok', { installed: true, cuda_available: true }), 'ready', false],
  [card('degraded', { installed: true, cuda_available: false }), 'hardware-unavailable', false],
  [card('unknown'), 'unverified', false],
  [card('degraded', { installed: false, prepared_runtime: { rejected: true } }), 'unavailable', false],
] as const) {
  assert.equal(premiumMatteAvailability(input), expected)
  assert.equal(hasMatteSetupAction(input), setup)
}
assert.equal(premiumMatteAvailability(null), 'unverified')
assert.equal(hasMatteSetupAction(card('degraded', {}, 'matte')), true)
assert.equal(hasMatteSetupAction(card('unknown', {}, 'matte')), false)
