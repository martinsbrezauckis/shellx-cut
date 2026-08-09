import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { MotionClipLink } from '../src/lib/motionLinkModel'
import MotionEffectsSection from '../src/panels/Inspector/MotionEffectsSection'

const root = resolve(import.meta.dirname, '..')
const read = (relative: string) => readFileSync(resolve(root, relative), 'utf8')
const panel = read('src/panels/Inspector/MotionEffectsSection.tsx')
const linkPanel = read('src/panels/Inspector/MotionLinkSection.tsx')
const model = read('src/lib/motionLinkModel.ts')

assert.match(model, /shellx-cut\/motion-effects-summary@1/, 'typed effects-summary schema is exposed')
assert.match(model, /editableInCut: false/, 'Motion effects remain explicitly opaque in Cut')
assert.match(model, /shellx-cut\/motion-import-attestation@1/, 'typed path-free Motion origin attestation is exposed')
assert.match(model, /'verified' \| 'legacy-unverified'/, 'typed model distinguishes verified lineage from legacy compatibility')
assert.match(model, /shellx-cut\/current-motion-package-lineage@1/, 'typed origin evidence exposes current-package comparison')
assert.match(model, /'exact' \| 'changed' \| 'unavailable'/, 'typed current-package evidence preserves three-state truth')
assert.match(panel, /data-cut-motion-effects/, 'effects panel has a stable debug selector')
assert.match(panel, /data-cut-motion-effect-layer/, 'effect rows have stable debug selectors')
assert.match(panel, /Spill/, 'spill suppression is visible')
assert.match(panel, /Matte cleanup/, 'matte cleanup is visible')
assert.match(panel, /Tracked/, 'tracked roto is visible')
assert.match(panel, /Refresh render to update Cut.s pixels/, 'stale rendered pixels are explained')
assert.match(linkPanel, /<MotionEffectsSection link=\{link\}/, 'linked Motion Inspector composes the bounded effects panel')

const link: MotionClipLink = {
  schema: 'shellx-cut/motion-link@1',
  clipId: 'clip-1',
  assetId: 'asset-1',
  motionSourceId: 'pkg:motion',
  packageId: 'pkg',
  motionId: 'motion',
  sourceRevision: 'a'.repeat(64),
  sourcePath: '/local/package',
  planPath: '/local/plan.json',
  mode: 'rendered_media',
  state: 'source-dirty',
  render: { path: '/local/render.mp4', sha256: 'b'.repeat(64), byteLength: 1, artifactHandleId: 'artifact-1' },
  fallbackPath: '/local/render.mp4',
  originAttestation: {
    schema: 'shellx-cut/motion-import-attestation@1',
    status: 'verified',
    artifactHandleId: 'artifact-1',
    artifactOperationHash: 'c'.repeat(64),
    artifactDescriptorSha256: 'd'.repeat(64),
    packageLineage: {
      schema: 'shellx-motion/package-render-lineage@1',
      manifestSha256: 'e'.repeat(64),
      motionSha256: 'f'.repeat(64),
    },
    currentPackage: {
      schema: 'shellx-cut/current-motion-package-lineage@1',
      status: 'exact',
      lineage: {
        schema: 'shellx-motion/package-render-lineage@1',
        manifestSha256: 'e'.repeat(64),
        motionSha256: 'f'.repeat(64),
      },
      changedFields: [],
      reason: null,
    },
    renderReceipt: { id: 'render-1', operation: 'render.final', status: 'passed', sha256: '1'.repeat(64) },
    connectorReceipt: null,
    cutPlanReceipt: { id: 'cut-import-1', operation: 'cut.import.plan', status: 'passed' },
  },
  effects: {
    schema: 'shellx-cut/motion-effects-summary@1',
    available: true,
    ownership: 'motion',
    editableInCut: false,
    keyedLayerCount: 1,
    rotoLayerCount: 2,
    trackedRotoLayerCount: 1,
    layers: [{
      id: 'subject',
      name: 'Hero subject',
      type: 'video',
      keying: { keyColor: '#00ff00', spillSuppression: 0.72, matteCleanup: true },
      roto: { frameCount: 4, tracked: true, model: 'similarity' },
    }],
  },
}
const rendered = renderToStaticMarkup(createElement(MotionEffectsSection, { link }))
assert.match(rendered, /1 keyed layer/, 'rendered panel shows keyed count')
assert.match(rendered, /2 roto layers/, 'rendered panel shows roto count')
assert.match(rendered, /Spill 72%/, 'rendered panel formats spill strength')
assert.match(rendered, /Roto · 4 frames/, 'rendered panel shows animated frame count')
assert.match(rendered, /Tracked · similarity/, 'rendered panel shows tracked roto model')
assert.match(rendered, /render-stale/, 'rendered panel marks source-dirty pixels as stale')

console.log('PASS motion-effects-ui-contract')
