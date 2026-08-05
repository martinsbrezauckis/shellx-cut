import type { LibItem } from '../../lib/client'
import { Icon } from '../../icons'

export type LibraryFolderMenuState = { x: number; y: number; name: string }
export type LibraryCardMenuState = { x: number; y: number; id: string }

export interface LibraryContextMenusProps {
  folderMenu: LibraryFolderMenuState | null
  cardMenu: LibraryCardMenuState | null
  cardMenuItem: LibItem | null
  hasProject: boolean
  onCloseFolderMenu: () => void
  onCloseCardMenu: () => void
  onStartRename: (name: string) => void
  onRemoveFolder: (name: string) => void
  onAddToProject: (item: LibItem) => void
  onToggleFavorite: (item: LibItem) => void
  onEditTags: (item: LibItem) => void
  onMakePortable: (item: LibItem) => void
  onRemove: (item: LibItem) => void
}

export function LibraryContextMenus({
  folderMenu,
  cardMenu,
  cardMenuItem,
  hasProject,
  onCloseFolderMenu,
  onCloseCardMenu,
  onStartRename,
  onRemoveFolder,
  onAddToProject,
  onToggleFavorite,
  onEditTags,
  onMakePortable,
  onRemove,
}: LibraryContextMenusProps) {
  return (
    <>
      {folderMenu && (
        <>
          <div
            className="lb-ctx-backdrop"
            data-cut-library-folder-ctx-backdrop
            onMouseDown={onCloseFolderMenu}
            onContextMenu={(e) => { e.preventDefault(); onCloseFolderMenu() }}
          />
          <div className="lb-ctx" role="menu" data-cut-library-folder-menu style={{ left: folderMenu.x, top: folderMenu.y }}>
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
          </div>
        </>
      )}

      {cardMenu && cardMenuItem && (
        <>
          <div
            className="lb-ctx-backdrop"
            data-cut-library-card-ctx-backdrop
            onMouseDown={onCloseCardMenu}
            onContextMenu={(e) => { e.preventDefault(); onCloseCardMenu() }}
          />
          <div className="lb-ctx" role="menu" data-cut-library-card-menu style={{ left: cardMenu.x, top: cardMenu.y }}>
            <button
              className="lb-ctx__item"
              data-cut-library-card-ctx="toproject"
              role="menuitem"
              disabled={!hasProject}
              onClick={() => {
                onAddToProject(cardMenuItem)
                onCloseCardMenu()
              }}
            >
              <Icon name="import" size={14} /> Add to project
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
            {cardMenuItem.src_path && !cardMenuItem.blob && (
              <button
                className="lb-ctx__item"
                data-cut-library-card-ctx="portable"
                role="menuitem"
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
          </div>
        </>
      )}
    </>
  )
}
