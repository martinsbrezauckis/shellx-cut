import { strict as assert } from 'node:assert'
import {
  KEY_ACTIONS,
  getBinding,
  resetKeymap,
  setBinding,
} from '../src/lib/keymap'
import {
  KEYMAP_PRESETS,
  KEYMAP_PROFILE_SCHEMA,
  applyKeymapPreset,
  importKeymapProfile,
  serializeKeymapProfile,
} from '../src/lib/keymapProfile'

const stored = new Map<string, string>()
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (key: string) => stored.get(key) ?? null,
    setItem: (key: string, value: string) => { stored.set(key, value) },
    removeItem: (key: string) => { stored.delete(key) },
  },
})
Object.defineProperty(globalThis, 'document', {
  configurable: true,
  value: { dispatchEvent: () => true },
})

resetKeymap()
assert.equal(setBinding('timeline.split', 'Alt+1'), true)
const serialized = serializeKeymapProfile()
const exported = JSON.parse(serialized)
assert.equal(exported.schema, KEYMAP_PROFILE_SCHEMA)
assert.equal(Object.keys(exported.bindings).length, KEY_ACTIONS.length, 'export includes every editable action')
assert.equal(exported.bindings['timeline.split'], 'Alt+1')
assert.equal(serialized.includes('media'), false, 'profile carries no media data')

resetKeymap()
assert.equal(getBinding('timeline.split'), 'S')
assert.deepEqual(importKeymapProfile(serialized), { ok: true, changed: 1, ignored: 0, reason: null })
assert.equal(getBinding('timeline.split'), 'Alt+1', 'valid profile applies exactly')

const forwardCompatible = JSON.stringify({
  schema: KEYMAP_PROFILE_SCHEMA,
  bindings: { 'timeline.split': 'Alt+2', 'future.action': 'F20' },
})
assert.deepEqual(importKeymapProfile(forwardCompatible), { ok: true, changed: 1, ignored: 1, reason: null })
assert.equal(getBinding('timeline.split'), 'Alt+2', 'known actions survive a future-action entry')

assert.deepEqual(KEYMAP_PRESETS.map((preset) => preset.id), ['cut', 'premiere', 'resolve', 'final-cut'])
assert.equal(applyKeymapPreset('premiere').ok, true)
assert.equal(getBinding('timeline.split'), 'Ctrl+K')
assert.equal(getBinding('timeline.razor'), 'C')
assert.equal(getBinding('timeline.snap'), 'S')
assert.equal(getBinding('timeline.nextMarker'), 'Shift+M')
assert.equal(applyKeymapPreset('resolve').ok, true)
assert.equal(getBinding('timeline.split'), 'Ctrl+\\')
assert.equal(getBinding('timeline.razor'), 'B', 'unmapped actions return to Cut defaults')
assert.equal(applyKeymapPreset('final-cut').ok, true)
assert.equal(getBinding('timeline.split'), 'Ctrl+B')
assert.equal(getBinding('preview.fullscreen'), 'Ctrl+Shift+F')
assert.equal(getBinding('timeline.prevMarker'), 'Ctrl+;')
assert.equal(getBinding('timeline.nextMarker'), "Ctrl+'")
assert.deepEqual(applyKeymapPreset('missing'), {
  ok: false,
  changed: 0,
  ignored: 0,
  reason: 'Unknown shortcut preset.',
})

const beforeRejectedProfile = getBinding('timeline.split')
const duplicate = importKeymapProfile(JSON.stringify({
  schema: KEYMAP_PROFILE_SCHEMA,
  bindings: { 'preview.playPause': 'F8', 'timeline.split': 'F8' },
}))
assert.equal(duplicate.ok, false)
assert.match(duplicate.reason, /assigned to both/)
assert.equal(getBinding('timeline.split'), beforeRejectedProfile, 'conflicting import is atomic')

const fixedCollision = importKeymapProfile(JSON.stringify({
  schema: KEYMAP_PROFILE_SCHEMA,
  bindings: { 'timeline.split': 'F9' },
}))
assert.equal(fixedCollision.ok, false)
assert.match(fixedCollision.reason, /Start \/ stop recording/)
assert.equal(getBinding('timeline.split'), beforeRejectedProfile, 'fixed-key collision does not mutate storage')

for (const [text, reason] of [
  ['not-json', /not valid JSON/],
  [JSON.stringify({ schema: 'shellx-cut/keymap@2', bindings: {} }), /shellx-cut\/keymap@1/],
  [JSON.stringify({ schema: KEYMAP_PROFILE_SCHEMA, bindings: { 'timeline.split': 'Ctrl++S' } }), /unsupported shortcut/],
  [JSON.stringify({ schema: KEYMAP_PROFILE_SCHEMA, bindings: { 'timeline.split': 's' } }), /unsupported shortcut/],
] as const) {
  const result = importKeymapProfile(text)
  assert.equal(result.ok, false)
  assert.match(result.reason, reason)
  assert.equal(getBinding('timeline.split'), beforeRejectedProfile, 'invalid import leaves the existing profile intact')
}

console.log('PASS portable keymap profile round-trip, compatibility, and atomic validation')
