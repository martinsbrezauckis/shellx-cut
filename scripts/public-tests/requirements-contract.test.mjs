#!/usr/bin/env node
// Every verb declares what it needs (schema/requirements.json): a new verb
// without a declaration fails here, and so does a declaration that names an
// unknown tool, a verb in two features, or an ffmpeg-gated verb that does not
// say it needs FFmpeg.
import assert from 'node:assert/strict'
import test from 'node:test'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  loadRequirements,
  loadVerbs,
  mediaToolGateVerbs,
  requirementProblems,
  verbRequirements,
} from '../lib/requirements-contract.mjs'
import { expectedDocuments } from '../generate-requirements-docs.mjs'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const registry = loadRequirements(ROOT)
const verbs = loadVerbs(ROOT)
const gate = mediaToolGateVerbs(ROOT)

test('the registry declares every verb once and is consistent with the verb contract and the ffmpeg gate', () => {
  assert.deepEqual(requirementProblems(registry, verbs, gate), [])
})

test('a verb without a declaration fails the gate', () => {
  const copy = structuredClone(registry)
  copy.features.find((feature) => feature.id === 'transcription').verbs = []
  assert.deepEqual(
    requirementProblems(copy, verbs, gate),
    ['verb media.transcribe has no requirements declaration (add it to a feature in schema/requirements.json; a feature with "requires": [] is the declaration for "needs nothing")'],
  )
})

test('an unknown requirement, a duplicate verb and a listed open project are refused', () => {
  const copy = structuredClone(registry)
  copy.features[0].requires = ['no_such_tool', 'open_project']
  copy.features[1].verbs.push(copy.features[0].verbs[0])
  const problems = requirementProblems(copy, verbs, gate)
  assert.ok(problems.some((line) => line.includes('unknown requirement no_such_tool')), problems.join('\n'))
  assert.ok(problems.some((line) => line.includes('open_project follows behavior.project_state')), problems.join('\n'))
  assert.ok(problems.some((line) => / is in both /.test(line)), problems.join('\n'))
})

test('a gated verb that drops ffmpeg is caught', () => {
  const copy = structuredClone(registry)
  const importFeature = copy.features.find((feature) => feature.id === 'import')
  importFeature.requires = importFeature.requires.filter((id) => id !== 'ffmpeg')
  const problems = requirementProblems(copy, verbs, gate)
  assert.ok(problems.some((line) => line.startsWith('verb media.import is refused without ffmpeg')), problems.join('\n'))
})

test('a verb gets an open project from its contract, not from the registry', () => {
  assert.equal(verbRequirements(registry, verbs, 'media.transcribe').requires[0], 'open_project')
  assert.ok(!verbRequirements(registry, verbs, 'project.create').requires.includes('open_project'))
  assert.deepEqual(verbRequirements(registry, verbs, 'render.final').optional.map((item) => item.id).slice(0, 1), ['gpu_encoder'])
})

test('every requirement says how its state is read and where to get it', () => {
  for (const item of registry.requirements) {
    assert.ok(item.get.verb || item.get.person, `${item.id} names no way forward`)
    assert.ok(Object.keys(item.state).length > 0, `${item.id} names no state source`)
  }
})

test('the agent documents are generated from the registry (node scripts/generate-requirements-docs.mjs --check)', () => {
  for (const [path, current, expected] of expectedDocuments(ROOT)) {
    assert.ok(current === expected, `${path} is stale: run node scripts/generate-requirements-docs.mjs`)
  }
})

test('every control, Settings page, verb and argument a registry text names exists', async () => {
  const { loadUiSources, registryNameProblems } = await import('../lib/requirements-names.mjs')
  assert.deepEqual(registryNameProblems(registry, verbs, loadUiSources(ROOT)), [])
})

test('the name check catches a missing control, verb and argument', async () => {
  const { loadUiSources, registryNameProblems } = await import('../lib/requirements-names.mjs')
  const copy = structuredClone(registry)
  const item = copy.requirements.find((entry) => entry.id === 'transcript')
  item.get.button = 'Transcribe this clip'
  item.get.person = 'use the Retranscribe button, or call media.retranscribe, or media.transcribe {asset, speed}'
  const problems = registryNameProblems(copy, verbs, loadUiSources(ROOT))
  assert.ok(problems.some((line) => line.includes("no control labelled 'Transcribe this clip'")), problems.join('\n'))
  assert.ok(problems.some((line) => line.includes("names a control 'Retranscribe'")), problems.join('\n'))
  assert.ok(problems.some((line) => line.includes('names verb media.retranscribe')), problems.join('\n'))
  assert.ok(problems.some((line) => line.includes("media.transcribe has no argument 'speed'")), problems.join('\n'))
})
