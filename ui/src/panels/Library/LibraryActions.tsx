import { Icon } from '../../icons'
import type { LibItem } from '../../lib/client'

type ItemAction = (item: LibItem) => void | Promise<void>
type MoveAction = (item: LibItem, folder: string) => void | Promise<void>

export interface LibraryActionsProps {
  item: LibItem
  hasProject: boolean
  busy: string | null
  folders: string[]
  onAddToProject: ItemAction
  onInsertAtPlayhead: ItemAction
  onMoveTo: MoveAction
  onEditTags: ItemAction
  onRelink: ItemAction
  onMakePortable: ItemAction
  onRemove: ItemAction
}

export function LibraryActions({
  item,
  hasProject,
  busy,
  folders,
  onAddToProject,
  onInsertAtPlayhead,
  onMoveTo,
  onEditTags,
  onRelink,
  onMakePortable,
  onRemove,
}: LibraryActionsProps) {
  return (
    <div className="lb-actions">
      <button
        className="lb-act lb-act--primary"
        data-cut-library-toproject={item.id}
        disabled={!hasProject || busy === item.id || item.media_ok === false}
        title={item.media_ok === false ? 'Relink the missing source first' : hasProject ? 'Import into the open project' : 'Open a project first'}
        onClick={() => { void onAddToProject(item) }}
      >
        <span className="lb-act-label">{busy === item.id ? 'Adding…' : 'Add to project'}</span>
      </button>
      <button
        className="lb-act lb-act--primary"
        data-cut-library-insert={item.id}
        disabled={!hasProject || busy === item.id || item.media_ok === false}
        title={hasProject ? 'Add this media and insert it at the current playhead' : 'Open a project first'}
        onClick={() => { void onInsertAtPlayhead(item) }}
      >
        <span className="lb-act-label">{busy === item.id ? 'Working…' : 'Insert at playhead'}</span>
      </button>
      <select
        className="lb-move"
        data-cut-library-move={item.id}
        value={item.folder ?? ''}
        title="Move to folder"
        onChange={(e) => { void onMoveTo(item, e.target.value) }}
      >
        <option value="">(no folder)</option>
        {folders.map((folder) => (
          <option key={folder} value={folder}>{folder}</option>
        ))}
      </select>
      <button
        className="lb-act"
        data-cut-library-tagbtn={item.id}
        title="Edit tags"
        onClick={() => { void onEditTags(item) }}
      >
        <Icon name="bookmark" size={14} /> <span className="lb-act-label">Tag</span>
      </button>
      {item.media_ok === false && item.src_path && !item.blob && (
        <button
          className="lb-act lb-act--primary"
          data-cut-library-relink={item.id}
          title="Choose the same media file at its new location"
          disabled={busy === item.id}
          onClick={() => { void onRelink(item) }}
        >
          <Icon name="link" size={14} /> <span className="lb-act-label">{busy === item.id ? 'Checking…' : 'Relink…'}</span>
        </button>
      )}
      {item.media_ok !== false && item.src_path && !item.blob && (
        <button
          className="lb-act lb-act--portable"
          data-cut-library-portable={item.id}
          title="Store a copy in the Library so the item still works if the original moves"
          disabled={busy === item.id}
          onClick={() => { void onMakePortable(item) }}
        >
          <Icon name="copy" size={14} /> <span className="lb-act-label lb-act-label--optional">Keep a copy</span>
        </button>
      )}
      <button
        className="lb-act lb-act--danger"
        data-cut-library-remove={item.id}
        title="Remove from library (does not delete the source file)"
        onClick={() => { void onRemove(item) }}
      >
        <Icon name="trash" size={14} />
      </button>
    </div>
  )
}
