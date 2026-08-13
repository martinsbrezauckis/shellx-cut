import { useState } from 'react'
import type { LibItem } from '../../lib/client'
import ContextMenuFrame from '../../components/ContextMenuFrame'
import { Icon } from '../../icons'

export type LibraryFolderMenuState = { x: number; y: number; name: string }
export type LibraryCardMenuState = { x: number; y: number; id: string }

export interface LibraryContextMenusProps {
  folderMenu: LibraryFolderMenuState | null
  cardMenu: LibraryCardMenuState | null
  cardMenuItem: LibItem | null
  hasProject: boolean
  busy: string | null
  folders: string[]
  onCloseFolderMenu: () => void
  onCloseCardMenu: () => void
  onStartRename: (name: string) => void
  onRemoveFolder: (name: string) => void
  onAddToProject: (item: LibItem) => void
  onInsertAtPlayhead: (item: LibItem) => void
  onMoveTo: (item: LibItem, folder: string) => void
  onToggleFavorite: (item: LibItem) => void
  onEditTags: (item: LibItem) => void
  onRelink: (item: LibItem) => void
  onMakePortable: (item: LibItem) => void
  onRemove: (item: LibItem) => void
}

export function LibraryContextMenus({
  folderMenu,
  cardMenu,
  cardMenuItem,
  hasProject,
  busy,
  folders,
  onCloseFolderMenu,
  onCloseCardMenu,
  onStartRename,
  onRemoveFolder,
  onAddToProject,
  onInsertAtPlayhead,
  onMoveTo,
  onToggleFavorite,
  onEditTags,
  onRelink,
  onMakePortable,
  onRemove,
}: LibraryContextMenusProps) {
  const [moveOpen, setMoveOpen] = useState(false)
  const canUseCard = !!cardMenuItem && hasProject && busy !== cardMenuItem.id && cardMenuItem.media_ok !== false
  const cardUseReason = !hasProject ? 'Open a project first'
    : cardMenuItem?.media_ok === false ? 'Relink the missing source first'
      : busy === cardMenuItem?.id ? 'This Library item is already being updated'
        : 'Use this Library item in the open project'
  return (
    <>
      {folderMenu && (
        <ContextMenuFrame
          x={folderMenu.x}
          y={folderMenu.y}
          menuId="data-cut-library-folder-menu"
          backdropId="data-cut-library-folder-ctx-backdrop"
          className="lb-ctx"
          backdropClassName="lb-ctx-backdrop"
          ariaLabel={`Folder ${folderMenu.name} menu`}
          onClose={onCloseFolderMenu}
        >
            <button className="lb-ctx__item" data-cut-library-folder-ctx="rename" role="menuitem" onClick={() => onStartRename(folderMenu.name)}>
              <Icon name="edit" size={14} /> Rename folder
            </button>
            <button
              className="lb-ctx__item lb-ctx__item--danger"
              data-cut-library-folder-ctx="delete"
              role="menuitem"
              onClick={() => {
                onRemoveFolder(folderMenu.name)
                onCloseFolderMenu()
              }}
            >
              <Icon name="trash" size={14} /> Delete folder
            </button>
        </ContextMenuFrame>
      )}

      {cardMenu && cardMenuItem && (
        <ContextMenuFrame
          x={cardMenu.x}
          y={cardMenu.y}
          menuId="data-cut-library-card-menu"
          backdropId="data-cut-library-card-ctx-backdrop"
          className="lb-ctx"
          backdropClassName="lb-ctx-backdrop"
          ariaLabel={`${cardMenuItem.name} Library menu`}
          onClose={onCloseCardMenu}
        >
            <button
              className="lb-ctx__item"
              data-cut-library-card-ctx="toproject"
              role="menuitem"
              disabled={!canUseCard}
              title={cardUseReason}
              aria-description={!canUseCard ? cardUseReason : undefined}
              onClick={() => {
                onAddToProject(cardMenuItem)
                onCloseCardMenu()
              }}
            >
              <Icon name="import" size={14} /> Add to project
            </button>
            <button
              className="lb-ctx__item"
              data-cut-library-card-ctx="insert"
              role="menuitem"
              disabled={!canUseCard}
              title={cardUseReason}
              aria-description={!canUseCard ? cardUseReason : undefined}
              onClick={() => {
                onInsertAtPlayhead(cardMenuItem)
                onCloseCardMenu()
              }}
            >
              <Icon name="plus" size={14} /> Insert at playhead
            </button>
            <button
              className="lb-ctx__item"
              data-cut-library-card-ctx="favorite"
              role="menuitem"
              onClick={() => {
                onToggleFavorite(cardMenuItem)
                onCloseCardMenu()
              }}
            >
              <Icon name="favorite" size={14} /> {cardMenuItem.favorite ? 'Unpin' : 'Pin to top'}
            </button>
            <button
              className="lb-ctx__item"
              data-cut-library-card-ctx="tag"
              role="menuitem"
              onClick={() => {
                onEditTags(cardMenuItem)
                onCloseCardMenu()
              }}
            >
              <Icon name="bookmark" size={14} /> Edit tags
            </button>
            <button
              className="lb-ctx__item"
              data-cut-library-card-ctx="move"
              role="menuitem"
              aria-expanded={moveOpen}
              title="Move this item to a Library folder"
              onClick={() => setMoveOpen((current) => !current)}
            >
              <Icon name="folder" size={14} /> Move to folder…
            </button>
            {moveOpen && (
              <div className="lb-ctx__sub" data-cut-library-card-move-list role="group">
                <button className="lb-ctx__item lb-ctx__item--sub" data-cut-library-card-move="" role="menuitem" onClick={() => { onMoveTo(cardMenuItem, ''); onCloseCardMenu() }}>No folder</button>
                {folders.map((folder) => <button key={folder} className="lb-ctx__item lb-ctx__item--sub" data-cut-library-card-move={folder} role="menuitem" onClick={() => { onMoveTo(cardMenuItem, folder); onCloseCardMenu() }}>{folder}</button>)}
              </div>
            )}
            {cardMenuItem.media_ok === false && cardMenuItem.src_path && !cardMenuItem.blob && (
              <button
                className="lb-ctx__item"
                data-cut-library-card-ctx="relink"
                role="menuitem"
                disabled={busy === cardMenuItem.id}
                title={busy === cardMenuItem.id ? 'This Library item is already being updated' : 'Choose the same media file at its new location'}
                aria-description={busy === cardMenuItem.id ? 'This Library item is already being updated' : undefined}
                onClick={() => { onRelink(cardMenuItem); onCloseCardMenu() }}
              >
                <Icon name="link" size={14} /> Relink source…
              </button>
            )}
            {cardMenuItem.media_ok !== false && cardMenuItem.src_path && !cardMenuItem.blob && (
              <button
                className="lb-ctx__item"
                data-cut-library-card-ctx="portable"
                role="menuitem"
                disabled={busy === cardMenuItem.id}
                title={busy === cardMenuItem.id ? 'This Library item is already being updated' : 'Store a copy in the Library so the item still works if the original moves'}
                aria-description={busy === cardMenuItem.id ? 'This Library item is already being updated' : undefined}
                onClick={() => {
                  onMakePortable(cardMenuItem)
                  onCloseCardMenu()
                }}
              >
                <Icon name="copy" size={14} /> Keep a copy
              </button>
            )}
            <button
              className="lb-ctx__item lb-ctx__item--danger"
              data-cut-library-card-ctx="remove"
              role="menuitem"
              onClick={() => {
                onRemove(cardMenuItem)
                onCloseCardMenu()
              }}
            >
              <Icon name="trash" size={14} /> Remove
            </button>
        </ContextMenuFrame>
      )}
    </>
  )
}
