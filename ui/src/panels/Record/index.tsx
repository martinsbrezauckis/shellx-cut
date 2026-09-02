// panels/Record — full-surface capture: Doctor → start → stop → optional polish.
// It defaults to an open-ended F9-stoppable recording; duration choices are caps.
// Selected monitor/window identities pass through the verb unchanged for native revalidation.

import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { withAuthorizedOutputPath } from '../../lib/exportDestination'
import { isBlockingOverlayActive, shouldIgnoreGlobalShortcut } from '../../lib/dom'
import { matchesFixedAction } from '../../lib/keymap'
import { isTauri, onRecordHotkey, pickExportOutput } from '../../lib/tauri'
import { runUserVerb } from '../../lib/userActionFeedback'
import { StudioControls } from './StudioControls'
import { StudioPreview } from './StudioPreview'
import { RecordingScenesControl } from './RecordingScenesControl'
import { RecordingPauseControl } from './RecordingPauseControl'
import { RegionPickerOverlay } from './RegionPickerOverlay'
import type { MonitorInfo, WindowInfo } from './RecordingSourceControl'
import {
  REGION_PICKER_UNAVAILABLE,
  type RecordingSourceKind,
} from './regionPickerModel'
import { MicInputControl } from './MicInputControl'
import { SystemAudioProbeControl } from './SystemAudioProbeControl'
import { RecordFrameRateControl } from './RecordFrameRateControl'
import { recordingFrameRateReason } from './recordingFrameRate'
import { RecordingCountdownControl } from './RecordingCountdownControl'
import { RecordingCountdownOverlay } from './RecordingCountdownOverlay'
import { RecordingRehearsal } from './RecordingRehearsal'
import { RecordingLiveControls } from './RecordingLiveControls'
import { RecordingAudioMeters } from './RecordingAudioMeters'
import { RecordingCaptureSafetyStatus } from './RecordingCaptureSafetyStatus'
import { RecordingSourceSetup } from './RecordingSourceSetup'
import { RecordingQualityControl } from './RecordingQualityControl'
import {
  cameraSelectionError,
  NO_CAMERA_CAPABILITY,
  type CameraCapability,
} from './CameraControl'
import { useRecordingCountdown } from './useRecordingCountdown'
import { useRecordingQuality } from './useRecordingQuality'
import {
  DUR_PRESETS,
  failureReason,
  fmtElapsed,
  outputFileLabel,
  recordCardLabel,
  type RecordCard,
} from './recordingUiModel'
import {
  probedAverageCadenceLabel,
  requestedCadenceLabel,
  type RecordingCadence,
} from './recordingCadence'
import { useRecordingExport } from './useRecordingExport'
import {
  clampCameraSize,
  defaultStudioState,
  placementForPosition,
  type CursorCorrelation,
  type StudioBackground,
  type StudioCameraPosition,
  type StudioCameraShape,
  type StudioEventPayload,
  type StudioRawStreams,
  type StudioState,
} from './studioTypes'
import {
  recordingSceneById,
  studioStateForRecordingScene,
  type RecordingSceneStartConfig,
} from './recordingScenes'
import { useRecordingScenes } from './useRecordingScenes'
import { useRecordingPause } from './useRecordingPause'
import { useRecordingAudioMeters } from './useRecordingAudioMeters'
import { useRecordingSourcePreview } from './useRecordingSourcePreview'
import './record.css'
export interface RecordProps {
  project: Project | null
  /** Re-sync App state after a polish drops a clip on the timeline. */
  onClipAdded?: () => void
  /** Open Settings at the shared default export folder row. */
  onOpenOutputSettings?: () => void
}
const OUTPUT_PATH_HINT = 'pick another file with "Choose file", or Clear it to use the default export folder'
type Phase = 'idle' | 'recording' | 'finalizing' | 'done' | 'error'

// The start/stop toggle binding uses one key rather than a multi-key chord. The
// desktop shell registers F9 as a GLOBAL OS hotkey (lib.rs) so it toggles even
// while another app is focused — essential for a recorder, whose own window is
// backgrounded the whole time it captures other apps. The same F9 also works as
// an in-page keydown FALLBACK here, covering the focused-window case and the
// plain web/dev build (where there is no shell to register the OS-level hotkey).
const SHORTCUT_LABEL = 'F9'

/** A marker is only announced after the Studio journal echoes the accepted event. */
function recordingMarkerAcknowledged(value: unknown, expectedLabel: string): boolean {
  if (!value || typeof value !== 'object') return false
  const result = value as { last_event?: unknown }
  if (!result.last_event || typeof result.last_event !== 'object') return false
  const event = result.last_event as Record<string, unknown>
  return event.source === 'recording'
    && event.kind === 'marker'
    && event.label === expectedLabel
    && typeof event.logical_ts === 'number'
    && Number.isSafeInteger(event.logical_ts)
    && event.logical_ts >= 0
}

