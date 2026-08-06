// panels/Timeline — tracks, clips, gaps, markers, captions, playhead, ruler,
// zoom, drag-trim/move/select.
//
// Interaction math implements the editor timeline contract:
//    one time↔px transform (layout.ts), device-pixel-aligned lines
//    5px/500ms drag-vs-click state machine; lastGestureWasDrag survives
//       mouseup→click ordering; drag-back-to-origin cancels
//    trim handles with outward overhang + per-side cursors
//    zoom-invariant 10-screen-px snap, Shift bypass per mousemove,
//       self-exclusion, snap guide line
//   /single non-passive capture wheel owner: ctrl=zoom (rAF-batched,
//       exp factor, cursor-anchored via the viewport-offset identity),
//       wheel=horizontal pan, shift=vertical
//    imperative playhead style writes (no clip re-renders per tick),
//       playback recenter-on-exit, edge auto-scroll while gesturing
//   NO optimistic commit: gestures render a ghost; release dispatches the
//       verb; committed state re-renders only when the project snapshot
//       (driven by op_applied) replaces it
//   clip components are React.memo'd; ruler ticks are windowed
//
// All mutations go through verbs (edit.trim/move/split/ripple_delete,
// edit.add_marker, edit.restore, project.save) — the panel never owns truth.
// GLOBAL-scope keys owned here: +/-/= and Ctrl/Cmd+=/- zoom,
// Shift+Z fit-to-window, S / Ctrl/Cmd+B split, B razor, I/O mark export-range
// in/out, Del/Backspace ripple-delete (Alt/Shift+Del lift), M marker, [ / ] seek
// prev/next marker (edit.seek_marker → onSeek), Ctrl+Z restore-last, Ctrl+S save.
// N snap toggle. Transport keys
// (Space/JKL/arrows/Home/End) live in panels/Preview (the playback owner).
// Rail-scope convention: a focused review rail sets
// document.documentElement.dataset.cutKbscope='rail'; we skip keys then.
//
// Human co-edit affordances add to this panel — every gesture still
// becomes an honest op through an EXISTING verb with a verb-shaped rationale:
//   • vertical drag-move = cross-track edit.move, but ONLY onto a same-KIND
//     track (the engine refuses mismatched kinds; we disable the drop and
//     keep the source track rather than send a verb that would error);
//   • a ripple-vs-lift readout reflects the real edit.move ripple flag; normal
//     drags move an exact linked A/V pair while Alt also opens time elsewhere;
//   • a live duration tooltip during trim, the snapped width readable as the
//     gesture commits to edit.trim;
//   • click-mode razor on the B key (NLE-canonical blade) + the toolbar toggle:
//     clicking a clip in razor mode splits it at the cursor via edit.split;
//   • split at the playhead on S and the canonical Cmd/Ctrl+B (conventions ref);
//   • a snapping toggle on the N key (Shift still bypasses per-mousemove).
//
// Callers: App.tsx. Dependencies: lib/client (verbs), layout.ts, timeline.css.

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { matchesAction } from '../../lib/keymap'
import { shouldIgnoreGlobalShortcut } from '../../lib/dom'
import { callVerb, type Marker, type Project } from '../../lib/client'
import type { MarkerColor } from '../../lib/clientModel'
import { runUserVerb } from '../../lib/userActionFeedback'
import { cycleTimeDisplay, useTimeDisplay } from '../../lib/timedisplay'
import {
  BASE_PPS,
  CLICK_MAX_MS,
  DRAG_THRESHOLD_PX,
  EDGE_SCROLL_MAX_PX,
  EDGE_SCROLL_ZONE_PX,
  MAX_ZOOM,
  RAIL_W,
  RULER_H,
  TRACK_HEIGHT,
  WHEEL_DELTA_CAP,
  WHEEL_PAN_CLAMP_PX,
  ZOOM_KEY_FACTOR,
  centeredLineLeft,
  dragRationale,
  imageAssetIds,
  laidToEditorialMs,
  layoutTrack,
  minZoomFor,
  msToPx,
  projectDurationMs,
  pxToMs,
  resolveSnap,
  rulerTicks,
  shortDur,
  snapCandidates,
  timecode,
  trackRowAtY,
  trackRows,
  trackSeams,
  type LaidItem,
  type RippleMode,
  type Seam,
  type TrackRow,
} from './layout'
import CrossfadePopover from './CrossfadePopover'
import MarkerContextMenu, { type MarkerMenuState } from './MarkerContextMenu'
import PasteAttributesDialog from './PasteAttributesDialog'
import TrimPopover from './TrimPopover'
import TimelineOverlays, { type AssetDropState } from './TimelineOverlays'
import TimelineToolbar from './TimelineToolbar'
import TimelineRuler from './TimelineRuler'
import TimelineEmptyState from './TimelineEmptyState'
import TimelineGestureHud, { type TimelineGestureHudState } from './TimelineGestureHud'
import TimelineGuides from './TimelineGuides'
import TimelineTrackRow from './TimelineTrackRow'
import ClipContextMenu, { type AssetPickMode, type ClipMenuState } from './ClipContextMenu'
import { useTimelineClipActions } from './useTimelineClipActions'
import { useWindowedThumbnails } from './useWindowedThumbnails'
import { useTimelineRangeSaves } from './useTimelineRangeSaves'
import { useTimelineAssetDrop } from './useTimelineAssetDrop'
import { sourceTrimAtTimelinePosition } from './rippleTrim'
import { mediaBasename } from '../../lib/mediaPath'
import './timeline.css'

export interface TimelineProps {
  project: Project | null
  playheadMs: number
  selectedClipIds: string[]
  headOpId: string
  onSeek: (atMs: number) => void
  onSelect: (clipIds: string[]) => void
  /** Explicit export span [in,out] painted by dragging the ruler — the exact
   *  range the export controls use (null = nothing selected). */
  exportRange: [number, number] | null
  onExportRange: (range: [number, number] | null) => void
  // --- Copy/Cut/Paste (clipboard lives in App.tsx; the context menu drives it
  //     through these callbacks so copy/cut/paste behave identically to the
  //     Ctrl/Cmd+C/X/V shortcuts). -------------------------------------------
  /** Copy a clip into the App clipboard (returns true if something was copied). */
  onCopyClip: (clipId: string) => boolean
  /** Cut a clip (copy + ripple-delete it + its linked sibling audio). */
  onCutClip: (clipId: string) => void
  /** Paste the clipboard's clip at the playhead on the active/selected track. */
  onPasteClip: () => void
  /** Whether the clipboard holds a clip (drives the Paste-disabled state). */
  clipboardHasContent: boolean
  clipboardClipId: string | null
}

// ---------------------------------------------------------------------------
// Gesture session (one at a time; lives in a ref — never re-renders per move)
// ---------------------------------------------------------------------------

// move/trim-l/trim-r/scrub/lane-seek form the base set; later additions include
// cap-move/cap-trim-l/cap-trim-r (caption clips via captions.set_range) and
// marker (ruler marker drag via edit.move_marker).
type GestureMode =
  | 'move'
  | 'trim-l'
  | 'trim-r'
  | 'scrub'
  | 'lane-seek'
  | 'cap-move'
  | 'cap-trim-l'
  | 'cap-trim-r'
  | 'marker'
  // Trim-tool gestures (edit.slip / edit.slide_edit / edit.roll — the same
  // verbs the TrimPopover steppers dispatch, so drag and popover stay identical):
  | 'slip'
  | 'slide'
  | 'roll-l'
  | 'roll-r'

interface Gesture {
  mode: GestureMode
  item?: LaidItem
  /** marker drag target id + its at_ms at grab time (mode === 'marker'). */
  marker?: { id: string; startAtMs: number }
  startX: number
  startY: number
  startT: number
  /** move/cap-move: keeps the grab point under the cursor (timeline behavior contract). */
  grabOffsetMs: number
  dragging: boolean
  lastClientX: number
  lastClientY: number
  shift: boolean
  /** Alt held this move — cross-track move ripple modifier. */
  alt: boolean
}

/** Ghost = proposed in-flight state, rendered until op_applied.
 * trackId is the DESTINATION track (may differ from the clip's source track on
 * a vertical drag-move); srcTrackId is where it came from (for the verb +
 * track-changed detection). */
interface Ghost {
  trackId: string
  srcTrackId: string
  startMs: number
  durMs: number
  /** true once the verb is dispatched and we await the op_applied snapshot. */
  pending: boolean
}

// ---------------------------------------------------------------------------
// Memoized clip — playhead ticks must NOT re-render clips (timeline behavior contract)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The panel
// ---------------------------------------------------------------------------

