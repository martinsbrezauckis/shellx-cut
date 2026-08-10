// keymap.ts — the central remappable keymap.
//
// Role: one table of user-remappable editor actions (id → default binding),
// with overrides persisted in localStorage (`cut.keymap`, the cut.* pref
// pattern). Handlers consult `matches(e, id)` per keydown — bindings are read
// live, so a remap applies instantly, no reload. KeymapOverlay derives its
// rows for these actions from THIS table (single source of truth — the old
// hand-mirrored list could silently drift); the Settings drawer hosts the
// editor (EnvironmentPanel → Keyboard shortcuts).
//
// Deliberately NOT here: multi-key system combos (Ctrl/Cmd+Z/S/A/B/T…),
// Escape, arrows — those are conventions, not preferences; remapping them
// creates more confusion than value. The table is the REMAPPABLE subset.
// Callers: Preview transport, Timeline keydown effects, KeymapOverlay,
// EnvironmentPanel. Dependencies: localStorage only.

export interface KeyAction {
  /** Stable id, e.g. 'timeline.split' — the localStorage override key. */
  id: string
  /** Human label for the editor + overlay. */
  label: string
  /** Where it fires (grouping only; matching is the caller's business). */
  group: 'preview' | 'timeline'
  /** Default binding, normalized (e.g. 'S', 'Space', 'Shift+Z'). */
  def: string
}

/** Every remappable action. Order = editor/overlay display order. */
export const KEY_ACTIONS: KeyAction[] = [
  { id: 'preview.playPause', label: 'Play / pause', group: 'preview', def: 'Space' },
  { id: 'preview.shuttleBack', label: 'Shuttle back (J)', group: 'preview', def: 'J' },
  { id: 'preview.stop', label: 'Pause (K)', group: 'preview', def: 'K' },
  { id: 'preview.shuttleFwd', label: 'Shuttle forward (L)', group: 'preview', def: 'L' },
  { id: 'preview.fullscreen', label: 'Full-screen preview', group: 'preview', def: 'F' },
  { id: 'preview.guides', label: 'Cycle safe-area guides', group: 'preview', def: 'G' },
  { id: 'timeline.split', label: 'Split at playhead', group: 'timeline', def: 'S' },
  { id: 'timeline.razor', label: 'Razor mode', group: 'timeline', def: 'B' },
  { id: 'timeline.snap', label: 'Snap magnet', group: 'timeline', def: 'N' },
  { id: 'timeline.rippleTrimStart', label: 'Ripple trim start to playhead', group: 'timeline', def: 'Q' },
  { id: 'timeline.rippleTrimEnd', label: 'Ripple trim end to playhead', group: 'timeline', def: 'W' },
  { id: 'timeline.marker', label: 'Add marker', group: 'timeline', def: 'M' },
  { id: 'timeline.markIn', label: 'Mark in', group: 'timeline', def: 'I' },
  { id: 'timeline.markOut', label: 'Mark out', group: 'timeline', def: 'O' },
  { id: 'timeline.trimTool', label: 'Cycle trim tool', group: 'timeline', def: 'T' },
  { id: 'timeline.prevMarker', label: 'Jump to previous marker', group: 'timeline', def: '[' },
  { id: 'timeline.nextMarker', label: 'Jump to next marker', group: 'timeline', def: ']' },
]

export interface FixedKeyAction {
  id: string
  label: string
  group: 'global' | 'recording'
  binding: string
}

/**
 * Fixed app/native shortcuts. These reserve their bindings in the remap editor,
 * but are never presented as customisable because their consumers are fixed
 * app-shell or OS registrations.
 */
export const FIXED_KEY_ACTIONS: FixedKeyAction[] = [
  { id: 'comments.toggle', label: 'Show / hide comments', group: 'global', binding: 'Ctrl+Shift+C' },
  { id: 'recording.toggle', label: 'Start / stop recording', group: 'recording', binding: 'F9' },
  { id: 'recording.cameraVisible', label: 'Show / hide camera overlay', group: 'recording', binding: 'F10' },
  { id: 'recording.cameraPosition', label: 'Cycle camera position', group: 'recording', binding: 'F11' },
  { id: 'recording.cameraPositionReverse', label: 'Reverse camera position cycle', group: 'recording', binding: 'Shift+F11' },
  { id: 'recording.marker', label: 'Add recording marker', group: 'recording', binding: 'F12' },
]

const STORE_KEY = 'cut.keymap'

/** Read the override map {actionId: binding}. try/catch: storage can be off. */
function readOverrides(): Record<string, string> {
  try {
    const raw = localStorage.getItem(STORE_KEY)
    if (!raw) return {}
    const v = JSON.parse(raw)
    return v && typeof v === 'object' ? (v as Record<string, string>) : {}
  } catch {
    return {}
  }
}

function writeOverrides(map: Record<string, string>): boolean {
  let written = false
  try {
    if (Object.keys(map).length === 0) localStorage.removeItem(STORE_KEY)
    else localStorage.setItem(STORE_KEY, JSON.stringify(map))
    written = true
  } catch {
    /* storage unavailable — remaps just don't persist */
  }
  document.dispatchEvent(new CustomEvent('cut:keymap-changed'))
  return written
}

export interface KeymapReplacementResult {
  ok: boolean
  changed: number
  reason: string | null
}

