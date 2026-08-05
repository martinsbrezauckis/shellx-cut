// panels/Preview — monitor playback engine (preview-panel contract).
// Shows the clip under the playhead, falls back to composed frames when needed,
// and owns playback timing/key handling. Render-only controls and chips live in
// sibling components so selectors stay isolated from the timing code.
// Callers: App.tsx. Dependencies: lib/client (frameUrl, types), preview.css.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { baseVideoTrackId } from '../../lib/layerStack'
import { matchesAction } from '../../lib/keymap'
import { shouldIgnoreGlobalShortcut } from '../../lib/dom'
import { callVerb, exportUrl, frameUrl, type ClipTransform, type Project } from '../../lib/client'
import { runUserVerb } from '../../lib/userActionFeedback'
import { isFfmpegMissing, type DoctorReport } from '../../lib/doctor'
import { TransformHandles } from './TransformHandles'
import { MaskOverlay, type MaskShape, type MaskGeometry } from './MaskOverlay'
import PreviewEmptyState from './PreviewEmptyState'
import { CaptionText, OverlayVideo } from './PreviewLayers'
import PreviewExactReview from './PreviewExactReview'
import PreviewMonitorBadges from './PreviewMonitorBadges'
import PreviewTransport, { type Rate } from './PreviewTransport'
import { monitorAudioResyncTarget } from './audioSync'
import { activeVideo, previewFrameMs, RVFC_SUPPORTED, videoFrameCallbacks } from './model'
import { PreviewFrameError, PreviewOfflineOverlays, PreviewOfflineStage, usePreviewOfflineMedia } from './PreviewOffline'
import { usePreviewExportActions } from './usePreviewExportActions'
import { usePreviewViewOptions } from './usePreviewViewOptions'
import { GuideOverlay } from './GuideOverlay'
import { useContainBox } from './useContainBox'
import { events } from '../../lib/events'
import {
  openVideoToolsGuide,
  openVideoToolsSettings,
  recheckVideoTools,
} from '../../lib/videoToolsSetup'
import { timelineMsAtSourcePosition } from '../../lib/mediaTime'
import { useTimeDisplay } from '../../lib/timedisplay'
import { contentExtentMs, imageAssetIds, timecode } from '../Timeline/layout'
import {
  baseGradeAt,
  gradeFilter,
  resolveCaptions,
  resolveOverlays,
  stageAspect,
  shouldUseLivePreviewSurface,
} from './composite'
import './preview.css'

declare global {
  interface Window {
    webkitAudioContext?: typeof AudioContext
  }
}

export interface PreviewProps {
  project: Project | null
  doctor?: DoctorReport | null
  playheadMs: number
  /** Playhead change from transport/playback → App dispatches ui.playhead. */
  onSeek: (atMs: number) => void
  /**
   * The latest applied op id, used as a cache-bust token on the /api/frame
   * poster. The composed frame at a given at_ms changes whenever the timeline
   * changes (a cut shifts what source frame lands there), but the URL otherwise
   * stays identical → the browser would serve a stale cached frame. Threading
   * the head op id forces a re-fetch exactly when the edit changes. Empty string
   * when no ops yet (no cuts → no staleness).
   */
  headOpId?: string
  /** Currently-selected clip ids — the "Render section" action
   * renders the EXACT composite over the selected clips' span. */
  selectedClipIds?: string[]
  /** Explicit export span [in,out] painted on the ruler — the EXACT range "Save
   * as clip" exports. Takes precedence over a clip selection; null = nothing
   * selected (the Section button is then disabled — no implicit 30s fallback). */
  exportRange?: [number, number] | null
}

const isObject = (v: unknown): v is object => v !== null && typeof v === 'object'
const isTitleAlign = (v: unknown): v is 'left' | 'center' | 'right' => v === 'left' || v === 'center' || v === 'right'
const isRedactMode = (v: unknown): v is 'blur' | 'pixelate' | 'box' => v === 'blur' || v === 'pixelate' || v === 'box'
const isMaskShape = (v: unknown): v is MaskShape => v === 'rect' || v === 'ellipse' || v === 'polygon'

