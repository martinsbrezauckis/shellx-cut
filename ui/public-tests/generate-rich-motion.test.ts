import assert from 'node:assert/strict'
import { seedParams, serializeParams, templateManifestFrom } from '../src/panels/GenerateTemplates/model'

const manifest = templateManifestFrom({
  id: 'builtin.motion.cinematic-fog-title',
  source: 'builtin',
  kind: 'motion',
  title: 'Cinematic fog title',
  summary: 'Footage-rich fog title',
  tags: ['motion', 'fog'],
  capabilities: ['preview', 'insert', 'quality_manifest'],
  params: {
    title: { type: 'string', required: true },
    fogDensity: {
      type: 'number',
      required: false,
      default: 0.66,
      minimum: 0.2,
      maximum: 0.9,
      step: 0.02,
    },
  },
  defaults: { duration_ms: 6000 },
  lowering: { verb: 'motion.template_to_cut', args: { template: 'cinematic-fog-title' } },
  verification: { expects: ['quality_manifest'] },
})

assert.ok(manifest, 'bounded rich Motion manifest should parse')
assert.deepEqual(manifest.params.fogDensity, {
  type: 'number',
  required: false,
  default: 0.66,
  minimum: 0.2,
  maximum: 0.9,
  step: 0.02,
})
assert.equal(seedParams(manifest).fogDensity, 0.66)
assert.deepEqual(
  serializeParams(manifest, { title: 'Beyond local', fogDensity: '0.74' }),
  { title: 'Beyond local', fogDensity: 0.74 },
)

console.log('PASS Generate rich Motion numeric controls')
