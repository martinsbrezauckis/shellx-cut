import type { KeyboardEvent } from 'react'
import { Icon } from '../../icons'
import type { LibItem } from '../../lib/client'
import { LibraryActions } from './LibraryActions'
import { LibraryPoster } from './LibraryPoster'
import { LibraryTags } from './LibraryTags'
import { shortDur } from './model'

type ItemAction = (item: LibItem) => void | Promise<void>
type SaveTagsAction = (item: LibItem, draft: string) => void | Promise<void>
type MoveAction = (item: LibItem, folder: string) => void | Promise<void>

export interface LibraryRowProps {
  item: LibItem
  inProject: boolean
  selected: boolean
  failedPoster: boolean
  hasProject: boolean
  busy: string | null
  folders: string[]
  tagDraft: string
  activeTagFilter: string | null
  editingTags: boolean
  keyboardTabIndex: number
  onOpenMenu: (x: number, y: number, id: string) => void
  onKeyboardFocus: (id: string) => void
  onKeyboardKeyDown: (id: string, event: KeyboardEvent<HTMLElement>) => void
  onPosterFail: (id: string) => void
  onToggleSelect: (id: string, shift: boolean) => void
  onToggleFavorite: ItemAction
  onAddToProject: ItemAction
  onInsertAtPlayhead: ItemAction
  onMoveTo: MoveAction
  onEditTags: ItemAction
  onRelink: ItemAction
  onMakePortable: ItemAction
  onRemove: ItemAction
  onTagDraftChange: (value: string) => void
  onSaveTags: SaveTagsAction
  onCancelTagEditor: () => void
  onToggleTagFilter: (tag: string) => void
}

export function LibraryRow({
  item,
  inProject,
  selected,
  failedPoster,
  hasProject,
  busy,
  folders,
  tagDraft,
  activeTagFilter,
  editingTags,
  keyboardTabIndex,
  onOpenMenu,
  onKeyboardFocus,
  onKeyboardKeyDown,
  onPosterFail,
  onToggleSelect,
  onToggleFavorite,
  onAddToProject,
  onInsertAtPlayhead,
  onMoveTo,
  onEditTags,
  onRelink,
  onMakePortable,
  onRemove,
  onTagDraftChange,
  onSaveTags,
  onCancelTagEditor,
  onToggleTagFilter,
}: LibraryRowProps) {
  return (
    <div
      className={`lb-row ${selected ? 'is-selected' : ''}`}
      data-cut-library-card={item.id}
      data-cut-library-type={item.type}
      data-cut-library-keyboard-item
      tabIndex={keyboardTabIndex}
      role="group"
      aria-label={`${item.name} Library media`}
      aria-keyshortcuts="Shift+F10 ContextMenu"
      onFocus={() => onKeyboardFocus(item.id)}
      onKeyDown={(event) => {
        if (event.target === event.currentTarget && (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10'))) {
          event.preventDefault()
          const rect = event.currentTarget.getBoundingClientRect()
          onOpenMenu(rect.left + Math.min(24, rect.width / 2), rect.top + Math.min(24, rect.height / 2), item.id)
          return
        }
        onKeyboardKeyDown(item.id, event)
      }}
      onContextMenu={(event) => { event.preventDefault(); onOpenMenu(event.clientX, event.clientY, item.id) }}
      onDragStart={(event) => event.preventDefault()}
      title={item.media_ok === false ? undefined : 'Use Insert at playhead or Add to project'}
    >
      <button
        className={`lb-select lb-select--row ${selected ? 'lb-select--on' : ''}`}
        data-cut-library-select={item.id}
        title={selected ? 'Deselect' : 'Select (shift-click for a range)'}
        aria-label={selected ? 'Deselect' : 'Select'}
        aria-pressed={selected}
        onClick={(event) => onToggleSelect(item.id, event.shiftKey)}
      >
        {selected ? <Icon name="check" size={14} label="Selected" /> : <span className="lb-select-empty" aria-hidden="true" />}
      </button>
      <div className={`lb-row-thumb lb-thumb--${item.type}`}>
        <LibraryPoster item={item} failed={failedPoster} onFail={onPosterFail} />
      </div>
      <button
        className={`lb-fav lb-fav--row ${item.favorite ? 'lb-fav--on' : ''}`}
        data-cut-library-fav={item.id}
        title={item.favorite ? 'Unpin' : 'Pin to top'}
        onClick={() => { void onToggleFavorite(item) }}
      >
        <Icon name="favorite" size={14} label={item.favorite ? 'Unpin' : 'Pin'} />
      </button>
      <div className="lb-row-name" data-cut-library-name title={item.src_path ?? item.name}>
        <span>{item.name}</span>
        {item.media_ok === false && (
          <span className="lb-missing-badge" data-cut-library-missing>
            {item.blob ? 'Managed copy missing' : 'Source missing'}
          </span>
        )}
        {inProject && <span className="lb-project-badge lb-project-badge--row" data-cut-library-in-project>In this project</span>}
      </div>
      <div className="lb-row-meta">
        <span>{item.type}</span>
        {shortDur(item.probe?.duration_ms) ? <span> · {shortDur(item.probe?.duration_ms)}</span> : null}
        <span> · {item.folder ?? 'All'}</span>
      </div>
      <div className="lb-row-actions">
        {editingTags ? (
          <LibraryTags
            item={item}
            editing
            draft={tagDraft}
            activeTagFilter={activeTagFilter}
            className="lb-tags"
            hideWhenEmpty
            exposeTagsHook
            onDraftChange={onTagDraftChange}
            onSave={onSaveTags}
            onCancelEdit={onCancelTagEditor}
            onToggleTagFilter={onToggleTagFilter}
          />
        ) : (
          <LibraryActions
            item={item}
            hasProject={hasProject}
            busy={busy}
            folders={folders}
            onAddToProject={onAddToProject}
            onInsertAtPlayhead={onInsertAtPlayhead}
            onMoveTo={onMoveTo}
            onEditTags={onEditTags}
            onRelink={onRelink}
            onMakePortable={onMakePortable}
            onRemove={onRemove}
            onOpenMenu={onOpenMenu}
          />
        )}
      </div>
      <LibraryTags
        item={item}
        editing={false}
        draft={tagDraft}
        activeTagFilter={activeTagFilter}
        className="lb-row-tags"
        onDraftChange={onTagDraftChange}
        onSave={onSaveTags}
        onCancelEdit={onCancelTagEditor}
        onToggleTagFilter={onToggleTagFilter}
      />
    </div>
  )
}
