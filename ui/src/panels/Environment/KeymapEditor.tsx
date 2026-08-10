// Settings shortcut manager. The compact summary stays useful while collapsed;
// the expanded surface adds bounded search/filtering, safe capture, conflicts,
// and reset controls without mixing fixed native commands with editable state.
import { useEffect, useRef, useState, type MouseEvent } from 'react'
import {
  KEY_ACTIONS,
  bindingFromEvent,
  conflictsFor,
  displayBinding,
  getBinding,
  isRemapped,
  resetKeymap,
  setBinding,
} from '../../lib/keymap'
import {
  SHORTCUT_GROUPS,
  buildShortcutSettingsRows,
  filterShortcutSettingsRows,
  shortcutGroupLabel,
  shortcutSettingsCounts,
  type ShortcutFilter,
  type ShortcutGroup,
} from './keymapSettingsModel'
import KeymapProfileTransfer from './KeymapProfileTransfer'
import './keymap-editor.css'

interface KeymapNotice {
  tone: 'info' | 'warning'
  text: string
}

const FILTERS: ReadonlyArray<{ value: ShortcutFilter; label: string }> = [
  { value: 'all', label: 'All' },
  { value: 'changed', label: 'Changed' },
  { value: 'conflicts', label: 'Conflicts' },
]

// Settings category bodies intentionally unmount while another destination is
// active. Preserve view state in memory so returning during the same app
// session resumes the user's place without turning filters into durable prefs.
const shortcutViewMemory: {
  expanded: boolean
  query: string
  group: ShortcutGroup
  filter: ShortcutFilter
} = {
  expanded: false,
  query: '',
  group: 'all',
  filter: 'all',
}

