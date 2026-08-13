// Dedicated, server-paged cross-project media workspace. Mounted by LibraryWorkspace.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { callVerb, type LibItem, type Project } from '../../lib/client'
import { libraryIdFromAssetHash } from '../../lib/mediaPath'
import { confirmAction, isTauri, pickMedia } from '../../lib/tauri'
import { Icon } from '../../icons'
import { LibraryBulkBar } from './LibraryBulkBar'
import { LibraryCard } from './LibraryCard'
import { LibraryCollections } from './LibraryCollections'
import LibraryContextMenuLayer, { type LibraryCardMenuState, type LibraryContextMenuController, type LibraryFolderMenuState } from './LibraryContextMenuLayer'
import { LibraryDetails } from './LibraryDetails'
import { LibraryFilters } from './LibraryFilters'
import { LibraryFolders } from './LibraryFolders'
import { LibraryPagination } from './LibraryPagination'
import { LibraryRow } from './LibraryRow'
import { insertLibraryItemAtPlayhead } from './libraryPlacement'
import { libraryDetailItem, type LibraryCollection, type SortKey, type TypeFilter, type ViewMode } from './model'
import { useLibraryQuery } from './useLibraryQuery'
import { useLibraryKeyboardNavigation } from './useLibraryKeyboardNavigation'
import { useLibraryRelink } from './useLibraryRelink'
import '../drawer.css'
import './library.css'
export interface LibraryPanelProps {
  /** Current project, used to mark media already attached to this edit. */
  project: Project | null
  /** Called after add_to_project so the App refreshes the timeline/assets. */
  onAddedToProject: () => void
  /** True while the Library surface is active (drives a refresh on activation). */
  active: boolean
  /** Live editor playhead used by the workspace's explicit Insert action. */
  playheadMs?: number
}