export default function Record({ project, onClipAdded, onOpenOutputSettings }: RecordProps) {
  const [cards, setCards] = useState<RecordCard[]>([])
  const [ready, setReady] = useState<boolean | null>(null)
  // `start_allowed` is deliberately narrower than Doctor `ready`: on Linux,
  // Start is the user-initiated portal picker while Doctor stays unknown/non-green.
  const [startAllowed, setStartAllowed] = useState<boolean | null>(null)
  // Monitor PICKER: the doctor's enumerated displays (empty on single-display /
  // Linux), and the chosen 1-based monitor index (null = primary / engine default).
  const [monitors, setMonitors] = useState<MonitorInfo[]>([])
  const [monitorIdx, setMonitorIdx] = useState<number | null>(null)
  // Source is intentionally first-level: choose Display or Window before its
  // exact target. Region joins that choice only after the private native path
  // receives compiled/native qualification for its one-use exact ticket.
  const [sourceKind, setSourceKind] = useState<RecordingSourceKind>('display')
  const [regionPickerOpen, setRegionPickerOpen] = useState(false)
  const regionPickerCapability = REGION_PICKER_UNAVAILABLE
  // Window picker: the doctor's enumerated app windows and the chosen opaque
  // native target id. A Window source with null has an explicit no-selection
  // state; it must never silently fall back to Display capture.
  const [windows, setWindows] = useState<WindowInfo[]>([])
  const [windowTargetId, setWindowTargetId] = useState<string | null>(null)
  const [cameraCapability, setCameraCapability] = useState<CameraCapability>(NO_CAMERA_CAPABILITY)
  const [cameraDeviceId, setCameraDeviceId] = useState<string | null>(null)
  const selectedWindowMissing = sourceKind === 'window'
    && windowTargetId !== null
    && !windows.some((window) => window.id === windowTargetId)
  const windowNeedsSelection = sourceKind === 'window' && windowTargetId === null

  // A region state can only arrive from a future capability-backed bridge. If
  // a stale restored state somehow reaches this candidate, repair the visible
  // source immediately; `start` still refuses it during this render.
  useEffect(() => {
    if (sourceKind !== 'region' || regionPickerCapability.availability !== 'unavailable') return
    setSourceKind('display')
    setRegionPickerOpen(false)
  }, [sourceKind, regionPickerCapability.availability])
  // `capMs === null` = open-ended (the default). Otherwise it is the cap in ms.
  const [capMs, setCapMs] = useState<number | null>(null)
  const [fps, setFps] = useState(30)
  const [customFps, setCustomFps] = useState('')
  const customFpsError = customFps.trim() ? recordingFrameRateReason(customFps) : null
  const [audio, setAudio] = useState(true)
  // Capture DESKTOP/SYSTEM audio (game/app sound) as a SEPARATE mixable track.
  // Defaults OFF for safety: desktop audio capture should be an explicit opt-in.
  // Linux uses the PulseAudio-compatible monitor source.
  const [systemAudio, setSystemAudio] = useState(false)
  const [audioProbeRunning, setAudioProbeRunning] = useState(false)
  const [micTestRunning, setMicTestRunning] = useState(false)
  const [keys, setKeys] = useState(false)
  // When ON (default), stop → auto-polish (auto-zoom, cursor, framing) — the
  // slower re-render. When OFF, stop → a FAST stream-copy (raw:true) so the clip lands
  // on the timeline almost immediately; the user can polish later if they want.
  // (Only meaningful in AUTO-EDIT mode; ignored in RAW capture.)
  const [autoPolish, setAutoPolish] = useState(true)
  // RAW CAPTURE mode. false = AUTO-EDIT (the flagship: record → autoedit
  // → polish → a clip on the timeline). true = RAW: keep ALL the same capture options
  // (source, fps, mic + system-audio sources) but on stop SKIP autoedit AND polish —
  // the engine just folds the streams into one raw.mp4 (screen_record.stop{mux_raw}).
  // We surface the file and OFFER to add it as-is; nothing is auto-edited or auto-placed.
  // Default AUTO-EDIT so existing behaviour is unchanged; raw is the explicit opt-in.
  const [rawCapture, setRawCapture] = useState(false)
  const [studio, setStudio] = useState<StudioState>(() => defaultStudioState())
  const [lastRawStreams, setLastRawStreams] = useState<StudioRawStreams | null>(null)
  const [lastCursorCorrelation, setLastCursorCorrelation] = useState<CursorCorrelation | null>(null)
  // Server evidence stays separate from the local selector: the UI never
  // relabels the backend integer request or legacy f32 as measured media.
  const [captureCadence, setCaptureCadence] = useState<RecordingCadence | null>(null)
  const {
    capability: qualityCapability, outputSize, profile, resolution: qualityResolution,
    request: qualityRequest, setOutputSize, setProfile,
    setCapability: setQualityCapability, setResolution: setQualityResolution,
    clearResolution: clearQualityResolution,
  } = useRecordingQuality()
  const [lastCapture, setLastCapture] = useState<{ source: string; plan: string } | null>(null)
  const [lastRaw, setLastRaw] = useState<{ path: string; hasMic: boolean; hasSystem: boolean } | null>(null)
  const [exportFmt, setExportFmt] = useState<'mp4' | 'gif'>('mp4')
  const [recordOutputPath, setRecordOutputPath] = useState<string | null>(null)
  const [recordOutputNote, setRecordOutputNote] = useState('')
  const [exportNote, setExportNote] = useState('')
  const { exportJob, exportClip, cancelExport } = useRecordingExport({
    capture: lastCapture,
    format: exportFmt,
    outputPath: recordOutputPath,
    setNote: setExportNote,
  })
  // Seconds elapsed in the finalize/bake phase, so the wait is not opaque.
  const [finalizeSec, setFinalizeSec] = useState(0)
  const [phase, setPhase] = useState<Phase>('idle')
  const [sceneCaptureId, setSceneCaptureId] = useState<string | null>(null)
  // `remaining` is the cap countdown (only meaningful when capMs != null);
  // `elapsed` is the wall-clock since start (the open-ended clock).
  const [remaining, setRemaining] = useState(0)
  const [elapsed, setElapsed] = useState(0)
  const [note, setNote] = useState('')
  const [err, setErr] = useState<string | null>(null)
  const [markerPending, setMarkerPending] = useState(false)
  const previewRecordingScene = useCallback((sceneId: string) => {
    setStudio((current) => studioStateForRecordingScene(recordingSceneById(sceneId), current))
  }, [])
  const {
    capability: sceneCapability,
    selectedScene,
    startConfig: sceneStartConfig,
    status: sceneStatus,
    timer: sceneTimer,
    timerStatus: sceneTimerStatus,
    recovery: sceneRecovery,
    setDoctorCapability: setDoctorSceneCapability,
    refreshRecovery: refreshSceneRecovery,
    selectScene,
    selectTimer: selectSceneTimer,
    controlTimer: controlSceneTimer,
    markCaptureStarted: markSceneCaptureStarted,
  } = useRecordingScenes({
    projectOpen: Boolean(project),
    recording: phase === 'recording',
    captureId: sceneCaptureId,
    onPreviewScene: previewRecordingScene,
  })
  const recordingPause = useRecordingPause()
  const sourcePreview = useRecordingSourcePreview({
    sourceKind,
    monitors,
    monitorIdx,
    windows,
    windowTargetId,
    recording: phase === 'recording',
  })
  const recordingAudioMeters = useRecordingAudioMeters(sceneCaptureId, phase === 'recording')
  // Only this stable action belongs to Doctor's mount-time probe.  The hook
  // returns live state as well, so depending on its wrapper object here would
  // recreate `probe` after every Doctor result and continuously re-run it.
  const { setDoctorCapability: setPauseDoctorCapability } = recordingPause
  const updateCustomFps = useCallback((value: string) => {
    setCustomFps(value)
    if (err?.startsWith('Frame rate needs correction:')) setErr(null)
  }, [err])
  const tickRef = useRef<number | null>(null)
  // The capture_id of the in-flight recording — drives the manual Stop + shortcut.
  const captureRef = useRef<string | null>(null)
  const recordStartedAtRef = useRef<number | null>(null)
  // Timestamp of the last F9 toggle. When the Cut window is FOCUSED, one physical
  // F9 press can reach BOTH the global OS hotkey (→ cut:record-hotkey event) AND
  // the in-page keydown — the OS does not swallow a global shortcut for the
  // focused window on most platforms — which would toggle twice and cancel out.
  // Collapsing toggles within a short window to one keeps a single press = a
  // single start/stop, whichever path(s) fire.
  const lastToggleRef = useRef(0)
  const markerRequestRef = useRef(0)

  const emitStudioEvent = useCallback(async (payload: StudioEventPayload): Promise<boolean> => {
    const captureId = captureRef.current
    const startedAt = recordStartedAtRef.current ?? Date.now()
    if (!captureId || startedAt === null) return false
    const tMs = Math.max(0, Date.now() - startedAt)
    const result = await runUserVerb('screen_record.studio_event', {
      capture_id: captureId,
      event: { t_ms: tMs, ...payload },
    }, 'Could not save the live recording change.')
    if (!result?.ok && captureRef.current === captureId) {
      setNote('Live Studio changes may not replay in polish.')
    }
    return Boolean(result?.ok)
  }, [])

  const setStudioBackground = useCallback((background: StudioBackground) => {
    setStudio((prev) => ({ ...prev, background }))
    void emitStudioEvent({ source: 'background', kind: 'style', background })
  }, [emitStudioEvent])

  const setStudioCameraEnabled = useCallback((enabled: boolean) => {
    setStudio((prev) => ({ ...prev, camera: { ...prev.camera, enabled } }))
  }, [])

  const setStudioCameraPosition = useCallback((position: StudioCameraPosition) => {
    setStudio((prev) => {
      const placement = placementForPosition(position, prev.camera.size)
      const camera = { ...prev.camera, ...placement, position }
      void emitStudioEvent({
        source: 'camera', kind: 'transform', x: camera.x, y: camera.y,
        size: camera.size, shape: camera.shape,
      })
      return { ...prev, camera }
    })
  }, [emitStudioEvent])

  const setStudioCameraShape = useCallback((shape: StudioCameraShape) => {
    setStudio((prev) => {
      void emitStudioEvent({ source: 'camera', kind: 'transform', shape })
      return { ...prev, camera: { ...prev.camera, shape } }
    })
  }, [emitStudioEvent])

  const setStudioCameraSize = useCallback((value: number) => {
    setStudio((prev) => {
      const size = clampCameraSize(value)
      const placement = placementForPosition(prev.camera.position, size)
      void emitStudioEvent({ source: 'camera', kind: 'transform', ...placement, size })
      return { ...prev, camera: { ...prev.camera, ...placement, size } }
    })
  }, [emitStudioEvent])

  const addRecordingMarker = useCallback(() => {
    const captureId = captureRef.current
    if (phase !== 'recording' || !captureId) {
      setNote('Start recording before adding a marker.')
      return
    }
    if (markerRequestRef.current !== 0) return
    const at = fmtElapsed(elapsed)
    const label = `Marker ${at}`
    const requestId = Date.now()
    markerRequestRef.current = requestId
    setMarkerPending(true)
    void runUserVerb('screen_record.studio_event', {
      capture_id: captureId,
      event: {
        t_ms: Math.max(0, Date.now() - (recordStartedAtRef.current ?? Date.now())),
        source: 'recording',
        kind: 'marker',
        label,
      },
    }, 'Could not save the recording marker.')
      .then((response) => {
        if (markerRequestRef.current !== requestId) return
        if (response?.ok && recordingMarkerAcknowledged(response.result, label)) {
          setNote(`Marker saved at ${at}`)
        } else if (response?.ok) {
          setNote('The recorder did not durably acknowledge the marker; no marker was confirmed here.')
        }
      })
      .finally(() => {
        if (markerRequestRef.current !== requestId) return
        markerRequestRef.current = 0
        setMarkerPending(false)
      })
  }, [elapsed, phase])

  const probe = useCallback(async () => {
    const r = await callVerb('screen_record.doctor', {})
    if (r.ok && r.result) {
      const res = r.result as { cards: RecordCard[]; ready: boolean; start_allowed?: boolean; monitors?: MonitorInfo[]; windows?: WindowInfo[]; camera?: CameraCapability; quality?: unknown; scenes?: unknown; pause?: unknown }
      setCards(res.cards)
      setReady(res.ready)
      // A pre-start server predates this field, so its strict `ready` result is
      // the safe fallback; never infer permission from an arbitrary unknown card.
      setStartAllowed(res.start_allowed ?? res.ready)
      const mons = res.monitors ?? []
      setMonitors(mons)
      // App windows for the window picker. Keep a vanished selected identity so
      // the UI can refuse visibly instead of silently falling back to a monitor.
      const wins = res.windows ?? []
      setWindows(wins)
      const camera = res.camera ?? NO_CAMERA_CAPABILITY
      setCameraCapability(camera)
      setCameraDeviceId((previous) => camera.devices.some((device) => device.id === previous)
        ? previous
        : (camera.devices[0]?.id ?? null))
      setQualityCapability(res.quality)
      setDoctorSceneCapability(res.scenes)
      setPauseDoctorCapability(res.pause)
      // Default the picker to the primary display (else the first), so the chosen
      // index is explicit once there's a list. Empty list ⇒ null (engine primary).
      setMonitorIdx((prev) => {
        if (mons.length === 0) return null
        if (prev !== null && mons.some((m) => m.index === prev)) return prev
        return (mons.find((m) => m.primary) ?? mons[0]).index
      })
    } else {
      setReady(false)
      setStartAllowed(false)
      setCameraCapability(NO_CAMERA_CAPABILITY)
      setQualityCapability(undefined)
      setDoctorSceneCapability(undefined)
      setPauseDoctorCapability(undefined)
    }
  }, [setPauseDoctorCapability, setDoctorSceneCapability, setQualityCapability])
  useEffect(() => { void probe() }, [probe])

  useEffect(() => {
    if (!navigator.mediaDevices?.addEventListener) return
    const refreshSources = () => {
      if (phase === 'idle') void probe()
    }
    navigator.mediaDevices.addEventListener('devicechange', refreshSources)
    return () => navigator.mediaDevices.removeEventListener('devicechange', refreshSources)
  }, [phase, probe])

  const clearTick = () => {
    if (tickRef.current) window.clearInterval(tickRef.current)
    tickRef.current = null
  }

  const finalize = useCallback(async (captureId: string, source: string | null) => {
    clearTick()
    if (captureRef.current !== captureId) return // already finalized / superseded
    captureRef.current = null
    setSceneCaptureId(null)
    recordingPause.clearCapture()
    recordStartedAtRef.current = null
    const rawMode = rawCapture
    const polishing = autoPolish
    setPhase('finalizing')
    setExportNote('')
    setFinalizeSec(0)
    setNote(rawMode ? 'Saving the raw recording…' : polishing ? 'Polishing — auto-zoom, cursor, framing…' : 'Preparing your clip…')
    // A visible elapsed clock so the bake is not an opaque wait (it scales with
    // recording length on the polish path; the raw path is near-instant).
    const finalizeStart = Date.now()
    tickRef.current = window.setInterval(() => {
      setFinalizeSec(Math.floor((Date.now() - finalizeStart) / 1000))
    }, 500)
    try {
      // RAW CAPTURE: stop WITHOUT autoedit and ask the engine to fold the captured
      // streams into ONE raw.mp4 (mux_raw). NO autoedit, NO polish, NO auto-place —
      // just the recording, exactly as captured. We surface the file path and offer
      // to add it to the timeline as-is (addRawToTimeline); nothing is post-processed.
      if (rawMode) {
        const rawOutputPath = recordOutputPath ?? undefined
        const stop = await withAuthorizedOutputPath(rawOutputPath, () =>
          callVerb('screen_record.stop', { capture_id: captureId, autoedit: false, mux_raw: true, raw_path: rawOutputPath }))
        if (!stop.ok) { setErr(`stop failed: ${stop.error?.message ?? 'error'}`); setPhase('error'); return }
        const sr = stop.result as { raw_path?: string; raw_has_mic?: boolean; raw_has_system?: boolean; source?: string; raw_streams?: StudioRawStreams; cursor_correlation?: CursorCorrelation; cadence?: RecordingCadence; quality?: unknown }
        setLastRawStreams(sr.raw_streams ?? null)
        setLastCursorCorrelation(sr.cursor_correlation ?? null)
        setCaptureCadence(sr.cadence ?? null)
        setQualityResolution(sr.quality)
        const rawPath = sr.raw_path ?? sr.source ?? null
        if (!rawPath) { setErr('capture produced no raw recording'); setPhase('error'); return }
        setLastRaw({ path: rawPath, hasMic: !!sr.raw_has_mic, hasSystem: !!sr.raw_has_system })
        setNote('Raw recording saved')
        setPhase('done')
        return
      }
      const stop = await callVerb('screen_record.stop', { capture_id: captureId, autoedit: true })
      if (!stop.ok) { setErr(`stop failed: ${stop.error?.message ?? 'error'}`); setPhase('error'); return }
      const sr = stop.result as { source?: string; plan?: string; raw_streams?: StudioRawStreams; cursor_correlation?: CursorCorrelation; cadence?: RecordingCadence; quality?: unknown }
      setLastRawStreams(sr.raw_streams ?? null)
      setLastCursorCorrelation(sr.cursor_correlation ?? null)
      setCaptureCadence(sr.cadence ?? null)
      setQualityResolution(sr.quality)
      const src = sr.source ?? source
      if (!src || !sr.plan) { setErr('capture produced no source/plan'); setPhase('error'); return }
      // Retain source+plan so "Export clip" can render a file later without re-recording.
      setLastCapture({ source: src, plan: sr.plan })
      // raw:true takes the FAST stream-copy path (no zoom/cursor re-render).
      const pol = await callVerb('screen_record.polish', { source: src, plan: sr.plan, raw: !polishing })
      if (!pol.ok) { setErr(`${polishing ? 'polish' : 'preparing clip'} failed: ${pol.error?.message ?? 'error'}`); setPhase('error'); return }
      const pr = pol.result as { clip_id?: string }
      setNote(`Done — clip ${pr.clip_id ?? ''} added to the timeline`)
      setPhase('done')
      onClipAdded?.()
    } catch (error) {
      // Same class as exportClip's catch: the RAW path authorizes the chosen Save As
      // folder through withAuthorizedOutputPath, which THROWS when the engine refuses
      // it. Reporting that as "server unreachable" sent the user to check the engine
      // when the actual fix is to pick another recording file — so name the real
      // reason, and keep the transport wording for the case that really is one
      // (fetch rejects with a TypeError when the connection fails).
      const reason = failureReason(error)
      setErr(rawMode && recordOutputPath
        ? `finalize failed: ${reason} — ${OUTPUT_PATH_HINT}`
        : `finalize failed: ${reason}`)
      setPhase('error')
    } finally {
      clearTick() // stop the finalize elapsed clock on every exit path
      void refreshSceneRecovery()
    }
  }, [onClipAdded, autoPolish, rawCapture, recordOutputPath, recordingPause, refreshSceneRecovery, setQualityResolution])

  useEffect(() => {
    const captureEndedDetail = recordingAudioMeters.captureEndDetail
    if (phase !== 'recording' || !sceneCaptureId || !captureEndedDetail) return
    // `screen_record.stop` is recovery-aware: even when the native owner
    // ended first, it can finalize the retained capture directory. The capture
    // ref is cleared synchronously by finalize, so a duration/status race
    // cannot send a second Stop for this id.
    void finalize(sceneCaptureId, null)
    setNote(captureEndedDetail)
  }, [finalize, phase, recordingAudioMeters.captureEndDetail, sceneCaptureId])

  // RAW mode: add the saved raw recording to the timeline AS-IS. `media.import`
  // auto-places the first clip into an empty timeline (the common fresh-recording
  // case) — no autoedit, no polish. The raw.mp4 already carries the combined audio
  // (mic + system folded by mux_raw), so a single import brings picture + sound.
  const addRawToTimeline = useCallback(async () => {
    if (!lastRaw) return
    setExportNote('Adding to the timeline…')
    const imp = await callVerb('media.import', { path: lastRaw.path })
    if (!imp.ok) { setExportNote(`add failed: ${imp.error?.message ?? 'error'}`); return }
    setExportNote('Added to the timeline')
    onClipAdded?.()
  }, [lastRaw, onClipAdded])

  // Start a capture. Open-ended by default (no duration_ms); a chosen cap is passed
  // as an upper bound AND drives a local countdown that auto-finalizes at the bound.
  const start = useCallback(async () => {
    setErr(null)
    setNote('')
    if (customFpsError) {
      setErr(`Frame rate needs correction: ${customFpsError}`)
      return
    }
    if (selectedWindowMissing) {
      setErr('The selected window is no longer available. Choose another source before recording.')
      setPhase('error')
      return
    }
    if (sourceKind === 'window' && !windowTargetId) {
      setErr('Choose an application window before recording.')
      setPhase('error')
      return
    }
    if (sourceKind === 'region') {
      setErr(regionPickerCapability.availability === 'unavailable'
        ? regionPickerCapability.reason
        : 'Region capture is not connected to the native recorder in this build.')
      setPhase('error')
      return
    }
    if (recordingPause.enabled) {
      if (!recordingPause.capability.supported) {
        setErr(recordingPause.capability.detail)
        setPhase('error')
        return
      }
      if (sourceKind !== 'display') {
        setErr('Pause & resume records one exact display. Choose Display before recording.')
        setPhase('error')
        return
      }
      const selected = monitorIdx === null ? undefined : monitors.find((monitor) => monitor.index === monitorIdx)
      if (!selected?.id) {
        setErr('Pause & resume needs a current exact display. Refresh Recorder and choose a listed display.')
        setPhase('error')
        return
      }
      if (!Number.isInteger(fps)) {
        setErr('Pause & resume needs a whole-number frame rate.')
        setPhase('error')
        return
      }
    }
    const cameraError = cameraSelectionError(
      cameraCapability,
      !recordingPause.enabled && studio.camera.enabled,
      cameraDeviceId,
      rawCapture,
    )
    if (cameraError) {
      setErr(cameraError)
      setPhase('error')
      return
    }
    // Record without a project: if none is open, create one on the fly so
    // you can record straight from the Record surface — the capture lands in a fresh
    // auto-named project. project.create OPENS it server-side; onClipAdded re-syncs App.
    if (!project) {
      const stamp = new Date().toISOString().slice(0, 16).replace(/[:T]/g, '-')
      const pc = await callVerb('project.create', { name: `Recording ${stamp}` })
      if (!pc.ok) { setErr(`could not create a project: ${pc.error?.message ?? 'error'}`); setPhase('error'); return }
      onClipAdded?.()
    }
    // RAW mode never renders the key-cast overlay (that's a polish pass), so don't
    // capture keystrokes in raw mode — a small privacy win (keys can reveal secrets).
    setLastRawStreams(null)
    setLastCursorCorrelation(null)
    setCaptureCadence(null)
    clearQualityResolution()
    setLastCapture(null)
    setLastRaw(null)
    const startArgs: {
      fps: number
      quality?: { output_size: 'source' | '1080p' | '720p'; profile: 'standard' | 'high' }
      audio: boolean
      system_audio: boolean
      keys: boolean
      duration_ms?: number
      monitor?: number
      monitor_id?: string
      window?: string
      camera_id?: string
      scenes?: RecordingSceneStartConfig
      pause?: { mode: 'enabled' }
      studio?: unknown
    } = {
      fps,
      audio,
      system_audio: systemAudio,
      keys: recordingPause.enabled || rawCapture ? false : keys,
      studio: {
        background: studio.background,
      },
    }
    if (capMs !== null) startArgs.duration_ms = capMs // omitted entirely = open-ended
    if (!recordingPause.enabled && qualityRequest) startArgs.quality = qualityRequest
    // Scene metadata is sent only after Doctor advertises the proposed,
    // versioned scene capability. Older engines keep the normal recorder path.
    if (!recordingPause.enabled && sceneStartConfig) startArgs.scenes = sceneStartConfig
    if (!recordingPause.enabled && studio.camera.enabled && !rawCapture && cameraDeviceId) {
      startArgs.camera_id = cameraDeviceId
    }
    if (recordingPause.enabled) startArgs.pause = { mode: 'enabled' }
    // Source selection: an opaque live window identity wins; otherwise a chosen
    // monitor on a multi-monitor setup. When Doctor supplied the selected monitor's
    // opaque identity, return it unchanged for the server/native exact-target path.
    if (sourceKind === 'window' && windowTargetId) startArgs.window = windowTargetId
    else {
      const selectedMonitor = monitorIdx === null
        ? undefined
        : monitors.find((monitor) => monitor.index === monitorIdx)
      if (monitorIdx !== null && monitors.length >= 2) startArgs.monitor = monitorIdx
      if (selectedMonitor?.id) startArgs.monitor_id = selectedMonitor.id
    }
    const r = await callVerb('screen_record.start', startArgs)
    if (!r.ok) {
      setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'could not start capture'}`)
      setPhase('error')
      return
    }
    const res = r.result as { capture_id: string; out_dir?: string; cadence?: RecordingCadence; scenes?: unknown; pause?: unknown }
    setCaptureCadence(res.cadence ?? null)
    captureRef.current = res.capture_id
    setSceneCaptureId(res.capture_id)
    markSceneCaptureStarted(res.scenes)
    recordingPause.acknowledgeStart(res.capture_id, res.pause)
    recordStartedAtRef.current = Date.now()
    if (!recordingPause.enabled && studio.camera.enabled && !rawCapture) {
      await emitStudioEvent({ source: 'camera', kind: 'visibility', visible: true })
      await emitStudioEvent({
        source: 'camera', kind: 'transform', x: studio.camera.x, y: studio.camera.y,
        size: studio.camera.size, shape: studio.camera.shape,
      })
    }
    setPhase('recording')
    setElapsed(0)
    setRemaining(capMs !== null ? Math.ceil(capMs / 1000) : 0)
    // HUD tick: count elapsed up (open-ended clock) and, when a cap is set, the
    // remaining down — auto-finalizing when the cap elapses. Manual Stop / the
    // shortcut can end it earlier at any time.
    const startedAt = recordStartedAtRef.current
    clearTick()
    tickRef.current = window.setInterval(() => {
      const elapsedSec = Math.floor((Date.now() - startedAt) / 1000)
      setElapsed(elapsedSec)
      if (capMs !== null) {
        const left = Math.max(0, Math.ceil((capMs - (Date.now() - startedAt)) / 1000))
        setRemaining(left)
        if (left <= 0) void finalize(res.capture_id, null)
      }
    }, 250)
  }, [capMs, fps, customFpsError, audio, systemAudio, keys, clearQualityResolution, qualityRequest, rawCapture, monitorIdx, monitors, sourceKind, windowTargetId, selectedWindowMissing, cameraCapability, cameraDeviceId, emitStudioEvent, finalize, markSceneCaptureStarted, onClipAdded, project, recordingPause, sceneStartConfig, studio])

  const preflightStartError = useCallback(() => {
    if (startAllowed === false) return 'Screen capture is not ready on this machine.'
    if (customFpsError) return `Frame rate needs correction: ${customFpsError}`
    if (selectedWindowMissing) return 'The selected window is no longer available. Choose another source before recording.'
    if (sourceKind === 'window' && !windowTargetId) return 'Choose an application window before recording.'
    if (sourceKind === 'region') {
      return regionPickerCapability.availability === 'unavailable'
        ? regionPickerCapability.reason
        : 'Region capture is not connected to the native recorder in this build.'
    }
    if (recordingPause.enabled) {
      if (!recordingPause.capability.supported) return recordingPause.capability.detail
      if (sourceKind !== 'display') return 'Pause & resume records one exact display. Choose Display before recording.'
      const selected = monitorIdx === null ? undefined : monitors.find((monitor) => monitor.index === monitorIdx)
      if (!selected?.id) return 'Pause & resume needs a current exact display. Refresh Recorder and choose a listed display.'
      if (!Number.isInteger(fps)) return 'Pause & resume needs a whole-number frame rate.'
    }
    const cameraError = cameraSelectionError(
      cameraCapability, !recordingPause.enabled && studio.camera.enabled, cameraDeviceId, rawCapture,
    )
    if (cameraError) return cameraError
    return null
  }, [cameraCapability, cameraDeviceId, customFpsError, fps, monitorIdx, monitors, rawCapture, recordingPause, regionPickerCapability, selectedWindowMissing, sourceKind, startAllowed, studio.camera.enabled, windowTargetId])
  const countdown = useRecordingCountdown({
    validate: preflightStartError,
    onPrepare: () => { setErr(null); setNote(''); setPhase('idle') },
    onStart: start,
    onInvalid: (message) => { setErr(message); setPhase('error') },
    onCancel: () => { setPhase('idle'); setErr(null); setNote('Countdown cancelled — nothing was recorded.') },
  })

  const stop = useCallback(() => {
    const id = captureRef.current
    if (id) void finalize(id, null)
  }, [finalize])

  const chooseOutputPath = useCallback(async () => {
    if (!isTauri()) {
      setRecordOutputNote('Choose file needs the desktop app.')
      return
    }
    const ext = rawCapture ? 'mp4' : exportFmt
    const path = await pickExportOutput({
      title: 'Choose recording output file — ShellX Cut',
      defaultPath: `recording.${ext}`,
      filters: ext === 'gif'
        ? [{ name: 'GIF image', extensions: ['gif'] }]
        : [{ name: 'MP4 video', extensions: ['mp4'] }],
    })
    if (!path) return
    setRecordOutputPath(path)
    setRecordOutputNote(rawCapture ? 'Raw recording will save to this file.' : 'Polished export will use this file when you export.')
    setExportNote('')
  }, [rawCapture, exportFmt])

  // The ONE start⇄stop toggle, shared by the global OS hotkey (F9) and the
  // in-page F9 keydown fallback — so every trigger does exactly the same thing.
  // Coalesced (lastToggleRef) so a single F9 press that reaches BOTH paths (Cut
  // window focused) toggles once, not twice. No-op while finalizing, or when a
  // project isn't open / capture cannot start (matches the button's disabled state).
  const toggle = useCallback(() => {
    if (audioProbeRunning || micTestRunning) return
    const now = Date.now()
    if (now - lastToggleRef.current < 400) return // de-dupe the global+in-page double-fire
    lastToggleRef.current = now
    if (countdown.active) {
      countdown.cancel()
    } else if (phase === 'recording') {
      stop()
    } else if (phase === 'idle' || phase === 'done' || phase === 'error') {
      countdown.requestStart()
    }
  }, [audioProbeRunning, countdown, micTestRunning, phase, stop])

  // A SINGLE key (F9) toggles Start ⇄ Stop — no 3-key chord.
  //
  // GLOBAL path (desktop): the shell registers F9 as an OS-level global hotkey
  // and emits `cut:record-hotkey`; we listen for it here so F9 STOPS a capture
  // even while the recorded app is focused (the whole reason a recorder needs a
  // global key). No-op outside Tauri.
  useEffect(() => onRecordHotkey(() => {
    if (!isBlockingOverlayActive()) toggle()
  }), [toggle])
  // FOCUSED-WINDOW fallback: the same F9 as a plain in-page keydown. Covers the
  // case where the Cut window itself is focused, and is the SOLE path in the
  // plain web/dev build (no shell ⇒ no global registration). Listener lives HERE
  // (not App.tsx) so it doesn't touch the layout agent's files; active only while
  // the Record surface is mounted.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (countdown.active && e.key === 'Escape') {
        e.preventDefault()
        countdown.cancel()
        return
      }
      if (shouldIgnoreGlobalShortcut(e)) return
      if (matchesFixedAction(e, 'recording.toggle')) {
        e.preventDefault()
        toggle()
        return
      }
      if (matchesFixedAction(e, 'recording.marker')) {
        e.preventDefault()
        addRecordingMarker()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [addRecordingMarker, countdown, toggle])

  useEffect(() => () => { clearTick() }, [])

  const busy = countdown.active || phase === 'recording' || phase === 'finalizing' || audioProbeRunning || micTestRunning
  const sceneControlsDisabled = countdown.active || phase === 'finalizing' || audioProbeRunning || micTestRunning
  const sceneCameraReason = rawCapture
    ? 'Camera scenes need Auto-edit mode.'
    : cameraSelectionError(cameraCapability, true, cameraDeviceId, false)
  const pauseCameraCapability = recordingPause.enabled
    ? { ...NO_CAMERA_CAPABILITY, detail: 'Camera is unavailable while Pause & resume is enabled.' }
    : cameraCapability
  const recordingSceneControl = (
    <RecordingScenesControl
      compact={phase === 'recording'}
      selectedScene={selectedScene}
      status={sceneStatus}
      recovery={sceneRecovery}
      timer={sceneTimer}
      timerStatus={sceneTimerStatus}
      recording={phase === 'recording'}
      disabled={sceneControlsDisabled}
      cameraReason={sceneCameraReason}
      unavailableReason={recordingPause.enabled ? 'Scenes are unavailable while Pause & resume is enabled.' : null}
      liveSupported={sceneCapability.supported}
      onSelect={(sceneId) => { void selectScene(sceneId) }}
      onTimerChange={selectSceneTimer}
      onTimerControl={(action) => { void controlSceneTimer(action) }}
      onRefreshRecovery={() => { void refreshSceneRecovery() }}
    />
  )

  // Staleness guard: the user's open windows change while the Record tab sits open
  // (they alt-tab, open Chrome, close an app), but the picker only probed on mount —
  // so it showed a stale list with only an old entry while Chrome and a terminal
  // were open). Re-probe every 3s while idle (NOT recording) so the window/monitor list
  // tracks the live desktop. Cheap enough (the same doctor call) and it stops during a
  // capture. The select's onMouseDown also re-probes for an instant refresh on click.
  useEffect(() => {
    if (busy) return
    const id = setInterval(() => { void probe() }, 3000)
    return () => clearInterval(id)
  }, [busy, probe])
  const cardStatus = (c: RecordCard) => (
    c.status === 'ok' ? 'ok' : c.status === 'degraded' ? 'degraded' : c.status === 'unknown' ? 'unknown' : 'missing'
  )
  const displayPhase = countdown.active ? 'countdown' : phase
  const studioElapsed = phase === 'recording'
    ? fmtElapsed(elapsed)
    : countdown.active
      ? `${countdown.remaining}`
    : phase === 'finalizing'
      ? `${finalizeSec}s`
      : '0:00'

  return (
    <section
      className="rec"
      data-cut-panel="record"
      data-cut-record-mode={rawCapture ? 'quick' : 'studio'}
      data-cut-record-phase={displayPhase}
    >
      <header className="rec__head">
        <div className="rec__title-wrap">
          <span className="rec__dot" data-cut-rec-phase={phase} aria-hidden="true" />
          <h1 className="rec__title">Record</h1>
        </div>
        <p className="rec__sub">
          {rawCapture
            ? 'Raw capture — record your screen and save it exactly as captured, no auto-edit.'
            : 'Capture your screen → a polished clip on the timeline, no editing.'}
        </p>
      </header>

      {!project && <p className="rec__source-note" data-cut-rec-no-project>No project open yet — Start creates one automatically and the recording lands inside it.</p>}

      {(
        <div className="rec__body">
          {/* Capability cards (screen_record.doctor) */}
          <div className="rec__cards" data-cut-rec-cards data-cut-rec-readiness>
            {cards.filter((c) => c.name !== 'webcam').map((c) => (
              <div key={c.name} className={`rec__card rec__card--${cardStatus(c)}`} data-cut-rec-card={c.name} data-cut-rec-card-status={cardStatus(c)}>
                <span className="rec__card-name">{recordCardLabel(c.name)}</span>
                <span className="rec__card-detail">{c.detail}</span>
              </div>
            ))}
            {ready === false && startAllowed === true && (
              <p className="rec__not-ready" data-cut-rec-portal-prompt>
                Screen capture is deliberately unverified until the Linux source picker runs.
                Start recording opens that picker; Doctor stays non-green until a later delivery proof.
              </p>
            )}
            {ready === false && startAllowed !== true && (
              <p className="rec__not-ready" data-cut-rec-not-ready>
                Screen capture isn’t verified on this machine — an unknown status is not ready.
                Recording needs a desktop session (Linux XDG portal / Windows / macOS) with ffmpeg. Core editing still works.
              </p>
            )}
            <RecordingRehearsal
              disabled={busy}
              sourceKind={sourceKind}
              fps={fps}
              monitor={monitors.length >= 2 ? monitorIdx : null}
              monitorId={monitorIdx === null ? null : monitors.find((monitor) => monitor.index === monitorIdx)?.id ?? null}
              windowId={windowTargetId}
              startAllowed={startAllowed}
              startError={preflightStartError()}
              onRefresh={probe}
            />
          </div>

          <div className="rec__studio" data-cut-rec-studio>
            <StudioPreview
              studio={studio}
              phase={displayPhase}
              elapsed={studioElapsed}
              sceneName={selectedScene.name}
              sceneState={sceneStatus.state}
              sourcePreview={sourcePreview.presentation}
            />
            <StudioControls
              sceneControl={phase === 'recording' ? undefined : recordingSceneControl}
              studio={studio}
              rawStreams={lastRawStreams}
              cursorCorrelation={lastCursorCorrelation}
              onBackground={setStudioBackground}
              cameraCapability={pauseCameraCapability}
              cameraDeviceId={cameraDeviceId}
              configurationDisabled={busy}
              liveAdjustDisabled={sceneControlsDisabled}
              rawCapture={rawCapture}
              onCameraEnabled={setStudioCameraEnabled}
              onCameraDevice={setCameraDeviceId}
              onCameraPosition={setStudioCameraPosition}
              onCameraShape={setStudioCameraShape}
              onCameraSize={setStudioCameraSize}
            />
          </div>

          {/* Capture setup stays compact. Less common output, cadence, and
              processing choices remain one disclosure away instead of
              competing with the composition preview. */}
          <aside className="rec__settings" data-cut-rec-settings data-cut-rec-quality-supported={Boolean(qualityCapability)}>
            <div className="rec__settings-head">
              <div>
                <span className="rec__eyebrow">Capture setup</span>
                <h2 className="rec__settings-title">Screen &amp; sound</h2>
              </div>
              <span className="rec__settings-state" data-cut-rec-setup-state={startAllowed === false ? 'attention' : 'ready'}>
                {startAllowed === false ? 'Check setup' : 'Ready to configure'}
              </span>
            </div>
            <div className="rec__settings-primary">
              <RecordingSourceSetup
                selection={{ sourceKind, monitors, monitorIdx, windows, windowTargetId, selectedWindowMissing }}
                disabled={busy}
                pauseEnabled={recordingPause.enabled}
                regionCapability={regionPickerCapability}
                onRefresh={() => { void probe() }}
                onSourceKindChange={(next) => { setSourceKind(next); setRegionPickerOpen(next === 'region' && regionPickerCapability.availability === 'available') }}
                onMonitorChange={setMonitorIdx}
                onWindowChange={setWindowTargetId}
                preview={sourcePreview}
              />
            {/* MODE: AUTO-EDIT (record → polished clip on the timeline) vs RAW CAPTURE
                (save the recording exactly as captured — no autoedit, no polish). The
                same source/length/fps/mic/system-audio options apply to BOTH modes. */}
            <div className="rec__field rec__field--mode">
              <span className="rec__label">Mode</span>
              <div className="rec__seg" role="group" aria-label="Recording mode">
                <button
                  type="button"
                  className={`rec__seg-btn${!rawCapture ? ' rec__seg-btn--on' : ''}`}
                  data-cut-rec-mode="auto"
                  aria-pressed={!rawCapture}
                  disabled={busy}
                  onClick={() => setRawCapture(false)}
                  title="Record → an auto-edited, polished clip on the timeline (zoom-to-cursor, framing)."
                >
                  Auto-edit
                </button>
                <button
                  type="button"
                  className={`rec__seg-btn${rawCapture ? ' rec__seg-btn--on' : ''}`}
                  data-cut-rec-mode="raw"
                  aria-pressed={rawCapture}
                  disabled={busy}
                  onClick={() => setRawCapture(true)}
                  title="Raw capture — save the recording exactly as captured (your sound sources included), with no auto-edit or polish."
                >
                  Raw capture
                </button>
              </div>
              <p className="rec__source-note" data-cut-rec-mode-note>
                {rawCapture
                  ? 'Raw capture saves the recording as-is — no auto-edit, no polish. Your mic and system/desktop audio are folded into one file; you choose whether to add it to the timeline.'
                  : 'Auto-edit records, then polishes (zoom-to-cursor, framing) and drops the finished clip on the timeline.'}
              </p>
            </div>
            <MicInputControl
              audio={audio}
              disabled={busy}
              idle={phase === 'idle'}
              onAudioChange={setAudio}
              onTestingChange={setMicTestRunning}
            />
            <label className="rec__toggle rec__toggle--system" data-cut-rec-system-audio-toggle title="Capture the desktop/app sound (e.g. a game) onto its OWN audio track, separate from the mic">
              <input type="checkbox" data-cut-rec-system-audio-toggle-input checked={systemAudio} disabled={busy} onChange={(e) => setSystemAudio(e.target.checked)} /> Capture system / desktop audio (game sound)
            </label>
            <SystemAudioProbeControl disabled={busy} onRunningChange={setAudioProbeRunning} />
            </div>

            <details className="rec__advanced" data-cut-rec-advanced>
              <summary data-cut-action="record-advanced-toggle">
                <span>Advanced options</span>
                <small>Timing, quality, output, and polish</small>
              </summary>
              <div className="rec__advanced-body">
                <div className="rec__field rec__field--length">
                  <span className="rec__label">Length</span>
                  <div className="rec__seg">
                    {DUR_PRESETS.map((d) => {
                      const on = capMs === d.ms
                      return (
                        <button
                          key={d.label}
                          type="button"
                          className={`rec__seg-btn${on ? ' rec__seg-btn--on' : ''}`}
                          data-cut-rec-dur={d.ms === null ? 'none' : d.ms}
                          disabled={busy}
                          onClick={() => setCapMs(d.ms)}
                        >
                          {d.label}
                        </button>
                      )
                    })}
                  </div>
                </div>
                <RecordingCountdownControl
                  value={countdown.seconds}
                  disabled={busy}
                  onValueChange={countdown.setSeconds}
                />
                <div className="rec__field rec__field--pause">
                  <span className="rec__label">Pause</span>
                  <RecordingPauseControl
                    capability={recordingPause.capability}
                    enabled={recordingPause.enabled}
                    state={recordingPause.state}
                    message={recordingPause.message}
                    disabled={busy}
                    onEnabled={(enabled) => {
                      recordingPause.setPauseEnabled(enabled)
                      if (!enabled) return
                      setSourceKind('display')
                      setWindowTargetId(null)
                      setRegionPickerOpen(false)
                      setKeys(false)
                      setStudio((current) => ({ ...current, camera: { ...current.camera, enabled: false } }))
                    }}
                    onControl={() => { void recordingPause.control() }}
                  />
                </div>
                <RecordFrameRateControl
                  value={fps}
                  customValue={customFps}
                  disabled={busy}
                  onValueChange={setFps}
                  onCustomValueChange={updateCustomFps}
                />
                <p className="rec__fps-status" data-cut-rec-cadence>
                  {requestedCadenceLabel(captureCadence, fps)} · {probedAverageCadenceLabel(captureCadence)}
                </p>
                {qualityCapability && (
                  <RecordingQualityControl
                    capability={qualityCapability}
                    outputSize={outputSize}
                    profile={profile}
                    resolution={qualityResolution}
                    cadence={captureCadence}
                    disabled={busy}
                    unavailableReason={recordingPause.enabled ? 'Quality is unavailable while Pause & resume is enabled.' : null}
                    onOutputSizeChange={setOutputSize}
                    onProfileChange={setProfile}
                  />
                )}
                <div
                  className="rec__field rec__field--output"
                  data-cut-rec-output-kind={recordOutputPath ? 'custom' : 'default'}
                >
                  <span className="rec__label">Recording file</span>
                  <div className="rec__output-row">
                    <code className="rec__output-path" data-cut-rec-output-name={recordOutputPath ? 'chosen' : 'default'}>
                      {recordOutputPath ? outputFileLabel(recordOutputPath) : 'Uses default export folder'}
                    </code>
                    <button
                      type="button"
                      className="rec__export-btn rec__export-btn--ghost rec__export-btn--small"
                      data-cut-action="record-output-default-folder"
                      disabled={busy || !onOpenOutputSettings}
                      onClick={() => onOpenOutputSettings?.()}
                    >
                      Default folder
                    </button>
                    <button
                      type="button"
                      className="rec__export-btn rec__export-btn--small"
                      data-cut-action="record-output-pick"
                      disabled={busy}
                      onClick={() => void chooseOutputPath()}
                    >
                      Choose file
                    </button>
                    {recordOutputPath && (
                      <button
                        type="button"
                        className="rec__export-btn rec__export-btn--ghost rec__export-btn--small"
                        data-cut-action="record-output-clear"
                        disabled={busy}
                        onClick={() => {
                          setRecordOutputPath(null)
                          setRecordOutputNote('Using the default export folder.')
                        }}
                      >
                        Clear
                      </button>
                    )}
                  </div>
                  {recordOutputNote && <p className="rec__source-note" data-cut-rec-output-note>{recordOutputNote}</p>}
                </div>
            {/* Key-cast + auto-polish are POLISH-pass features (a burned-in overlay / a
                zoom-cursor-framing re-render). RAW capture skips polish entirely, so
                these controls are hidden in raw mode rather than left as dead toggles. */}
            {!rawCapture && (
              <label className="rec__toggle rec__toggle--keys" data-cut-rec-keys-toggle title="Keystrokes can reveal passwords — off by default">
                <input type="checkbox" data-cut-rec-keys-toggle-input checked={recordingPause.enabled ? false : keys} disabled={busy || recordingPause.enabled} onChange={(e) => setKeys(e.target.checked)} /> {recordingPause.enabled ? 'Keystrokes unavailable with Pause & resume' : 'Show keystrokes (key-cast)'}
              </label>
            )}
            {!rawCapture && (
              <label className="rec__toggle rec__toggle--polish" data-cut-rec-autopolish-toggle title="Auto-polish adds zoom-to-cursor, cursor smoothing and a framed background after you stop — a short render that scales with length. Turn off to drop the RAW recording onto the timeline instantly and polish later.">
                <input type="checkbox" data-cut-rec-autopolish-toggle-input checked={autoPolish} disabled={busy} onChange={(e) => setAutoPolish(e.target.checked)} /> Auto-polish after recording (zoom, cursor, framing)
              </label>
            )}
              </div>
            </details>
          </aside>

          {/* Transport / HUD */}
          <div className="rec__transport" data-cut-studio-result={displayPhase} data-cut-rec-primary-transport>
            {countdown.active ? (
              <div className="rec__hud rec__hud--countdown" data-cut-rec-countdown-transport>
                Starting in {countdown.remaining}… Press Escape to cancel.
              </div>
            ) : phase === 'recording' ? (
              <RecordingLiveControls
                elapsed={capMs !== null ? `${remaining}s left` : fmtElapsed(elapsed)}
                sceneName={selectedScene.name}
                sceneControl={recordingSceneControl}
                audioMeters={<RecordingAudioMeters {...recordingAudioMeters} />}
                captureSafety={(
                  <RecordingCaptureSafetyStatus
                    sourceLifecycle={recordingAudioMeters.sourceLifecycle}
                    controllerPlacement={recordingAudioMeters.controllerPlacement}
                  />
                )}
                pauseCapability={recordingPause.capability}
                pauseEnabled={recordingPause.enabled}
                pauseState={recordingPause.state}
                pauseMessage={recordingPause.message}
                markerPending={markerPending}
                onMarker={addRecordingMarker}
                onPauseControl={() => { void recordingPause.control() }}
                onStop={stop}
              />
            ) : phase === 'finalizing' ? (
              <div className="rec__hud rec__hud--finalize" data-cut-rec-finalizing data-cut-rec-finalize-sec={finalizeSec}>
                {note || 'finalizing…'}{finalizeSec > 0 ? ` (${finalizeSec}s)` : ''}
              </div>
            ) : (
              <button
                type="button"
                className="rec__start"
                data-cut-action="record-start"
                disabled={busy || startAllowed === false || selectedWindowMissing || windowNeedsSelection || sourceKind === 'region' || Boolean(customFpsError) || Boolean(cameraSelectionError(cameraCapability, !recordingPause.enabled && studio.camera.enabled, cameraDeviceId, rawCapture))}
                onClick={countdown.requestStart}
              >
                ● Start recording ({SHORTCUT_LABEL})
              </button>
            )}
            {/* RAW-mode done: surface the saved raw file + offer to add it as-is. No
                autoedit/polish ran; "Add to timeline" imports the raw.mp4 (which already
                carries the combined mic+system audio) straight onto the timeline. */}
            {phase === 'done' && rawCapture && lastRaw && (
              <div className="rec__export" data-cut-rec-raw-done>
                <p className="rec__done" data-cut-rec-done data-cut-rec-raw-output="saved">
                  Raw recording saved.
                </p>
                <p className="rec__source-note" data-cut-rec-raw-audio={lastRaw.hasMic && lastRaw.hasSystem ? 'mic+system' : lastRaw.hasMic ? 'mic' : lastRaw.hasSystem ? 'system' : 'none'}>
                  {lastRaw.hasMic && lastRaw.hasSystem
                    ? 'Includes your microphone + system/desktop audio (folded into one track).'
                    : lastRaw.hasMic
                      ? 'Includes your microphone audio.'
                      : lastRaw.hasSystem
                        ? 'Includes system/desktop audio.'
                        : 'Video only — no audio sources were captured.'}
                </p>
                <button type="button" className="rec__export-btn" data-cut-action="record-add-raw" onClick={() => void addRawToTimeline()}>
                  Add to timeline
                </button>
                {exportNote && <p className="rec__export-note" data-cut-rec-raw-note>{exportNote}</p>}
              </div>
            )}
            {phase === 'done' && !rawCapture && <p className="rec__done" data-cut-rec-done>{note}</p>}
            {phase === 'done' && !rawCapture && lastCapture && (
              <div className="rec__export" data-cut-rec-export>
                <span className="rec__label">Export this recording as a file</span>
                <div className="rec__seg">
                  {(['mp4', 'gif'] as const).map((f) => (
                    <button
                      key={f}
                      type="button"
                      className={`rec__seg-btn${exportFmt === f ? ' rec__seg-btn--on' : ''}`}
                      data-cut-rec-export-fmt={f}
                      disabled={!!exportJob}
                      onClick={() => setExportFmt(f)}
                    >
                      {f.toUpperCase()}
                    </button>
                  ))}
                  <button type="button" className="rec__export-btn" data-cut-action="record-export" disabled={!!exportJob} onClick={() => void exportClip()}>
                    Export clip…
                  </button>
                  {exportJob && (
                    <button type="button" className="rec__export-btn" data-cut-action="record-export-cancel" onClick={() => void cancelExport()}>
                      Cancel export
                    </button>
                  )}
                </div>
                {exportNote && <p className="rec__export-note" data-cut-rec-export-note data-cut-rec-export-progress={exportJob ? 'active' : 'idle'}>{exportNote}</p>}
              </div>
            )}
            {err && <p className="rec__err" data-cut-rec-error>{err}</p>}
            {phase === 'recording' && (
              <p className="rec__hint">
                {capMs !== null
                  ? `Stops automatically at the limit, or press Stop / ${SHORTCUT_LABEL} any time.`
                  : `Recording until you stop — press Stop or ${SHORTCUT_LABEL}.`}
                {' '}{SHORTCUT_LABEL} works globally — even when another app is focused.
                {' '}The first capture on this machine pops a one-time screen-share consent dialog.
              </p>
            )}
          </div>
        </div>
      )}
      {regionPickerOpen && regionPickerCapability.availability === 'available' && (
        <RegionPickerOverlay
          capability={regionPickerCapability}
          onCancel={() => setRegionPickerOpen(false)}
          onConfirm={() => {
            // Fail closed: this source slice has no desktop/server bridge that
            // can turn a rendered rectangle into the native one-use ticket.
            setRegionPickerOpen(false)
            setSourceKind('display')
            setErr('Region capture is not connected to the native recorder in this build.')
          }}
        />
      )}
      {countdown.active && (
        <RecordingCountdownOverlay
          remaining={countdown.remaining}
          onCancel={countdown.cancel}
        />
      )}
    </section>
  )
}