export default function KeymapEditor() {
  const detailsRef = useRef<HTMLDetailsElement>(null)
  const [capturing, setCapturing] = useState<string | null>(null)
  const [notice, setNotice] = useState<KeymapNotice | null>(null)
  const [expanded, setExpanded] = useState(shortcutViewMemory.expanded)
  const [query, setQuery] = useState(shortcutViewMemory.query)
  const [group, setGroup] = useState<ShortcutGroup>(shortcutViewMemory.group)
  const [filter, setFilter] = useState<ShortcutFilter>(shortcutViewMemory.filter)
  const [, setRevision] = useState(0)

  useEffect(() => {
    const refresh = () => setRevision((revision) => revision + 1)
    document.addEventListener('cut:keymap-changed', refresh)
    return () => document.removeEventListener('cut:keymap-changed', refresh)
  }, [])

  useEffect(() => {
    Object.assign(shortcutViewMemory, { expanded, query, group, filter })
  }, [expanded, filter, group, query])

  useEffect(() => {
    if (!capturing) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Tab') {
        setCapturing(null)
        setNotice(null)
        return
      }
      event.preventDefault()
      event.stopPropagation()
      if (event.key === 'Escape') {
        setCapturing(null)
        setNotice({ tone: 'info', text: 'Shortcut change cancelled.' })
        return
      }
      const binding = bindingFromEvent(event)
      if (!binding) return
      const conflicts = conflictsFor(binding, capturing)
      if (conflicts.length > 0) {
        setNotice({
          tone: 'warning',
          text: `${displayBinding(binding)} is already used by ${conflicts[0].label}. Choose another key or reset that command first.`,
        })
        return
      }
      const action = KEY_ACTIONS.find((candidate) => candidate.id === capturing)
      if (!setBinding(capturing, binding)) {
        setNotice({ tone: 'warning', text: 'That shortcut could not be saved. Choose another key.' })
        return
      }
      setNotice({
        tone: 'info',
        text: `${action?.label ?? 'Shortcut'} changed to ${displayBinding(binding)}.`,
      })
      setCapturing(null)
      if (filter === 'conflicts') setFilter('all')
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [capturing, filter])

  const rows = buildShortcutSettingsRows(getBinding, isRemapped)
  const counts = shortcutSettingsCounts(rows)
  const visibleRows = filterShortcutSettingsRows(rows, query, group, filter)

  const cancelCapture = () => {
    if (!capturing) return
    setCapturing(null)
    setNotice(null)
  }

  const resetOne = (event: MouseEvent<HTMLButtonElement>, id: string) => {
    const bindingButton = event.currentTarget.previousElementSibling
    setBinding(id, null)
    setNotice({ tone: 'info', text: 'Shortcut restored to its default.' })
    if (filter === 'changed' && counts.changed === 1) setFilter('all')
    requestAnimationFrame(() => {
      if (bindingButton instanceof HTMLElement && bindingButton.isConnected) bindingButton.focus()
    })
  }

  const resetAll = () => {
    resetKeymap()
    setCapturing(null)
    setFilter('all')
    setNotice({ tone: 'info', text: 'All custom shortcuts restored to their defaults.' })
    requestAnimationFrame(() => {
      detailsRef.current?.querySelector<HTMLInputElement>('[data-cut-keymap-search]')?.focus()
    })
  }

  return (
    <section className="env-keymap" data-cut-keymap-editor>
      <details
        ref={detailsRef}
        className="env-keymap-details"
        data-cut-keymap-details
        open={expanded}
        onToggle={(event) => setExpanded(event.currentTarget.open)}
      >
        <summary className="env-keymap-summary" data-cut-keymap-toggle>
          <span>
            <strong>Keyboard shortcuts</strong>
            <small>
              <span data-cut-keymap-command-count>{counts.commands} commands</span>
              {' · '}
              <span data-cut-keymap-changed-count>{counts.changed} changed</span>
              {' · '}
              <span data-cut-keymap-conflict-count>{counts.conflicts} conflicts</span>
            </small>
          </span>
          <span className="env-keymap-summary-action">
            <span className="env-keymap-summary-closed">Review & customise</span>
            <span className="env-keymap-summary-open">Hide shortcuts</span>
          </span>
        </summary>

        <div className="env-keymap-content">
          <div className="env-keymap-head">
            <p className="env-keymap-intro" id="cut-keymap-help">
              Select an editable key, then press its replacement. Fixed app and recording controls stay locked.
            </p>
            {counts.changed > 0 && (
              <button
                type="button"
                className="env-btn env-btn--ghost"
                data-cut-action="keymap-reset-all"
                onClick={resetAll}
              >
                Reset all
              </button>
            )}
          </div>

          <KeymapProfileTransfer
            onInfo={(text) => setNotice({ tone: 'info', text })}
            onWarning={(text) => setNotice({ tone: 'warning', text })}
            onBegin={cancelCapture}
          />

          <div className="env-keymap-toolbar">
            <label className="env-keymap-field">
              <span>Find a command</span>
              <input
                type="search"
                value={query}
                placeholder="Name or shortcut"
                data-cut-keymap-search
                onFocus={cancelCapture}
                onChange={(event) => setQuery(event.currentTarget.value)}
              />
            </label>
            <label className="env-keymap-field">
              <span>Area</span>
              <select
                value={group}
                data-cut-keymap-group
                onFocus={cancelCapture}
                onChange={(event) => setGroup(event.currentTarget.value as ShortcutGroup)}
              >
                {SHORTCUT_GROUPS.map((option) => (
                  <option key={option.value} value={option.value}>{option.label}</option>
                ))}
              </select>
            </label>
          </div>

          <div className="env-keymap-filters" aria-label="Shortcut status filters">
            {FILTERS.map((option) => {
              const count = option.value === 'changed' ? counts.changed : option.value === 'conflicts' ? counts.conflicts : counts.commands
              return (
                <button
                  key={option.value}
                  type="button"
                  aria-pressed={filter === option.value}
                  data-cut-keymap-filter={option.value}
                  disabled={option.value !== 'all' && count === 0}
                  onClick={() => {
                    cancelCapture()
                    setFilter(option.value)
                  }}
                >
                  {option.label} <span>{count}</span>
                </button>
              )
            })}
          </div>

          {notice && (
            <p
              className={`env-keymap-note env-keymap-note--${notice.tone}`}
              role={notice.tone === 'warning' ? 'alert' : 'status'}
              data-cut-keymap-note
            >
              {notice.text}
            </p>
          )}

          <p className="env-keymap-results" aria-live="polite" data-cut-keymap-results>
            Showing {visibleRows.length} of {counts.commands}
          </p>
          <div className="env-keymap-rows">
            {visibleRows.map((row) => (
              <div
                className={`env-keymap-row${row.conflict ? ' env-keymap-row--conflict' : ''}`}
                key={row.id}
                data-cut-keymap-row={row.editable ? row.id : undefined}
                data-cut-keymap-fixed-row={row.editable ? undefined : row.id}
              >
                <span className="env-keymap-group">{shortcutGroupLabel(row.group)}</span>
                <span className="env-keymap-label">
                  {row.label}
                  <span className="env-keymap-badges">
                    {!row.editable && <span>Fixed</span>}
                    {row.changed && <span>Changed</span>}
                    {row.conflict && <span className="env-keymap-badge--warning">Conflict</span>}
                  </span>
                </span>
                {row.editable ? (
                  <>
                    <button
                      type="button"
                      className={`env-keymap-key${capturing === row.id ? ' env-keymap-key--armed' : ''}${row.changed ? ' env-keymap-key--remapped' : ''}`}
                      data-cut-keymap-bind={row.id}
                      aria-describedby="cut-keymap-help"
                      aria-label={`${row.label}: ${capturing === row.id ? 'press a replacement shortcut' : row.displayBinding}`}
                      onClick={() => {
                        setNotice(null)
                        setCapturing(capturing === row.id ? null : row.id)
                      }}
                    >
                      {capturing === row.id ? 'Press a key…' : row.displayBinding}
                    </button>
                    {row.changed && capturing !== row.id && (
                      <button
                        type="button"
                        className="env-keymap-clear"
                        data-cut-keymap-clear={row.id}
                        aria-label={`Reset ${row.label} to ${KEY_ACTIONS.find((action) => action.id === row.id)?.def}`}
                        onClick={(event) => resetOne(event, row.id)}
                      >
                        Reset
                      </button>
                    )}
                  </>
                ) : (
                  <kbd className="env-keymap-key env-keymap-key--fixed" aria-label={`${row.displayBinding}, fixed`}>
                    {row.displayBinding}
                  </kbd>
                )}
              </div>
            ))}
          </div>
          {visibleRows.length === 0 && (
            <div className="env-keymap-empty" data-cut-keymap-empty>
              No shortcuts match these filters.
            </div>
          )}
        </div>
      </details>
    </section>
  )
}