/** Accept only the normalized, single-stroke bindings this module emits. */
export function isSupportedBinding(binding: string): boolean {
  if (!binding || binding.length > 64 || /[\r\n\t]/.test(binding)) return false
  const parts = binding.split('+')
  const key = parts.at(-1) ?? ''
  const modifiers = parts.slice(0, -1)
  const expected = ['Ctrl', 'Alt', 'Shift'].filter((modifier) => modifiers.includes(modifier))
  if (modifiers.length !== expected.length || modifiers.some((modifier, index) => modifier !== expected[index])) return false
  if (!key || ['Control', 'Ctrl', 'Alt', 'Shift', 'Meta', 'Escape', 'Tab', 'Enter'].includes(key)) return false
  if (key.length === 1) return key === key.toUpperCase()
  return /^[A-Za-z][A-Za-z0-9]{0,31}$/.test(key)
}

/** Replace the complete editable profile atomically. Missing actions return to
 * defaults; unknown actions must be filtered by the portable-profile parser. */
export function replaceKeymapBindings(bindings: Record<string, string>): KeymapReplacementResult {
  const owners = new Map<string, string>()
  for (const action of FIXED_KEY_ACTIONS) owners.set(action.binding, action.label)
  const overrides: Record<string, string> = {}
  for (const action of KEY_ACTIONS) {
    const binding = Object.hasOwn(bindings, action.id) ? bindings[action.id] : action.def
    if (typeof binding !== 'string' || !isSupportedBinding(binding)) {
      return { ok: false, changed: 0, reason: `${action.label} has an unsupported shortcut.` }
    }
    const owner = owners.get(binding)
    if (owner) {
      return { ok: false, changed: 0, reason: `${displayBinding(binding)} is assigned to both ${owner} and ${action.label}.` }
    }
    owners.set(binding, action.label)
    if (binding !== action.def) overrides[action.id] = binding
  }
  if (!writeOverrides(overrides)) {
    return { ok: false, changed: 0, reason: 'Shortcut storage is unavailable.' }
  }
  return { ok: true, changed: Object.keys(overrides).length, reason: null }
}

/** The live binding for an action (override ?? default). */
export function getBinding(id: string): string {
  const ov = readOverrides()[id]
  if (ov) return ov
  return KEY_ACTIONS.find((a) => a.id === id)?.def ?? ''
}

/** Set (or clear with null) one action's binding. */
export function setBinding(id: string, binding: string | null): boolean {
  if (!KEY_ACTIONS.some((action) => action.id === id)) return false
  if (binding !== null && !isSupportedBinding(binding)) return false
  if (binding !== null && conflictsFor(binding, id).length > 0) return false
  const map = readOverrides()
  if (binding === null) delete map[id]
  else map[id] = binding
  return writeOverrides(map)
}

/** Drop every override (back to defaults). */
export function resetKeymap() {
  writeOverrides({})
}

/** True when the action carries a non-default binding. */
export function isRemapped(id: string): boolean {
  return readOverrides()[id] !== undefined
}

/** Normalize a KeyboardEvent to a binding string, or null when it's just a
 *  modifier / unusable key. Single keys uppercase ('S'), specials by name
 *  ('Space', '['), modifiers prefixed in fixed order (Ctrl+Alt+Shift+X). */
export function bindingFromEvent(e: KeyboardEvent): string | null {
  const k = e.key
  if (k === 'Control' || k === 'Alt' || k === 'Shift' || k === 'Meta') return null
  if (k === 'Escape' || k === 'Tab' || k === 'Enter') return null // reserved
  let name = k === ' ' ? 'Space' : k.length === 1 ? k.toUpperCase() : k
  const mods: string[] = []
  if (e.ctrlKey || e.metaKey) mods.push('Ctrl')
  if (e.altKey) mods.push('Alt')
  // Plain shifted letters carry shift in the resulting character, but modified
  // chords need the Shift token so Ctrl+Shift+C is distinct from Ctrl+C.
  if (e.shiftKey && (k.length > 1 || e.ctrlKey || e.metaKey || e.altKey)) mods.push('Shift')
  if (mods.length) name = `${mods.join('+')}+${name}`
  return name
}

/** Does this keydown match the action's LIVE binding? Callers still apply
 *  their own guards (editable targets, scope) before asking. */
export function matchesBinding(e: KeyboardEvent, binding: string): boolean {
  if (!binding) return false
  const parts = binding.split('+')
  const key = parts[parts.length - 1]
  const wantCtrl = parts.includes('Ctrl')
  const wantAlt = parts.includes('Alt')
  const wantShift = parts.includes('Shift')
  const evKey = e.key === ' ' ? 'Space' : e.key.length === 1 ? e.key.toUpperCase() : e.key
  if (evKey !== key) return false
  if ((e.ctrlKey || e.metaKey) !== wantCtrl) return false
  if (e.altKey !== wantAlt) return false
  // single-char bindings: shift changes the char itself (e.g. Z vs shift+z both
  // report 'Z' uppercase) — only enforce shift for multi-char keys.
  if (key.length > 1 && e.shiftKey !== wantShift) return false
  if (key.length === 1 && e.shiftKey !== wantShift) return false
  return true
}

export function matchesAction(e: KeyboardEvent, id: string): boolean {
  return matchesBinding(e, getBinding(id))
}

export function matchesFixedAction(e: KeyboardEvent, id: string): boolean {
  const binding = FIXED_KEY_ACTIONS.find((action) => action.id === id)?.binding ?? ''
  return matchesBinding(e, binding)
}

/** Cross-platform label for bindings whose Ctrl token also matches Command. */
export function displayBinding(binding: string): string {
  return binding.replace(/^Ctrl(?=\+|$)/, 'Ctrl/Cmd')
}

/** Other actions currently bound to `binding` (conflict check for the editor). */
export function conflictsFor(binding: string, exceptId: string): Array<KeyAction | FixedKeyAction> {
  return [
    ...KEY_ACTIONS.filter((action) => action.id !== exceptId && getBinding(action.id) === binding),
    ...FIXED_KEY_ACTIONS.filter((action) => action.id !== exceptId && action.binding === binding),
  ]
}
