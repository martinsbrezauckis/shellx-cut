// panels/Assets — the imported-media tray (left sidebar "Assets" tab).
// Role: lists every asset in project.assets (each import lands here, live via
// the op_applied/job_progress project refresh in App) as a draggable card.
// Two placement paths, both one verb under the zero-local-mutation contract:
//   • DRAG a card onto the timeline → Timeline's onDrop computes the at_ms from
//     the drop x and the track from the drop y, then dispatches edit.insert.
//     Dropping INSIDE a clip auto-splits it (engine splice_into_track) — that
//     is the "cut and pull the video apart, drop a clip in the middle" flow.
//   • The card's "Insert" button → edit.insert at the live playhead (ripple, so
//     downstream content shifts to keep AV sync) — the non-drag path.
// The card carries kind/duration/dimensions from the asset's probe and shows how
// many timeline clips already reference it (so re-using an asset is obvious).
// ADD-MEDIA AREA (header): "+ Import" opens the native OS picker. "Generate"
// jumps to the adjacent Generate tab (assets.generate, image/video from a prompt
// via the user's own codex/grok CLI). Both land the result here in the tray.
// Callers: panels/LeftPanel. Dependencies: lib/client (verbs + types), icons.

import { useCallback, useEffect, useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import { callVerb, type Project } from '../../lib/client'
import { placeLinkedAV, planAssetInsertAtPlayhead } from '../../lib/placement'
import { ASSET_DRAG_DROP } from '../../lib/dnd'
import { useAssetCardDrag, type AssetCardDragItem } from '../../lib/useAssetCardDrag'
import { libraryIdFromAssetHash, mediaBasename } from '../../lib/mediaPath'
import { confirmAction, isTauri, pickMedia } from '../../lib/tauri'
import { getGenerateProxies, setGenerateProxies } from '../../lib/proxyPref'
import { openCutManual } from '../../lib/manual'
import type { DoctorReport } from '../../lib/doctor'
import { Icon } from '../../icons'
import { useOfflineMedia } from '../../app/OfflineMediaContext'
import { libraryMembershipBatches } from './libraryMembership'
import { assetReadiness, mediaCapabilitiesFromDoctor, summarizeMediaReadiness, type MediaReadinessAsset } from './mediaReadiness'
import SourceMonitor, { type SourceMonitorAsset } from './SourceMonitor'
import AssetContextMenu, { type AssetContextMenuState } from './AssetContextMenu'
import './assets.css'

export interface AssetsProps {
  project: Project | null
  doctor: DoctorReport | null
  /** Live playhead (timeline ms) — the target for the "Insert" button. */
  playheadMs: number
}

/** The fields we read off an asset's `probe` (media.probe result — see
 *  app/media/src/probe.rs). All optional so a half-probed asset still renders. */
interface ProbeView {
  kind?: string // "video" | "audio" | "image"
  duration_ms?: number
  width?: number
  height?: number
  fps?: number
  has_audio?: boolean
}

interface AssetRow {
  id: string
  path: string
  film?: string
  proxy?: string
  transcript?: string
  perception?: string
  probe: ProbeView
}

interface SmartBinRow {
  name: string
  kind?: string
  text?: string
  unused?: boolean
  min_width?: number
  min_height?: number
  offline?: boolean
  modified_after_ms?: number
  modified_before_ms?: number
  matches?: string[]
  match_count: number
}

/** Default image insert length (no intrinsic duration — edit.insert needs one). */
const IMAGE_DEFAULT_MS = 3000
const RECENT_WINDOW_MS = 30 * 24 * 60 * 60 * 1000
const LARGE_MIN_DIMENSION = 2160

function sourceMonitorRequest(value: unknown): { asset: string; atMs: number } | null {
  if (typeof value !== 'object' || value === null) return null
  const asset = Reflect.get(value, 'asset')
  if (typeof asset !== 'string') return null
  const at = Number(Reflect.get(value, 'at_ms'))
  return { asset, atMs: Number.isFinite(at) ? Math.max(0, Math.round(at)) : 0 }
}

/** `4.2s` / `1:03` short duration for a card. */
function shortDur(ms?: number): string {
  if (!ms || ms <= 0) return '—'
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`
  const m = Math.floor(ms / 60_000)
  const s = Math.round((ms % 60_000) / 1000)
  return `${m}:${String(s).padStart(2, '0')}`
}

function binCriteriaLabel(b: SmartBinRow): string {
  return [
    b.kind && b.kind,
    b.text && `name "${b.text}"`,
    b.unused === true && 'unused',
    b.unused === false && 'on timeline',
    (b.min_width || b.min_height) && '4K+',
    b.offline === true && 'missing',
    b.offline === false && 'online',
    b.modified_after_ms != null && 'recent',
    b.modified_before_ms != null && 'before date',
  ].filter(Boolean).join(' · ') || 'saved filter'
}


/** Glyph per media kind — icon-only, no external assets (theme-coloured). */
function KindIcon({ kind }: { kind?: string }) {
  if (kind === 'audio') return <Icon name="audioClip" size={18} />
  if (kind === 'image') return <Icon name="image" size={18} />
  // video (default)
  return <Icon name="video" size={18} />
}

/** Thumb geometry (px) — matches .assets__thumb in assets.css. */
const THUMB_W = 64
const THUMB_H = 36

/** HOVER-SCRUB thumbnail: video assets show a real frame sliced from the
 *  whole-asset filmstrip strip (built at import, same technique as the
 *  Timeline's "frames in the time bar"); moving the mouse across it scrubs
 *  the strip — hover-x fraction → strip position. The strip's natural size is
 *  measured once via Image() (frame count stays server-side); non-video or
 *  strip-less assets keep the kind glyph. */
function AssetThumb({ assetId, kind, film }: { assetId: string; kind?: string; film?: string }) {
  const [stripW, setStripW] = useState<number | null>(null)
  const [frac, setFrac] = useState(0.15) // resting poster position
  const url = film ? `/${film}` : null
  useEffect(() => {
    if (!url) return
    let alive = true
    const img = new window.Image()
    img.onload = () => {
      if (alive && img.naturalHeight > 0) setStripW(img.naturalWidth * (THUMB_H / img.naturalHeight))
    }
    img.src = url
    return () => {
      alive = false
    }
  }, [url])
  if (!url || kind !== 'video') {
    return (
      <span className={`assets__icon assets__icon--${kind ?? 'video'}`}>
        <KindIcon kind={kind} />
      </span>
    )
  }
  const x = stripW ? -frac * Math.max(0, stripW - THUMB_W) : 0
  return (
    <span
      className="assets__thumb"
      data-cut-asset-thumb={assetId}
      data-cut-thumb-frac={frac.toFixed(2)}
      title="Hover to scrub through the clip"
      onMouseMove={(e) => {
        const r = e.currentTarget.getBoundingClientRect()
        setFrac(Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)))
      }}
      onMouseLeave={() => setFrac(0.15)}
      style={{
        backgroundImage: `url(${url})`,
        backgroundSize: 'auto 100%',
        backgroundPosition: `${x}px 0`,
      }}
    />
  )
}

export default function Assets({ project, doctor, playheadMs }: AssetsProps) {
  const [busy, setBusy] = useState<string | null>(null) // assetId currently inserting
  const [note, setNote] = useState<string | null>(null)
  const [sourceMonitorId, setSourceMonitorId] = useState<string | null>(null)
  const [sourceMonitorAtMs, setSourceMonitorAtMs] = useState(0)
  const [assetMenu, setAssetMenu] = useState<AssetContextMenuState | null>(null)
  const closeSourceMonitor = useCallback(() => setSourceMonitorId(null), [])
  // "Generate proxies" toggle (persisted) — off = heavy files import instantly.
  const [proxiesOn, setProxiesOn] = useState<boolean>(getGenerateProxies())
  const dropAsset = useCallback((item: AssetCardDragItem, clientX: number, clientY: number, alt: boolean) => {
    document.dispatchEvent(new CustomEvent(ASSET_DRAG_DROP, {
      detail: { asset: item.asset, kind: item.kind, clientX, clientY, alt },
    }))
  }, [])
  const { ghost, startPointerDrag, startMouseDrag, preventNativeDrag } = useAssetCardDrag(dropAsset)
  const {
    offlineAssetIds: offline,
    modifiedMs,
    refresh: refreshOfflineNow,
    relinkAsset,
    relinkingAssetId,
  } = useOfflineMedia()
  const assetCount = Object.keys(project?.assets ?? {}).length

  const assetRows = useMemo<AssetRow[]>(() => (
    Object.entries(project?.assets ?? {})
      .map(([id, a]) => ({
        id,
        path: a.path,
        film: a.filmstrip,
        proxy: a.proxy,
        transcript: a.transcript,
        perception: a.perception,
        probe: (a.probe ?? {}) as ProbeView,
      }))
  ), [project])

  const sourceMonitorAsset = useMemo<SourceMonitorAsset | null>(() => {
    const row = sourceMonitorId ? assetRows.find((asset) => asset.id === sourceMonitorId) : null
    if (!row || (row.probe.kind !== 'video' && row.probe.kind !== 'audio')) return null
    return {
      id: row.id,
      name: mediaBasename(row.path),
      kind: row.probe.kind,
      durationMs: Math.max(0, row.probe.duration_ms ?? 0),
      hasAudio: row.probe.kind === 'video' && !!row.probe.has_audio,
      proxy: row.proxy,
    }
  }, [assetRows, sourceMonitorId])

  useEffect(() => {
    const openFromSearch = (event: Event) => {
      const request = event instanceof CustomEvent ? sourceMonitorRequest(event.detail) : null
      if (!request) return
      const row = assetRows.find((asset) => asset.id === request.asset)
      if (!row || (row.probe.kind !== 'video' && row.probe.kind !== 'audio')) return
      if (offline.has(row.id)) {
        setNote(`Cannot open "${mediaBasename(row.path)}": source file is offline`)
        return
      }
      setSourceMonitorAtMs(request.atMs)
      setSourceMonitorId(row.id)
    }
    document.addEventListener('cut:open-source-monitor', openFromSearch)
    return () => document.removeEventListener('cut:open-source-monitor', openFromSearch)
  }, [assetRows, offline])

  // assetId → number of timeline clips referencing it (re-use indicator).
  const usage = useMemo(() => {
    const m = new Map<string, number>()
    for (const t of project?.tracks ?? []) {
      for (const c of t.clips ?? []) {
        const asset = (c as { asset?: string }).asset
        if (asset) m.set(asset, (m.get(asset) ?? 0) + 1)
      }
    }
    return m
  }, [project])

  // SMART BINS: saved searches over this tray (media.bin_*). The filter
  // controls below share the SAME criteria semantics as the engine's bin match
  // (kind = probe kind, text = basename substring, unused = no timeline refs,
  // 4K+ = probed dimensions, missing = fs check, recent = source mtime),
  // so "save current filter as bin" is exact, not approximate.
  const [filterText, setFilterText] = useState('')
  const [filterKind, setFilterKind] = useState<'' | 'video' | 'audio' | 'image'>('')
  const [filterUnused, setFilterUnused] = useState(false)
  const [filterLarge, setFilterLarge] = useState(false)
  const [filterOffline, setFilterOffline] = useState(false)
  const [filterRecent, setFilterRecent] = useState(false)
  const [filterNeedsAction, setFilterNeedsAction] = useState(false)
  const [libraryIds, setLibraryIds] = useState<Set<string>>(new Set())
  const [activeBin, setActiveBin] = useState<string | null>(null)
  const [bins, setBins] = useState<SmartBinRow[]>([])
  useEffect(() => {
    let alive = true
    if (!project) {
      setBins([])
      return
    }
    void callVerb('media.bin_list', {}).then((r) => {
      if (!alive || !r.ok) return
      setBins((r.result as { bins?: SmartBinRow[] })?.bins ?? [])
    })
    return () => {
      alive = false
    }
  }, [project])

  /** Apply a bin: copy its query into the filter controls (visible + editable). */
  const applyBin = (bin: (typeof bins)[number] | null) => {
    setActiveBin(bin?.name ?? null)
    setFilterText(bin?.text ?? '')
    setFilterKind((bin?.kind as typeof filterKind) ?? '')
    setFilterUnused(bin?.unused ?? false)
    setFilterLarge(!!bin?.min_width || !!bin?.min_height)
    setFilterOffline(bin?.offline === true)
    setFilterRecent(bin?.modified_after_ms != null || bin?.modified_before_ms != null)
    setFilterNeedsAction(false)
  }

  const saveBin = async () => {
    if (!filterText && !filterKind && !filterUnused && !filterLarge && !filterOffline && !filterRecent) return
    const name = window.prompt('Save this filter as a smart bin — name:')?.trim()
    if (!name) return
    const recentAfter = Date.now() - RECENT_WINDOW_MS
    const r = await callVerb('media.bin_save', {
      name,
      ...(filterKind ? { kind: filterKind } : {}),
      ...(filterText ? { text: filterText } : {}),
      ...(filterUnused ? { unused: true } : {}),
      ...(filterLarge ? { min_width: LARGE_MIN_DIMENSION, min_height: LARGE_MIN_DIMENSION } : {}),
      ...(filterOffline ? { offline: true } : {}),
      ...(filterRecent ? { modified_after_ms: recentAfter } : {}),
      rationale: 'user: save Assets filter as smart bin',
    })
    setNote(r.ok ? `Saved bin "${name}"` : `Save failed: ${r.error?.message ?? 'error'}`)
    if (r.ok) {
      const listed = await callVerb('media.bin_list', {})
      if (listed.ok) setBins((listed.result as { bins?: SmartBinRow[] })?.bins ?? [])
      setActiveBin(name)
    }
    setTimeout(() => setNote(null), 3500)
  }

  const deleteBin = async (name: string) => {
    const r = await callVerb('media.bin_delete', { name, rationale: 'user: delete smart bin' })
    if (r.ok) {
      setBins((current) => current.filter((bin) => bin.name !== name))
      if (activeBin === name) applyBin(null)
    }
    setNote(r.ok ? `Deleted bin "${name}"` : `Delete failed: ${r.error?.message ?? 'error'}`)
    setTimeout(() => setNote(null), 3500)
  }

  const readinessRows = useMemo<MediaReadinessAsset[]>(() => (
    assetRows.map((row) => ({
      ...row,
      offline: offline.has(row.id),
      used: usage.get(row.id) ?? 0,
    }))
  ), [assetRows, offline, usage])

  const readinessById = useMemo(() => (
    new Map(readinessRows.map((row) => [row.id, assetReadiness(row)]))
  ), [readinessRows])

  const projectLibraryIds = useMemo(() => Object.values(project?.assets ?? {})
    .map((asset) => libraryIdFromAssetHash(asset.hash))
    .filter((id): id is string => !!id)
    .sort(), [project])
  const projectLibraryKey = projectLibraryIds.join('\0')

  useEffect(() => {
    if (!projectLibraryKey) {
      setLibraryIds(new Set())
      return
    }
    let alive = true
    const refresh = () => {
      const batches = libraryMembershipBatches(projectLibraryIds)
      void Promise.all(batches.map((ids) => callVerb('library.list', {
        ids,
        offset: 0,
        limit: ids.length,
      }))).then((results) => {
        if (!alive || results.some((result) => !result.ok || !result.result)) return
        setLibraryIds(new Set(results.flatMap((result) => (
          result.result?.items.map((item) => item.id) ?? []
        ))))
      }).catch(() => { /* Library availability does not block project media. */ })
    }
    refresh()
    document.addEventListener('cut:library-changed', refresh)
    return () => {
      alive = false
      document.removeEventListener('cut:library-changed', refresh)
    }
  }, [projectLibraryIds, projectLibraryKey])

  const mediaCapabilities = useMemo(() => mediaCapabilitiesFromDoctor(doctor), [doctor])
  const mediaHealth = useMemo(
    () => summarizeMediaReadiness(readinessRows, mediaCapabilities),
    [readinessRows, mediaCapabilities],
  )
  const mediaDimensions = useMemo(() => ([
    ['source', 'Source', mediaHealth.dimensions.source],
    ['edit', 'Edit', mediaHealth.dimensions.edit],
    ['proxy', 'Proxy', mediaHealth.dimensions.proxy],
    ['speech', 'Speech', mediaHealth.dimensions.speech],
    ['perception', 'Captions & transcription', mediaHealth.dimensions.perception],
    ['services', 'Optional', mediaHealth.dimensions.services],
  ] as const), [mediaHealth.dimensions])

  const firstOfflineRow = useMemo(() => (
    mediaHealth.firstOffline ? assetRows.find((row) => row.id === mediaHealth.firstOffline) ?? null : null
  ), [assetRows, mediaHealth.firstOffline])

  const activeBinMatchIds = useMemo(() => {
    const bin = activeBin ? bins.find((b) => b.name === activeBin) : null
    return bin?.matches ? new Set(bin.matches) : null
  }, [activeBin, bins])

  const items = useMemo(() => {
    const text = filterText.trim().toLowerCase()
    const recentAfter = Date.now() - RECENT_WINDOW_MS
    return assetRows
      .filter(({ id, path, probe }) => {
        if (activeBinMatchIds && !activeBinMatchIds.has(id)) return false
        if (filterKind && (probe.kind ?? null) !== filterKind) return false
        if (text && !mediaBasename(path).toLowerCase().includes(text)) return false
        if (filterUnused && (usage.get(id) ?? 0) > 0) return false
        if (filterLarge) {
          const w = probe.width ?? 0
          const h = probe.height ?? 0
          if (w < LARGE_MIN_DIMENSION || h < LARGE_MIN_DIMENSION) return false
        }
        if (filterOffline && !offline.has(id)) return false
        if (filterRecent && (modifiedMs.get(id) ?? 0) < recentAfter) return false
        if (filterNeedsAction && !readinessById.get(id)?.needsAction) return false
        return true
      })
      .sort((x, y) => x.id.localeCompare(y.id))
  }, [activeBinMatchIds, assetRows, filterText, filterKind, filterUnused, filterLarge, filterOffline, filterRecent, filterNeedsAction, modifiedMs, offline, readinessById, usage])

  /** Import media INTO the project AND mirror it into the GLOBAL library, marked as a
   *  project asset (upload inside the asset bar adds to the global library and
   *  marked as a project asset"). Browse = the native OS picker — no path typing. */
  const importPaths = async (paths: string[]) => {
    const list = paths.map((p) => p.trim()).filter(Boolean)
    if (!list.length) return
    setBusy('import')
    let imported = 0
    let libraryMisses = 0
    let firstErr: string | null = null
    for (const path of list) {
      const r = await callVerb('media.import', { path, proxy: getGenerateProxies(), rationale: 'user import from Assets' })
      if (r.ok) {
        imported++
        const res = r.result as { asset_id?: string; job_id?: string }
        if (res?.asset_id) {
          // Mirror into the global library, tagged with the project name. A Library
          // failure must not roll back a successful project import, but awaiting the
          // result keeps the cross-surface badge truthful instead of racing list().
          const libraryResult = await callVerb('library.add', {
            asset: res.asset_id,
            source: 'user',
            tags: project?.name ? [project.name] : [],
          }).catch(() => null)
          const libraryId = libraryResult?.ok ? libraryResult.result?.item.id : null
          if (libraryId) {
            setLibraryIds((current) => new Set(current).add(libraryId))
          } else {
            libraryMisses++
          }
          await placeImported(res.asset_id, res.job_id)
        }
      } else if (!firstErr) firstErr = r.error?.message ?? r.error?.code ?? 'import failed'
    }
    setBusy(null)
    setNote(imported > 0
      ? `Imported ${imported} file${imported === 1 ? '' : 's'}${libraryMisses ? `; ${libraryMisses} not added to Library` : ''}`
      : `Import failed: ${firstErr ?? 'error'}`)
    setTimeout(() => setNote(null), 3500)
  }

  /** Browse for media with the native OS picker (desktop). */
  const browseImport = async () => {
    if (!isTauri()) {
      setNote('Open the desktop app to browse for files')
      setTimeout(() => setNote(null), 3500)
      return
    }
    const paths = await pickMedia()
    if (paths.length) await importPaths(paths)
  }

  // Subsequent imports are no longer auto-placed. The
  // user decides each clip's placement by DROPPING it or via the Insert button.
  // The FIRST import still becomes the timeline (engine auto-place); the rest
  // wait in the Assets tray.
  const placeImported = async (_assetId: string, _jobId?: string) => {
    /* intentionally a no-op — imports wait in Assets for an explicit drop/Insert */
  }

  /** Add an asset to the base timeline at the playhead. Video-with-audio is
   *  placed as a linked video/audio pair so preview and export stay audible.
   *  Overlay tracks remain explicit: Alt-drag onto the timeline or drop on an
   *  existing overlay lane. */
  const insertAtPlayhead = async (assetId: string, probe: ProbeView) => {
    if (busy) return
    setBusy(assetId)
    setNote(null)
    const kind = probe.kind ?? 'video'
    const plan = planAssetInsertAtPlayhead({
      asset: assetId,
      kind,
      at_ms: Math.max(0, Math.round(playheadMs)),
      duration_ms: probe.kind === 'image' ? IMAGE_DEFAULT_MS : undefined,
    })
    const res = await placeLinkedAV({
      ...plan,
      project,
    })
    setBusy(null)
    const where = res.audioLinked ? `${res.videoTrack} + ${res.audioTrack}` : (res.videoTrack ?? res.audioTrack ?? '')
    setNote(res.ok ? `Inserted on ${where || 'the base timeline'}` : `Add failed: ${res.error ?? 'error'}`)
    setTimeout(() => setNote(null), 3500)
  }

  /** Remove an asset from the project (media.remove). SAFE: if any timeline clip
   *  still uses it we don't even call the verb — we tell the user to delete those
   *  clips first (the server would refuse with the same message anyway). The
   *  SOURCE file on disk is kept; only the regenerable proxy/thumbnails go. Not
   *  undoable (re-import to restore) → a confirm gate. */
  const removeAsset = async (assetId: string, name: string, used: number) => {
    if (busy) return
    if (used > 0) {
      setNote(`"${name}" is used by ${used} clip${used === 1 ? '' : 's'} — delete them from the timeline first.`)
      setTimeout(() => setNote(null), 4500)
      return
    }
    if (
      !await confirmAction(
        `Remove "${name}" from this project?\n\n` +
          `Its proxy and thumbnails are deleted (regenerable). The SOURCE file on disk is kept. ` +
          `This is not undoable — re-import to restore.`,
        { title: 'Remove project asset?', okLabel: 'Remove asset', cancelLabel: 'Keep asset' },
      )
    )
      return
    setBusy(assetId)
    const r = await callVerb('media.remove', { asset: assetId, rationale: 'user: remove asset from project (Assets tray)' })
    setBusy(null)
    setNote(r.ok ? `Removed "${name}" — source file kept` : `Remove failed: ${r.error?.message ?? r.error?.code ?? 'error'}`)
    setTimeout(() => setNote(null), 3500)
  }

  const hasProject = !!project
  const hasAssets = items.length > 0
  const assetMenuRow = assetMenu ? assetRows.find((asset) => asset.id === assetMenu.assetId) ?? null : null
  const assetMenuUsed = assetMenuRow ? usage.get(assetMenuRow.id) ?? 0 : 0

  return (
    <>
    <section className="panel assets" data-panel="assets" data-cut-panel="assets">
      <div className="panel__header assets__header">
        <span>Assets <small className="assets__scope">this project</small></span>
        <span className="assets__count" data-cut-asset-count={items.length}>
          {items.length === assetCount
            ? `${items.length} item${items.length === 1 ? '' : 's'}`
            : `${items.length} of ${assetCount}`}
        </span>
        <span className="assets__header-spacer" />
        <label
          className="assets__proxytoggle"
          title="Generate the editing proxy on import. Turn OFF for HEAVY files (large FHD / multi-GB raw) so they import instantly — editing uses the source (final render quality is unaffected); smooth proxy playback is unavailable until a proxy exists."
        >
          <input
            type="checkbox"
            data-cut-proxy-toggle
            checked={proxiesOn}
            onChange={(e) => {
              setProxiesOn(e.target.checked)
              setGenerateProxies(e.target.checked)
            }}
          />
          Proxies
        </label>
        <button
          className="assets__import"
          data-cut-action="import-asset"
          disabled={!hasProject || busy === 'import'}
          onClick={() => void browseImport()}
          title="Browse for a video, audio, or image; also keep it in your Library"
        >
          {busy === 'import' ? 'Importing…' : '+ Import'}
        </button>
        {/* Generated-media placement: Generate has its own Library-adjacent tab; this is a shortcut. */}
        <button
          className="assets__generate"
          data-cut-action="generate-asset"
          disabled={!hasProject}
          onClick={() => document.dispatchEvent(new CustomEvent('cut:open-generate', { detail: { tab: 'media' } }))}
          title="Create an image or short video from a prompt with your chosen local agent"
        >
          <Icon name="effect" size={14} tone="brand" /> Generate
        </button>
      </div>

      {hasProject && (
        <div className="assets__filters" data-cut-asset-filters>
          <input
            className="assets__filter-input"
            data-cut-asset-filter
            placeholder="filter by name…"
            value={filterText}
            onChange={(e) => { setFilterText(e.target.value); setActiveBin(null) }}
          />
          <span className="assets__kind-chips" role="group" aria-label="Kind filter">
            {(['', 'video', 'audio', 'image'] as const).map((k) => (
              <button
                key={k || 'all'}
                type="button"
                className={`assets__chip${filterKind === k ? ' assets__chip--on' : ''}`}
                data-cut-asset-kind-filter={k || 'all'}
                onClick={() => { setFilterKind(k); setActiveBin(null) }}
              >
                {k || 'all'}
              </button>
            ))}
            <button
              type="button"
              className={`assets__chip${filterUnused ? ' assets__chip--on' : ''}`}
              data-cut-asset-unused-filter
              title="Only assets not used by any timeline clip"
              onClick={() => { setFilterUnused((v) => !v); setActiveBin(null) }}
            >
              unused
            </button>
            <button
              type="button"
              className={`assets__chip${filterLarge ? ' assets__chip--on' : ''}`}
              data-cut-asset-resolution-filter
              title="Only high-resolution clips, including 4K phone and camera footage"
              onClick={() => { setFilterLarge((v) => !v); setActiveBin(null) }}
            >
              4K+
            </button>
            <button
              type="button"
              className={`assets__chip${filterOffline ? ' assets__chip--on' : ''}`}
              data-cut-asset-offline-filter
              title="Only missing source files that need relinking"
              onClick={() => { setFilterOffline((v) => !v); setActiveBin(null) }}
            >
              missing
            </button>
            <button
              type="button"
              className={`assets__chip${filterRecent ? ' assets__chip--on' : ''}`}
              data-cut-asset-recent-filter
              title="Only files modified in the last 30 days"
              onClick={() => { setFilterRecent((v) => !v); setActiveBin(null) }}
            >
              recent
            </button>
            <button
              type="button"
              className={`assets__chip${filterNeedsAction ? ' assets__chip--on assets__chip--attention' : ''}`}
              data-cut-asset-attention-filter
              title="Only clips that need a user action, such as relink or proxy attention"
              onClick={() => { setFilterNeedsAction((v) => !v); setActiveBin(null) }}
            >
              needs action
            </button>
          </span>
          {(filterText || filterKind || filterUnused || filterLarge || filterOffline || filterRecent) && (
            <button
              type="button"
              className="assets__chip assets__chip--save"
              data-cut-action="bin-save"
              title="Save the current filter as a smart bin; membership stays live"
              onClick={() => void saveBin()}
            >
              ★ save bin
            </button>
          )}
          {bins.length > 0 && (
            <span className="assets__bins" role="group" aria-label="Smart bins" data-cut-bins={bins.length}>
              {bins.map((b) => (
                <span key={b.name} className={`assets__bin${activeBin === b.name ? ' assets__bin--on' : ''}`} data-cut-bin={b.name} {...(activeBin === b.name ? { 'data-cut-bin-active': b.name } : {})}>
                  <button
                    type="button"
                    className="assets__bin-btn"
                    data-cut-bin-open={b.name}
                    title={`Smart bin — ${binCriteriaLabel(b)}`}
                    onClick={() => applyBin(activeBin === b.name ? null : b)}
                  >
                    {b.name} <span className="assets__bin-count" data-cut-bin-count={b.match_count}>{b.match_count}</span>
                  </button>
                  {activeBin === b.name && (
                    <button
                      type="button"
                      className="assets__bin-del"
                      data-cut-action="bin-delete"
                      data-cut-bin-delete={b.name}
                      title="Delete this smart bin (assets are untouched)"
                      onClick={() => void deleteBin(b.name)}
                    >
                      ×
                    </button>
                  )}
                </span>
              ))}
            </span>
          )}
        </div>
      )}
      {hasProject && (
        <button
          type="button"
          className="assets__timeline-import"
          data-cut-import-otio
          onClick={() => document.dispatchEvent(new CustomEvent('cut:import-otio'))}
          title="Import an OpenTimelineIO .otio file and preview it before replacing this timeline"
        >
          <Icon name="import" size={14} /> Import timeline (.otio)
        </button>
      )}
      {hasProject && assetCount > 0 && (
        <section
          className={`assets__health assets__health--${mediaHealth.level}`}
          data-cut-media-health
          data-cut-media-health-status={mediaHealth.level}
          title={mediaHealth.hint}
        >
          <div className="assets__health-main">
            <span className="assets__health-dot" aria-hidden="true" />
            <div className="assets__health-copy">
              <strong data-cut-media-health-title>{mediaHealth.title}</strong>
              <span data-cut-media-health-hint>{mediaHealth.hint}</span>
            </div>
          </div>
          <div className="assets__health-actions">
            {firstOfflineRow && (
              <button
                type="button"
                className="assets__health-btn assets__health-btn--primary"
                data-cut-media-health-relink-first={firstOfflineRow.id}
                disabled={busy === firstOfflineRow.id || relinkingAssetId === firstOfflineRow.id}
                onClick={() => void relinkAsset(firstOfflineRow.id)}
                title="Browse to the moved source file"
              >
                {relinkingAssetId === firstOfflineRow.id ? '…' : 'Relink'}
              </button>
            )}
            <button
              type="button"
              className="assets__health-btn"
              data-cut-media-health-proxies
              data-cut-media-health-proxies-on={proxiesOn ? 'true' : 'false'}
              onClick={() => {
                const next = !proxiesOn
                setProxiesOn(next)
                setGenerateProxies(next)
              }}
              title={proxiesOn ? 'Turn off proxy generation for future imports' : 'Turn on proxy generation for future imports'}
            >
              {proxiesOn ? 'Proxies on' : 'Turn on proxies'}
            </button>
            <button
              type="button"
              className="assets__health-btn"
              data-cut-media-health-refresh
              onClick={() => void refreshOfflineNow()}
              title="Check whether source files are still available"
            >
              Refresh
            </button>
            <button
              type="button"
              className="assets__health-btn assets__health-btn--ghost"
              data-cut-media-health-manual
              onClick={() => openCutManual('cut.left.media_health')}
              title="Open the Media Health manual section"
            >
              ?
            </button>
          </div>
          <div className="assets__health-dimensions" role="list" aria-label="Media readiness">
            {mediaDimensions.map(([key, label, dimension]) => (
              <div
                key={key}
                role="listitem"
                className={`assets__health-dimension assets__health-dimension--${dimension.state}`}
                data-cut-informational="true"
                data-cut-media-health-dimension={key}
                data-cut-media-health-dimension-state={dimension.state}
                aria-label={`${label}: ${dimension.value}. ${dimension.detail}`}
                title={dimension.detail}
              >
                <span>{label}</span>
                <strong>{dimension.value}</strong>
              </div>
            ))}
          </div>
          <details className="assets__health-advanced" data-cut-media-health-advanced>
            <summary data-cut-media-health-advanced-toggle>Advanced</summary>
            <dl>
              <div><dt>Assets</dt><dd>{mediaHealth.total}</dd></div>
              <div><dt>Videos</dt><dd>{mediaHealth.videos}</dd></div>
              <div><dt>Missing</dt><dd>{mediaHealth.offline}{mediaHealth.usedOffline > 0 ? ` (${mediaHealth.usedOffline} on timeline)` : ''}</dd></div>
              <div><dt>Source playback</dt><dd>{mediaHealth.sourceOnly}</dd></div>
              <div><dt>Large source clips</dt><dd>{mediaHealth.heavySource}</dd></div>
              <div><dt>Proxy ready</dt><dd>{mediaHealth.proxyReady}</dd></div>
              <div><dt>Needs action</dt><dd>{mediaHealth.needsAction}</dd></div>
              <div><dt>Filmstrips pending</dt><dd>{mediaHealth.filmstripMissing}</dd></div>
              <div><dt>Analysis</dt><dd>{mediaHealth.analysis}</dd></div>
            </dl>
          </details>
        </section>
      )}
      <div className="panel__body assets__body">
        {!hasProject && <div className="assets__empty">Open or create a project in the Projects tab to begin.</div>}
        {hasProject && assetCount === 0 && (
          <button
            type="button"
            className="assets__empty assets__empty--cta"
            data-cut-import-cta
            onClick={() => void browseImport()}
          >
            ⬑ Import media — browse for files; clips appear here, drag them onto the timeline.
          </button>
        )}
        {hasProject && assetCount > 0 && !hasAssets && (
          <div className="assets__empty" data-cut-asset-filter-empty>
            No assets match the current filter{activeBin ? ` (bin "${activeBin}")` : ''}.
          </div>
        )}
        {note && <div className="assets__note" data-cut-asset-note>{note}</div>}
        {hasAssets && (
          <ul className="assets__list">
            {items.map(({ id, path, film, probe }) => {
              const used = usage.get(id) ?? 0
              const dims = probe.width && probe.height ? `${probe.width}×${probe.height}` : null
              const isOffline = offline.has(id)
              const readiness = readinessById.get(id)
              const libraryId = libraryIdFromAssetHash(project?.assets[id]?.hash)
              const inLibrary = !!libraryId && libraryIds.has(libraryId)
              return (
                <li
                  key={id}
                  className={`assets__card${isOffline ? ' assets__card--offline' : ''}${readiness?.needsAction ? ' assets__card--attention' : ''}`}
                  data-cut-asset-card={id}
                  data-cut-asset-kind={probe.kind ?? 'video'}
                  data-cut-asset-readiness={readiness?.level}
                  {...(isOffline ? { 'data-cut-asset-offline': id } : {})}
                  {...(readiness?.needsAction ? { 'data-cut-asset-needs-action': id } : {})}
                  onPointerDown={(e) => startPointerDrag(e, { asset: id, kind: probe.kind ?? 'video', name: mediaBasename(path) })}
                  onMouseDown={(e) => startMouseDrag(e, { asset: id, kind: probe.kind ?? 'video', name: mediaBasename(path) })}
                  onContextMenu={(event) => {
                    event.preventDefault()
                    setAssetMenu({ x: event.clientX, y: event.clientY, assetId: id })
                  }}
                  onDragStart={preventNativeDrag}
                  title={
                    isOffline
                      ? `${path}\nSOURCE FILE MISSING — renders will fail. Relink to its new location.`
                      : `${path}\n${readiness?.hint ?? 'Drag onto the timeline, or use Add at playhead.'}\nDrag onto the timeline, or use Add at playhead.`
                  }
                >
                  <AssetThumb assetId={id} kind={probe.kind} film={film} />
                  <span className="assets__meta">
                    <span className="assets__name" title={mediaBasename(path)}>{mediaBasename(path)}</span>
                    <span className="assets__sub">
                      {isOffline && <span className="assets__offline" title={`Source file missing: ${path}`}>offline</span>}
                      <span className="assets__kind">{probe.kind ?? 'video'}</span>
                      <span className="assets__dot">·</span>
                      <span className="assets__dur">{shortDur(probe.duration_ms)}</span>
                      {dims && <><span className="assets__dot">·</span><span className="assets__dims">{dims}</span></>}
                      {used > 0 && <span className="assets__used" title={`${used} clip${used === 1 ? '' : 's'} on the timeline`}>on timeline ×{used}</span>}
                      {inLibrary && <span className="assets__library-badge" data-cut-asset-in-library title="This source is also available in the cross-project Library">In Library</span>}
                      {readiness?.badges.map((badge) => (
                        <span
                          key={badge.label}
                          className={`assets__readiness-badge assets__readiness-badge--${badge.tone}`}
                          data-cut-asset-readiness-badge={badge.label}
                          title={badge.title}
                        >
                          {badge.label}
                        </span>
                      ))}
                    </span>
                  </span>
                  {isOffline && (
                    <button
                      className="assets__relink"
                      data-cut-action="relink-asset"
                      data-cut-asset-relink={id}
                      disabled={busy === id || relinkingAssetId === id}
                      onPointerDown={(e) => e.stopPropagation()}
                      onClick={() => void relinkAsset(id)}
                      title="The source file is missing — browse to its new location. Same file keeps editing media; different file regenerates it."
                    >
                      {busy === id || relinkingAssetId === id ? '…' : 'Relink…'}
                    </button>
                  )}
                  {(probe.kind === 'video' || probe.kind === 'audio') && !isOffline && (
                    <button
                      type="button"
                      className="assets__source-monitor"
                      data-cut-action="open-source-monitor"
                      data-cut-source-monitor-open={id}
                      onPointerDown={(e) => e.stopPropagation()}
                      onClick={() => { setSourceMonitorAtMs(0); setSourceMonitorId(id) }}
                      title="Open in Source monitor"
                      aria-label={`Open ${mediaBasename(path)} in Source monitor`}
                    >
                      <Icon name="screenPlay" size={14} />
                    </button>
                  )}
                  <button
                    className="assets__insert"
                    data-cut-action="insert-asset"
                    disabled={busy === id}
                    onPointerDown={(e) => e.stopPropagation()}
                    onClick={() => void insertAtPlayhead(id, probe)}
                    title="Adds the asset on the base timeline at the playhead. Alt-drag or drop on an overlay lane to place video on top."
                  >
                    {busy === id ? '…' : 'Add at playhead'}
                  </button>
                  <button
                    className="assets__remove"
                    data-cut-action="remove-asset"
                    data-cut-asset-remove={id}
                    disabled={busy === id}
                    onPointerDown={(e) => e.stopPropagation()}
                    onClick={() => void removeAsset(id, mediaBasename(path), used)}
                    title={
                      used > 0
                        ? `Used by ${used} clip${used === 1 ? '' : 's'} — delete those first, then remove`
                        : 'Remove from project (deletes proxy/thumbnails; keeps the source file)'
                    }
                    aria-label={`Remove ${mediaBasename(path)} from project`}
                  >
                    🗑
                  </button>
                </li>
              )
            })}
          </ul>
        )}
        {hasAssets && (
          <p className="assets__hint" data-cut-asset-hint>
            Drag to the base track to split and insert. Alt-drag or drop on an overlay lane to place on top.
          </p>
        )}
      </div>
    </section>
    {assetMenu && (
      <AssetContextMenu
        menu={assetMenu}
        asset={assetMenuRow && {
          id: assetMenuRow.id,
          name: mediaBasename(assetMenuRow.path),
          kind: assetMenuRow.probe.kind,
          offline: offline.has(assetMenuRow.id),
          used: assetMenuUsed,
        }}
        busy={busy !== null || relinkingAssetId === assetMenuRow?.id}
        onOpenSource={(assetId) => { setSourceMonitorAtMs(0); setSourceMonitorId(assetId) }}
        onAddAtPlayhead={(assetId) => {
          const asset = assetRows.find((candidate) => candidate.id === assetId)
          if (asset) void insertAtPlayhead(asset.id, asset.probe)
        }}
        onRelink={(assetId) => { void relinkAsset(assetId) }}
        onRemove={(assetId) => {
          const asset = assetRows.find((candidate) => candidate.id === assetId)
          if (asset) void removeAsset(asset.id, mediaBasename(asset.path), usage.get(asset.id) ?? 0)
        }}
        onClose={() => setAssetMenu(null)}
      />
    )}
    {/* floating drag ghost (portal to body so panel overflow never clips it) */}
    {ghost && createPortal(
      <div className="assets__ghost" data-cut-asset-ghost style={{ left: ghost.x + 12, top: ghost.y + 12 }}>
        <span className={`assets__icon assets__icon--${ghost.kind}`}><KindIcon kind={ghost.kind} /></span>
        <span className="assets__ghost-name">{ghost.name}</span>
      </div>,
      document.body,
    )}
    {sourceMonitorAsset && project && (
      <SourceMonitor
        key={`${sourceMonitorAsset.id}:${sourceMonitorAtMs}`}
        asset={sourceMonitorAsset}
        project={project}
        playheadMs={playheadMs}
        initialMs={sourceMonitorAtMs}
        onClose={closeSourceMonitor}
      />
    )}
    </>
  )
}
