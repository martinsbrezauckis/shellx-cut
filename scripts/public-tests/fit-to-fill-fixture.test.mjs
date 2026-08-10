import assert from 'node:assert/strict'
import test from 'node:test'
import { selectFitToFillCandidate } from '../../ui/public-tests/lib/fitToFillFixture.mjs'

test('fit-to-fill never falls back to the active clip source hidden by the picker', () => {
  const result = selectFitToFillCandidate(
    ['long', 'primary'],
    {
      long: { probe: { duration_ms: 54_067 } },
      primary: { probe: { duration_ms: 13_313 } },
    },
    9_813,
    'primary',
  )
  assert.equal(result.selected, null)
  assert.equal(result.candidates[0].speed > 4, true)
  assert.deepEqual(result.candidates.map(({ assetId }) => assetId), ['long'])
})

test('fit-to-fill selects a compatible imported source distinct from the active clip', () => {
  const result = selectFitToFillCandidate(
    ['short', 'long'],
    {
      short: { probe: { duration_ms: 13_313 } },
      long: { probe: { duration_ms: 54_067 } },
    },
    40_000,
    'long',
  )
  assert.equal(result.selected?.assetId, 'short')
  assert.equal(Math.abs((result.selected?.speed || 0) - 2) < 0.001, true)
})

test('fit-to-fill returns diagnostics when no source fits the available gap', () => {
  const result = selectFitToFillCandidate(
    ['long'],
    { long: { probe: { duration_ms: 54_067 } } },
    9_813,
  )
  assert.equal(result.selected, null)
  assert.deepEqual(result.candidates.map(({ assetId }) => assetId), ['long'])
})
