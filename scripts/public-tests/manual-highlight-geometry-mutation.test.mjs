// Controlled browser mutations guard against a center-only geometry check.
// They alter width or height of Track controls while preserving its center, so
// only feature ids mapped to that reviewed target fail on desktop and mobile.

import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'

import { loadManualData, loadTargetAtlas, manualBehavior, root, verifyAtlasAssets, verifyFeatureSet } from './manual-highlight-atlas.mjs'
import { runMatrix, startStaticSite } from './manual-highlight-browser.mjs'

const requireFromUi = createRequire(resolve(root, 'ui/package.json'))
const { chromium } = requireFromUi('playwright')
const original = 'trackControls: { left: 0.004, top: 0.812, width: 0.078, height: 0.071, label: "Track controls" }'
const mutations = [
  { name: 'track-controls-narrowed-at-same-center', replacement: 'trackControls: { left: 0.013, top: 0.812, width: 0.06, height: 0.071, label: "Track controls" }' },
  { name: 'track-controls-shortened-at-same-center', replacement: 'trackControls: { left: 0.004, top: 0.8225, width: 0.078, height: 0.05, label: "Track controls" }' },
]

assert.equal(0.004 + 0.078 / 2, 0.013 + 0.06 / 2, 'width mutation preserves the reviewed horizontal center')
assert.equal(0.812 + 0.071 / 2, 0.8225 + 0.05 / 2, 'height mutation preserves the reviewed vertical center')
const manual = loadManualData(await readFile(manualBehavior, 'utf8'))
const { atlas } = await loadTargetAtlas()
await verifyAtlasAssets(atlas)
const { targetsByFeature } = verifyFeatureSet(manual.features, atlas)
const trackControlFeatureIds = Object.entries(atlas.featureTargets)
  .filter(([, target]) => target === 'track-controls')
  .map(([id]) => id)
  .sort()
const results = []
for (const mutation of mutations) {
  const local = await startStaticSite({
    transform(path, source) {
      if (path !== manualBehavior) return source
      const text = source.toString('utf8')
      assert.ok(text.includes(original), 'controlled mutation still matches the calibrated Track controls area')
      return Buffer.from(text.replace(original, mutation.replacement))
    },
  })
  try {
    const matrix = await runMatrix(chromium, local.url, manual.features, manual.legacyFeatureAliases, targetsByFeature)
    assert.equal(matrix.failures.length, trackControlFeatureIds.length * 2, `${mutation.name} fails every mapped id on both viewports`)
    const failedIds = [...new Set(matrix.failures.map((failure) => failure.id))].sort()
    assert.deepEqual(failedIds, trackControlFeatureIds)
    assert.deepEqual([...new Set(matrix.failures.map((failure) => failure.issue))], ['highlight dimensions no longer match the independently reviewed UI target'])
    results.push({ controlledMutation: mutation.name, failureCount: matrix.failures.length, failedIds })
  } finally {
    await new Promise((resolveClose) => local.server.close(resolveClose))
  }
}
console.log(JSON.stringify({ result: 'PASS', mutations: results }))
