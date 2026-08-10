import {
  KEY_ACTIONS,
  getBinding,
  replaceKeymapBindings,
} from './keymap'

export const KEYMAP_PROFILE_SCHEMA = 'shellx-cut/keymap@1'
const MAX_PROFILE_BYTES = 64 * 1024

export type KeymapPresetId = 'cut' | 'premiere' | 'resolve' | 'final-cut'

export interface KeymapPreset {
  id: KeymapPresetId
  label: string
  description: string
  bindings: Record<string, string>
}

function completePreset(overrides: Record<string, string>): Record<string, string> {
  return Object.fromEntries(KEY_ACTIONS.map((action) => [action.id, overrides[action.id] ?? action.def]))
}

/** Familiar-style mappings are intentionally limited to commands Cut owns.
 * They are not imports of another editor's complete shortcut catalog. */
export const KEYMAP_PRESETS: ReadonlyArray<KeymapPreset> = [
  {
    id: 'cut',
    label: 'Cut default',
    description: 'Restore Cut’s compact editing defaults.',
    bindings: completePreset({}),
  },
  {
    id: 'premiere',
    label: 'Premiere-style',
    description: 'Add Edit, Razor, Snap, and marker navigation use familiar Premiere keys.',
    bindings: completePreset({
      'timeline.split': 'Ctrl+K',
      'timeline.razor': 'C',
      'timeline.snap': 'S',
      'timeline.prevMarker': 'Ctrl+Shift+M',
      'timeline.nextMarker': 'Shift+M',
    }),
  },
  {
    id: 'resolve',
    label: 'Resolve-style',
    description: 'Split Clip uses Ctrl/Cmd+Backslash; J-K-L, Blade, Snap, In/Out, and Marker already align.',
    bindings: completePreset({ 'timeline.split': 'Ctrl+\\' }),
  },
  {
    id: 'final-cut',
    label: 'Final Cut-style',
    description: 'Blade, full-screen playback, snapping, and marker navigation use familiar Final Cut keys.',
    bindings: completePreset({
      'timeline.split': 'Ctrl+B',
      'preview.fullscreen': 'Ctrl+Shift+F',
      'timeline.prevMarker': 'Ctrl+;',
      'timeline.nextMarker': "Ctrl+'",
    }),
  },
]

export interface PortableKeymapProfile {
  schema: typeof KEYMAP_PROFILE_SCHEMA
  bindings: Record<string, string>
}

export type KeymapProfileImportResult =
  | { ok: true; changed: number; ignored: number; reason: null }
  | { ok: false; changed: 0; ignored: 0; reason: string }

export function currentKeymapProfile(): PortableKeymapProfile {
  return {
    schema: KEYMAP_PROFILE_SCHEMA,
    bindings: Object.fromEntries(KEY_ACTIONS.map((action) => [action.id, getBinding(action.id)])),
  }
}

export function serializeKeymapProfile(): string {
  return `${JSON.stringify(currentKeymapProfile(), null, 2)}\n`
}

export function applyKeymapPreset(id: string): KeymapProfileImportResult {
  const preset = KEYMAP_PRESETS.find((candidate) => candidate.id === id)
  if (!preset) return { ok: false, changed: 0, ignored: 0, reason: 'Unknown shortcut preset.' }
  const replacement = replaceKeymapBindings(preset.bindings)
  if (!replacement.ok) {
    return { ok: false, changed: 0, ignored: 0, reason: replacement.reason ?? 'Shortcut preset could not be applied.' }
  }
  return { ok: true, changed: replacement.changed, ignored: 0, reason: null }
}

export function importKeymapProfile(text: string): KeymapProfileImportResult {
  if (new TextEncoder().encode(text).byteLength > MAX_PROFILE_BYTES) {
    return { ok: false, changed: 0, ignored: 0, reason: 'Shortcut profile is larger than 64 KiB.' }
  }
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    return { ok: false, changed: 0, ignored: 0, reason: 'Shortcut profile is not valid JSON.' }
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return { ok: false, changed: 0, ignored: 0, reason: 'Shortcut profile must be a JSON object.' }
  }
  const profile = parsed as { schema?: unknown; bindings?: unknown }
  if (profile.schema !== KEYMAP_PROFILE_SCHEMA) {
    return { ok: false, changed: 0, ignored: 0, reason: `Shortcut profile must use ${KEYMAP_PROFILE_SCHEMA}.` }
  }
  if (!profile.bindings || typeof profile.bindings !== 'object' || Array.isArray(profile.bindings)) {
    return { ok: false, changed: 0, ignored: 0, reason: 'Shortcut profile needs a bindings object.' }
  }

  const known = new Set(KEY_ACTIONS.map((action) => action.id))
  const entries = Object.entries(profile.bindings as Record<string, unknown>)
  const bindings: Record<string, string> = {}
  for (const [id, binding] of entries) {
    if (!known.has(id)) continue
    if (typeof binding !== 'string') {
      return { ok: false, changed: 0, ignored: 0, reason: `${id} must contain a shortcut string.` }
    }
    bindings[id] = binding
  }
  const replacement = replaceKeymapBindings(bindings)
  if (!replacement.ok) {
    return { ok: false, changed: 0, ignored: 0, reason: replacement.reason ?? 'Shortcut profile could not be applied.' }
  }
  return {
    ok: true,
    changed: replacement.changed,
    ignored: entries.filter(([id]) => !known.has(id)).length,
    reason: null,
  }
}
