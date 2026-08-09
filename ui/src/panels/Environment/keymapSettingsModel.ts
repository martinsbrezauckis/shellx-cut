import {
  FIXED_KEY_ACTIONS,
  KEY_ACTIONS,
  displayBinding,
  type FixedKeyAction,
  type KeyAction,
} from '../../lib/keymap'

export type ShortcutGroup = 'all' | KeyAction['group'] | FixedKeyAction['group']
export type ShortcutFilter = 'all' | 'changed' | 'conflicts'

export interface ShortcutSettingsRow {
  id: string
  label: string
  group: Exclude<ShortcutGroup, 'all'>
  binding: string
  displayBinding: string
  editable: boolean
  changed: boolean
  conflict: boolean
}

export interface ShortcutSettingsCounts {
  commands: number
  changed: number
  conflicts: number
}

export const SHORTCUT_GROUPS: ReadonlyArray<{ value: ShortcutGroup; label: string }> = [
  { value: 'all', label: 'All areas' },
  { value: 'preview', label: 'Playback' },
  { value: 'timeline', label: 'Timeline' },
  { value: 'global', label: 'App' },
  { value: 'recording', label: 'Recording' },
]

export function shortcutGroupLabel(group: Exclude<ShortcutGroup, 'all'>): string {
  return SHORTCUT_GROUPS.find((entry) => entry.value === group)?.label ?? group
}

export function buildShortcutSettingsRows(
  bindingFor: (id: string) => string,
  changedFor: (id: string) => boolean,
): ShortcutSettingsRow[] {
  const base = [
    ...KEY_ACTIONS.map((action) => ({
      id: action.id,
      label: action.label,
      group: action.group,
      binding: bindingFor(action.id),
      editable: true,
      changed: changedFor(action.id),
    })),
    ...FIXED_KEY_ACTIONS.map((action) => ({
      id: action.id,
      label: action.label,
      group: action.group,
      binding: action.binding,
      editable: false,
      changed: false,
    })),
  ]

  const owners = new Map<string, number>()
  for (const row of base) owners.set(row.binding, (owners.get(row.binding) ?? 0) + 1)

  return base.map((row) => ({
    ...row,
    displayBinding: displayBinding(row.binding),
    conflict: (owners.get(row.binding) ?? 0) > 1,
  }))
}

export function shortcutSettingsCounts(rows: readonly ShortcutSettingsRow[]): ShortcutSettingsCounts {
  return {
    commands: rows.length,
    changed: rows.filter((row) => row.changed).length,
    conflicts: rows.filter((row) => row.conflict).length,
  }
}

export function filterShortcutSettingsRows(
  rows: readonly ShortcutSettingsRow[],
  query: string,
  group: ShortcutGroup,
  filter: ShortcutFilter,
): ShortcutSettingsRow[] {
  const needle = query.trim().toLocaleLowerCase()
  return rows.filter((row) => {
    if (group !== 'all' && row.group !== group) return false
    if (filter === 'changed' && !row.changed) return false
    if (filter === 'conflicts' && !row.conflict) return false
    if (!needle) return true
    return [
      row.label,
      row.id,
      shortcutGroupLabel(row.group),
      row.displayBinding,
      row.editable ? 'customisable editable' : 'fixed native',
    ].some((value) => value.toLocaleLowerCase().includes(needle))
  })
}
