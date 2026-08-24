import assert from 'node:assert/strict'
import {
  assessFullCoverageOnlyExecution,
  matchesFullCoverageOnly,
  normalizeFullCoverageOnly,
} from './lib/fullCoverageScenarioSelection.mjs'

assert.equal(normalizeFullCoverageOnly(' Stock '), 'stock')
assert.equal(matchesFullCoverageOnly('Stock', 'assets.fetch(Stock · sticker import)'), true)
assert.equal(matchesFullCoverageOnly('stock', 'assets.fetch(Stock · sticker import)'), true)
assert.equal(matchesFullCoverageOnly('missing-control', 'assets.fetch(Stock · sticker import)'), false)

assert.deepEqual(
  assessFullCoverageOnlyExecution({ only: 'Stock', declaredProbes: 5, executedProbes: 5 }),
  { ok: true, filter: 'stock', reason: '' },
  'canonical Stock selector records the declared and executed actions',
)
assert.deepEqual(
  assessFullCoverageOnlyExecution({ only: 'stock', declaredProbes: 5, executedProbes: 5 }),
  { ok: true, filter: 'stock', reason: '' },
  'lowercase stock selector is equivalent to the canonical spelling',
)
assert.equal(
  assessFullCoverageOnlyExecution({ only: 'missing-control', declaredProbes: 0, executedProbes: 0 }).ok,
  false,
  'a nonmatching focused selector cannot exit green with zero declared/zero executed actions',
)

console.log('PASS full coverage scenario selection')
