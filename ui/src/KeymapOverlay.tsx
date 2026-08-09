// KeymapOverlay.tsx — the `?` keyboard-map modal:
// "`?` | any | keyboard map overlay (modal, --z-modal)").
// Role: self-contained — listens for `?` globally (ignored while typing in
// form fields), renders the table grouped by scope, closes on Esc / scrim
// click / `?` again. Pure view: no verbs, no state mutation.
// Callers: App.tsx (mounted once). Deps: react + keymap.css only.

import { useEffect, useState } from 'react'
import { FIXED_KEY_ACTIONS, KEY_ACTIONS, displayBinding, getBinding, isRemapped } from './lib/keymap'
import { useBlockingOverlay } from './components/overlay/useBlockingOverlay'
import './keymap.css'

/** REMAPPABLE actions: rows DERIVED live from lib/keymap.ts — the single
 *  source of truth, so a remap in Settings shows here instantly and the old
 *  hand-mirrored-list drift hazard is gone for these. */
function liveRows(): Array<[scope: string, keys: string[], action: string]> {
  return KEY_ACTIONS.map((a) => [
    a.group,
    [getBinding(a.id) + (isRemapped(a.id) ? ' (remapped)' : '')],
    `${a.label} — remappable in Settings › Keyboard`,
  ])
}

/** The FIXED map — scope · keys · action (conventions, not preferences).
 *  Remappable actions live in liveRows() above; keep them out of here. */
const ROWS: Array<[scope: string, keys: string[], action: string]> = [
  ['global', ['←', '→'], 'nudge playhead −/+1 frame'],
  ['global', ['Shift+←', 'Shift+→'], 'nudge −/+10 frames'],
  ['global', ['Home', 'End'], 'playhead to start / end'],
  ['global', ['+', '−', 'Ctrl/Cmd+=', 'Ctrl/Cmd+−'], 'timeline zoom in / out (anchor playhead)'],
  ['global', ['Shift+Z'], 'fit the whole timeline to the window'],
  ['global', ['Ctrl/Cmd+B'], 'edit.split at playhead (fixed twin of the remappable split key)'],
  ['global', ['Del'], 'ripple-delete selected clip (close the gap)'],
  ['global', ['Alt+Del', 'Shift+Del'], 'lift-delete selected clip (leave the gap)'],
  ['global', ['Ctrl/Cmd+click'], 'add / remove a clip from the selection'],
  ['global', ['drag empty lane', 'Shift+drag'], 'marquee select the clips inside the rectangle / add them to the selection'],
  ['global', ['Ctrl/Cmd+A'], 'select all clips (then delete / speed / grade as one)'],
  ['global', ['Ctrl/Cmd+T'], 'default transition (500ms dissolve) at the selected / nearest cut'],
  ['global', ['Ctrl/Cmd+Alt+V'], 'paste ATTRIBUTES of the copied clip onto the selection (dialog)'],
  ['global', ['Alt+←', 'Alt+→'], 'SLIP the selected clip ±1 frame (Shift+Alt = ±10) — content shifts, position stays'],
  ['global', ['Ctrl/Cmd+Z'], 'undo — step back one edit (project.undo)'],
  ['global', ['Ctrl/Cmd+Shift+Z', 'Ctrl/Cmd+Y'], 'redo — step forward one edit (project.redo)'],
  ['global', ['Ctrl/Cmd+S'], 'project.save'],
  ['global', ['R'], 'focus review rail'],
  ['global', ['\\'], 'collapse / expand review rail'],
  ['rail', ['j', 'k'], 'op cursor down / up'],
  ['rail', ['a', 'x'], 'accept op / reject (edit.restore)'],
  ['rail', ['Enter'], 'seek playhead to op location'],
  ['any', ['Esc'], 'clear selection / leave rail / close overlay'],
  ['any', ['?'], 'this keyboard map'],
]

function fixedRows(): Array<[scope: string, keys: string[], action: string]> {
  return FIXED_KEY_ACTIONS.map((action) => [
    action.group,
    [displayBinding(action.binding)],
    `${action.label} — fixed app${action.group === 'recording' ? ' / OS' : ''} shortcut`,
  ])
}

export default function KeymapOverlay() {
  const [open, setOpen] = useState(false)
  const close = () => setOpen(false)
  const overlay = useBlockingOverlay<HTMLDivElement>(close, open)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'SELECT' || t.tagName === 'TEXTAREA')) return
      if (e.key === '?' && !e.ctrlKey && !e.metaKey && !e.altKey) {
        e.preventDefault()
        setOpen((o) => !o)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  if (!open) return null
  return (
    <div className="km__scrim" onMouseDown={overlay.onScrimMouseDown}>
      <div
        ref={overlay.dialogRef}
        className="km"
        role="dialog"
        aria-modal="true"
        aria-label="keyboard map"
        data-cut-keymap=""
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
      >
        <div className="km__title">
          Keyboard map
          <span className="km__hint">every shortcut dispatches a public verb</span>
        </div>
        <div className="km__rows">
          {[...liveRows(), ...fixedRows(), ...ROWS].map(([scope, keys, action], i) => (
            <div className="km__row" key={i}>
              <span className={`km__scope km__scope--${scope}`}>{scope}</span>
              <span className="km__keys">
                {keys.map((k) => (
                  <kbd key={k}>{k}</kbd>
                ))}
              </span>
              <span className="km__action">{action}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  )
}
