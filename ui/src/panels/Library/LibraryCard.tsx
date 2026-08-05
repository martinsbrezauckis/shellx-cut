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

export interface LibraryCardProps {
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

export function LibraryCard({
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
}: LibraryCardProps) {
  return (
    <div
      className={`lb-card ${selected ? 'is-selected' : ''}`}
      data-cut-library-card={item.id}
      data-cut-library-type={item.type}
      data-cut-library-keyboard-item
      tabIndex={keyboardTabIndex}
      role="group"
      aria-label={`${item.name} Library media`}
      onFocus={() => onKeyboardFocus(item.id)}
      onKeyDown={(event) => onKeyboardKeyDown(item.id, event)}
      onContextMenu={(event) => { event.preventDefault(); onOpenMenu(event.clientX, event.clientY, item.id) }}
      onDragStart={(event) => event.preventDefault()}
      title={item.media_ok === false ? undefined : 'Use Insert at playhead or Add to project'}
    >
      <div className={`lb-thumb lb-thumb--${item.type}`}>
        <LibraryPoster item={item} failed={failedPoster} onFail={onPosterFail} />
        <button
          className={`lb-select ${selected ? 'lb-select--on' : ''}`}
          data-cut-library-select={item.id}
          title={selected ? 'Deselect' : 'Select (shift-click for a range)'}
          aria-label={selected ? 'Deselect' : 'Select'}
          aria-pressed={selected}
          onClick={(event) => onToggleSelect(item.id, event.shiftKey)}
        >
          {selected ? <Icon name="check" size={14} label="Selected" /> : <span className="lb-select-empty" aria-hidden="true" />}
        </button>
        <button
          className={`lb-fav ${item.favorite ? 'lb-fav--on' : ''}`}
          data-cut-library-fav={item.id}
          title={item.favorite ? 'Unpin' : 'Pin to top'}
          onClick={() => { void onToggleFavorite(item) }}
        >
          <Icon name="favorite" size={16} label={item.favorite ? 'Unpin' : 'Pin to top'} />
        </button>
        {item.type !== 'image' ? <span className="lb-thumb-kind">{shortDur(item.probe?.duration_ms) || item.type}</span> : null}
      </div>

      <div className="lb-card-body">
        <div className="lb-name" data-cut-library-name title={item.src_path ?? item.name}>{item.name}</div>
        {item.media_ok === false && (
          <span className="lb-missing-badge" data-cut-library-missing>
            {item.blob ? 'Managed copy missing' : 'Source missing'}
          </span>
        )}
        {inProject && <span className="lb-project-badge" data-cut-library-in-project>In this project</span>}
        <div className="lb-meta">
          <span className="lb-kind">{item.type}</span>
          {item.probe?.duration_ms ? <span> · {shortDur(item.probe.duration_ms)}</span> : null}
          {item.probe?.width && item.probe?.height ? <span> · {item.probe.width}×{item.probe.height}</span> : null}
          {item.blob ? <span className="lb-portable" title="Stored copy in Library"> · <Icon name="copy" size={14} tone="brand" label="Stored copy" /></span> : null}
        </div>
        <LibraryTags
          item={item}
          editing={editingTags}
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
        />
      </div>
    </div>
  )
}