export default function LibraryPanel({
  project,
  onAddedToProject,
  active,
  playheadMs = 0,
}: LibraryPanelProps) {
  const hasProject = !!project
  const [actionErr, setErr] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [busy, setBusy] = useState<string | null>(null)

  // Filters — ALL pushed to library.list server-side (scales past load-all-then-filter).
  const [type, setType] = useState<TypeFilter>('all')
  const [folder, setFolder] = useState<string | null>(null) // null = all folders
  const [tagFilter, setTagFilter] = useState<string | null>(null) // null = no tag filter
  const [q, setQ] = useState('')
  const [sort, setSort] = useState<SortKey>('added')
  const [collection, setCollection] = useState<LibraryCollection>('all')
  const {
    items,
    folders,
    tags: collectionTags,
    loading,
    error: queryError,
    total,
    offset,
    limit,
    nextOffset,
    pageNumber,
    pageCount,
    qDebounced,
    queryKey,
    reload: reloadQuery,
    previousPage,
    nextPage,
  } = useLibraryQuery({
    active,
    type,
    folder,
    tag: tagFilter,
    search: q,
    sort,
    collection,
  })
  const visibleItems = items
  const itemIds = useMemo(() => visibleItems.map((item) => item.id), [visibleItems])
  const keyboardNavigation = useLibraryKeyboardNavigation(itemIds)
  const err = actionErr ?? queryError

  useEffect(() => {
    setErr(null)
  }, [queryKey])

  const reload = useCallback(() => {
    setErr(null)
    reloadQuery()
  }, [reloadQuery])

  // Density + multi-select.
  const [view, setView] = useState<ViewMode>('list')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [anchorId, setAnchorId] = useState<string | null>(null) // shift-range pivot

  // Posters that failed to render (non-media / no audio stream) → fall back to a glyph
  // instead of re-requesting forever.
  const [posterFail, setPosterFail] = useState<Set<string>>(new Set())
  useEffect(() => {
    setPosterFail(new Set())
  }, [items])

  // Add-by-path + new-folder + per-card / bulk tag editing.
  const [addCopy, setAddCopy] = useState(false)
  const [newFolder, setNewFolder] = useState('')
  const [tagging, setTagging] = useState<string | null>(null) // item id being tag-edited
  const [tagDraft, setTagDraft] = useState('')
  const [bulkTagOpen, setBulkTagOpen] = useState(false)
  const [bulkTagDraft, setBulkTagDraft] = useState('')

  // Folder rename / context menu + a per-card context menu.
  const [folderMenu, setFolderMenu] = useState<LibraryFolderMenuState | null>(null)
  const [renaming, setRenaming] = useState<string | null>(null) // folder name being renamed
  const [renameDraft, setRenameDraft] = useState('')
  const [cardMenu, setCardMenu] = useState<LibraryCardMenuState | null>(null)

  const flash = useCallback((msg: string) => {
    setNote(msg)
    setTimeout(() => setNote(null), 3000)
  }, [])

  const selectionFilterKey = `${queryKey}\u0000${offset}`
  const previousSelectionFilterKey = useRef(selectionFilterKey)
  const selectionClearedByFilter = useRef(false)

  useEffect(() => {
    if (previousSelectionFilterKey.current === selectionFilterKey) return
    previousSelectionFilterKey.current = selectionFilterKey
    if (selected.size === 0) return
    selectionClearedByFilter.current = true
    setSelected(new Set())
    setAnchorId(null)
    flash('Selection cleared because filters changed.')
  }, [selectionFilterKey, selected.size, flash])

  // ---- mutations (each reloads to stay truthful) ----
  // Browse for media with the native OS picker, without requiring path typing. Adds each
  // chosen file to the GLOBAL library; addCopy stores a managed blob copy.
  const browseAdd = useCallback(async () => {
    if (busy) return
    if (!isTauri()) {
      setErr('Open the desktop app to browse for files')
      return
    }
    const paths = await pickMedia()
    if (!paths.length) return
    setBusy('add')
    setErr(null)
    let added = 0
    let firstErr: string | null = null
    for (const path of paths) {
      const r = await callVerb('library.add', { path, copy: addCopy, source: 'user' })
      if (r.ok) added++
      else if (!firstErr) firstErr = `${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'could not add (is it a media file?)'}`
    }
    setBusy(null)
    if (added > 0) {
      flash(`Added ${added} to library`)
      void reload()
    }
    if (firstErr) setErr(firstErr)
  }, [addCopy, busy, reload])

  const toggleFavorite = useCallback(
    async (it: LibItem) => {
      await callVerb('library.favorite', { id: it.id, on: !it.favorite })
      void reload()
    },
    [reload],
  )

  const makePortable = useCallback(
    async (it: LibItem) => {
      if (!it.src_path || busy) return
      setBusy(it.id)
      const r = await callVerb('library.add', { path: it.src_path, copy: true, source: it.source })
      setBusy(null)
      if (r.ok) flash('Stored a copy in the Library')
      else setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'could not store a copy'}`)
      void reload()
    },
    [busy, reload],
  )

  const relinkMissing = useLibraryRelink({
    busy,
    setBusy,
    setError: setErr,
    flash,
    reload,
  })

  const moveTo = useCallback(
    async (it: LibItem, dest: string) => {
      await callVerb('library.move', { id: it.id, folder: dest })
      void reload()
    },
    [reload],
  )

  const remove = useCallback(
    async (it: LibItem) => {
      await callVerb('library.remove', { id: it.id })
      setSelected((s) => { const n = new Set(s); n.delete(it.id); return n })
      void reload()
    },
    [reload],
  )

  const saveTags = useCallback(
    async (it: LibItem, draftOverride?: string) => {
      const rawDraft = draftOverride ?? tagDraft
      const tags = rawDraft.split(',').map((t) => t.trim()).filter(Boolean)
      await callVerb('library.tag', { id: it.id, tags })
      setTagging(null)
      void reload()
    },
    [tagDraft, reload],
  )

  const addFolder = useCallback(async () => {
    const name = newFolder.trim()
    if (!name) return
    await callVerb('library.folder_add', { name })
    setNewFolder('')
    void reload()
  }, [newFolder, reload])

  // Commit a folder RENAME → library.folder_rename {old, new}. Re-points every item
  // filed under it (engine-side). No-op on an empty/unchanged name; keeps the active
  // filter pointing at the renamed folder so the view doesn't jump.
  const commitRename = useCallback(async (oldName: string) => {
    const next = renameDraft.trim()
    setRenaming(null)
    if (!next || next === oldName) return
    const r = await callVerb('library.folder_rename', { old: oldName, new: next })
    if (r.ok && (r.result as { renamed?: boolean } | undefined)?.renamed) {
      if (folder === oldName) setFolder(next) // follow the rename
      flash(`Renamed folder to "${next}"`)
    } else {
      setErr(`Could not rename — "${next}" may be empty or already taken`)
    }
    void reload()
  }, [renameDraft, folder, reload])

  // Begin renaming a folder (shared by the visible pencil AND the right-click menu).
  const startRename = useCallback((name: string) => {
    setRenameDraft(name)
    setRenaming(name)
    setFolderMenu(null)
  }, [])

  // Remove a folder → library.folder_remove {name}. Its items are un-foldered (moved
  // to root, NOT deleted). Drop the filter back to "All" if it pointed here.
  const removeFolder = useCallback(async (name: string) => {
    const r = await callVerb('library.folder_remove', { name })
    if (r.ok && (r.result as { removed?: boolean } | undefined)?.removed) {
      if (folder === name) setFolder(null)
      flash(`Removed folder "${name}" (its items moved to All)`)
    } else {
      setErr(`Could not remove folder "${name}"`)
    }
    void reload()
  }, [folder, reload])

  const addToProject = useCallback(
    async (it: LibItem) => {
      if (!hasProject || busy) return
      setBusy(it.id)
      const r = await callVerb('library.add_to_project', { id: it.id })
      setBusy(null)
      if (r.ok) {
        flash(`Added "${it.name}" to the project`)
        onAddedToProject()
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'could not add to project'}`)
      }
      void reload()
    },
    [hasProject, busy, onAddedToProject, reload],
  )

  const insertAtPlayhead = useCallback(
    async (it: LibItem) => {
      if (!hasProject || busy) return
      setBusy(it.id)
      setErr(null)
      const placement = await insertLibraryItemAtPlayhead({ item: it, project, playheadMs })
      setBusy(null)
      if (placement.ok) {
        flash(placement.message)
      } else {
        setErr(placement.message)
      }
      if (placement.projectChanged) onAddedToProject()
      void reload()
    },
    [busy, hasProject, onAddedToProject, playheadMs, project, reload],
  )

  // Close any open context menu on Escape (a full-screen backdrop handles click-away).
  useEffect(() => {
    if (!folderMenu && !cardMenu) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') { setFolderMenu(null); setCardMenu(null) } }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [folderMenu, cardMenu])

  // ---- multi-select ----
  const selCount = selected.size
  // Toggle one item; shift+click selects the contiguous range from the anchor to the
  // clicked item in the CURRENT (server-ordered) list — the standard file-manager gesture.
  const toggleSelect = useCallback(
    (id: string, shift: boolean) => {
      setSelected((prev) => {
        const next = new Set(prev)
        if (shift && anchorId) {
          const order = visibleItems.map((it) => it.id)
          const a = order.indexOf(anchorId)
          const b = order.indexOf(id)
          if (a >= 0 && b >= 0) {
            const [lo, hi] = a < b ? [a, b] : [b, a]
            for (let i = lo; i <= hi; i++) next.add(order[i])
            return next
          }
        }
        if (next.has(id)) next.delete(id)
        else next.add(id)
        return next
      })
      setAnchorId(id)
    },
    [anchorId, visibleItems],
  )
  const clearSelection = useCallback(() => { setSelected(new Set()); setAnchorId(null) }, [])
  // Keep selection truthful if an item disappears after an external library change.
  useEffect(() => {
    if (selectionClearedByFilter.current) {
      selectionClearedByFilter.current = false
      return
    }
    setSelected((prev) => {
      if (prev.size === 0) return prev
      const present = new Set(items.map((it) => it.id))
      const next = new Set([...prev].filter((id) => present.has(id)))
      return next.size === prev.size ? prev : next
    })
  }, [items])

  const selectedItems = useMemo(
    () => visibleItems.filter((it) => selected.has(it.id)),
    [selected, visibleItems],
  )

  // ---- bulk actions (iterate the selection; one reload at the end) ----
  const bulkMove = useCallback(async (dest: string) => {
    let ok = 0
    let failed = 0
    for (const it of selectedItems) {
      const moveResult = await callVerb('library.move', { id: it.id, folder: dest })
      if (moveResult.ok) ok++
      else failed++
    }
    flash(`Moved ${ok}/${selectedItems.length} to ${dest ? `"${dest}"` : 'All'}${failed ? ` (${failed} failed)` : ''}`)
    clearSelection()
    void reload()
  }, [selectedItems, clearSelection, reload])

  const bulkAddTags = useCallback(async () => {
    const add = bulkTagDraft.split(',').map((t) => t.trim()).filter(Boolean)
    if (add.length) {
      let ok = 0
      let failed = 0
      for (const it of selectedItems) {
        // ADD to existing tags (library.tag replaces the set), de-duped.
        const merged = Array.from(new Set([...it.tags, ...add]))
        const tagResult = await callVerb('library.tag', { id: it.id, tags: merged })
        if (tagResult.ok) ok++
        else failed++
      }
      flash(`Tagged ${ok}/${selectedItems.length} items${failed ? ` (${failed} failed)` : ''}`)
    }
    setBulkTagOpen(false)
    setBulkTagDraft('')
    clearSelection()
    void reload()
  }, [bulkTagDraft, selectedItems, clearSelection, reload])

  const bulkAddToProject = useCallback(async () => {
    if (!hasProject) return
    let n = 0
    for (const it of selectedItems) {
      const r = await callVerb('library.add_to_project', { id: it.id })
      if (r.ok) n++
    }
    if (n > 0) { flash(`Added ${n} to the project`); onAddedToProject() }
    clearSelection()
    void reload()
  }, [hasProject, selectedItems, onAddedToProject, clearSelection, reload])

  const bulkRemove = useCallback(async () => {
    if (!await confirmAction(`Remove ${selectedItems.length} item(s) from the library? Source files are not deleted.`, { title: 'Remove from Library?', okLabel: 'Remove', cancelLabel: 'Keep' })) return
    let ok = 0
    let failed = 0
    for (const it of selectedItems) {
      const removeResult = await callVerb('library.remove', { id: it.id })
      if (removeResult.ok) ok++
      else failed++
    }
    flash(`Removed ${ok}/${selectedItems.length} from the library${failed ? ` (${failed} failed)` : ''}`)
    clearSelection()
    void reload()
  }, [selectedItems, clearSelection, reload])

  const markPosterFail = useCallback((id: string) => {
    setPosterFail((s) => { if (s.has(id)) return s; const n = new Set(s); n.add(id); return n })
  }, [])

  const openTagEditor = useCallback((it: LibItem) => {
    setTagging(it.id)
    setTagDraft(it.tags.join(', '))
  }, [])

  const cancelTagEditor = useCallback(() => setTagging(null), [])

  const toggleTagFilter = useCallback((tag: string) => {
    if (collection === 'recent') setSort('added')
    setCollection('all')
    setTagFilter((cur) => (cur === tag ? null : tag))
  }, [collection])

  const openCardMenu = useCallback((x: number, y: number, id: string) => {
    setCardMenu({ x, y, id })
  }, [])

  const anyFilter = collection !== 'all' || type !== 'all' || folder != null || tagFilter != null || qDebounced.trim() !== ''
  const projectLibraryIds = useMemo(() => new Set(Object.values(project?.assets ?? {})
    .map((asset) => libraryIdFromAssetHash(asset.hash))
    .filter((id): id is string => !!id)), [project])
  const cardMenuItem = cardMenu ? (visibleItems.find((it) => it.id === cardMenu.id) ?? null) : null
  const detailItem = libraryDetailItem(visibleItems, selectedItems, keyboardNavigation.activeId)
  const contextMenuController: LibraryContextMenuController = {
    hasProject, busy, folders, onStartRename: startRename,
    onCloseFolderMenu: () => setFolderMenu(null), onCloseCardMenu: () => setCardMenu(null),
    onRemoveFolder: removeFolder, onAddToProject: addToProject, onInsertAtPlayhead: insertAtPlayhead,
    onMoveTo: moveTo, onToggleFavorite: toggleFavorite, onEditTags: openTagEditor,
    onRelink: relinkMissing, onMakePortable: makePortable, onRemove: remove,
  }
  const emptyMessage = collection === 'favorites'
    ? 'No favorites match this view yet.'
    : collection === 'missing'
      ? 'No missing media matches this view.'
      : anyFilter
        ? 'Nothing matches these filters.'
        : 'Your library is empty. Browse to add reusable media; direct agent imports stay in this project’s Assets.'

  return (
    <section className="panel library-panel library-panel--workspace" data-cut-panel="library" data-cut-library>
      <div className="cd-body lb-panel-body">
        <div className="lb-command-stack">
        {/* Browse to add media + an optional managed-copy toggle (no path typing). */}
        <div className="lb-add" data-cut-library-add>
          <button
            className="lb-browse"
            data-cut-library-browse
            data-cut-library-addbtn
            disabled={busy === 'add'}
            onClick={() => void browseAdd()}
          >
            {busy === 'add' ? (
              'Adding…'
            ) : (
              <>
                <Icon name="import" size={14} tone="asset" /> Browse files…
              </>
            )}
          </button>
          <label className="lb-copy" title="Store a managed copy in the Library folder so the original file can move">
            <input type="checkbox" data-cut-library-portable-toggle checked={addCopy} onChange={(e) => setAddCopy(e.target.checked)} />
            Keep a copy
          </label>
          <span className="lb-add-spacer" />
          {selCount > 0 ? (
            <span className="lb-count lb-count--sel">{selCount} selected</span>
          ) : (
            <span className="lb-count" data-cut-library-count={total}>{total}{anyFilter ? ' matching' : ' total'}</span>
          )}
          {/* Density toggle: Grid ↔ List. */}
          <div className="lb-view" role="group" aria-label="View density">
            <button
              className={`lb-view-btn ${view === 'grid' ? 'lb-view-btn--on' : ''}`}
              data-cut-library-view-grid
              data-cut-on={view === 'grid'}
              title="Grid view"
              onClick={() => setView('grid')}
            >
              <Icon name="grid" size={16} label="Grid" />
            </button>
            <button
              className={`lb-view-btn ${view === 'list' ? 'lb-view-btn--on' : ''}`}
              data-cut-library-view-list
              data-cut-on={view === 'list'}
              title="List view"
              onClick={() => setView('list')}
            >
              <Icon name="list" size={16} label="List" />
            </button>
          </div>
        </div>

        {/* Type tabs + sort + search */}
        <LibraryFilters
          type={type}
          sort={sort}
          search={q}
          activeTagFilter={tagFilter}
          onTypeChange={setType}
          onSortChange={(next) => {
            if (collection === 'recent') setCollection('all')
            setSort(next)
          }}
          onSearchChange={setQ}
          onClearTagFilter={() => setTagFilter(null)}
        />
        </div>

        <div className="lb-browser">
          <aside className="lb-collection-rail" aria-label="Library collections">
            <p className="lb-collection-rail__eyebrow">Collections</p>
            <h2>Browse</h2>
            <LibraryCollections
              active={collection}
              tags={collectionTags}
              activeTag={tagFilter}
              allMediaActive={collection === 'all' && folder === null}
              onSelect={(next) => {
                if (next !== 'recent' && collection === 'recent') setSort('added')
                setCollection(next)
                setTagFilter(null)
                setFolder(null)
                if (next === 'recent') setSort('recent')
              }}
              onSelectTag={(tag) => {
                if (collection === 'recent') setSort('added')
                setCollection('all')
                setFolder(null)
                setTagFilter((current) => (current === tag ? null : tag))
              }}
            />
            <p className="lb-collection-rail__label">Folders</p>
            <LibraryFolders
              folders={folders}
              activeFolder={folder}
              renaming={renaming}
              renameDraft={renameDraft}
              newFolder={newFolder}
              onSelectFolder={(next) => {
                if (collection === 'recent') setSort('added')
                setCollection('all')
                setFolder(next)
              }}
              onOpenMenu={(x, y, name) => setFolderMenu({ x, y, name })}
              onStartRename={startRename}
              onRenameDraftChange={setRenameDraft}
              onCommitRename={commitRename}
              onCancelRename={() => setRenaming(null)}
              onNewFolderChange={setNewFolder}
              onAddFolder={addFolder}
              onRemoveFolder={(name) => { void removeFolder(name) }}
            />
          </aside>

          <main className="lb-results" aria-label="Library media">
        <div className="lb-results-scroll" data-cut-library-results-scroll>
        {note && <div className="cd-note lb-flash" data-cut-library-note>{note}</div>}
        {err && <div className="cd-err" data-cut-library-error role="alert">{err}</div>}
        {loading && <div className="cd-empty" data-cut-library-loading>Loading library…</div>}
        {!loading && !err && visibleItems.length === 0 && (
          <div className="cd-empty" data-cut-library-empty>
            {emptyMessage}
          </div>
        )}

        {/* Grid/list body. data-cut-library-grid stays on both containers for legacy harnesses. */}
        {view === 'grid' ? (
          <div className="lb-grid" data-cut-library-grid>
            {visibleItems.map((it) => (
              <LibraryCard
                key={it.id}
                item={it}
                inProject={projectLibraryIds.has(it.id)}
                selected={selected.has(it.id)}
                failedPoster={posterFail.has(it.id)}
                hasProject={hasProject}
                busy={busy}
                folders={folders}
                tagDraft={tagDraft}
                activeTagFilter={tagFilter}
                editingTags={tagging === it.id}
                keyboardTabIndex={keyboardNavigation.tabIndexFor(it.id)}
                onOpenMenu={openCardMenu}
                onKeyboardFocus={keyboardNavigation.onItemFocus}
                onKeyboardKeyDown={keyboardNavigation.onItemKeyDown}
                onPosterFail={markPosterFail}
                onToggleSelect={toggleSelect}
                onToggleFavorite={toggleFavorite}
                onAddToProject={addToProject}
                onInsertAtPlayhead={insertAtPlayhead}
                onMoveTo={moveTo}
                onEditTags={openTagEditor}
                onRelink={relinkMissing}
                onMakePortable={makePortable}
                onRemove={remove}
                onTagDraftChange={setTagDraft}
                onSaveTags={saveTags}
                onCancelTagEditor={cancelTagEditor}
                onToggleTagFilter={toggleTagFilter}
              />
            ))}
          </div>
        ) : (
          <div className="lb-list" data-cut-library-grid data-cut-library-list>
            {visibleItems.length > 0 && (
              <div className="lb-list-head" data-cut-library-list-header>
                <span className="lb-list-h" aria-label="Selection" title="Select">✓</span>
                <span className="lb-list-h">Media</span>
                <button className="lb-list-h lb-list-h--sort" data-cut-library-list-sort-name title="Sort by name" onClick={() => setSort('name')}>Name</button>
                <span className="lb-list-h" title="Favorite">Pin</span>
              </div>
            )}
            {visibleItems.map((it) => (
              <LibraryRow
                key={it.id}
                item={it}
                inProject={projectLibraryIds.has(it.id)}
                selected={selected.has(it.id)}
                failedPoster={posterFail.has(it.id)}
                hasProject={hasProject}
                busy={busy}
                folders={folders}
                tagDraft={tagDraft}
                activeTagFilter={tagFilter}
                editingTags={tagging === it.id}
                keyboardTabIndex={keyboardNavigation.tabIndexFor(it.id)}
                onOpenMenu={openCardMenu}
                onKeyboardFocus={keyboardNavigation.onItemFocus}
                onKeyboardKeyDown={keyboardNavigation.onItemKeyDown}
                onPosterFail={markPosterFail}
                onToggleSelect={toggleSelect}
                onToggleFavorite={toggleFavorite}
                onAddToProject={addToProject}
                onInsertAtPlayhead={insertAtPlayhead}
                onMoveTo={moveTo}
                onEditTags={openTagEditor}
                onRelink={relinkMissing}
                onMakePortable={makePortable}
                onRemove={remove}
                onTagDraftChange={setTagDraft}
                onSaveTags={saveTags}
                onCancelTagEditor={cancelTagEditor}
                onToggleTagFilter={toggleTagFilter}
              />
            ))}
          </div>
        )}
        </div>
            <LibraryPagination
              offset={offset}
              limit={limit}
              total={total}
              pageNumber={pageNumber}
              pageCount={pageCount}
              hasNext={nextOffset != null}
              loading={loading}
              onPrevious={previousPage}
              onNext={nextPage}
            />
          </main>

          <LibraryDetails
            item={detailItem}
            selectedCount={selectedItems.length}
            failedPoster={detailItem ? posterFail.has(detailItem.id) : false}
            inProject={detailItem ? projectLibraryIds.has(detailItem.id) : false}
            onPosterFail={markPosterFail}
          />
        </div>
      </div>

      {/* Bulk action bar — appears when ≥1 item is selected. */}
      {selCount > 0 && (
        <LibraryBulkBar
          selectedCount={selCount}
          folders={folders}
          hasProject={hasProject}
          tagEditorOpen={bulkTagOpen}
          tagDraft={bulkTagDraft}
          onStartTagEdit={() => { setBulkTagDraft(''); setBulkTagOpen(true) }}
          onTagDraftChange={setBulkTagDraft}
          onSaveTags={bulkAddTags}
          onCancelTagEdit={() => setBulkTagOpen(false)}
          onMove={bulkMove}
          onAddToProject={bulkAddToProject}
          onRemove={bulkRemove}
          onClearSelection={clearSelection}
        />
      )}

      <LibraryContextMenuLayer
        folderMenu={folderMenu}
        cardMenu={cardMenu}
        cardMenuItem={cardMenuItem}
        controller={contextMenuController}
      />

    </section>
  )
}
