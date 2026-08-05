import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const read = (path) => readFileSync(resolve(ROOT, path), 'utf8')
const escapeRegExp = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
const schema = JSON.parse(read('schema/verbs.json'))
const recipes = JSON.parse(read('schema/recipes.json')).recipes
const domains = [...new Set(schema.verbs.map((verb) => verb.domain))].sort()
const uiOpenSurfaceIds = schema.verbs
  .find((verb) => verb.name === 'ui.open')
  ?.args?.properties?.panel?.enum

test('schema event registry exactly matches emitted server events', () => {
  const source = read('app/server/src/events.rs')
  const enumBody = source.match(/pub enum Event \{([\s\S]*?)\n\}/)?.[1] || ''
  const emitted = [...enumBody.matchAll(/^ {4}([A-Z][A-Za-z0-9]+)(?:\s*\{|,)/gm)]
    .map((match) => match[1].replace(/([a-z0-9])([A-Z])/g, '$1_$2').toLowerCase())
    .sort()
  assert.deepEqual([...schema.events.types].sort(), emitted)
})

test('bundled event docs name every machine-contract event', () => {
  for (const path of [
    'README.md',
    'docs/public/DEBUG_API.md',
    'skill/shellx-cut/SKILL.md',
    'skill/shellx-cut/reference.md',
  ]) {
    const document = read(path)
    for (const eventType of schema.events.types) {
      assert.match(document, new RegExp(`\\b${escapeRegExp(eventType)}\\b`), `${path} omits ${eventType}`)
    }
  }
})

test('README domain table covers every schema domain exactly once', () => {
  const listed = [...read('README.md').matchAll(/^\| \*\*([^*]+)\*\* \|/gm)]
    .map((match) => match[1])
    .sort()
  assert.deepEqual(listed, domains)
})

test('public feature inventory covers every agent-openable UI surface', () => {
  assert.ok(Array.isArray(uiOpenSurfaceIds) && uiOpenSurfaceIds.length > 0)
  const features = read('docs/public/FEATURES.md')
  for (const surface of uiOpenSurfaceIds) {
    assert.match(features, new RegExp(`\\b${escapeRegExp(surface)}\\b`), `missing ui.open surface ${surface}`)
  }
})

test('public feature inventory covers current workflows and recipes', () => {
  const features = read('docs/public/FEATURES.md')
  for (const feature of ['assemble.repurpose', 'autopilot.run', 'clip.candidates', 'plugins.call']) {
    assert.match(features, new RegExp(escapeRegExp(feature)), `missing ${feature}`)
  }
  for (const recipe of recipes) {
    const title = escapeRegExp(recipe.title).replaceAll(' ', '\\s+')
    assert.match(features, new RegExp(title, 'i'), `missing recipe ${recipe.title}`)
  }
})

test('agent-facing docs expose the scoped plugin gateway', () => {
  for (const path of ['README.md', 'docs/public/FEATURES.md', 'docs/public/DEBUG_API.md', 'skill/shellx-cut/SKILL.md']) {
    assert.match(read(path), /plugins\.call/, `${path} omits plugins.call`)
  }
})

test('typed client retains schema-only advanced arguments', () => {
  const client = read('ui/src/lib/client.ts')
  assert.match(client, /'media\.import':[^\n]+capture_manifest\?: string/)
  assert.match(client, /'media\.import':[^\n]+include_inverse\?: boolean/)
  assert.match(client, /'captions\.kinetic':[^\n]+per_word\?: boolean/)
})

test('About version remains engine-derived instead of hard-coded', () => {
  const about = read('ui/src/panels/Environment/About.tsx')
  assert.match(about, /report\?\.app_version/)
  assert.doesNotMatch(about, /0\.6\.\d+/)
})
