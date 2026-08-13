import type { LibItem } from '../../lib/client'
import './library-context-menu.css'
import {
  LibraryContextMenus,
  type LibraryCardMenuState,
  type LibraryFolderMenuState,
} from './LibraryContextMenus'

export type { LibraryCardMenuState, LibraryFolderMenuState } from './LibraryContextMenus'

type ItemAction = (item: LibItem) => void | Promise<void>

export interface LibraryContextMenuController {
  hasProject: boolean
  busy: string | null
  folders: string[]
  onCloseFolderMenu: () => void
  onCloseCardMenu: () => void
  onStartRename: (name: string) => void
  onRemoveFolder: (name: string) => void | Promise<void>
  onAddToProject: ItemAction
  onInsertAtPlayhead: ItemAction
  onMoveTo: (item: LibItem, folder: string) => void | Promise<void>
  onToggleFavorite: ItemAction
  onEditTags: ItemAction
  onRelink: ItemAction
  onMakePortable: ItemAction
  onRemove: ItemAction
}

interface LibraryContextMenuLayerProps {
  folderMenu: LibraryFolderMenuState | null
  cardMenu: LibraryCardMenuState | null
  cardMenuItem: LibItem | null
  controller: LibraryContextMenuController
}

/** Adapts the Library panel's state/actions to the bounded context-menu owner. */
export default function LibraryContextMenuLayer({
  folderMenu,
  cardMenu,
  cardMenuItem,
  controller,
}: LibraryContextMenuLayerProps) {
  return <LibraryContextMenus
    folderMenu={folderMenu}
    cardMenu={cardMenu}
    cardMenuItem={cardMenuItem}
    {...controller}
  />
}