export default function Timeline({ project, playheadMs, selectedClipIds, headOpId, onSeek, onSelect, exportRange, onExportRange, onCopyClip, onCutClip, onPasteClip, clipboardHasContent, clipboardClipId }: TimelineProps) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const playheadRef = useRef<HTMLDivElement>(null)

  const [zoom, setZoom] = useState(1)
  const [viewW, setViewW] = useState(1200)
  // Scroll window for ruler tick windowing (rAF-throttled from onScroll).
  const [scrollX, setScrollX] = useState(0)
  const [ghost, setGhost] = useState<Ghost | null>(null)
  const [snapLineMs, setSnapLineMs] = useState<number | null>(null)
  const [draggingClipId, setDraggingClipId] = useState<string | null>(null)
  // Snapping toggle (magnet). Shift still bypasses per-mousemove;
  // this is the persistent on/off the user controls (N key / toolbar button).
  const [snapEnabled, setSnapEnabled] = useState(true)
  // Razor click-mode — cursor becomes a blade; clicking a clip splits it
  // at the cursor time via edit.split (alongside the S key).
  const [razorMode, setRazorMode] = useState(false)
  // The TRIM TOOL mode (FCP T / Resolve trim-mode convention) — a tool
  // state, NOT a drag modifier, because Alt (ripple-move) and Ctrl/Cmd
  // (toggle-select) are TAKEN on clip drags. `t` cycles select→slip→slide→
  // roll; Escape returns to select. In slip/slide the clip BODY drag slips/
  // slides; in roll the EDGE drag rolls the shared cut.
  const [trimTool, setTrimTool] = useState<'select' | 'slip' | 'slide' | 'roll'>('select')
  // Live readout during a gesture — trim duration tooltip, drag target /
  // ripple-vs-lift hint. Positioned at the cursor; pointer-events none.
  const [hud, setHud] = useState<TimelineGestureHudState | null>(null)
  // Which track row the cursor is over during a move (drop highlight) and
  // whether the engine would accept the drop (same-kind only).
  const [dropTrackId, setDropTrackId] = useState<string | null>(null)
  const [dropInvalid, setDropInvalid] = useState(false)
  // Marker drag ghost — the proposed at_ms while dragging a marker
  // triangle on the ruler (rendered as a ghost triangle until op_applied).
  const [markerGhost, setMarkerGhost] = useState<{ id: string; atMs: number } | null>(null)
  // The seam (clip-to-clip cut) the user is editing a crossfade on
  // — selected by clicking a seam handle; opens the duration popover.
  const [activeSeam, setActiveSeam] = useState<Seam | null>(null)
  // Close the crossfade popover on an outside click or Escape (it had no
  // dismiss-on-click-away, so it stayed open after editing.
  // Exclude clicks on the popover itself and on a seam handle (which toggles it).
  useEffect(() => {
    if (!activeSeam) return
    const onDown = (e: MouseEvent) => {
      const t = e.target instanceof HTMLElement ? e.target : null
      if (t?.closest('.tl-xfade-pop') || t?.closest('[data-cut-seam]')) return
      setActiveSeam(null)
    }
    const onEsc = (e: KeyboardEvent) => { if (e.key === 'Escape') setActiveSeam(null) }
    // CAPTURE phase: fires on the way DOWN, before any element's bubble-phase
    // stopPropagation (e.g. the resize dividers stop it), so clicking ANY outside
    // element still dismisses the popover — a real-mouse click on a divider did
    // not close it with a bubble-phase listener.
    document.addEventListener('mousedown', onDown, true)
    document.addEventListener('keydown', onEsc)
    return () => {
      document.removeEventListener('mousedown', onDown, true)
      document.removeEventListener('keydown', onEsc)
    }
  }, [activeSeam])
  // Asset drag-in (from the Assets tray): the proposed drop point while dragging
  // an asset card over the timeline — drives the insertion line + track tint.
  // null = no asset hovering. Resolved on drop → ONE edit.insert.
  const [assetDnd, setAssetDnd] = useState<AssetDropState | null>(null)

  const durationMs = useMemo(() => projectDurationMs(project), [project])
  // Still-image assets (probe kind=image) → photo tint on their clips.
  const imageAssets = useMemo(() => imageAssetIds(project), [project])
  // assetId → {strip url, asset duration} for clips that show "frames in the
  // time bar" (video assets with a built filmstrip + a known duration).
  const filmstrips = useMemo(() => {
    const m = new Map<string, { url: string; assetDurMs: number }>()
    for (const [id, a] of Object.entries(project?.assets ?? {})) {
      // Any asset with a strip: video (sliced by duration) OR image (a single
      // thumbnail tiled; assetDurMs unused → 0). durMs guards only video slicing.
      if (a.filmstrip) m.set(id, { url: `/${a.filmstrip}`, assetDurMs: (a.probe as { duration_ms?: number } | undefined)?.duration_ms ?? 0 })
    }
    return m
  }, [project])
  const assetLabels = useMemo(() => new Map(
    Object.entries(project?.assets ?? {}).map(([id, asset]) => [id, mediaBasename(asset.path)]),
  ), [project])
  const laidTracks = useMemo(
    () => (project ? project.tracks.map((t) => ({ track: t, items: layoutTrack(t, imageAssets) })) : []),
    [project, imageAssets],
  )
  // Media↔media seams per track are the crossfade affordance points.
  // Keyed by track id so each lane draws its own seam handles.
  const seamsByTrack = useMemo(() => {
    const map: Record<string, Seam[]> = {}
    for (const { track, items } of laidTracks) map[track.id] = trackSeams(items)
    return map
  }, [laidTracks])
  // First video track = the base canvas; every later video track is an
  // OVERLAY (track order = stacking order, edit.transform semantics).
  const baseVideoId = useMemo(() => project?.tracks.find((t) => t.kind === 'video')?.id, [project])
  const allItems = useMemo(() => laidTracks.flatMap((t) => t.items), [laidTracks])

  const windowedTiles = useWindowedThumbnails({ allItems, filmstrips, zoom, viewW, scrollX })

  const markers: Marker[] = project?.markers ?? []
  const fps = project?.settings.fps ?? 30
  // Shared time-readout mode (ms / frames / SMPTE) — drives the ruler labels +
  // the tc chip; click the chip to cycle. Receipt rationales stay raw-ms.
  const timeMode = useTimeDisplay()
  const minZoom = minZoomFor(durationMs, viewW - RAIL_W)
  // Track-row geometry for vertical drag-move drop resolution + the
  // ripple-vs-lift feedback (base of a kind = ripple, overlay/extra = lift).
  const rows = useMemo(() => trackRows(project), [project])

  const contentW = Math.max(msToPx(durationMs, zoom) + 240, viewW - RAIL_W)
  const tracksH = laidTracks.reduce((h, { track }) => h + (TRACK_HEIGHT[track.kind] ?? 40) + 1, 0)

  // configRef: gesture/wheel handlers read CURRENT values without re-binding
  // listeners every render.
  const cfg = useRef({ zoom, durationMs, allItems, markers, playheadMs, fps, selectedClipIds, onSeek, onSelect, minZoom, rows, snapEnabled, razorMode, trimTool, exportRange, onExportRange })
  cfg.current = { zoom, durationMs, allItems, markers, playheadMs, fps, selectedClipIds, onSeek, onSelect, minZoom, rows, snapEnabled, razorMode, trimTool, exportRange, onExportRange }
  const { savingRange, savingGif, saveNote, onSaveRange, onSaveGif } = useTimelineRangeSaves(cfg)
  const {
    syncNote,
    showVerbFailure,
    addTrack,
    rippleTrimAtPlayhead,
    deleteSelection,
    removeItemById,
    removeTrackById,
    splitItemAt,
    splitAtPlayhead,
    fadeItem,
    trimItemTo,
    reverseItem,
    freezeItem,
    stabilizeItem,
    speedItem,
    crossfadeAdjacent,
    muteItem,
    cleanVoiceItem,
    blurFacesItem,
    syncByAudio,
    detachAudioItem,
    splitEditItem,
    replaceClipSource,
    fitToFillAdjacent,
    nestSelection,
    cutToBeat,
    multicamSwitch,
    applySpeed,
    applyCrossfade,
  } = useTimelineClipActions({ cfg, setActiveSeam })

  // --- measure viewport + initial fit-zoom ---------------------------------
  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    let frame = 0
    const commitWidth = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(() => setViewW((current) => current === el.clientWidth ? current : el.clientWidth)) }
    const ro = new ResizeObserver(commitWidth); ro.observe(el); commitWidth()
    return () => { cancelAnimationFrame(frame); ro.disconnect() }
  }, [])

  const didFit = useRef(false)
  useEffect(() => {
    // First project load: fit the whole timeline to ~80% of the lane width.
    if (!project || didFit.current) return
    didFit.current = true
    const lane = Math.max(200, viewW - RAIL_W)
    const fit = (lane * 0.8) / ((durationMs / 1000) * 50)
    setZoom(Math.min(MAX_ZOOM, Math.max(minZoom, fit)))
  }, [project, durationMs, viewW, minZoom])

  // --- ghost lifecycle: clear when the snapshot (op_applied) replaces it ----
  useEffect(() => {
    setGhost((g) => (g?.pending ? null : g))
  }, [project])
  useEffect(() => {
    // Safety: a pending ghost never outlives 5s (failed verb / lost event).
    if (!ghost?.pending) return
    const t = setTimeout(() => setGhost(null), 5000)
    return () => clearTimeout(t)
  }, [ghost])

  // --- coordinate helpers ----------------------------------------------------
  /** clientX → timeline ms (clamped to [0, duration]). */
  const clientXToMs = useCallback((clientX: number): number => {
    const el = scrollRef.current
    if (!el) return 0
    const rect = el.getBoundingClientRect()
    const contentX = clientX - rect.left + el.scrollLeft - RAIL_W
    const c = cfg.current
    return Math.min(c.durationMs, Math.max(0, pxToMs(contentX, c.zoom)))
  }, [])

  /** clientY → the track row under the cursor (or null above/below the lanes).
   * The track area starts at RULER_H below the scroll viewport top and scrolls
   * vertically with el.scrollTop. Used for vertical drag-move drop resolution. */
  const clientYToRow = useCallback((clientY: number): TrackRow | null => {
    const el = scrollRef.current
    if (!el) return null
    const rect = el.getBoundingClientRect()
    const yInTrackArea = clientY - rect.top + el.scrollTop - RULER_H
    return trackRowAtY(cfg.current.rows, yInTrackArea)
  }, [])

  const isTrackLocked = useCallback((trackId: string): boolean => {
    return !!cfg.current.rows.find((r) => r.id === trackId)?.locked
  }, [])

  // Convert a LAID drop position to the EDITORIAL at_ms edit.insert keys on
  // (engine cumulative-track cursor). Conversion runs against the drop row's
  // own laid items when it has any, else the base video track (the default
  // placement target); an empty timeline is identity (the clocks start
  // aligned). Editorial clocks stay in lockstep across linked-edited tracks
  // (a crossfade shortens LAID time only), so one converted value serves the
  // linked video+audio pair placeLinkedAV inserts.
  const dropMsToEditorial = useCallback((laidMs: number, row: TrackRow | null): number => {
    const c = cfg.current
    const trackId = row && c.allItems.some((i) => i.trackId === row.id)
      ? row.id
      : c.rows.find((r) => r.kind === 'video' && r.kindIndex === 0)?.id
    if (!trackId) return Math.round(laidMs)
    return Math.round(laidToEditorialMs(c.allItems.filter((i) => i.trackId === trackId), laidMs))
  }, [])

  useTimelineAssetDrop({ scrollRef, clientXToMs, clientYToRow, setAssetDnd, dropMsToEditorial })

  // --- zoom with anchor identity (timeline behavior contract) ----------------------------
  const pendingAnchor = useRef<{ anchorMs: number; viewportOffset: number } | null>(null)
  const applyZoom = useCallback((nextZoom: number, anchorMs: number) => {
    const el = scrollRef.current
    const c = cfg.current
    const z = Math.min(MAX_ZOOM, Math.max(c.minZoom, nextZoom))
    if (el) {
      // viewportOffset = anchor's screen x BEFORE the zoom state applies.
      pendingAnchor.current = {
        anchorMs,
        viewportOffset: msToPx(anchorMs, c.zoom) - el.scrollLeft,
      }
    }
    setZoom(z)
  }, [])
  useLayoutEffect(() => {
    // After React applied the new zoom: keep the anchor stationary.
    const a = pendingAnchor.current
    const el = scrollRef.current
    if (!a || !el) return
    pendingAnchor.current = null
    const target = msToPx(a.anchorMs, zoom) - a.viewportOffset
    el.scrollLeft = Math.max(0, Math.min(target, el.scrollWidth - el.clientWidth))
  }, [zoom])

  // Fit-to-window (Shift+Z, the FCP/Resolve "zoom to fit") — same formula as the
  // first-load fit (line ~1163): scale so the whole timeline fills ~80% of the
  // lane, clamped to [minZoom, MAX_ZOOM], and scroll back to the start so the
  // entire project is visible at once. BASE_PPS is px-per-second at zoom=1.
  const fitToWindow = useCallback(() => {
    const el = scrollRef.current
    const c = cfg.current
    const lane = Math.max(200, (el?.clientWidth ?? RAIL_W) - RAIL_W)
    const fit = (lane * 0.8) / ((Math.max(1, c.durationMs) / 1000) * BASE_PPS)
    setZoom(Math.min(MAX_ZOOM, Math.max(c.minZoom, fit)))
    if (el) el.scrollLeft = 0
  }, [])

  // --- imperative playhead (timeline behavior contract — style writes, no clip renders) --
  const gestureRef = useRef<Gesture | null>(null)
  useLayoutEffect(() => {
    const ph = playheadRef.current
    const el = scrollRef.current
    if (!ph || !el) return
    const px = msToPx(playheadMs, zoom)
    ph.style.left = `${RAIL_W + centeredLineLeft(px, 2)}px`
    // Playback follow: recenter only when the playhead exits the viewport
    // (calm jump, not continuous scroll) — skip while the user gestures.
    if (!gestureRef.current) {
      const viewLeft = el.scrollLeft
      const laneW = el.clientWidth - RAIL_W
      if (px < viewLeft - 2 || px > viewLeft + laneW + 2) {
        el.scrollLeft = Math.max(0, px - laneW / 2)
      }
    }
  }, [playheadMs, zoom, contentW])

  // --- gesture state machine (timeline behavior contract) --------------------------------
  const lastGestureWasDrag = useRef(false)
  const edgeScrollRaf = useRef(0)
  // The EXACT window handlers a gesture bound. onWinMove/onWinUp are recreated whenever
  // clientXToMs changes (zoom/scroll/layout), but endGesture's stable closure would remove
  // the FIRST-render versions — so the live handlers a later gesture added were never removed
  // and PILED UP. After "moving a clip several times" every mousemove fired N stale handlers
  // → the timeline froze. Removing via these refs makes the
  // add (beginGesture) and remove (endGesture) reference the same function, always.
  const winMoveRef = useRef<((e: MouseEvent) => void) | null>(null)
  const winUpRef = useRef<((e: MouseEvent) => void) | null>(null)

  const endGesture = useCallback(() => {
    gestureRef.current = null
    setDraggingClipId(null)
    setSnapLineMs(null)
    setHud(null) // clear the floating readout
    setDropTrackId(null) // clear the drop-track highlight
    setDropInvalid(false)
    setMarkerGhost(null) // clear the marker-drag ghost
    setMarquee(null) // clear the rubber-band rectangle
    cancelAnimationFrame(edgeScrollRaf.current)
    // Remove the EXACT handlers beginGesture bound (via refs) — not the possibly-stale
    // onWinMove/onWinUp this stable closure captured — so nothing leaks across gestures.
    if (winMoveRef.current) window.removeEventListener('mousemove', winMoveRef.current)
    if (winUpRef.current) window.removeEventListener('mouseup', winUpRef.current)
    winMoveRef.current = null
    winUpRef.current = null
  }, [])

  // --- Marquee / rubber-band select ------------------------------------------
  // A DRAG that starts on empty lane background (the 'lane-seek' gesture, whose
  // drag was previously dead — only the ≤5px click seeks) draws a selection
  // rectangle. Selection updates LIVE during the drag (the conventional
  // behavior); Shift ADDS to the selection present at mousedown. Hit-testing is
  // client-rect based against the rendered [data-cut-clip] elements — no second
  // layout model to drift; gaps are excluded via the item table.
  const [marquee, setMarquee] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null)
  const applyMarqueeSelection = useCallback((rect: { x0: number; y0: number; x1: number; y1: number }, baseSel: string[]) => {
    const c = cfg.current
    const left = Math.min(rect.x0, rect.x1)
    const right = Math.max(rect.x0, rect.x1)
    const top = Math.min(rect.y0, rect.y1)
    const bottom = Math.max(rect.y0, rect.y1)
    const selectable = new Set(c.allItems.filter((i) => i.kind !== 'gap').map((i) => i.id))
    const hit: string[] = []
    document.querySelectorAll('[data-cut-clip]').forEach((el) => {
      const id = el.getAttribute('data-cut-clip')
      if (!id || !selectable.has(id)) return
      const r = el.getBoundingClientRect()
      if (r.left < right && r.right > left && r.top < bottom && r.bottom > top) hit.push(id)
    })
    const next = baseSel.length ? [...new Set([...baseSel, ...hit])] : hit
    // Only dispatch when membership actually changed (live drags fire per move).
    const cur = c.selectedClipIds
    if (next.length !== cur.length || next.some((id) => !cur.includes(id))) c.onSelect(next)
  }, [])
  // Selection present when the marquee started — the Shift-union base.
  const marqueeBaseSel = useRef<string[]>([])

  /** Recompute the gesture's proposal from the last pointer position.
   * `bypass` = snapping released for this move: Shift held OR the magnet is
   * toggled off (snapEnabled=false). */
  const updateProposal = useCallback(() => {
    const g = gestureRef.current
    if (!g || !g.dragging) return
    const c = cfg.current
    const bypass = g.shift || !c.snapEnabled
    const mouseMs = clientXToMs(g.lastClientX)
    if (g.mode === 'lane-seek') {
      // An empty-lane DRAG rubber-bands a selection (live).
      const rect = { x0: g.startX, y0: g.startY, x1: g.lastClientX, y1: g.lastClientY }
      setMarquee(rect)
      applyMarqueeSelection(rect, g.shift ? marqueeBaseSel.current : [])
      return
    }
    if (g.mode === 'scrub') {
      // Live seek per move; element-snap while dragging (not on initial press).
      const cands = snapCandidates(c.allItems, c.markers, -1, new Set())
      const snapped = resolveSnap(mouseMs, 0, cands, c.zoom, bypass)
      setSnapLineMs(snapped.snappedTo)
      c.onSeek(snapped.ms)
      return
    }
    // --- Marker drag → edit.move_marker ------------------------------------
    if (g.mode === 'marker' && g.marker) {
      // Snap the marker to clip edges / other markers / playhead like any edge.
      const cands = snapCandidates(c.allItems, c.markers, c.playheadMs, new Set())
      const snapped = resolveSnap(mouseMs, 0, cands, c.zoom, bypass)
      setSnapLineMs(snapped.snappedTo)
      const nextMarkerGhost = { id: g.marker.id, atMs: snapped.ms }; markerGhostRef.current = nextMarkerGhost; setMarkerGhost(nextMarkerGhost)
      const delta = snapped.ms - g.marker.startAtMs
      const secs = (delta / 1000).toFixed(2)
      const signed = `${delta >= 0 ? '+' : '−'}${Math.abs(Number(secs)).toFixed(2)}s`
      setHud({ x: g.lastClientX, y: g.lastClientY, label: signed, sub: `marker → ${timecode(snapped.ms)}`, tone: 'info' })
      return
    }
    if (!g.item) return
    const it = g.item
    const exclude = new Set([it.id])
    const cands = snapCandidates(c.allItems, c.markers, c.playheadMs, exclude)
    const minDur = Math.round(1000 / c.fps)
    // --- Caption clip drag/trim → captions.set_range ------------------------
    // Captions carry an ABSOLUTE timeline range; edit.move/trim refuse them, so
    // these gestures fold to captions.set_range. cap-move shifts both edges,
    // cap-trim-l/r moves one edge. No cross-track for captions (one cap track).
    if (g.mode === 'cap-move') {
      const proposed = Math.max(0, mouseMs - g.grabOffsetMs)
      const snapped = resolveSnap(proposed, it.durMs, cands, c.zoom, bypass)
      setSnapLineMs(snapped.snappedTo)
      const delta = snapped.ms - it.startMs
      const secs = (delta / 1000).toFixed(2)
      const signed = `${delta >= 0 ? '+' : '−'}${Math.abs(Number(secs)).toFixed(2)}s`
      setHud({ x: g.lastClientX, y: g.lastClientY, label: signed, sub: 'caption retime', tone: 'info' })
      setGhost({ trackId: it.trackId, srcTrackId: it.trackId, startMs: snapped.ms, durMs: it.durMs, pending: false })
      return
    }
    if (g.mode === 'cap-trim-l') {
      const endMs = it.startMs + it.durMs
      const snapped = resolveSnap(mouseMs, 0, cands, c.zoom, bypass)
      const startMs = Math.min(endMs - minDur, Math.max(0, snapped.ms))
      setSnapLineMs(snapped.snappedTo)
      const durMs = endMs - startMs
      setHud({ x: g.lastClientX, y: g.lastClientY, label: shortDur(durMs), sub: 'caption trim in', tone: 'info' })
      setGhost({ trackId: it.trackId, srcTrackId: it.trackId, startMs, durMs, pending: false })
      return
    }
    if (g.mode === 'cap-trim-r') {
      const snapped = resolveSnap(mouseMs, 0, cands, c.zoom, bypass)
      const endMs = Math.max(it.startMs + minDur, snapped.ms)
      setSnapLineMs(snapped.snappedTo)
      const durMs = endMs - it.startMs
      setHud({ x: g.lastClientX, y: g.lastClientY, label: shortDur(durMs), sub: 'caption trim out', tone: 'info' })
      setGhost({ trackId: it.trackId, srcTrackId: it.trackId, startMs: it.startMs, durMs, pending: false })
      return
    }
    // --- Trim-tool gestures: frame-rounded horizontal delta readouts --------
    // slip: the clip's BOX never moves — the source window shifts inside it, so
    // the ghost stays pinned and the HUD carries the meaning. slide: the box
    // moves and neighbors absorb — ghost tracks horizontally. roll: the shared
    // cut moves — the snap line tracks the proposed seam.
    if (g.mode === 'slip' || g.mode === 'slide' || g.mode === 'roll-l' || g.mode === 'roll-r') {
      const frame = Math.round(1000 / c.fps)
      const rawDelta = mouseMs - clientXToMs(g.startX)
      const by = Math.round(rawDelta / frame) * frame
      const secs = (by / 1000).toFixed(2)
      const signed = `${by >= 0 ? '+' : '−'}${Math.abs(Number(secs)).toFixed(2)}s`
      if (g.mode === 'slip') {
        setHud({ x: g.lastClientX, y: g.lastClientY, label: signed, sub: 'slip source (timeline unchanged)', tone: 'info' })
        setGhost({ trackId: it.trackId, srcTrackId: it.trackId, startMs: it.startMs, durMs: it.durMs, pending: false })
      } else if (g.mode === 'slide') {
        const startMs = Math.max(0, it.startMs + by)
        setHud({ x: g.lastClientX, y: g.lastClientY, label: signed, sub: 'slide (neighbors absorb)', tone: 'info' })
        setGhost({ trackId: it.trackId, srcTrackId: it.trackId, startMs, durMs: it.durMs, pending: false })
      } else {
        const seam = g.mode === 'roll-l' ? it.startMs : it.startMs + it.durMs
        setSnapLineMs(seam + by)
        setHud({ x: g.lastClientX, y: g.lastClientY, label: signed, sub: `roll cut → ${timecode(seam + by)}`, tone: 'info' })
      }
      return
    }
    if (g.mode === 'move') {
      const proposed = Math.max(0, mouseMs - g.grabOffsetMs)
      const snapped = resolveSnap(proposed, it.durMs, cands, c.zoom, bypass)
      setSnapLineMs(snapped.snappedTo)
      // --- Vertical drop-track resolution ------------------------------------
      // The engine's edit.move requires from_kind == to_kind (captions refused).
      // Resolve the row under the cursor; accept it only if same-kind, else
      // keep the source track and flag the drop invalid (graceful disable).
      const row = clientYToRow(g.lastClientY)
      const srcKind = it.kind === 'audio' ? 'audio' : 'video'
      let destTrack = it.trackId
      let invalid = false
      let invalidReason = ''
      let trackChanged = false
      if (row && row.id !== it.trackId) {
        if (row.locked) {
          invalid = true
          invalidReason = `${row.id} is locked`
        } else if (row.kind === srcKind) {
          destTrack = row.id
          trackChanged = true
        } else {
          // hovering a different-KIND track: the engine would refuse — disable.
          invalid = true
          invalidReason = `can't drop on ${row.kind} track`
        }
      }
      setDropTrackId(row?.id ?? null)
      setDropInvalid(invalid)
      // edit.move takes a REAL ripple flag. The cursor HUD
      // shows the mode that WILL be sent: hold Alt to force an AV-sync ripple
      // at the destination (siblings + captions/markers/duck-windows shift),
      // default = linked float (the exact A/V pair moves; unrelated tracks stay
      // fixed). We surface the resolved value, not a feedback-only prediction.
      const rippleOn = g.alt
      const delta = snapped.ms - it.startMs
      const secs = (delta / 1000).toFixed(2)
      const signed = `${delta >= 0 ? '+' : '−'}${Math.abs(Number(secs)).toFixed(2)}s`
      const modeWord: RippleMode = rippleOn ? 'ripple' : 'lift'
      setHud({
        x: g.lastClientX,
        y: g.lastClientY,
        label: signed,
        sub: invalid
          ? invalidReason
          : trackChanged
            ? `→ ${destTrack} · ${modeWord}${rippleOn ? '' : ' · ⌥ ripple'}`
            : `on ${destTrack} · ${modeWord}${rippleOn ? '' : ' · ⌥ ripple'}`,
        tone: invalid ? 'warn' : 'info',
      })
      setGhost({ trackId: destTrack, srcTrackId: it.trackId, startMs: snapped.ms, durMs: it.durMs, pending: false })
    } else if (g.mode === 'trim-l') {
      const endMs = it.startMs + it.durMs
      // Can't trim past source start, below one frame, or past the clip end.
      const lo = Math.max(0, it.startMs - (it.srcInMs ?? 0))
      const snapped = resolveSnap(mouseMs, 0, cands, c.zoom, bypass)
      const startMs = Math.min(endMs - minDur, Math.max(lo, snapped.ms))
      setSnapLineMs(snapped.snappedTo)
      const durMs = endMs - startMs
      setHud({ x: g.lastClientX, y: g.lastClientY, label: shortDur(durMs), sub: 'trim in', tone: 'info' })
      setGhost({ trackId: it.trackId, srcTrackId: it.trackId, startMs, durMs, pending: false })
    } else if (g.mode === 'trim-r') {
      const snapped = resolveSnap(mouseMs, 0, cands, c.zoom, bypass)
      const endMs = Math.max(it.startMs + minDur, snapped.ms)
      setSnapLineMs(snapped.snappedTo)
      const durMs = endMs - it.startMs
      setHud({ x: g.lastClientX, y: g.lastClientY, label: shortDur(durMs), sub: 'trim out', tone: 'info' })
      setGhost({ trackId: it.trackId, srcTrackId: it.trackId, startMs: it.startMs, durMs, pending: false })
    }
  }, [clientXToMs, clientYToRow, applyMarqueeSelection])

  const onWinMove = useCallback((e: MouseEvent) => {
    const g = gestureRef.current
    if (!g) return
    g.lastClientX = e.clientX
    g.lastClientY = e.clientY
    g.shift = e.shiftKey // Shift releases the magnet per-mousemove
    g.alt = e.altKey // Alt is the cross-track move ripple modifier
    if (!g.dragging) {
      const dx = Math.abs(e.clientX - g.startX)
      const dy = Math.abs(e.clientY - g.startY)
      if (g.mode === 'scrub') g.dragging = true // ruler scrub drags immediately
      else if (dx > DRAG_THRESHOLD_PX || dy > DRAG_THRESHOLD_PX) {
        g.dragging = true
        if (g.mode === 'move' && g.item) setDraggingClipId(g.item.id)
      }
    }
    if (g.dragging) updateProposal()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [updateProposal])

  const onWinUp = useCallback((e: MouseEvent) => {
    const g = gestureRef.current
    if (!g) return
    const c = cfg.current
    // Alt at mouseup (or held through the drag) decides the move ripple flag.
    altAtUpRef.current = g.alt || e.altKey
    const wasDrag = g.dragging && g.mode !== 'scrub' && g.mode !== 'lane-seek'
    lastGestureWasDrag.current = wasDrag
    const dx = Math.abs(e.clientX - g.startX)
    const dy = Math.abs(e.clientY - g.startY)
    const dt = performance.now() - g.startT

    if (g.mode === 'lane-seek') {
      // Background click-to-seek: ≤5px AND ≤500ms (timeline behavior contract);
      // empty-area click also clears selection. A real DRAG here was the
      // Marquee selection was applied live per move; do NOT clear or
      // seek on release (endGesture below removes the rectangle).
      if (dx <= DRAG_THRESHOLD_PX && dy <= DRAG_THRESHOLD_PX && dt <= CLICK_MAX_MS) {
        c.onSelect([])
        c.onSeek(clientXToMs(e.clientX))
      }
    } else if (g.mode === 'scrub') {
      c.onSeek(clientXToMs(e.clientX)) // re-issue final seek
    } else if (g.mode === 'marker' && g.marker) {
      // Marker drag → edit.move_marker (id PRESERVED). Cancel if
      // it didn't actually move (≤5px) — no zero-delta op.
      const mg = markerGhostRef.current
      if (wasDrag && mg && dx > DRAG_THRESHOLD_PX && mg.atMs !== g.marker.startAtMs) {
        const delta = mg.atMs - g.marker.startAtMs
        void runUserVerb('edit.move_marker', {
          id: g.marker.id, at_ms: mg.atMs,
          rationale: `user drag: marker ${g.marker.id} ${delta >= 0 ? '+' : '−'}${Math.abs(delta / 1000).toFixed(2)}s → ${timecode(mg.atMs)}`,
        }, `Could not move marker ${g.marker.id}.`)
      } else if (!wasDrag && dx <= DRAG_THRESHOLD_PX && dy <= DRAG_THRESHOLD_PX && dt <= CLICK_MAX_MS) {
        // A pure click on a marker jumps the playhead to it
        // (the standard NLE bookmark behaviour). Seek to its grab-time at_ms via
        // the existing onSeek path — the panel never owns the playhead.
        c.onSeek(g.marker.startAtMs)
      }
      setMarkerGhost(null)
    } else if (wasDrag && g.item && (g.mode === 'slip' || g.mode === 'slide' || g.mode === 'roll-l' || g.mode === 'roll-r')) {
      // Trim tool: dispatch the SAME verbs as the TrimPopover steppers, with
      // the frame-rounded horizontal delta. Zero delta / sub-threshold = no op.
      const it = g.item
      if (it.kind !== 'caption' && it.kind !== 'gap' && dx > DRAG_THRESHOLD_PX) {
        const frame = Math.round(1000 / c.fps)
        const rawDelta = clientXToMs(e.clientX) - clientXToMs(g.startX)
        const by = Math.round(rawDelta / frame) * frame
        if (by !== 0) {
          setGhost((gh) => (gh ? { ...gh, pending: true } : gh))
          const secs = `${by >= 0 ? '+' : '−'}${Math.abs(by / 1000).toFixed(2)}s`
          if (g.mode === 'slip') {
            void runUserVerb('edit.slip', { clip: it.id, by_ms: by, rationale: `user slip drag: ${it.id} ${secs}` }, `Could not slip clip ${it.id}.`).then((result) => { if (!result?.ok) setGhost(null) })
          } else if (g.mode === 'slide') {
            void runUserVerb('edit.slide_edit', { clip: it.id, by_ms: by, rationale: `user slide drag: ${it.id} ${secs}` }, `Could not slide clip ${it.id}.`).then((result) => { if (!result?.ok) setGhost(null) })
          } else {
            // edit.roll keys the cut on EDITORIAL time (cumulative clip-
            // duration sum, app/core/src/trim_edit.rs roll) — the laid edge
            // (drawn position) drifts left of it after an upstream crossfade.
            // The live HUD/snap-line during the drag stays laid (display).
            const seam = g.mode === 'roll-l' ? it.editorialStartMs : it.editorialStartMs + it.durMs
            void runUserVerb('edit.roll', { track: it.trackId, at_ms: seam, by_ms: by, rationale: `user roll drag: cut @ ${timecode(seam)} ${secs}` }, 'Could not roll this edit.').then((result) => { if (!result?.ok) setGhost(null) })
          }
        } else setGhost(null)
      } else setGhost(null)
    } else if (wasDrag && g.item && g.item.kind === 'caption') {
      // Caption clip move/trim → captions.set_range (the ONLY way
      // to reposition a caption clip; edit.move/trim refuse them). One verb
      // covers retime (both edges) and trim (one edge): we send the ghost's
      // resolved [start, end) range.
      const it = g.item
      const final = ghostFinal.current
      if (dx <= DRAG_THRESHOLD_PX && dy <= DRAG_THRESHOLD_PX) {
        setGhost(null) // back to origin = cancel
      } else if (final) {
        const newStart = final.startMs
        const newEnd = final.startMs + final.durMs
        const changed = newStart !== it.startMs || newEnd !== it.startMs + it.durMs
        if (changed) {
          setGhost((gh) => (gh ? { ...gh, pending: true } : gh))
          const verbWord = g.mode === 'cap-move' ? 'retime' : 'trim'
          void runUserVerb('captions.set_range', {
            clip: it.id,
            range_ms: [newStart, newEnd],
            rationale: `user ${verbWord}: caption ${it.id} → ${timecode(newStart)}–${timecode(newEnd)}`,
          }, `Could not ${verbWord} caption ${it.id}.`).then((result) => { if (!result?.ok) setGhost(null) })
        } else setGhost(null)
      }
    } else if (wasDrag && g.item) {
      if (dx <= DRAG_THRESHOLD_PX && dy <= DRAG_THRESHOLD_PX) {
        setGhost(null) // dragged back to origin = cancel, no verb
      } else {
        const it = g.item
        const final = ghostFinal.current
        const invalidDrop = dropInvalidRef.current
        if (final && it.kind !== 'caption' && it.kind !== 'gap') {
          if (g.mode === 'move') {
            const trackChanged = final.trackId !== it.trackId
            const moved = final.startMs !== it.startMs || trackChanged
            // Invalid drop (different-KIND track the engine would refuse) =
            // cancel: keep truth, dispatch nothing, no error op (graceful
            // disable — unsupported moves must stay explicit instead of silently mutating state).
            if (invalidDrop || !moved) {
              setGhost(null) // no-op / blocked drop = zero ops
            } else {
              setGhost((gh) => (gh ? { ...gh, pending: true } : gh))
              // edit.move takes a REAL ripple flag. Alt held at
              // drop = ripple:true (AV-sync; siblings + captions/markers/duck-
              // windows shift); default = linked float (the pair moves while
              // unrelated tracks stay fixed). We pass both resolved flags.
              const ripple = altAtUpRef.current
              // edit.move splices at EDITORIAL time on the destination track
              // (app/core/src/edit.rs splice_into_track walks the cumulative
              // cursor); the ghost's startMs is the LAID drop position, so
              // convert through the destination track's laid→editorial map.
              // (The engine gap-fills the source slot first, so the dest
              // track's editorial layout is unchanged by the removal.)
              const destItems = c.allItems.filter((i) => i.trackId === final.trackId)
              void callVerb('edit.move', {
                clip: it.id,
                to_track: final.trackId,
                at_ms: Math.round(laidToEditorialMs(destItems, final.startMs)),
                ripple,
                linked: true,
                rationale: dragRationale(it.id, final.startMs - it.startMs, final.trackId, trackChanged, ripple ? 'ripple' : 'lift'),
              }).then((result) => {
                if (showVerbFailure(result, 'Could not move the linked clip.')) setGhost(null)
              }).catch(() => {
                showVerbFailure({ ok: false }, 'Could not move the linked clip: server unreachable.')
                setGhost(null)
              })
            }
          } else if (g.mode === 'trim-l') {
            const delta = final.startMs - it.startMs
            if (delta !== 0) {
              setGhost((gh) => (gh ? { ...gh, pending: true } : gh))
              const trim = sourceTrimAtTimelinePosition(it, 'start', final.startMs)
              if (!trim) { setGhost(null); endGesture(); return }
              void callVerb('edit.trim', {
                clip: it.id,
                ...trim,
                linked: true,
                rationale: `user trim: clip ${it.id} in ${delta >= 0 ? '+' : '−'}${Math.abs(delta / 1000).toFixed(2)}s → ${shortDur(final.durMs)}`,
              }).then((result) => {
                if (showVerbFailure(result, 'Could not trim the linked clip start.')) setGhost(null)
              }).catch(() => {
                showVerbFailure({ ok: false }, 'Could not trim the linked clip start: server unreachable.')
                setGhost(null)
              })
            } else setGhost(null)
          } else if (g.mode === 'trim-r') {
            const delta = final.startMs + final.durMs - (it.startMs + it.durMs)
            if (delta !== 0) {
              setGhost((gh) => (gh ? { ...gh, pending: true } : gh))
              const trim = sourceTrimAtTimelinePosition(it, 'end', final.startMs + final.durMs)
              if (!trim) { setGhost(null); endGesture(); return }
              void callVerb('edit.trim', {
                clip: it.id,
                ...trim,
                linked: true,
                rationale: `user trim: clip ${it.id} out ${delta >= 0 ? '+' : '−'}${Math.abs(delta / 1000).toFixed(2)}s → ${shortDur(final.durMs)}`,
              }).then((result) => {
                if (showVerbFailure(result, 'Could not trim the linked clip end.')) setGhost(null)
              }).catch(() => {
                showVerbFailure({ ok: false }, 'Could not trim the linked clip end: server unreachable.')
                setGhost(null)
              })
            } else setGhost(null)
          }
        }
      }
    }
    endGesture()
  }, [clientXToMs, endGesture, showVerbFailure])

  // Latest ghost, readable inside onWinUp without re-binding.
  const ghostFinal = useRef<Ghost | null>(null)
  ghostFinal.current = ghost
  // Latest drop-validity, readable inside onWinUp (cross-track guard).
  const dropInvalidRef = useRef(false)
  dropInvalidRef.current = dropInvalid
  // Latest marker ghost, readable inside onWinUp.
  const markerGhostRef = useRef<{ id: string; atMs: number } | null>(null)
  markerGhostRef.current = markerGhost
  // Latest Alt state from the gesture, read at mouseup to resolve
  // the edit.move ripple flag (the gesture ref is cleared in endGesture, so we
  // mirror its last alt here for the dispatch).
  const altAtUpRef = useRef(false)

  /** Start the edge auto-scroll rAF loop (timeline behavior contract). */
  const startEdgeScroll = useCallback(() => {
    const loop = () => {
      const g = gestureRef.current
      const el = scrollRef.current
      if (!g || !el) return
      if (g.dragging) {
        const rect = el.getBoundingClientRect()
        const leftEdge = rect.left + RAIL_W
        const rightEdge = rect.right
        let v = 0
        if (g.lastClientX < leftEdge + EDGE_SCROLL_ZONE_PX) {
          const d = Math.max(0, g.lastClientX - leftEdge)
          v = -EDGE_SCROLL_MAX_PX * (1 - d / EDGE_SCROLL_ZONE_PX)
        } else if (g.lastClientX > rightEdge - EDGE_SCROLL_ZONE_PX) {
          const d = Math.max(0, rightEdge - g.lastClientX)
          v = EDGE_SCROLL_MAX_PX * (1 - d / EDGE_SCROLL_ZONE_PX)
        }
        if (v !== 0) {
          el.scrollLeft += v
          updateProposal() // pointer stationary at the edge still moves time
        }
      }
      edgeScrollRaf.current = requestAnimationFrame(loop)
    }
    edgeScrollRaf.current = requestAnimationFrame(loop)
  }, [updateProposal])

  const beginGesture = useCallback(
    (e: React.MouseEvent | MouseEvent, mode: GestureMode, item?: LaidItem, marker?: { id: string; startAtMs: number }) => {
      if (e.button !== 0) return
      if (gestureRef.current) endGesture() // a new gesture cancels an abandoned one
      const mouseMs = clientXToMs(e.clientX)
      gestureRef.current = {
        mode,
        item,
        marker,
        startX: e.clientX,
        startY: e.clientY,
        startT: performance.now(),
        grabOffsetMs: item ? mouseMs - item.startMs : 0,
        dragging: false,
        lastClientX: e.clientX,
        lastClientY: e.clientY,
        shift: e.shiftKey,
        alt: e.altKey,
      }
      // Capture the exact handlers we bind so endGesture removes THESE (onWinMove/onWinUp
      // may be recreated before the gesture ends → otherwise they'd leak; see the refs above).
      winMoveRef.current = onWinMove
      winUpRef.current = onWinUp
      window.addEventListener('mousemove', onWinMove)
      window.addEventListener('mouseup', onWinUp)
      startEdgeScroll()
    },
    [clientXToMs, endGesture, onWinMove, onWinUp, startEdgeScroll],
  )

  // Clip mousedown: selection at mousedown (multi-key + implicit-select-before-
  // drag, timeline behavior contract), then a move/trim gesture.
  const onClipDown = useCallback((e: React.MouseEvent, item: LaidItem, mode: GestureMode) => {
    e.stopPropagation() // keep the lane background from starting a seek
    if (item.kind === 'gap') return
    // Razor interaction guard: only the LEFT button drives razor-split / drag /
    // trim. Right-click is now a real gesture (the context menu) — without this
    // guard, right-clicking a clip in razor mode SPLIT it (destructive) before the
    // context menu opened. The context menu's own handler selects the clicked clip.
    if (e.button !== 0) return
    const c = cfg.current
    const updateSelection = () => {
      if (e.ctrlKey || e.metaKey) {
        const next = c.selectedClipIds.includes(item.id)
          ? c.selectedClipIds.filter((id) => id !== item.id)
          : [...c.selectedClipIds, item.id]
        c.onSelect(next)
      } else if (!c.selectedClipIds.includes(item.id)) {
        c.onSelect([item.id]) // implicit select before drag
      }
    }
    if (isTrackLocked(item.trackId)) {
      e.preventDefault()
      updateSelection()
      return
    }
    // --- Razor click-mode: a clip click splits it at the cursor time ---------
    // Captions resize via verbs, not razor. The shared splitItemAt action owns
    // the actual dispatch: laid→EDITORIAL conversion (edit.split keys on the
    // engine's clip-duration-sum cursor, not the drawn position) AND the
    // linked A/V propagation (a razor cut lands on the video clip and its
    // exact linked audio counterpart together — one group, one undo).
    if (c.razorMode && item.kind !== 'caption') {
      e.preventDefault()
      const atMs = clientXToMs(e.clientX)
      if (atMs > item.startMs && atMs < item.startMs + item.durMs) {
        splitItemAt(item.id, atMs, 'razor')
      }
      return // razor consumes the gesture — no move/trim
    }
    updateSelection()
    if (mode !== 'move') e.preventDefault() // trim: kill text selection
    // Caption clips carry an absolute range, not a source range — their move/
    // Trim gestures fold to captions.set_range, so re-map the mode to the
    // caption variants here. AV clips keep their normal modes.
    let m: GestureMode =
      item.kind === 'caption'
        ? mode === 'move'
          ? 'cap-move'
          : mode === 'trim-l'
            ? 'cap-trim-l'
            : 'cap-trim-r'
        : mode
    // Trim tool: remap MEDIA-clip gestures to slip/slide/roll. Body drag →
    // slip/slide (the verbs address one clip); edge drag in roll → roll the
    // shared cut at that edge. Captions keep their own gestures (set_range).
    if (item.kind !== 'caption' && c.trimTool !== 'select') {
      if (c.trimTool === 'slip' && mode === 'move') m = 'slip'
      else if (c.trimTool === 'slide' && mode === 'move') m = 'slide'
      else if (c.trimTool === 'roll' && (mode === 'trim-l' || mode === 'trim-r'))
        m = mode === 'trim-l' ? 'roll-l' : 'roll-r'
      if (m !== mode) e.preventDefault() // tool gestures never text-select
    }
    beginGesture(e, m, item)
  }, [beginGesture, clientXToMs, isTrackLocked, splitItemAt])

  // Click handler suppression after a drag — the click event fires
  // after mouseup; the flag outlives the session.
  const onLaneClick = useCallback(() => {
    if (lastGestureWasDrag.current) lastGestureWasDrag.current = false
  }, [])

  // Ruler / playhead handle: seek immediately + scrub session (no element
  // snap on the initial press — timeline behavior contract asymmetry). Used by the PLAYHEAD
  // HANDLE (drag = scrub); the ruler BODY uses onRulerRangeDown below.
  const onRulerDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    cfg.current.onSeek(clientXToMs(e.clientX))
    beginGesture(e, 'scrub')
  }, [beginGesture, clientXToMs])

  // Ruler BODY: drag to PAINT an export range [in,out] (the explicit span the
  // export controls use); a plain click (no drag) SEEKS and clears any range. Scrubbing
  // moves to the playhead handle (onRulerDown above). Self-contained window-
  // pointer session (mirrors the Assets drag) — does NOT touch the gesture FSM.
  const rangeDrag = useRef<{ startMs: number; startX: number } | null>(null)
  const [dragRange, setDragRange] = useState<[number, number] | null>(null)
  const onRulerRangeDown = useCallback((e: React.MouseEvent) => {
    if (e.button !== 0) return
    e.preventDefault()
    const startMs = clientXToMs(e.clientX)
    rangeDrag.current = { startMs, startX: e.clientX }
    const onMove = (ev: MouseEvent) => {
      const rd = rangeDrag.current
      if (!rd) return
      const cur = clientXToMs(ev.clientX)
      setDragRange([Math.min(rd.startMs, cur), Math.max(rd.startMs, cur)])
    }
    const onUp = (ev: MouseEvent) => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
      const rd = rangeDrag.current
      rangeDrag.current = null
      setDragRange(null)
      if (!rd) return
      if (Math.abs(ev.clientX - rd.startX) < 4) {
        // pure click → seek + clear the range (start a fresh selection)
        cfg.current.onSeek(rd.startMs)
        onExportRange(null)
        return
      }
      const cur = clientXToMs(ev.clientX)
      const lo = Math.round(Math.min(rd.startMs, cur))
      const hi = Math.round(Math.max(rd.startMs, cur))
      if (hi - lo >= 50) onExportRange([lo, hi]) // ignore a hair-thin smear
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
  }, [clientXToMs, onExportRange])

  // Empty lane background: pending click-to-seek (no dead zones, checklist 19).
  const onLaneDown = useCallback((e: React.MouseEvent) => {
    // Snapshot the selection for a possible Shift+marquee union.
    marqueeBaseSel.current = e.shiftKey ? [...cfg.current.selectedClipIds] : []
    beginGesture(e, 'lane-seek')
  }, [beginGesture])

  // Marker triangle mousedown: start a marker-drag gesture so the
  // user can reposition it on the ruler → edit.move_marker (id preserved).
  // stopPropagation keeps the ruler scrub from also firing. A pure click (no
  // drag past 5px) is a no-op here — selecting/relabeling markers is a future
  // affordance; the engine has no select verb, only move/add/remove.
  const onMarkerDown = useCallback((e: React.MouseEvent, m: Marker) => {
    e.stopPropagation()
    e.preventDefault()
    beginGesture(e, 'marker', undefined, { id: m.id, startAtMs: m.at_ms })
  }, [beginGesture])

  // --- marker context menu (right-click on a ruler marker) — the DISCOVERABLE
  // "Delete marker" + "Seek here" that the engine has verbs for (edit.remove_marker
  // / edit.seek_marker) but a human couldn't reach. Mirrors the clip context menu
  // Markers vanish from project.markers on the op_applied snapshot. ----------------
  const [markerMenu, setMarkerMenu] = useState<MarkerMenuState | null>(null)
  const onMarkerContextMenu = useCallback((e: React.MouseEvent, m: Marker) => {
    e.preventDefault()
    e.stopPropagation()
    setMarkerMenu({ x: e.clientX, y: e.clientY, id: m.id, atMs: m.at_ms, label: m.label, note: m.note, color: m.color })
  }, [])
  // Delete a marker by id → edit.remove_marker. The marker disappears from
  // project.markers once the op_applied snapshot lands (panel never owns truth).
  const removeMarkerById = useCallback((id: string, label: string) => {
    void runUserVerb('edit.remove_marker', { id, rationale: `user delete marker ${id} (“${label}”) (context menu)` }, `Could not delete marker “${label}”.`)
  }, [])
  // Rename/recolor/note → edit.update_marker (ONE op, id + position preserved).
  const renameMarkerById = useCallback((id: string, label: string) => {
    void runUserVerb('edit.update_marker', { id, label, rationale: `user rename marker ${id} → “${label}” (context menu)` }, 'Could not rename the marker.')
  }, [])
  const noteMarkerById = useCallback((id: string, note: string) => {
    void runUserVerb('edit.update_marker', { id, note, rationale: note ? `user saved marker ${id} note (context menu)` : `user cleared marker ${id} note (context menu)` }, 'Could not save the marker note.')
  }, [])
  const colorMarkerById = useCallback((id: string, color: MarkerColor | 'none') => {
    void runUserVerb('edit.update_marker', { id, color, rationale: `user set marker ${id} color ${color} (context menu)` }, 'Could not change the marker color.')
  }, [])
  // Close the marker menu on Escape (the backdrop handles click-away).
  useEffect(() => {
    if (!markerMenu) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setMarkerMenu(null) }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [markerMenu])

  // Seam handle click: select the cut between two adjacent media
  // clips → open the crossfade duration popover. The seam handle sits AT the
  // boundary; clicking it (not dragging a clip) opens the editor.
  const onSeamDown = useCallback((e: React.MouseEvent, seam: Seam) => {
    e.stopPropagation()
    e.preventDefault()
    setActiveSeam((cur) => (cur && cur.leftId === seam.leftId && cur.rightId === seam.rightId ? null : seam))
  }, [])

  // --- clip context menu (right-click) — the DISCOVERABLE remove/split that a
  // users could not find when the only affordances were keyboard + jargon
  // toolbar buttons. Event-delegated off the scroll container so the
  // memoized clip components stay untouched. -----------------------------------
  const [clipMenu, setClipMenu] = useState<ClipMenuState | null>(null)
  // Which clip-menu action has its inline asset picker expanded (Replace / Fit-to-fill);
  // null = collapsed. Reset whenever the menu closes so a reopen starts clean.
  const [assetPick, setAssetPick] = useState<AssetPickMode | null>(null)
  const onTimelineContextMenu = useCallback((e: React.MouseEvent) => {
    const el = e.target instanceof HTMLElement ? e.target.closest('[data-cut-clip]') : null
    const itemId = el?.getAttribute('data-cut-clip')
    if (!itemId) return // empty timeline area → let the browser default be
    const it = cfg.current.allItems.find((i) => i.id === itemId)
    if (!it || it.kind === 'gap') return
    e.preventDefault()
    // NLE convention: right-clicking a clip that is ALREADY part of a multi-clip
    // selection KEEPS the whole selection (so selection-scoped menu actions — Nest —
    // still see the full run); otherwise select just the clicked clip.
    const sel = cfg.current.selectedClipIds
    if (!(sel.length > 1 && sel.includes(itemId))) cfg.current.onSelect([itemId])
    if (isTrackLocked(it.trackId)) return
    setClipMenu({ x: e.clientX, y: e.clientY, itemId, atMs: clientXToMs(e.clientX) })
  }, [clientXToMs, isTrackLocked])
  // Close the clip menu on Escape (the backdrop handles click-away).
  useEffect(() => {
    if (!clipMenu) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') setClipMenu(null) }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [clipMenu])
  // Collapse the inline asset picker whenever the clip menu closes (or retargets a
  // different clip), so a reopened menu always starts with the picker collapsed.
  useEffect(() => { setAssetPick(null) }, [clipMenu])

  // --- wheel: ONE non-passive capture listener (timeline behavior contract) -------------
  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    let pendingZoom = 0
    let zoomRaf = 0
    let zoomClientX = 0
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      if (e.ctrlKey || e.metaKey) {
        // Zoom: normalize deltaMode, cap per event, batch per rAF.
        const normalized = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY
        pendingZoom += Math.sign(normalized) * Math.min(Math.abs(normalized), WHEEL_DELTA_CAP)
        zoomClientX = e.clientX
        if (!zoomRaf) {
          zoomRaf = requestAnimationFrame(() => {
            zoomRaf = 0
            const factor = Math.exp(-pendingZoom / 300)
            pendingZoom = 0
            // Cursor-anchored: time under pointer stays put.
            applyZoom(cfg.current.zoom * factor, clientXToMs(zoomClientX))
          })
        }
        return
      }
      if (e.shiftKey) {
        el.scrollTop += e.deltaY // Shift+wheel = vertical
        return
      }
      // Default axis = horizontal pan; trackpads with dominant deltaX pan too.
      const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY
      el.scrollLeft += Math.sign(d) * Math.min(Math.abs(d), WHEEL_PAN_CLAMP_PX)
    }
    el.addEventListener('wheel', onWheel, { passive: false, capture: true })
    return () => el.removeEventListener('wheel', onWheel, { capture: true } as EventListenerOptions)
  }, [applyZoom, clientXToMs])

  // --- scroll → ruler window (rAF-throttled) ---------------------------------
  useEffect(() => {
    const el = scrollRef.current
    if (!el) return
    let raf = 0
    const onScroll = () => {
      if (raf) return
      raf = requestAnimationFrame(() => {
        raf = 0
        setScrollX(el.scrollLeft)
      })
    }
    el.addEventListener('scroll', onScroll, { passive: true })
    return () => el.removeEventListener('scroll', onScroll)
  }, [])

  // --- GLOBAL keyboard scope: every key maps to a public verb ----------------
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (shouldIgnoreGlobalShortcut(e)) return
      if (document.documentElement.dataset.cutKbscope === 'rail') return
      const c = cfg.current
      // Split at the playhead — the hook's splitAtPlayhead owns the whole job:
      // selected clips' tracks (else all video tracks), laid→EDITORIAL at_ms
      // conversion per track, and linked A/V propagation. Shared by the bare
      // S key and the Cmd/Ctrl+B canonical "cut here" (conventions).
      if (e.key === 'Escape') {
        if (gestureRef.current) { // cancel gesture, restore visuals, no verb
          setGhost(null)
          endGesture()
        } else if (c.trimTool !== 'select') {
          setTrimTool('select') // Escape drops back to the select tool
        } else c.onSelect([])
        return
      }
      // Ctrl/Cmd+A selects ALL clips (every non-gap item) so the existing
      // multi-clip operations (delete / speed / grade…) apply to the whole
      // timeline at once. Pairs with Ctrl/Cmd-click toggle-select; Escape clears.
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
        e.preventDefault()
        c.onSelect(c.allItems.filter((i) => i.kind !== 'gap' && !isTrackLocked(i.trackId)).map((i) => i.id))
        return
      }
      // Ctrl/Cmd+Z (undo) and Ctrl/Cmd+Shift+Z / Ctrl/Cmd+Y (redo) are handled
      // by the single GLOBAL handler in App.tsx via the linear history cursor
      // (project.undo / project.redo). This handler is ALSO on window, so a Z
      // block here would double-fire (and project.undo, unlike the old
      // edit.restore{tip}, is not idempotent → two cursor steps). Intentionally
      // not handled here.
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
        e.preventDefault()
        void runUserVerb('project.save', {}, 'Could not save the project.')
        return
      }
      // Cut at the playhead — Cmd/Ctrl+B, the common editor shortcut.
      // "cut here" key (alias of the bare S key). Conventions reference.
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === 'b') {
        e.preventDefault()
        splitAtPlayhead()
        return
      }
      // LIFT-delete = Alt+Del / Alt+Backspace. Distinct from plain
      // Del (ripple — close the gap): ripple:false so the gap STAYS OPEN
      // (nothing downstream moves). Handled BEFORE the modifier guard below
      // (which drops all alt-combos). The hook's deleteSelection owns the
      // shared delete rules for BOTH keyboard variants and the toolbar:
      // linked-A/V expansion, EDITORIAL range_ms, one undo group.
      if (((e.altKey && !e.ctrlKey && !e.metaKey) || (e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey)) && (e.key === 'Delete' || e.key === 'Backspace')) {
        e.preventDefault()
        void deleteSelection(false)
        return
      }
      // Cmd/Ctrl+= (and ) / Cmd/Ctrl+- — the OS-canonical zoom aliases (browsers,
      // editors). The bare +/-/= keys already zoom (below); these add the modified
      // form users reach for first, and preventDefault stops the WebView's own
      // page-zoom from firing on Cmd/Ctrl+=. Conventions reference.
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && (e.key === '=' || e.key === '+')) {
        e.preventDefault()
        applyZoom(c.zoom * ZOOM_KEY_FACTOR, c.playheadMs)
        return
      }
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key === '-') {
        e.preventDefault()
        applyZoom(c.zoom / ZOOM_KEY_FACTOR, c.playheadMs)
        return
      }
      // Shift+Z — fit the whole timeline to the window (FCP/Resolve "zoom to fit").
      // Handled before the modifier guard so the bare Z key stays free.
      if (e.shiftKey && !e.ctrlKey && !e.metaKey && !e.altKey && e.key.toLowerCase() === 'z') {
        e.preventDefault()
        fitToWindow()
        return
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return
      // The single-key editor actions consult the remappable keymap
      // (lib/keymap.ts) — bindings read live, remaps apply instantly. Zoom
      // keys / Delete / Shift+Z stay literal (conventions, not preferences).
      if (matchesAction(e, 'timeline.split')) {
        splitAtPlayhead() // split at the playhead (also Cmd/Ctrl+B)
        e.preventDefault()
        return
      }
      if (matchesAction(e, 'timeline.razor')) {
        // Blade/razor TOOL toggle — the NLE-canonical default B (FCP/Resolve).
        setRazorMode((m) => !m)
        e.preventDefault()
        return
      }
      if (matchesAction(e, 'timeline.snap')) {
        setSnapEnabled((s) => !s)
        e.preventDefault()
        return
      }
      if (matchesAction(e, 'timeline.rippleTrimStart')) {
        e.preventDefault()
        void rippleTrimAtPlayhead('start')
        return
      }
      if (matchesAction(e, 'timeline.rippleTrimEnd')) {
        e.preventDefault()
        void rippleTrimAtPlayhead('end')
        return
      }
      if (matchesAction(e, 'timeline.marker')) {
        void runUserVerb('edit.add_marker', { at_ms: c.playheadMs, label: `m @ ${timecode(c.playheadMs)}` }, 'Could not add a marker.')
        e.preventDefault()
        return
      }
      if (matchesAction(e, 'timeline.prevMarker') || matchesAction(e, 'timeline.nextMarker')) {
        // JUMP the playhead to the prev/next marker (NLE bookmark-step keys).
        // edit.seek_marker is a pure READ; the playhead moves via onSeek.
        const direction = matchesAction(e, 'timeline.prevMarker') ? 'prev' : 'next'
        void runUserVerb('edit.seek_marker', { from_ms: Math.round(c.playheadMs), direction }, 'Could not find the next marker.').then((r) => {
          const at = r?.ok ? (r.result as { marker?: { at_ms?: number } | null })?.marker?.at_ms : undefined
          if (typeof at === 'number') cfg.current.onSeek(at)
        })
        e.preventDefault()
        return
      }
      if (matchesAction(e, 'timeline.markIn')) {
        // Mark IN at the playhead — export-range START (canonical NLE "I").
        const cur = c.exportRange
        const inMs = Math.round(c.playheadMs)
        const outMs = cur && cur[1] > inMs ? cur[1] : Math.round(c.durationMs)
        if (outMs - inMs >= 50) c.onExportRange([inMs, outMs])
        e.preventDefault()
        return
      }
      if (matchesAction(e, 'timeline.markOut')) {
        // Mark OUT at the playhead — export-range END (NLE "O").
        const cur = c.exportRange
        const outMs = Math.round(c.playheadMs)
        const inMs = cur && cur[0] < outMs ? cur[0] : 0
        if (outMs - inMs >= 50) c.onExportRange([inMs, outMs])
        e.preventDefault()
        return
      }
      switch (e.key) {
        case '+': case '=':
          applyZoom(c.zoom * ZOOM_KEY_FACTOR, c.playheadMs) // anchor playhead
          break
        case '-':
          applyZoom(c.zoom / ZOOM_KEY_FACTOR, c.playheadMs)
          break
        case 'Delete': case 'Backspace': {
          // RIPPLE-delete (close the gap). The hook's deleteSelection is the
          // single delete implementation for keyboard + toolbar: linked-A/V
          // expansion (a video clip takes its exact linked audio counterpart
          // with it — import places muxed files as v1 video + a1t audio, and
          // an orphaned audio half kept the timeline long → black-tail
          // export), EDITORIAL range_ms, and one group id so a single Ctrl+Z
          // restores the whole set.
          void deleteSelection(true)
          break
        }
        default:
          return
      }
      e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [applyZoom, deleteSelection, endGesture, fitToWindow, isTrackLocked, rippleTrimAtPlayhead, splitAtPlayhead])

  // laidTracks snapshot for the keyboard handler (avoids re-binding on data).
  const laidTracksRef = useRef(laidTracks)
  laidTracksRef.current = laidTracks

  // Ctrl/Cmd+T applies the DEFAULT transition (500ms
  // dissolve — the same default the seam popover opens with) at the most
  // intentional seam: a seam touching the selection if there is one, else the
  // seam nearest the playhead. One keypress = the universal NLE reflex; the
  // popover remains the place to change duration/style.
  const applyDefaultTransition = useCallback(() => {
    const c = cfg.current
    const seams = laidTracksRef.current
      .filter(({ track }) => !isTrackLocked(track.id))
      .flatMap(({ items }) => trackSeams(items))
    if (!seams.length) return
    const sel = c.selectedClipIds
    const nearSelection = sel.length
      ? seams.filter((s) => sel.includes(s.leftId) || sel.includes(s.rightId))
      : []
    const pool = nearSelection.length ? nearSelection : seams
    // Nearest-to-playhead compares in LAID/render space (the playhead's own
    // clock); seam.atMs is editorial and only for the dispatch itself.
    const pick = pool.reduce((best, s) =>
      Math.abs(s.laidMs - c.playheadMs) < Math.abs(best.laidMs - c.playheadMs) ? s : best,
    )
    applyCrossfade(pick, 500, 'dissolve')
  }, [applyCrossfade, isTrackLocked])
  // Trim popover (slip / slide / roll) — opened from the clip context menu.
  const [trimPop, setTrimPop] = useState<{ x: number; y: number; clipId: string; trackId: string; clipEndMs: number } | null>(null)
  const openTrimPopover = useCallback((itemId: string, x: number, y: number) => {
    const it = cfg.current.allItems.find((i) => i.id === itemId)
    if (!it || it.kind === 'gap' || it.kind === 'caption') return
    // clipEndMs feeds the popover's ROLL stepper → edit.roll at_ms, which is
    // EDITORIAL time (engine cumulative cursor) — not the drawn (laid) end.
    setTrimPop({ x, y, clipId: it.id, trackId: it.trackId, clipEndMs: it.editorialStartMs + it.durMs })
  }, [])
  // Paste attributes: the checkbox dialog. Opens from the clip context menu
  // or Ctrl/Cmd+Alt+V; targets = the selection at open time (the server verb
  // filters the source clip out and re-validates it still exists).
  const [pasteAttr, setPasteAttr] = useState<{ from: string; to: string[] } | null>(null)
  const clipboardClipIdRef = useRef(clipboardClipId)
  clipboardClipIdRef.current = clipboardClipId
  const openPasteAttributes = useCallback((targetIds?: string[]) => {
    const from = clipboardClipIdRef.current
    const to = targetIds && targetIds.length ? targetIds : cfg.current.selectedClipIds
    if (!from || to.length === 0) return
    setPasteAttr({ from, to })
  }, [])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (shouldIgnoreGlobalShortcut(e)) return
      if (document.documentElement.dataset.cutKbscope === 'rail') return
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 't') {
        e.preventDefault()
        applyDefaultTransition()
      }
      // The TRIM TOOL cycle key (default T) is remappable via lib/keymap.
      if (!e.ctrlKey && !e.metaKey && !e.altKey && matchesAction(e, 'timeline.trimTool')) {
        e.preventDefault()
        setTrimTool((cur) => (cur === 'select' ? 'slip' : cur === 'slip' ? 'slide' : cur === 'slide' ? 'roll' : 'select'))
      }
      // Ctrl/Cmd+Alt+V = Paste attributes… (plain Ctrl+V pastes the clip).
      if ((e.ctrlKey || e.metaKey) && e.altKey && !e.shiftKey && e.key.toLowerCase() === 'v') {
        e.preventDefault()
        openPasteAttributes()
      }
      // Alt+←/→ SLIPS the single selected clip by 1 frame (Shift+Alt = 10).
      // Alt+arrows were unbound (Preview's transport handler returns on Alt).
      if (e.altKey && !e.ctrlKey && !e.metaKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
        const c = cfg.current
        if (c.selectedClipIds.length !== 1) return
        const it = c.allItems.find((i) => i.id === c.selectedClipIds[0])
        if (!it || isTrackLocked(it.trackId)) return
        e.preventDefault()
        const frame = Math.max(1, Math.round(1000 / (c.fps || 30)))
        const frames = (e.shiftKey ? 10 : 1) * (e.key === 'ArrowRight' ? 1 : -1)
        void runUserVerb('edit.slip', {
          clip: c.selectedClipIds[0],
          by_ms: frames * frame,
          rationale: `user slip ${c.selectedClipIds[0]} ${frames > 0 ? '+' : ''}${frames}f (Alt+arrow)`,
        }, `Could not slip clip ${c.selectedClipIds[0]}.`)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [applyDefaultTransition, openPasteAttributes, isTrackLocked])

  // --- render -----------------------------------------------------------------
  const ticks = useMemo(() => {
    const fromMs = pxToMs(Math.max(0, scrollX - 200), zoom)
    const toMs = pxToMs(scrollX + viewW + 200, zoom)
    return rulerTicks(fromMs, Math.min(toMs, durationMs + pxToMs(240, zoom)), zoom, fps, timeMode)
  }, [scrollX, viewW, zoom, durationMs, fps, timeMode])

  const innerW = RAIL_W + contentW
  const fullH = RULER_H + tracksH

  // Selected MEDIA clips drive the toolbar speed control. selectionSpeed = the
  // shared speed (1 = normal) when the whole selection agrees, else undefined
  // (mixed → no preset highlighted, placeholder shows "—").
  const selectedMedia = allItems.filter(
    (i) => selectedClipIds.includes(i.id) && (i.kind === 'video' || i.kind === 'audio'),
  )
  // Control gating: "Cut to beat" needs beat:N markers (audio.add_music); "Auto
  // multicam" needs ≥2 video tracks holding media (the angles to switch between).
  const hasBeatMarkers = markers.some((m) => m.label === 'beat')
  const videoTracksWithMedia = (project?.tracks ?? []).filter(
    (t) => t.kind === 'video' && (t.clips ?? []).some((c) => 'asset' in c),
  ).length
  const canMulticam = videoTracksWithMedia >= 2
  const selectionSpeed = selectedMedia.length
    ? selectedMedia.every((i) => (i.speed ?? 1) === (selectedMedia[0].speed ?? 1))
      ? selectedMedia[0].speed ?? 1
      : undefined
    : undefined

  return (
    <section className="panel tl-root" data-panel="timeline" data-cut-panel="timeline" tabIndex={-1}>
      <TimelineToolbar
        playheadMs={playheadMs}
        project={project}
        fps={fps}
        timeMode={timeMode}
        zoom={zoom}
        razorMode={razorMode}
        trimTool={trimTool}
        onCycleTrimTool={() => setTrimTool((cur) => (cur === 'select' ? 'slip' : cur === 'slip' ? 'slide' : cur === 'slide' ? 'roll' : 'select'))}
        snapEnabled={snapEnabled}
        selectedClipCount={selectedClipIds.length}
        selectedMedia={selectedMedia}
        selectionSpeed={selectionSpeed}
        hasBeatMarkers={hasBeatMarkers}
        canMulticam={canMulticam}
        syncNote={syncNote}
        savingRange={savingRange}
        savingGif={savingGif}
        saveNote={saveNote}
        onCycleTimeDisplay={cycleTimeDisplay}
        onZoom={applyZoom}
        onToggleRazor={() => setRazorMode((r) => !r)}
        onToggleSnap={() => setSnapEnabled((s) => !s)}
        onAddTrack={addTrack}
        onRippleTrim={rippleTrimAtPlayhead}
        onDeleteSelection={deleteSelection}
        onSetSpeed={applySpeed}
        onSyncByAudio={syncByAudio}
        onMulticamSwitch={multicamSwitch}
        onCutToBeat={cutToBeat}
        onSaveRange={onSaveRange}
        onSaveGif={onSaveGif}
      />

      <div
        className={`tl-scroll ${razorMode ? 'tl-scroll--razor' : ''}${assetDnd ? ' tl-scroll--asset-dnd' : ''}`}
        ref={scrollRef}
        data-cut-timeline-scroll
        data-cut-razor={razorMode || undefined}
        data-cut-trimtool={trimTool !== 'select' ? trimTool : undefined}
        onClick={onLaneClick}
        onContextMenu={onTimelineContextMenu}
      >
        <div className="tl-inner" style={{ width: innerW, minHeight: '100%' }}>
          <TimelineOverlays
            assetDnd={assetDnd}
            dragRange={dragRange}
            exportRange={exportRange}
            zoom={zoom}
            fullH={fullH}
          />
          <TimelineRuler
            innerW={innerW}
            contentW={contentW}
            zoom={zoom}
            ticks={ticks}
            markers={markers}
            comments={project?.comments ?? []}
            project={project}
            markerGhost={markerGhost}
            onRulerRangeDown={onRulerRangeDown}
            onMarkerDown={onMarkerDown}
            onMarkerContextMenu={onMarkerContextMenu}
            onSeek={(atMs) => cfg.current.onSeek(atMs)}
            onOpenComment={(id) => document.dispatchEvent(new CustomEvent('cut:open-comment', { detail: { id } }))}
          />

          {laidTracks.map(({ track, items }, ti) => {
            const isDrop = dropTrackId === track.id && draggingClipId !== null
            const groupStart = ti > 0 && laidTracks[ti - 1].track.kind !== track.kind
            return (
              <TimelineTrackRow
                key={track.id}
                track={track}
                tracks={project?.tracks ?? []}
                items={items}
                groupStart={groupStart}
                isDrop={isDrop}
                dropInvalid={dropInvalid}
                baseVideoId={baseVideoId}
                contentW={contentW}
                zoom={zoom}
                selectedClipIds={selectedClipIds}
                draggingClipId={draggingClipId}
                filmstrips={filmstrips}
                windowedTiles={windowedTiles}
                assetLabels={assetLabels}
                seams={seamsByTrack[track.id] ?? []}
                activeSeam={activeSeam}
                ghost={ghost}
                auditionRevisionKey={`${project?.name ?? ''}:${headOpId}`}
                onLaneDown={onLaneDown}
                onClipDown={onClipDown}
                onSeamDown={onSeamDown}
              />
            )
          })}

          <TimelineGuides
            markers={markers}
            zoom={zoom}
            tracksH={tracksH}
            fullH={fullH}
            snapLineMs={snapLineMs}
            playheadRef={playheadRef}
            onPlayheadMouseDown={onRulerDown}
          />

          {/* Crossfade duration popover, pinned just below the
              selected seam's lane. Content-space coords (inside .tl-inner). */}
          {activeSeam && (() => {
            const row = rows.find((r) => r.id === activeSeam.trackId)
            const top = RULER_H + (row ? row.top + row.height + 2 : 0)
            // laidMs = visible boundary (render space); atMs is the EDITORIAL
            // dispatch coordinate and drifts left of the drawn seam after an
            // upstream crossfade.
            const left = RAIL_W + msToPx(activeSeam.laidMs, zoom)
            return (
              <CrossfadePopover
                seam={activeSeam}
                leftPx={left}
                topPx={top}
                onApply={(ms, transition) => applyCrossfade(activeSeam, ms, transition)}
                onClose={() => setActiveSeam(null)}
              />
            )
          })()}

          <TimelineEmptyState
            hasProject={!!project}
            hasTracks={laidTracks.length > 0}
            onImport={() => document.dispatchEvent(new CustomEvent('cut:open-import'))}
          />
        </div>
      </div>

      <TimelineGestureHud hud={hud} />

      {/* Clip context menu (right-click): render-only component; state/actions stay here. */}
      {clipMenu && (
        <ClipContextMenu
          menu={clipMenu}
          project={project}
          allItems={allItems}
          selectedClipIds={selectedClipIds}
          assetPick={assetPick}
          setAssetPick={setAssetPick}
          clipboardHasContent={clipboardHasContent}
          onPasteAttributes={openPasteAttributes}
          onOpenTrim={openTrimPopover}
          onClose={() => setClipMenu(null)}
          onCopyClip={onCopyClip}
          onCutClip={onCutClip}
          onPasteClip={onPasteClip}
          onSelect={(clipIds) => cfg.current.onSelect(clipIds)}
          onSeek={(atMs) => cfg.current.onSeek(atMs)}
          removeItemById={removeItemById}
          removeTrackById={removeTrackById}
          splitItemAt={splitItemAt}
          fadeItem={fadeItem}
          trimItemTo={trimItemTo}
          reverseItem={reverseItem}
          freezeItem={freezeItem}
          stabilizeItem={stabilizeItem}
          speedItem={speedItem}
          crossfadeAdjacent={crossfadeAdjacent}
          muteItem={muteItem}
          cleanVoiceItem={cleanVoiceItem}
          blurFacesItem={blurFacesItem}
          detachAudioItem={detachAudioItem}
          splitEditItem={splitEditItem}
          replaceClipSource={replaceClipSource}
          fitToFillAdjacent={fitToFillAdjacent}
          nestSelection={nestSelection}
        />
      )}

      {/* Paste-attributes dialog */}
      {pasteAttr && (
        <PasteAttributesDialog
          fromClip={pasteAttr.from}
          toClips={pasteAttr.to}
          onClose={() => setPasteAttr(null)}
        />
      )}

      {/* Trim popover (slip, slide, roll) */}
      {trimPop && (
        <TrimPopover
          x={trimPop.x}
          y={trimPop.y}
          clipId={trimPop.clipId}
          trackId={trimPop.trackId}
          clipEndMs={trimPop.clipEndMs}
          fps={fps}
          onClose={() => setTrimPop(null)}
        />
      )}

      {/* Marquee rectangle — client-fixed overlay while rubber-banding. */}
      {marquee && (
        <div
          className="tl-marquee"
          data-cut-marquee
          style={{
            left: Math.min(marquee.x0, marquee.x1),
            top: Math.min(marquee.y0, marquee.y1),
            width: Math.abs(marquee.x1 - marquee.x0),
            height: Math.abs(marquee.y1 - marquee.y0),
          }}
        />
      )}

      {markerMenu && (
        <MarkerContextMenu
          menu={markerMenu}
          onSeek={(atMs) => cfg.current.onSeek(atMs)}
          onRename={renameMarkerById}
          onNote={noteMarkerById}
          onColor={colorMarkerById}
          onDelete={removeMarkerById}
          onClose={() => setMarkerMenu(null)}
        />
      )}
    </section>
  )
}