export default function Preview({ project, doctor = null, playheadMs, onSeek, headOpId, selectedClipIds, exportRange }: PreviewProps) {
  const [rate, setRate] = useState<Rate>(0)
  const [posterStale, setPosterStale] = useState(false)
  const [showSpinner, setShowSpinner] = useState(false)
  // COMPOSED mode: show the engine's composed frame (/api/frame?compose=1 —
  // grade, titles, kinetic overlays, crop, fades all applied) instead of the raw
  // proxy <video>, which shows NONE of those. The PROXY/COMPOSED toggle drives
  // it; a visual edit (grade/title/kinetic) also flips it on via cut:show-composed
  // so the drawers' "see it in the preview" receipt is actually TRUE.
  const [composed, setComposed] = useState(false)
  // #193b: a DRAGGABLE ghost of the title the Title drawer is placing (free mode). The
  // drawer broadcasts {x,y,text,align}; we render it on the frame and send back x/y as the
  // user drags — so a title is positioned directly on the picture, not an abstract pad.
  const [titleGhost, setTitleGhost] = useState<
    { x: number; y: number; text: string; align: 'left' | 'center' | 'right' } | null
  >(null)
  // Privacy draw-region (Inspector "Draw region"): when ARMED the Inspector
  // sends {clip, mode}; the user then drags a box on the stage. We capture the
  // drag in NORMALIZED stage fractions (0..1 of the letterboxed frame) and, on
  // pointer-up, fire edit.redact {shape:'rect', points:[[x0,y0],[x1,y1]], mode}.
  // redactArm = the armed target (null = idle); drawBox = the live drag rect.
  const [redactArm, setRedactArm] = useState<{ clip: string; mode: 'blur' | 'pixelate' | 'box' } | null>(null)
  const [drawBox, setDrawBox] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null)
  const drawingRef = useRef(false)
  // Region-MASK draw (Mask drawer, Q2): the drawer arms {active, clip, shape, nonce};
  // the user then draws/edits a rect/ellipse/polygon on the stage (Preview/MaskOverlay,
  // the TransformHandles pattern). MaskOverlay owns the geometry and reports it back via
  // cut:mask-geometry; the drawer's Apply fires edit.add_mask. `nonce` (bumped on the
  // drawer's "Clear shape") keys a fresh overlay so the drawn shape resets. null = idle.
  const [maskArm, setMaskArm] = useState<{ clip: string; shape: MaskShape; nonce: number } | null>(null)
  // Source files that failed to decode in <video> (raw / unsupported
  // codec) — once a source errors, stop offering it and use the poster until its
  // proxy lands. Keyed by asset id; only genuine decode/format errors land here
  // (transient network errors don't, so a blip doesn't permanently disable preview).
  const [failedSources, setFailedSources] = useState<Set<string>>(() => new Set())
  // Reset decode-failure memory when the OPEN PROJECT changes: a same-id asset in a
  // different project must not inherit the old project's failure, and this clears the
  // stale-source state left over from a project switch (the /api/source transient).
  const projectKey = project?.name ?? null
  useEffect(() => {
    setFailedSources(new Set())
  }, [projectKey])
  const videoRef = useRef<HTMLVideoElement>(null)

  // --- timeline audio monitoring --------------------------------------------
  // The base + overlay <video>s are MUTED (a single proxy clip's embedded track
  // is NOT the timeline mix), so the live preview was silent while
  // the export includes audio. Render the FULL timeline mix once (export.audio —
  // the SAME audio graph as render.final: per-track gains/fades/speed/ducking, so
  // it's WYSIWYG with the export) and play it through a hidden <audio> synced to
  // the playhead. Cached by the head op id (the edit version) → re-rendered lazily
  // on the next play after an edit. v1 = audio during 1× forward play (+ a drift
  // nudge to the authoritative video playhead); shuttle/reverse play stay silent.
  const audioRef = useRef<HTMLAudioElement>(null)
  const [audioOn, setAudioOn] = useState(true)
  const [mixUrl, setMixUrl] = useState<string | null>(null)
  const [mixForOp, setMixForOp] = useState<string | null>(null)
  const [mixBusy, setMixBusy] = useState(false)
  const lastAudioResyncAt = useRef(0)
  // Double-buffer the monitor mix across two files. The pre-warm re-renders the
  // mix as edits land; export.audio writes a FIXED path, so re-rendering the
  // SAME file the <audio> is mid-fetch races (on Windows the write-lock makes the
  // in-flight read 404 — caught by console-clean). Alternating _monitor_a/_b.mp3
  // means a re-render never overwrites the file the element is currently reading.
  const mixBuf = useRef<'a' | 'b'>('a')

  // --- master output meter (Audio Monitoring v2a) ---------------------------
  // Tap a Web Audio AnalyserNode off the SAME <audio> that plays the export mix,
  // so the meter reads the EXACT export level (WYSIWYG — no JS re-mix). A
  // MediaElementAudioSourceNode can be created only ONCE per element AND it
  // REROUTES the element's audio through the graph, so we build it lazily inside
  // the <audio>'s onPlay (a gesture-unlocked moment → the AudioContext can resume
  // and the element keeps making sound). If Web Audio is unavailable / capture is
  // blocked, we never capture the element, so v1 playback is unaffected (no meter).
  const meterCtxRef = useRef<AudioContext | null>(null)
  const meterSrcRef = useRef<MediaElementAudioSourceNode | null>(null)
  const [meterAnalyser, setMeterAnalyser] = useState<AnalyserNode | null>(null)
  const ffmpegMissing = isFfmpegMissing(doctor)
  const openVideoToolsSetup = useCallback(openVideoToolsSettings, [])
  const setupMeter = useCallback(() => {
    if (meterCtxRef.current) {
      void meterCtxRef.current.resume().catch(() => {})
      return
    }
    const el = audioRef.current
    if (!el) return
    const Ctx = window.AudioContext || window.webkitAudioContext
    if (!Ctx) return
    try {
      const ctx = new Ctx()
      const src = ctx.createMediaElementSource(el)
      const an = ctx.createAnalyser()
      an.fftSize = 1024
      an.smoothingTimeConstant = 0.4
      src.connect(an)
      an.connect(ctx.destination)
      meterCtxRef.current = ctx
      meterSrcRef.current = src
      setMeterAnalyser(an)
      void ctx.resume().catch(() => {})
    } catch {
      // createMediaElementSource threw (already captured / cross-origin / no Web
      // Audio) — the element was NOT rerouted, so it still plays normally; just no meter.
    }
  }, [])

  useEffect(() => {
    const onShow = () => setComposed(true)
    document.addEventListener('cut:show-composed', onShow)
    return () => document.removeEventListener('cut:show-composed', onShow)
  }, [])

  useEffect(() => {
    const onFocus = () => {
      rootRef.current?.scrollIntoView({ block: 'nearest' })
      rootRef.current?.focus({ preventScroll: true })
    }
    document.addEventListener('cut:focus-preview', onFocus)
    return () => document.removeEventListener('cut:focus-preview', onFocus)
  }, [])

  // #193b: receive the Title drawer's live placement → show/hide the draggable ghost.
  useEffect(() => {
    const onPlace = (e: Event) => {
      if (!(e instanceof CustomEvent) || !isObject(e.detail)) {
        setTitleGhost(null)
        return
      }
      const d = e.detail
      const active = 'active' in d && d.active === true
      const x = 'x' in d && typeof d.x === 'number' ? d.x : 0.5
      const y = 'y' in d && typeof d.y === 'number' ? d.y : 0.85
      const text = 'text' in d && typeof d.text === 'string' ? d.text : 'Title'
      const align = 'align' in d && isTitleAlign(d.align) ? d.align : 'center'
      setTitleGhost(
        active
          ? { x, y, text, align }
          : null,
      )
    }
    document.addEventListener('cut:title-place', onPlace)
    return () => document.removeEventListener('cut:title-place', onPlace)
  }, [])
  // Drag the ghost over the stage → normalized x/y back to the Title drawer.
  const onTitleGhostDrag = useCallback((e: React.PointerEvent) => {
    const r = stageRef.current?.getBoundingClientRect()
    if (!r || r.width === 0 || r.height === 0) return
    const x = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width))
    const y = Math.min(1, Math.max(0, (e.clientY - r.top) / r.height))
    document.dispatchEvent(new CustomEvent('cut:title-place-move', { detail: { x, y } }))
  }, [])

  // Arm/disarm draw-region mode from the Inspector's "Draw region" toggle.
  // detail = {active:true, clip, mode} to arm, {active:false} to disarm. Clears
  // any half-drawn box when disarmed so a cancel leaves no ghost.
  useEffect(() => {
    const onArm = (e: Event) => {
      const d = e instanceof CustomEvent && isObject(e.detail) ? e.detail : null
      const active = d && 'active' in d && d.active === true
      const clip = d && 'clip' in d && typeof d.clip === 'string' ? d.clip : ''
      const mode = d && 'mode' in d && isRedactMode(d.mode) ? d.mode : 'blur'
      if (active && clip) {
        setRedactArm({ clip, mode })
      } else {
        setRedactArm(null)
        setDrawBox(null)
        drawingRef.current = false
      }
    }
    document.addEventListener('cut:redact-draw', onArm)
    return () => document.removeEventListener('cut:redact-draw', onArm)
  }, [])

  // Arm/disarm the region-MASK draw from the Mask drawer. detail = {active:true, clip,
  // shape, nonce} arms (or re-arms with a new shape / cleared shape); {active:false}
  // disarms. Mirrors the redact arm above; MaskOverlay (keyed by clip+shape+nonce)
  // resets its geometry on any change.
  useEffect(() => {
    const onArm = (e: Event) => {
      const d = e instanceof CustomEvent && isObject(e.detail) ? e.detail : null
      const active = d && 'active' in d && d.active === true
      const clip = d && 'clip' in d && typeof d.clip === 'string' ? d.clip : ''
      const shape = d && 'shape' in d && isMaskShape(d.shape) ? d.shape : 'rect'
      const nonce = d && 'nonce' in d && typeof d.nonce === 'number' ? d.nonce : 0
      if (active && clip) setMaskArm({ clip, shape, nonce })
      else setMaskArm(null)
    }
    document.addEventListener('cut:mask-draw', onArm)
    return () => document.removeEventListener('cut:mask-draw', onArm)
  }, [])

  /** Map a clientX/Y to NORMALIZED fractions of the LETTERBOXED frame. The
   *  stageRef element IS the contain-fitted frame box (its rect already excludes
   *  the black letterbox bars), so (clientX - rect.left)/rect.width is exactly
   *  the frame-space X fraction. Clamped to [0,1] so a drag off-frame still
   *  yields valid coords. */
  const stageFrac = useCallback((clientX: number, clientY: number): { x: number; y: number } | null => {
    const r = stageRef.current?.getBoundingClientRect()
    if (!r || r.width === 0 || r.height === 0) return null
    return {
      x: Math.min(1, Math.max(0, (clientX - r.left) / r.width)),
      y: Math.min(1, Math.max(0, (clientY - r.top) / r.height)),
    }
  }, [])

  // Drag lifecycle on the stage while a redact draw is ARMED. down = anchor the
  // box; move = grow it; up = commit. Coordinates are normalized frame fractions
  // throughout (no px stored), so the box is resolution-independent.
  const onRedactDown = useCallback((e: React.PointerEvent) => {
    if (!redactArm) return
    const p = stageFrac(e.clientX, e.clientY)
    if (!p) return
    e.preventDefault()
    e.currentTarget.setPointerCapture(e.pointerId)
    drawingRef.current = true
    setDrawBox({ x0: p.x, y0: p.y, x1: p.x, y1: p.y })
  }, [redactArm, stageFrac])
  const onRedactMove = useCallback((e: React.PointerEvent) => {
    if (!drawingRef.current) return
    const p = stageFrac(e.clientX, e.clientY)
    if (!p) return
    setDrawBox((b) => (b ? { ...b, x1: p.x, y1: p.y } : b))
  }, [stageFrac])
  const onRedactUp = useCallback((e: React.PointerEvent) => {
    if (!drawingRef.current) return
    drawingRef.current = false
    try { e.currentTarget.releasePointerCapture(e.pointerId) } catch { /* not captured */ }
    const arm = redactArm
    const b = drawBox
    setDrawBox(null)
    if (!arm || !b) return
    // Normalize corners to top-left / bottom-right and reject a too-small box (a
    // stray click, not a drag) — < ~2% of frame in either axis.
    const x0 = Math.min(b.x0, b.x1), y0 = Math.min(b.y0, b.y1)
    const x1 = Math.max(b.x0, b.x1), y1 = Math.max(b.y0, b.y1)
    if (x1 - x0 < 0.02 || y1 - y0 < 0.02) {
      document.dispatchEvent(new CustomEvent('cut:redact-draw-done', { detail: { ok: false } }))
      return
    }
    const points: [number, number][] = [
      [+x0.toFixed(4), +y0.toFixed(4)],
      [+x1.toFixed(4), +y1.toFixed(4)],
    ]
    // box mode uses a solid fill — give it 0 strength (no blur radius); blur/
    // pixelate get a sensible default radius the engine clamps.
    const strength = arm.mode === 'blur' ? 25 : arm.mode === 'pixelate' ? 16 : 0
    void runUserVerb('edit.redact', {
      clip: arm.clip,
      shape: 'rect',
      points,
      mode: arm.mode,
      strength,
      rationale: `inspector: redact ${arm.mode} drawn region`,
    }, 'Could not obscure the drawn region.').then((r) => {
      if (r?.ok) document.dispatchEvent(new CustomEvent('cut:show-composed'))
      document.dispatchEvent(new CustomEvent('cut:redact-draw-done', { detail: { ok: Boolean(r?.ok) } }))
    })
  }, [redactArm, drawBox])

  // Honest "building proxy..." indicator. The import chain emits a
  // coarse milestone (message "proxy" at 0.3) when it ENTERS the proxy encode and
  // moves to "transcribe"/"perception"/"done" when it leaves — make_proxy itself
  // runs as one blocking call with no sub-progress, so we show an INDETERMINATE
  // "building" state (the pulsing dot), never a fake stuck percentage. Global (any
  // import's proxy step), which fits the common "import one heavy file" flow.
  const [proxyBuilding, setProxyBuilding] = useState(false)
  useEffect(() => {
    return events.onEvent((ev) => {
      if (ev.type !== 'job_progress') return
      const m = ev.message ?? ''
      if (m === 'proxy') setProxyBuilding(true)
      else if (m === 'transcribe' || m === 'perception' || m.startsWith('done')) setProxyBuilding(false)
    })
  }, [])

  // Playback stops at the real content end (contentExtentMs), not projectDurationMs
  // (which floors at a 60s editing canvas) — otherwise a short clip/recording plays into
  // black up to 60s with the playhead crawling the empty region (and the transport wiggling).
  const durationMs = useMemo(() => contentExtentMs(project), [project])
  const fps = project?.settings.fps ?? 30
  const frameMs = 1000 / fps
  const timeMode = useTimeDisplay() // shared ms/frames/SMPTE readout (toggle in the timeline)
  const video = useMemo(
    () => activeVideo(project, playheadMs, failedSources),
    [project, playheadMs, failedSources],
  )
  const hasAssets = !!project && Object.keys(project.assets).length > 0
  // --- live composite stage --------------------------------------------------
  // The monitor is measured so the stage gets an exact letterboxed pixel box at
  // the project aspect; overlays then position in normalized stage-% and land
  // precisely (incl. vertical/shorts geometry), captions size in cqh.
  const monitorRef = useRef<HTMLDivElement>(null)
  // The letterboxed stage element — the normalized coordinate space the on-canvas
  // transform handles measure against (Δpx / stageRect.dim = normalized delta).
  const stageRef = useRef<HTMLDivElement>(null)
  // The whole Preview panel — the Fullscreen API target: monitor AND
  // transport go full-screen together so playback stays controllable.
  const rootRef = useRef<HTMLElement>(null)
  const { isFullscreen, toggleFullscreen, guides, cycleGuides } = usePreviewViewOptions(rootRef)
  // While dragging an overlay's on-canvas handles, the LIVE transform (so the
  // overlay <video> + the handle box move together before the edit.transform commit).
  const [dragOverride, setDragOverride] = useState<{ clipId: string; transform: Required<ClipTransform> } | null>(null)
  const imageAssets = useMemo(() => imageAssetIds(project), [project])
  // COMPOSED playback must remain a playback surface. Exact engine frames stay
  // authoritative while paused/scrubbing; while the clock runs, the existing
  // GPU/DOM composite presents the base proxy, grade, overlays, and captions in
  // real time. Otherwise one synchronous FFmpeg/JPEG render gates every visible
  // frame and a simple brightness edit can make playback look frozen on macOS.
  const primaryTrackId = useMemo(() => baseVideoTrackId(project?.tracks ?? []), [project])
  const aspect = useMemo(() => stageAspect(project), [project])
  const stageBox = useContainBox(monitorRef, aspect)
  const projectHeight = project?.settings.height ?? 1080
  // Overlay video layers above the base, caption clips, and the base clip's
  // grade (CSS-approximated). During COMPOSED playback these keep the monitor
  // responsive; when playback stops, the exact composed poster replaces them.
  const { overlays, dropped: overlaysDropped } = useMemo(
    () => resolveOverlays(project, playheadMs, primaryTrackId, imageAssets),
    [project, playheadMs, primaryTrackId, imageAssets],
  )
  const { baseOffline, onlineOverlays, offlineOverlays, refresh: refreshOfflineMedia,
    relinkAsset, relinkingAssetId } = usePreviewOfflineMedia(project, playheadMs, overlays)
  const showVideo = !baseOffline && shouldUseLivePreviewSurface(!!video && !imageAssets.has(video.assetId), composed, rate)
  const liveComposedPlayback = composed && showVideo
  const captions = useMemo(() => resolveCaptions(project, playheadMs), [project, playheadMs])
  const baseGrade = useMemo(
    () => baseGradeAt(project, playheadMs, primaryTrackId, imageAssets),
    [project, playheadMs, primaryTrackId, imageAssets],
  )
  const baseFilter = gradeFilter(baseGrade)
  // Free-run applies only to forward 1× play of a mounted <video>. rVFC is the
  // primary clock; a watchdog below falls back to currentTime when a source codec
  // plays but does not deliver frame callbacks. Reverse/shuttle and composed/
  // poster modes use the seek-driven rAF clock below.
  const canFreeRun = rate === 1 && showVideo && RVFC_SUPPORTED && !video?.reverse

  // cfg ref: the playback clock + key handler read current values without
  // re-binding (same configRef pattern as the Timeline gestures). `video`
  // carries the active clip placement so the rVFC clock maps presented source
  // time → timeline position without re-subscribing each frame.
  const cfg = useRef({ playheadMs, durationMs, frameMs, onSeek, rate, video })
  cfg.current = { playheadMs, durationMs, frameMs, onSeek, rate, video }

  // --- playback clock --------------------------------------------------------
  // FREE-RUN (forward 1×, <video> mounted): the element plays itself, hardware-
  // decoded + vsync-timed; we read the presented frame back via rVFC when it is
  // available, with a currentTime watchdog for playable sources whose rVFC never
  // fires. SEEK PATH
  // (reverse / 2×/4× shuttle / composed-poster with no <video>): the original
  // rAF clock that repositions the playhead ~10×/s. playbackSrcKey re-arms the
  // effect when the clip under the playhead changes (proxy src swap at a cut),
  // so play resumes on the new source.
  const playbackSrcKey = canFreeRun && video
    ? `${video.clipId}:${video.src}:${video.srcInMs}:${video.srcOutMs}:${video.speed}`
    : null
  useEffect(() => {
    if (rate === 0) return
    const v = videoRef.current

    const callbacks = v ? videoFrameCallbacks(v) : null
    if (canFreeRun && v && callbacks) {
      let rafId = 0
      let fallbackRaf = 0
      let stopped = false
      let handedOff = false
      let lastDispatch = 0
      let lastFrameAt = performance.now()
      const startPlay = () => {
        const c = cfg.current
        const want = c.video ? c.video.srcMs / 1000 : v.currentTime
        if (Math.abs(v.currentTime - want) > c.frameMs / 1000) v.currentTime = want
        v.playbackRate = c.video?.speed ?? 1
        void v.play().catch(() => {})
      }
      // Cross the clip's out point → jump the playhead to the next clip start;
      // the src swap re-arms this effect, which reseeks + plays the next proxy.
      const advance = (toMs: number) => {
        if (handedOff) return
        handedOff = true
        cfg.current.onSeek(Math.round(toMs))
      }
      const syncFromMediaTime = (now: number, mediaTime: number) => {
        if (stopped) return
        const c = cfg.current
        const vid = c.video
        if (vid) {
          const timelineMs = timelineMsAtSourcePosition(vid, mediaTime * 1000)
          const clipEndMs = vid.startMs + vid.durMs
          if (timelineMs >= c.durationMs - 1) { c.onSeek(Math.round(c.durationMs)); setRate(0); return }
          if (timelineMs >= clipEndMs - 1) advance(Math.min(c.durationMs, clipEndMs))
          else if (now - lastDispatch >= 100) { lastDispatch = now; c.onSeek(Math.round(timelineMs)) }
        }
      }
      const onFrame = (now: number, meta: { mediaTime: number }) => {
        if (stopped) return
        lastFrameAt = now
        syncFromMediaTime(now, meta.mediaTime)
        rafId = callbacks.request(onFrame)
      }
      const fallbackTick = (now: number) => {
        if (stopped) return
        if (!v.paused && v.readyState >= 2 && now - lastFrameAt > 250) syncFromMediaTime(now, v.currentTime)
        fallbackRaf = requestAnimationFrame(fallbackTick)
      }
      const onReady = () => startPlay()
      const onEnded = () => {
        const c = cfg.current
        if (c.video) advance(Math.min(c.durationMs, c.video.startMs + c.video.durMs))
      }
      v.addEventListener('canplay', onReady)
      v.addEventListener('ended', onEnded)
      if (v.readyState >= 2) startPlay()
      rafId = callbacks.request(onFrame)
      fallbackRaf = requestAnimationFrame(fallbackTick)
      return () => {
        stopped = true
        v.removeEventListener('canplay', onReady)
        v.removeEventListener('ended', onEnded)
        callbacks.cancel(rafId)
        cancelAnimationFrame(fallbackRaf)
        v.pause()
        const c = cfg.current
        c.onSeek(Math.round(Math.max(0, Math.min(c.durationMs, c.playheadMs))))
      }
    }

    // SEEK PATH (unchanged 10Hz rAF clock).
    let raf = 0
    let last = performance.now()
    let acc = cfg.current.playheadMs
    let lastDispatch = 0
    const tick = (now: number) => {
      const c = cfg.current
      acc += (now - last) * c.rate
      last = now
      if (acc <= 0 || acc >= c.durationMs) {
        const clamped = Math.max(0, Math.min(c.durationMs, Math.round(acc)))
        c.onSeek(clamped)
        setRate(0)
        return
      }
      if (now - lastDispatch >= 100) {
        lastDispatch = now
        c.onSeek(Math.round(acc))
      }
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    return () => {
      cancelAnimationFrame(raf)
      cfg.current.onSeek(Math.round(Math.max(0, Math.min(cfg.current.durationMs, acc))))
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rate, playbackSrcKey])

  // --- preview <video> follows the playhead WHEN NOT free-running ------------
  // During forward 1× free-run the element owns currentTime (its own clock);
  // seeking it here would fight playback. Otherwise (paused scrub, reverse/
  // shuttle seek path) keep the displayed frame synced to the playhead.
  useEffect(() => {
    const v = videoRef.current
    if (!v || !video) return
    if (canFreeRun) return
    const want = video.srcMs / 1000
    if (Math.abs(v.currentTime - want) > frameMs / 2000) v.currentTime = want
  }, [video, frameMs, canFreeRun])

  // --- poster staleness: dim immediately, spinner only past 150ms ------
  // Cache-bust the poster with the head op id so it refreshes after any
  // edit (the composed frame at the same at_ms changes when the timeline does).
  // Show the proxy <video> only in PROXY mode where a proxy exists; otherwise a
  // composed-frame poster (compose=1 in COMPOSED mode so visual edits show).
  // The composed/proxy poster is SERVER-rendered. While PLAYING
  // (rate≠0) the playhead ticks ~10×/s and, in COMPOSED mode (no <video> to
  // free-run), each tick would refetch a fresh composed frame. A TITLED composed
  // frame is slow to rasterize (resvg + ffmpeg overlay), so 10 requests/sec back
  // renders back up and saturate cutd, making the whole app hang during title
  // edits. SELF-CLOCK it: keep at most ONE
  // composed frame in flight during play. A request marks the slot busy; the slot
  // frees only when that frame has loaded (onPosterSettled), and the next playhead
  // tick then requests the freshest position. So it degrades to "as fast as the
  // server can render" and can NEVER queue — no storm regardless of title cost.
  // When PAUSED (rate===0) the poster tracks the playhead exactly (scrubbing is
  // human-paced, so there is no storm to throttle).
  const [posterMs, setPosterMs] = useState(playheadMs)
  const posterInFlight = useRef(false)
  useEffect(() => {
    if (rate === 0) { posterInFlight.current = false; setPosterMs(playheadMs); return }
    if (!posterInFlight.current) { posterInFlight.current = true; setPosterMs(playheadMs) }
  }, [playheadMs, rate])
  // Bounded retry for a poster frame that errors transiently — a freshly imported
  // asset's render pipeline can briefly 4xx before it's warm (the preview would
  // otherwise flash broken + log a console error on import). `r` is an inert
  // cache-bust the server ignores; retries reset whenever the frame identity moves.
  const [posterNonce, setPosterNonce] = useState(0)
  const [posterFailed, setPosterFailed] = useState(false)
  const posterRetries = useRef(0)
  useEffect(() => { posterRetries.current = 0; setPosterFailed(false) }, [posterMs, headOpId, composed])
  const posterRequestMs = previewFrameMs(posterMs, durationMs, frameMs)
  const posterSrc = !baseOffline && !showVideo && hasAssets ? `${frameUrl(posterRequestMs, headOpId, composed)}${posterNonce ? `&r=${posterNonce}` : ''}` : null
  const onPosterError = () => {
    if (posterRetries.current < 2) { posterRetries.current += 1; setTimeout(() => setPosterNonce((n) => n + 1), 400); return }
    setPosterFailed(true)
    void refreshOfflineMedia()
    onPosterSettled()
  }
  // Spinner timer in a ref so onLoad/onError can CANCEL it. The bug: a poster
  // that loads within 150ms (small/cached frames — every still image) fired
  // onLoad → spinner off, but the pending 150ms timer then fired → spinner ON
  // and STUCK (no posterSrc change to clean it up). That was the "endless
  // rendering" over an image. Clearing the timer on load/error fixes it.
  const spinnerTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const clearSpinnerTimer = () => { if (spinnerTimer.current) { clearTimeout(spinnerTimer.current); spinnerTimer.current = null } }
  // A composed/proxy poster finished (loaded or errored): drop the stale tint +
  // spinner, AND free the in-flight slot so the next playhead tick can request
  // the next frame (the ≤1-in-flight self-clock).
  const onPosterSettled = () => {
    clearSpinnerTimer()
    setPosterStale(false)
    setShowSpinner(false)
    posterInFlight.current = false
  }
  useEffect(() => {
    if (!posterSrc) return
    setPosterStale(true)
    clearSpinnerTimer()
    spinnerTimer.current = setTimeout(() => setShowSpinner(true), 150)
    return () => { clearSpinnerTimer(); setShowSpinner(false) }
  }, [posterSrc])

  // --- timeline audio mix ---------------------------------------------------
  // Render (or reuse) the timeline-audio mix for the current edit. Lazy: only
  // when monitoring is on and the cached mix is stale for the head op id.
  // export.audio is synchronous (ffmpeg) → a short build on the first play after
  // an edit; during continuous play with no edits it never re-renders. A timeline
  // with no audio just yields no usable mix → the preview stays silent (no error).
  const ensureMix = useCallback(async () => {
    if (!audioOn || mixBusy) return
    if (mixForOp === (headOpId ?? '0')) return // already RESOLVED for this edit (built OR confirmed no audio)
    setMixBusy(true)
    try {
      // Render to the OTHER buffer than the one the <audio> is currently reading,
      // then flip — so an edit-driven re-render never overwrites the live file.
      const buf = mixBuf.current === 'a' ? 'b' : 'a'
      const r = await callVerb('export.audio', {
        format: 'mp3',
        path: `exports/_monitor_${buf}.mp3`,
        rationale: 'timeline audio monitor',
      })
      const path = r.ok ? (r.result as { path?: string })?.path : undefined
      if (path) {
        mixBuf.current = buf
        const base = exportUrl(path)
        setMixUrl(`${base}${base.includes('?') ? '&' : '?'}v=${encodeURIComponent(headOpId ?? '0')}`)
      }
    } finally {
      // Mark this edit's mix resolved regardless of outcome (built / no-audio /
      // failed) so the pre-warm effect fires at most ONCE per edit. Before this, a project with
      // NO usable audio never set mixUrl → the freshness guard never tripped → the pre-warm
      // re-scheduled ensureMix every 700ms FOREVER; each pass pulsed mixBusy true→false, flipping
      // the Audio button label 'Audio'⇄'…' → its width changed → the bar's right group reflowed =
      // the transport-bar wiggle. (Edit-driven rebuilds still fire: a new headOpId restales mixForOp.)
      setMixForOp(headOpId ?? '0')
      setMixBusy(false)
    }
  }, [audioOn, mixBusy, mixForOp, headOpId])

  // Kick a mix render when 1× playback starts (or monitoring is toggled on) and
  // the cached mix is stale for the current edit.
  useEffect(() => {
    if (audioOn && rate === 1) void ensureMix()
  }, [rate, audioOn, headOpId, ensureMix])

  // Pre-warm the mix when an edit SETTLES, even while paused, so the FIRST play
  // after a new clip/recording already has audio. The previous race produced
  // "plays on the second replay, not the first" — the mix used to build only on play,
  // so the first play raced ahead of the ffmpeg export.audio render and was
  // silent; the 2nd play found the cached mix. Debounced 700ms so rapid edits
  // don't hammer export.audio; the freshness + mixBusy guards in ensureMix make
  // a redundant fire a no-op. preload="auto" on the <audio> then buffers it so
  // the first transport play is immediate.
  useEffect(() => {
    if (!audioOn) return
    if (mixForOp === (headOpId ?? '0')) return // already resolved for this edit (keyed on mixForOp, not mixUrl)
    const t = setTimeout(() => { void ensureMix() }, 700)
    return () => clearTimeout(t)
  }, [audioOn, headOpId, mixForOp, ensureMix])

  // On project switch, drop the monitor mix immediately. mixUrl points at the OLD
  // project's exports/_monitor_*.mp3, but /api/export resolves against the CURRENT
  // project's dir → a console 404 on the stale file until the new mix renders.
  // Imperatively abort the <audio> first (pause + remove src + load) so an
  // in-flight fetch is cancelled cleanly rather than completing as a late 404,
  // then clear state; the pre-warm above rebuilds the mix for the new project.
  useEffect(() => {
    const a = audioRef.current
    if (a) {
      try { a.pause(); a.removeAttribute('src'); a.load() } catch { /* element gone */ }
    }
    lastAudioResyncAt.current = 0
    setMixUrl(null)
    setMixForOp(null)
  }, [projectKey])

  // Play/pause the mix with the transport — 1× forward only (no audio scrub).
  useEffect(() => {
    const a = audioRef.current
    if (!a) return
    if (audioOn && mixUrl && rate === 1) {
      a.currentTime = cfg.current.playheadMs / 1000
      lastAudioResyncAt.current = performance.now()
      void a.play().catch(() => {})
    } else {
      a.pause()
    }
  }, [rate, mixUrl, audioOn])

  // Drift nudge: the VIDEO/seek playhead is authoritative; if the mix <audio>
  // drifts past ~250ms during 1× play, snap it back. Runs on the 100ms playhead
  // dispatch and corrects only on real drift (no per-tick reseek stutter).
  useEffect(() => {
    const a = audioRef.current
    if (!a || !audioOn || !mixUrl || rate !== 1) return
    const now = performance.now()
    const target = monitorAudioResyncTarget({
      audioTimeS: a.currentTime,
      playheadMs,
      nowMs: now,
      lastResyncAtMs: lastAudioResyncAt.current,
    })
    if (target != null) {
      a.currentTime = target
      lastAudioResyncAt.current = now
    }
  }, [playheadMs, rate, audioOn, mixUrl])

  // --- transport actions ------------------------------------------------------
  const playPause = useCallback(() => setRate((r) => (r === 0 ? 1 : 0)), [])
  /** J/L shuttle ladder: repeat presses double the rate up to 4×. */
  const shuttle = useCallback((dir: -1 | 1) => {
    setRate((r) => {
      const sameDir = Math.sign(r) === dir
      const mag = sameDir ? Math.min(4, Math.abs(r) * 2) : 1
      return (dir * mag) as Rate
    })
  }, [])
  const nudge = useCallback((frames: number) => {
    const c = cfg.current
    setRate(0)
    c.onSeek(Math.max(0, Math.min(c.durationMs, Math.round(c.playheadMs + frames * c.frameMs))))
  }, [])
  const seekTo = useCallback((ms: number) => {
    setRate(0)
    cfg.current.onSeek(Math.max(0, Math.min(cfg.current.durationMs, ms)))
  }, [])

  // --- GLOBAL transport keys -------------------------------------------------
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (shouldIgnoreGlobalShortcut(e)) return
      // Rail-scope convention (set by the review rail when focused).
      if (document.documentElement.dataset.cutKbscope === 'rail') return
      if (e.ctrlKey || e.metaKey || e.altKey) return
      // Transport actions consult the REMAPPABLE keymap (lib/keymap.ts) —
      // bindings read live so a remap in Settings applies instantly. Arrows/
      // Home/End are conventions, not preferences — they stay literal.
      if (matchesAction(e, 'preview.playPause')) playPause()
      else if (matchesAction(e, 'preview.shuttleBack')) shuttle(-1)
      else if (matchesAction(e, 'preview.shuttleFwd')) shuttle(1)
      else if (matchesAction(e, 'preview.stop')) setRate(0)
      else if (matchesAction(e, 'preview.fullscreen')) toggleFullscreen()
      else if (matchesAction(e, 'preview.guides')) cycleGuides()
      else {
        switch (e.key) {
          case 'ArrowLeft': nudge(e.shiftKey ? -10 : -1); break
          case 'ArrowRight': nudge(e.shiftKey ? 10 : 1); break
          case 'Home': seekTo(0); break
          case 'End': seekTo(cfg.current.durationMs); break
          default: return
        }
      }
      e.preventDefault()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [playPause, shuttle, nudge, seekTo, toggleFullscreen, cycleGuides])

  const {
    exact,
    exactBusy,
    exactNote,
    hasSection,
    saveBusy,
    snapBusy,
    snapNote,
    closeExactReview,
    renderSection,
    saveSection,
    snapFrame,
  } = usePreviewExportActions({ project, playheadMs, selectedClipIds, exportRange, setRate })

  // Commit an on-canvas transform drag (edit.transform) and flip to the COMPOSED
  // frame so the exact composite is visible. Clearing the override after the verb
  // returns avoids a one-frame jump back to the pre-drag position.
  const commitTransform = useCallback(async (clipId: string, t: Required<ClipTransform>) => {
    const r = await runUserVerb('edit.transform', {
      clip: clipId,
      x: +t.x.toFixed(4),
      y: +t.y.toFixed(4),
      scale: +t.scale.toFixed(4),
      opacity: +t.opacity.toFixed(4),
      rationale: 'user: on-canvas transform',
    }, 'Could not move or resize this overlay.')
    setDragOverride(null)
    if (r?.ok) document.dispatchEvent(new CustomEvent('cut:show-composed'))
  }, [])
  // The selected overlays currently visible at the playhead → show their handles.
  const handleOverlays = useMemo(
    () => onlineOverlays.filter((o) => (selectedClipIds ?? []).includes(o.clipId)),
    [onlineOverlays, selectedClipIds],
  )

  const playing = rate !== 0
  return (
    <section ref={rootRef} tabIndex={-1} className="panel pv-root" data-panel="preview" data-cut-panel="preview" data-cut-playing={playing ? 'true' : 'false'} data-cut-fullscreen={isFullscreen ? 'true' : 'false'}>
      {/* Hidden timeline-audio element: the sole sound source — the preview
          <video>s are muted, so this plays the full mix synced to the playhead. */}
      <audio
        ref={audioRef}
        src={mixUrl ?? undefined}
        preload="auto"
        data-cut-timeline-audio
        style={{ display: 'none' }}
        onPlay={setupMeter}
        onLoadedMetadata={() => {
          const a = audioRef.current
          if (a && audioOn && cfg.current.rate === 1) {
            a.currentTime = cfg.current.playheadMs / 1000
            lastAudioResyncAt.current = performance.now()
            void a.play().catch(() => {})
          }
        }}
      />
      <div ref={monitorRef} className={`pv-monitor ${playing ? 'pv-monitor--playing' : ''}`} data-cut-monitor>
        {ffmpegMissing && (
          <div className="pv-setup" data-cut-preview-ffmpeg-setup role="status" aria-live="polite">
            <div className="pv-setup-copy">
              <strong>Video tools needed</strong>
              <span>Install FFmpeg to preview, import, and export media.</span>
            </div>
            <div className="pv-setup-actions">
              <button
                type="button"
                className="pv-setup-btn"
                data-cut-preview-install-ffmpeg
                onClick={openVideoToolsSetup}
                title="Open Settings and install video processing"
              >
                Install FFmpeg
              </button>
              <button
                type="button"
                className="pv-setup-btn pv-setup-btn--ghost"
                data-cut-preview-ffmpeg-guide
                onClick={openVideoToolsGuide}
                title="Open the FFmpeg setup guide"
              >
                Guide
              </button>
              <button
                type="button"
                className="pv-setup-btn pv-setup-btn--ghost"
                data-cut-preview-ffmpeg-recheck
                onClick={recheckVideoTools}
                title="Re-check video tools after installing FFmpeg"
              >
                Re-check
              </button>
            </div>
          </div>
        )}
        {baseOffline ? (
          <PreviewOfflineStage stageRef={stageRef} stageBox={stageBox} asset={baseOffline}
            relinking={relinkingAssetId === baseOffline.id} onRelink={relinkAsset} />
        ) : showVideo || posterSrc ? (
          // Composite STAGE: an explicit letterboxed box at the project aspect.
          // Overlays + captions position in normalized stage coords so PiP /
          // vertical-shorts geometry lands exactly in the live composite.
          <div
            ref={stageRef}
            className="pv-stage"
            data-cut-stage
            data-cut-preview-surface={liveComposedPlayback ? 'live-composite' : showVideo ? 'live-source' : composed ? 'exact-frame' : 'scrub-frame'}
            style={{ width: stageBox.w || undefined, height: stageBox.h || undefined }}
          >
            {showVideo ? (
              <>
                <video
                  ref={videoRef}
                  className="pv-base"
                  src={video!.src}
                  muted
                  playsInline
                  data-cut-video
                  data-cut-video-kind={video!.kind}
                  // Base clip's color grade, CSS-approximated (exact = COMPOSED / final).
                  style={baseFilter ? { filter: baseFilter } : undefined}
                  // If the source cannot be decoded (raw / unsupported
                  // codec → MediaError DECODE/SRC_NOT_SUPPORTED), stop offering it and
                  // fall back to the composed poster until the proxy lands. Ignore
                  // transient ABORTED/NETWORK errors so a blip doesn't disable preview.
                  onError={(e) => {
                    if (video!.kind !== 'source') return
                    const code = e.currentTarget.error?.code
                    if (code === 3 || code === 4) {
                      const id = video!.assetId
                      setFailedSources((s) => (s.has(id) ? s : new Set(s).add(id)))
                      void refreshOfflineMedia()
                    }
                  }}
                />
                {/* Overlay video tracks above the base (PiP / B-roll / lower-thirds). */}
                {onlineOverlays.map((o) => (
                  <OverlayVideo
                    key={o.clipId}
                    layer={o}
                    playheadMs={playheadMs}
                    rate={rate}
                    override={dragOverride?.clipId === o.clipId ? dragOverride.transform : null}
                  />
                ))}
                <PreviewOfflineOverlays entries={offlineOverlays}
                  relinkingAssetId={relinkingAssetId} onRelink={relinkAsset} />
                {/* Live caption text. */}
                {captions.map((c) => (
                  <CaptionText key={c.id} cap={c} projectHeight={projectHeight} />
                ))}
              </>
            ) : posterFailed ? (
              <PreviewFrameError />
            ) : (
              <img
                src={posterSrc!}
                alt={`frame at ${timecode(playheadMs)}`}
                className={posterStale ? 'pv-stale' : ''}
                onLoad={onPosterSettled}
                // A frame that fails to load (engine error / past content) must NOT
                // leave the spinner spinning forever. Retry up to 2× (a freshly
                // imported asset can 4xx before its pipeline is warm); after that,
                // settle — clear the spinner, drop the stale tint, free the slot.
                onError={onPosterError}
                data-cut-poster
              />
            )}
            {/* Framing guides: thirds and safe-area overlay — pure SVG,
                pointer-events none, tracks the letterboxed stage in %. Under the
                transform handles so interaction chrome stays on top. */}
            <GuideOverlay mode={guides} />
            {/* On-canvas transform handles for the selected, currently-visible
                overlay(s): drag the body to move, a corner to scale. Normalized
                state; commits edit.transform on pointer-up. Rendered in BOTH live
                and composed modes (the box tracks the stage either way). */}
            {handleOverlays.map((o) => (
              <TransformHandles
                key={o.clipId}
                clipId={o.clipId}
                transform={dragOverride?.clipId === o.clipId ? dragOverride.transform : o.transform}
                stageRef={stageRef}
                onLive={(t) => setDragOverride({ clipId: o.clipId, transform: t })}
                onCommit={(t) => void commitTransform(o.clipId, t)}
              />
            ))}
            {/* #193b: draggable TITLE GHOST — drag the title to any spot on the actual
                frame while the Title drawer's "Place anywhere" mode is active. */}
            {titleGhost && (
              <div
                className="pv-title-ghost"
                data-cut-title-ghost
                style={{ left: `${titleGhost.x * 100}%`, top: `${titleGhost.y * 100}%`, textAlign: titleGhost.align }}
                onPointerDown={(e) => { e.currentTarget.setPointerCapture(e.pointerId); onTitleGhostDrag(e) }}
                onPointerMove={(e) => { if (e.buttons === 1) onTitleGhostDrag(e) }}
              >
                {titleGhost.text}
              </div>
            )}
            {/* Privacy DRAW-REGION capture layer — present only when the Inspector
                has armed draw mode. It overlays the whole frame, grabs pointer
                input (crosshair cursor), and renders the live drag box. On
                pointer-up it fires edit.redact with the normalized rect (handler
                above). pointer-events sit on THIS layer only when armed, so normal
                preview interaction is untouched the rest of the time. */}
            {redactArm && (
              <div
                className="pv-redact-capture"
                data-cut-redact-capture
                onPointerDown={onRedactDown}
                onPointerMove={onRedactMove}
                onPointerUp={onRedactUp}
              >
                {drawBox && (
                  <div
                    className="pv-redact-box"
                    data-cut-redact-box
                    style={{
                      left: `${Math.min(drawBox.x0, drawBox.x1) * 100}%`,
                      top: `${Math.min(drawBox.y0, drawBox.y1) * 100}%`,
                      width: `${Math.abs(drawBox.x1 - drawBox.x0) * 100}%`,
                      height: `${Math.abs(drawBox.y1 - drawBox.y0) * 100}%`,
                    }}
                  />
                )}
              </div>
            )}
            {/* Region-MASK draw overlay (Mask drawer, Q2) — draw + drag-resize a
                rect/ellipse/polygon over the frame; reports the verb-ready geometry
                back to the drawer (cut:mask-geometry). Keyed by clip+shape+nonce so a
                shape change / "Clear shape" gives a fresh draw. */}
            {maskArm && (
              <MaskOverlay
                key={`${maskArm.clip}-${maskArm.shape}-${maskArm.nonce}`}
                shape={maskArm.shape}
                stageRef={stageRef}
                onGeometry={(g: MaskGeometry) =>
                  document.dispatchEvent(new CustomEvent('cut:mask-geometry', { detail: g }))
                }
              />
            )}
          </div>
        ) : (
          <PreviewEmptyState
            hasProject={!!project}
            onImport={() => document.dispatchEvent(new CustomEvent('cut:open-import'))}
          />
        )}

        <PreviewMonitorBadges
          showVideo={showVideo}
          video={video}
          proxyBuilding={proxyBuilding}
          overlaysDropped={overlaysDropped}
          posterActive={!!posterSrc}
          showSpinner={showSpinner}
          composed={composed}
          liveComposedPlayback={liveComposedPlayback}
        />

        {exact && (
          <PreviewExactReview
            exact={exact}
            exactNote={exactNote}
            saveBusy={saveBusy}
            onSave={() => void saveSection()}
            onExit={closeExactReview}
          />
        )}
      </div>

      <PreviewTransport
        playheadMs={playheadMs}
        durationMs={durationMs}
        fps={fps}
        timeMode={timeMode}
        rate={rate}
        playing={playing}
        snapNote={snapNote}
        snapBusy={snapBusy}
        hasProject={!!project}
        exactBusy={exactBusy}
        hasSection={hasSection}
        audioOn={audioOn}
        mixBusy={mixBusy}
        meterAnalyser={meterAnalyser}
        composed={composed}
        video={video}
        onSeekTo={seekTo}
        onShuttle={shuttle}
        onPlayPause={playPause}
        onSnapFrame={() => void snapFrame()}
        onRenderSection={() => void renderSection()}
        onAudioToggle={() => setAudioOn((v) => !v)}
        onComposedToggle={() => setComposed((c) => !c)}
        guides={guides}
        onCycleGuides={cycleGuides}
        isFullscreen={isFullscreen}
        onFullscreenToggle={toggleFullscreen}
      />
    </section>
  )
}
