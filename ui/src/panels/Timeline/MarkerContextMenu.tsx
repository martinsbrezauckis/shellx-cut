// MarkerContextMenu.tsx — right-click menu for a ruler marker.
// Role: the discoverable human surface for the marker verbs — seek
// (edit.seek_marker's UI analog), rename + color + note (edit.update_marker,
// core editing), delete (edit.remove_marker). The parent owns all verb
// dispatch; this component owns the visible menu, the inline rename input,
// the color swatch row, and stable `data-cut-*` selectors.
// Callers: panels/Timeline/index.tsx. Deps: icons + clientModel swatch map.

import { useState } from 'react'
import { Icon } from '../../icons'
import { MARKER_COLOR_SWATCH, type MarkerColor } from '../../lib/clientModel'

export interface MarkerMenuState {
  x: number
  y: number
  id: string
  atMs: number
  label: string
  note?: string
  color?: MarkerColor
}

interface MarkerContextMenuProps {
  menu: MarkerMenuState
  onSeek: (atMs: number) => void
  onRename: (id: string, label: string) => void
  onNote: (id: string, note: string) => void
  onColor: (id: string, color: MarkerColor | 'none') => void
  onDelete: (id: string, label: string) => void
  onClose: () => void
}

function clampMenu(el: HTMLDivElement, x: number, y: number): void {
  const margin = 8
  const rect = el.getBoundingClientRect()
  el.style.left = `${Math.max(margin, Math.min(x, window.innerWidth - rect.width - margin))}px`
  el.style.top = `${Math.max(margin, Math.min(y, window.innerHeight - rect.height - margin))}px`
}

/** Right-click ruler marker menu. The parent owns marker mutations; this
 * component owns the visible menu and stable debug selectors. */
export default function MarkerContextMenu({
  menu,
  onSeek,
  onRename,
  onNote,
  onColor,
  onDelete,
  onClose,
}: MarkerContextMenuProps) {
  const [draft, setDraft] = useState(menu.label)
  const [noteDraft, setNoteDraft] = useState(menu.note ?? '')
  const commitRename = () => {
    const label = draft.trim()
    if (label && label !== menu.label) onRename(menu.id, label)
    onClose()
  }
  const commitNote = () => {
    const note = noteDraft.trim()
    if (note !== (menu.note ?? '')) onNote(menu.id, note)
    onClose()
  }
  return (
    <>
      <div
        className="tl-ctx-backdrop"
        data-cut-marker-ctx-backdrop
        onMouseDown={onClose}
        onContextMenu={(e) => {
          e.preventDefault()
          onClose()
        }}
      />
      <div
        className="tl-ctx"
        role="menu"
        data-cut-marker-menu
        style={{ left: menu.x, top: menu.y }}
        ref={(el) => { if (el) clampMenu(el, menu.x, menu.y) }}
      >
        {/* Rename — inline input, Enter commits, Escape closes without change. */}
        <div className="tl-ctx__rename" data-cut-marker-ctx="rename">
          <input
            className="tl-ctx__rename-input"
            data-cut-marker-rename-input
            value={draft}
            autoFocus
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') commitRename()
              if (e.key === 'Escape') onClose()
              e.stopPropagation()
            }}
            onMouseDown={(e) => e.stopPropagation()}
            aria-label="marker name"
          />
          <button
            className="tl-ctx__rename-ok"
            data-cut-marker-ctx="rename-commit"
            title="Rename marker (Enter)"
            onClick={commitRename}
          >
            <Icon name="return" size={14} />
          </button>
        </div>
        <span className="tl-ctx__label" aria-hidden="true">Note</span>
        <div className="tl-ctx__note" data-cut-marker-ctx="note">
          <textarea
            className="tl-ctx__note-input"
            data-cut-marker-note-input
            value={noteDraft}
            rows={3}
            placeholder="Add a marker note"
            onChange={(e) => setNoteDraft(e.target.value)}
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') commitNote()
              if (e.key === 'Escape') onClose()
              e.stopPropagation()
            }}
            onMouseDown={(e) => e.stopPropagation()}
            aria-label="marker note"
          />
          <button
            className="tl-ctx__note-ok"
            data-cut-marker-ctx="note-commit"
            title="Save marker note"
            onClick={commitNote}
          >
            <Icon name="return" size={14} />
          </button>
        </div>
        {/* Color swatches — one click sets the color; the hollow one clears. */}
        <div className="tl-ctx__swatches" data-cut-marker-ctx="colors" role="group" aria-label="marker color">
          {(Object.keys(MARKER_COLOR_SWATCH) as MarkerColor[]).map((c) => (
            <button
              key={c}
              className={`tl-ctx__swatch${menu.color === c ? ' tl-ctx__swatch--active' : ''}`}
              style={{ background: MARKER_COLOR_SWATCH[c] }}
              title={c}
              data-cut-marker-color-swatch={c}
              onClick={() => {
                onColor(menu.id, c)
                onClose()
              }}
            />
          ))}
          <button
            className={`tl-ctx__swatch tl-ctx__swatch--none${!menu.color ? ' tl-ctx__swatch--active' : ''}`}
            title="default (no color)"
            data-cut-marker-color-swatch="none"
            onClick={() => {
              onColor(menu.id, 'none')
              onClose()
            }}
          />
        </div>
        <button
          className="tl-ctx__item"
          data-cut-marker-ctx="seek"
          role="menuitem"
          onClick={() => {
            onSeek(menu.atMs)
            onClose()
          }}
        >
          <Icon name="marker" size={14} /> Seek to marker
        </button>
        <button
          className="tl-ctx__item tl-ctx__item--danger"
          data-cut-marker-ctx="delete"
          role="menuitem"
          onClick={() => {
            onDelete(menu.id, menu.label)
            onClose()
          }}
        >
          <Icon name="trash" size={14} /> Delete marker
        </button>
      </div>
    </>
  )
}
