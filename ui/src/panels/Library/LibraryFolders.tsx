import { Icon } from '../../icons'

export interface LibraryFoldersProps {
  folders: string[]
  activeFolder: string | null
  renaming: string | null
  renameDraft: string
  newFolder: string
  onSelectFolder: (folder: string | null) => void
  onOpenMenu: (x: number, y: number, name: string) => void
  onStartRename: (folder: string) => void
  onRenameDraftChange: (value: string) => void
  onCommitRename: (folder: string) => void | Promise<void>
  onCancelRename: () => void
  onNewFolderChange: (value: string) => void
  onAddFolder: () => void | Promise<void>
  /** Visible ✕ on each chip — folder remove was context-menu-only and
   *  undiscoverable. Non-destructive: items move to All. */
  onRemoveFolder: (folder: string) => void
}

export function LibraryFolders({
  folders,
  activeFolder,
  renaming,
  renameDraft,
  newFolder,
  onSelectFolder,
  onOpenMenu,
  onStartRename,
  onRenameDraftChange,
  onCommitRename,
  onCancelRename,
  onNewFolderChange,
  onAddFolder,
  onRemoveFolder,
}: LibraryFoldersProps) {
  return (
    <div className="lb-folders" data-cut-library-folders>
      {folders.map((folder) =>
        renaming === folder ? (
          <input
            key={folder}
            className="lb-folder-rename"
            data-cut-library-folder-rename={folder}
            autoFocus
            value={renameDraft}
            onChange={(event) => onRenameDraftChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter') void onCommitRename(folder)
              else if (event.key === 'Escape') onCancelRename()
            }}
            onBlur={onCancelRename}
          />
        ) : (
          <span className={`lb-chip-wrap ${activeFolder === folder ? 'lb-chip-wrap--on' : ''}`} key={folder}>
            <button
              className={`lb-chip ${activeFolder === folder ? 'lb-chip--on' : ''}`}
              data-cut-library-folder={folder}
              onClick={() => onSelectFolder(folder)}
              onContextMenu={(event) => { event.preventDefault(); onOpenMenu(event.clientX, event.clientY, folder) }}
              title={folder}
            >
              {folder}
            </button>
            <button
              className="lb-chip-edit"
              data-cut-library-folder-rename-btn={folder}
              title={`Rename "${folder}"`}
              onClick={() => onStartRename(folder)}
            >
              <Icon name="edit" size={14} label="Rename folder" />
            </button>
            <button
              className="lb-chip-edit lb-chip-remove"
              data-cut-library-folder-remove-btn={folder}
              title={`Remove folder "${folder}" — its items move back to All (nothing is deleted)`}
              onClick={() => onRemoveFolder(folder)}
            >
              <Icon name="close" size={14} label="Remove folder" />
            </button>
          </span>
        ),
      )}
      <input
        className="lb-folder-new"
        data-cut-library-newfolder
        placeholder="+ folder"
        value={newFolder}
        onChange={(event) => onNewFolderChange(event.target.value)}
        onKeyDown={(event) => { if (event.key === 'Enter') void onAddFolder() }}
      />
    </div>
  )
}
