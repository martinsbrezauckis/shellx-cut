import { Icon } from '../../icons'

type Action = () => void | Promise<void>
type MoveAction = (folder: string) => void | Promise<void>

export interface LibraryBulkBarProps {
  selectedCount: number
  folders: string[]
  hasProject: boolean
  tagEditorOpen: boolean
  tagDraft: string
  onStartTagEdit: () => void
  onTagDraftChange: (value: string) => void
  onSaveTags: Action
  onCancelTagEdit: () => void
  onMove: MoveAction
  onAddToProject: Action
  onRemove: Action
  onClearSelection: () => void
}

export function LibraryBulkBar({
  selectedCount,
  folders,
  hasProject,
  tagEditorOpen,
  tagDraft,
  onStartTagEdit,
  onTagDraftChange,
  onSaveTags,
  onCancelTagEdit,
  onMove,
  onAddToProject,
  onRemove,
  onClearSelection,
}: LibraryBulkBarProps) {
  return (
    <div className="lb-bulkbar" data-cut-library-bulkbar>
      <span className="lb-bulk-count">{selectedCount} selected</span>
      {tagEditorOpen ? (
        <input
          className="lb-taginput lb-bulk-taginput"
          data-cut-library-bulk-taginput
          autoFocus
          value={tagDraft}
          placeholder="add tags, comma, separated"
          onChange={(event) => onTagDraftChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') void onSaveTags()
            if (event.key === 'Escape') onCancelTagEdit()
          }}
          onBlur={() => { void onSaveTags() }}
        />
      ) : (
        <button
          className="lb-act"
          data-cut-library-bulk-tag
          title="Add tags to the selected items"
          onClick={onStartTagEdit}
        >
          <Icon name="bookmark" size={14} /> Tag
        </button>
      )}
      <select
        className="lb-move lb-bulk-move"
        data-cut-library-bulk-move
        value="__none"
        title="Move selected to a folder"
        onChange={(event) => {
          if (event.target.value !== '__none') void onMove(event.target.value === '__root' ? '' : event.target.value)
        }}
      >
        <option value="__none" disabled>Move to{'\u2026'}</option>
        <option value="__root">(no folder)</option>
        {folders.map((folder) => <option key={folder} value={folder}>{folder}</option>)}
      </select>
      <button
        className="lb-act lb-act--primary"
        data-cut-library-bulk-toproject
        disabled={!hasProject}
        title={hasProject ? 'Import selected into the open project' : 'Open a project first'}
        onClick={() => { void onAddToProject() }}
      >
        Add to project
      </button>
      <button
        className="lb-act lb-act--danger"
        data-cut-library-bulk-remove
        title="Remove selected from the library"
        onClick={() => { void onRemove() }}
      >
        <Icon name="trash" size={14} /> Remove
      </button>
      <span className="lb-bulk-spacer" />
      <button className="lb-act" data-cut-library-bulk-clear title="Clear selection" onClick={onClearSelection}>Clear</button>
    </div>
  )
}
