// panels/Record — full-surface capture: Doctor → start → stop → optional polish.
// It defaults to an open-ended F9-stoppable recording; duration choices are caps.
// Selected monitor/window identities pass through the verb unchanged for native revalidation.

import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { shouldIgnoreGlobalShortcut } from '../../lib/dom'
import { matchesFixedAction } from '../../lib/keymap'
import {
  getRecordHotkeyCapability,
  onRecordHotkeyCapability,
  type RecordHotkeyCapability,
} from '../../lib/tauri'
import { runUserVerb } from '../../lib/userActionFeedback'
import { StudioControls } from './StudioControls'
import { StudioPreview } from './StudioPreview'
import { RecordingVideoTimerControl } from './RecordingVideoTimerControl'
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
import { recordingFrameRateDraftError } from './recordingFrameRate'
import { RecordingCountdownControl } from './RecordingCountdownControl'
import { RecordingCountdownOverlay } from './RecordingCountdownOverlay'
import { RecordingRehearsal } from './RecordingRehearsal'
import { RecordingReadinessSummary } from './RecordingReadinessSummary'
import { RecordingLiveControls } from './RecordingLiveControls'
import { RecordingAudioMeters } from './RecordingAudioMeters'
import { RecordingCaptureSafetyStatus } from './RecordingCaptureSafetyStatus'
import { RecordingSourceSetup } from './RecordingSourceSetup'
import { RecordingSourcePreview } from './RecordingSourcePreview'
import { RecordingResultPanel } from './RecordingResultPanel'
import { RecordingQualityControl } from './RecordingQualityControl'
import {
  cameraSelectionError,
  NO_CAMERA_CAPABILITY,
  type CameraCapability,
} from './CameraControl'
import { useAppRecordingSession } from '../../app/RecordingSessionContext'
import { doctorAllowsPortalDisplay, type RecordingPreset } from '../../app/recordingPreset'
import { useRecordingQuality } from './useRecordingQuality'
import {
  DUR_PRESETS,
  fmtElapsed,
  type RecordCard,
} from './recordingUiModel'
import {
  probedAverageCadenceLabel,
  requestedCadenceLabel,
  type RecordingCadence,
} from './recordingCadence'
import {
  useRecordingStartAdmission,
} from './recordingStartAdmission'
import {
  recordingWorkspaceAdmission,
  type RecordingWorkspaceAdmission,
} from './recordingWorkspaceAdmission'
import { useRecordingExport } from './useRecordingExport'
import { useRawRecordingCopy } from './useRawRecordingCopy'
import { addRawRecordingToTimeline } from './addRawRecordingToTimeline'
import {
  clampCameraSize,
  defaultStudioState,
  placementForPosition,
  STUDIO_POSITIONS,
  type StudioBackground,
  type StudioCameraPosition,
  type StudioCameraShape,
  type StudioEventPayload,
  type StudioState,
} from './studioTypes'
import {
  recordingSceneById,
  resetCameraLayoutForRecordingScene,
  studioStateForRecordingScene,
  type RecordingScenePreset,
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
  onOpenEdit?: () => void
  /** Synchronous admission for a workspace exit that would unmount Record. */
  onWorkspaceAdmissionChange?: (admission: RecordingWorkspaceAdmission) => void
}
const UNKNOWN_START_ADMISSION = 'The recorder returned an incomplete start response, so ShellX Cut cannot confirm or control the capture. Recorder diagnostics can inspect the state, but cannot clear this lock. Restart ShellX Cut before another recording.'
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

export default function Record({ project, onClipAdded, onOpenOutputSettings, onOpenEdit, onWorkspaceAdmissionChange }: RecordProps) {
  const session = useAppRecordingSession()
  const [cards, setCards] = useState<RecordCard[]>([])
  const [ready, setReady] = useState<boolean | null>(null)
  // `start_allowed` is deliberately narrower than Doctor `ready`: on Linux,
  // Start is the user-initiated portal picker while Doctor stays unknown/non-green.
  const [startAllowed, setStartAllowed] = useState<boolean | null>(null)
  // Monitor PICKER: the doctor's enumerated displays (empty on single-display /
  // Linux), and the chosen 1-based monitor index (null = primary / engine default).
  const [monitors, setMonitors] = useState<MonitorInfo[]>([])
  const [monitorIdx, setMonitorIdx] = useState<number | null>(null)
  const [monitorTargetId, setMonitorTargetId] = useState<string | null>(null)
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
  const [windowCaptureSupported, setWindowCaptureSupported] = useState<boolean | null>(null)
  const [windowTargetId, setWindowTargetId] = useState<string | null>(null)
  const [cameraCapability, setCameraCapability] = useState<CameraCapability>(NO_CAMERA_CAPABILITY)
  const [cameraDeviceId, setCameraDeviceId] = useState<string | null>(null)
  const selectedWindowMissing = sourceKind === 'window'
    && windowTargetId !== null
    && !windows.some((window) => window.id === windowTargetId)

  // A region state can only arrive from a future capability-backed bridge. If
  // a stale restored state somehow reaches this candidate, repair the visible
  // source immediately; `start` still refuses it during this render.
  useEffect(() => {
    if (sourceKind !== 'region' || regionPickerCapability.availability !== 'unavailable') return
    setSourceKind('display')
    setRegionPickerOpen(false)
  }, [sourceKind, regionPickerCapability.availability])
  useEffect(() => {
    if (windowCaptureSupported !== false || sourceKind !== 'window') return
    setSourceKind('display')
    setWindowTargetId(null)
  }, [windowCaptureSupported, sourceKind])
  // `capMs === null` = open-ended (the default). Otherwise it is the cap in ms.
  const [capMs, setCapMs] = useState<number | null>(null)
  const [fps, setFps] = useState(30)
  const [customFps, setCustomFps] = useState('')
  const [audio, setAudio] = useState(true)
  // Capture DESKTOP/SYSTEM audio (game/app sound) as a SEPARATE mixable track.
  // Defaults OFF for safety: desktop audio capture should be an explicit opt-in.
  // Linux uses the PulseAudio-compatible monitor source.
  const [systemAudio, setSystemAudio] = useState(false)
  const [audioProbeRunning, setAudioProbeRunning] = useState(false)
  const [micTestRunning, setMicTestRunning] = useState(false)
  const [keys, setKeys] = useState(false)
  // RAW CAPTURE mode. false = AUTO-EDIT (the flagship: record → autoedit
  // → polish → a clip on the timeline). true = RAW: keep ALL the same capture options
  // (source, fps, mic + system-audio sources) but on stop SKIP autoedit AND polish —
  // the engine just folds the streams into one raw.mp4 (screen_record.stop{mux_raw}).
  // We surface the file and OFFER to add it as-is; nothing is auto-edited or auto-placed.
  // Default AUTO-EDIT so existing behaviour is unchanged; raw is the explicit opt-in.
  const [rawCapture, setRawCapture] = useState(false)
  const savedPresetRef = useRef(session.preset)
  const savedMonitorAppliedRef = useRef(false)
  useEffect(() => {
    const saved = savedPresetRef.current
    if (!saved) return
    setSourceKind(saved.source.kind === 'portal_display' ? 'display' : saved.source.kind)
    if (saved.source.kind === 'window') setWindowTargetId(saved.source.windowId)
    else if (saved.source.kind === 'display') setMonitorTargetId(saved.source.monitorId)
    setFps(saved.fps)
    setCapMs(saved.durationMs)
    setAudio(saved.audio)
    setSystemAudio(saved.systemAudio)
    setKeys(saved.keys)
    setRawCapture(saved.raw)
    if (saved.cameraId) setCameraDeviceId(saved.cameraId)
  }, [])
  useEffect(() => {
    const saved = savedPresetRef.current
    if (saved?.source.kind !== 'display' || savedMonitorAppliedRef.current) return
    const id = saved.source.monitorId
    const match = monitors.find((monitor) => monitor.id === id)
    if (match) { savedMonitorAppliedRef.current = true; setMonitorIdx(match.index) }
  }, [monitors])
  useEffect(() => {
    if (monitorTargetId || monitorIdx === null) return
    const id = monitors.find((monitor) => monitor.index === monitorIdx)?.id
    if (id) setMonitorTargetId(id)
  }, [monitorIdx, monitorTargetId, monitors])
  const selectedMonitorCurrent = monitorTargetId !== null
    && monitors.some((monitor) => monitor.index === monitorIdx && monitor.id === monitorTargetId)
  const [studio, setStudio] = useState<StudioState>(() => defaultStudioState())
  // React may evaluate a functional state updater more than once in development
  // StrictMode. Keep the current Studio draft explicitly so live controls can
  // calculate their next transform once, update React state, then send exactly
  // one durable event outside React's updater evaluation.
  const studioRef = useRef(studio)
  const cameraLayoutCustomizedRef = useRef(false)
  const selectedSceneRef = useRef<RecordingScenePreset | null>(null)
  const selectSceneRef = useRef<((sceneId: string) => Promise<void>) | null>(null)
  const updateStudio = useCallback((update: (current: StudioState) => StudioState) => {
    const next = update(studioRef.current)
    studioRef.current = next
    setStudio(next)
    return next
  }, [])
  useEffect(() => {
    studioRef.current = studio
  }, [studio])
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
  const [recordHotkeyCapability, setRecordHotkeyCapability] = useState<RecordHotkeyCapability | null>(null)
  const [stopRetryRequired, setStopRetryRequired] = useState(false)
  const [exportNote, setExportNote] = useState('')
  const { exportJob, exportClip, cancelExport } = useRecordingExport({
    capture: lastCapture,
    format: exportFmt,
    outputPath: null,
    setNote: setExportNote,
  })
  const rawCopy = useRawRecordingCopy(lastRaw?.path ?? null)
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
  const startAdmission = useRecordingStartAdmission()
  const startAdmissionUnknown = startAdmission === 'unknown'
  const [markerPending, setMarkerPending] = useState(false)
  const previewRecordingScene = useCallback((sceneId: string) => {
    updateStudio((current) => studioStateForRecordingScene(recordingSceneById(sceneId), current, cameraLayoutCustomizedRef.current))
  }, [updateStudio])
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
  selectedSceneRef.current = selectedScene
  selectSceneRef.current = selectScene
  const savedSceneAppliedRef = useRef(false)
  useEffect(() => {
    if (savedSceneAppliedRef.current) return
    savedSceneAppliedRef.current = true
    const saved = savedPresetRef.current
    if (!saved) return
    const scene = saved.scenes as { initial_scene_id?: string } | undefined
    if (scene?.initial_scene_id) void selectScene(scene.initial_scene_id)
    else if (saved.cameraId && !saved.raw) void selectScene('presenter-corner')
  }, [selectScene])
  const recordingPause = useRecordingPause()
  const sourcePreview = useRecordingSourcePreview({
    sourceKind,
    monitors: sourceKind === 'display' && monitorTargetId && !selectedMonitorCurrent ? [] : monitors,
    monitorIdx,
    windows,
    windowTargetId,
    recording: phase === 'recording',
  })
  useEffect(() => {
    session.setPreviewRelease(async () => { await sourcePreview.stop() })
    return () => session.setPreviewRelease(null)
  }, [session.setPreviewRelease, sourcePreview.stop])
  const recordingAudioMeters = useRecordingAudioMeters(sceneCaptureId, phase === 'recording')
  const admittedSceneCaptureRef = useRef<string | null>(null)
  useEffect(() => {
    const active = session.state
    captureRef.current = active.captureId
    recordStartedAtRef.current = active.startedAt
    setSceneCaptureId(active.captureId)
    setPhase(active.phase === 'recovery' ? 'error' : active.phase === 'countdown' || active.phase === 'starting' ? 'idle' : active.phase)
    if (active.phase === 'recording' || active.phase === 'finalizing' || active.phase === 'recovery' || active.phase === 'done') setRawCapture(active.raw)
    setStopRetryRequired(active.recoveryAction === 'retry_stop')
    setNote(active.message)
    setErr(active.phase === 'error' || active.phase === 'recovery' ? active.message : null)
    if (active.startResult && active.captureId && admittedSceneCaptureRef.current !== active.captureId) {
      admittedSceneCaptureRef.current = active.captureId
      markSceneCaptureStarted(active.startResult.scenes)
      recordingPause.acknowledgeStart(active.captureId, active.startResult.pause)
    }
    if (!active.captureId && admittedSceneCaptureRef.current) {
      admittedSceneCaptureRef.current = null
      recordingPause.clearCapture()
    }
    setCaptureCadence(active.cadence)
    if (active.quality) setQualityResolution(active.quality)
    else clearQualityResolution()
    setLastCapture(active.source && active.plan ? { source: active.source, plan: active.plan } : null)
    setLastRaw(active.rawPath ? { path: active.rawPath, hasMic: active.rawHasMic, hasSystem: active.rawHasSystem } : null)
    const admissionPhase = active.phase === 'countdown' || active.phase === 'starting'
      || active.phase === 'recording' || active.phase === 'finalizing' || active.phase === 'recovery'
      ? active.phase : 'idle'
    onWorkspaceAdmissionChange?.(recordingWorkspaceAdmission(admissionPhase))
  }, [session.state, markSceneCaptureStarted, onWorkspaceAdmissionChange, recordingPause.acknowledgeStart, recordingPause.clearCapture, setQualityResolution, clearQualityResolution])
  useEffect(() => {
    if (session.state.phase !== 'recording' || !session.state.startedAt) return
    const update = () => {
      const elapsedMs = Date.now() - session.state.startedAt!
      setElapsed(Math.floor(elapsedMs / 1000))
      setRemaining(capMs === null ? 0 : Math.max(0, Math.ceil((capMs - elapsedMs) / 1000)))
    }
    update()
    const timer = window.setInterval(update, 250)
    return () => window.clearInterval(timer)
  }, [session.state.phase, session.state.startedAt, capMs])
  useEffect(() => {
    if (session.state.phase !== 'finalizing') { setFinalizeSec(0); return }
    const started = Date.now()
    const timer = window.setInterval(() => setFinalizeSec(Math.floor((Date.now() - started) / 1000)), 500)
    return () => window.clearInterval(timer)
  }, [session.state.phase])
  // Only this stable action belongs to Doctor's mount-time probe.  The hook
  // returns live state as well, so depending on its wrapper object here would
  // recreate `probe` after every Doctor result and continuously re-run it.
  const { setDoctorCapability: setPauseDoctorCapability } = recordingPause
  const updateCustomFps = useCallback((value: string) => {
    setCustomFps(value)
    if (err?.startsWith('Frame rate needs correction:')) setErr(null)
  }, [err])
  // An explicit source refresh is an admission decision. A background/device
  // probe that began before it must not re-enable Start while that decision is
  // still pending or has just reported a visible failure.
  const captureSetupRefreshSequenceRef = useRef(0)
  const activeCaptureSetupRefreshRef = useRef<number | null>(null)
  // A refused explicit refresh invalidates the previously enumerated source
  // set. Advisory probes cannot make that stale set startable again; only a
  // later explicit refresh that returns a complete Doctor result clears this
  // epoch barrier.
  const failedCaptureSetupRefreshRef = useRef<number | null>(null)
  // The capture_id of the in-flight recording — drives the manual Stop + shortcut.
  const captureRef = useRef<string | null>(null)
  // While this exact id is non-null, a native capture may still be active.
  // It is released only after screen_record.stop positively acknowledges it.
  const recordStartedAtRef = useRef<number | null>(null)
  // Timestamp of the last F9 toggle. When the Cut window is FOCUSED, one physical
  // F9 press can reach BOTH the global OS hotkey (→ cut:record-hotkey event) AND
  // the in-page keydown — the OS does not swallow a global shortcut for the
  // focused window on most platforms — which would toggle twice and cancel out.
  // Collapsing toggles within a short window to one keeps a single press = a
  // single start/stop, whichever path(s) fire.
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
    updateStudio((current) => ({ ...current, background }))
    void emitStudioEvent({ source: 'background', kind: 'style', background })
  }, [emitStudioEvent, updateStudio])

  const setStudioCameraEnabled = useCallback((enabled: boolean) => {
    void selectSceneRef.current?.(enabled ? 'presenter-corner' : 'screen')
  }, [])

  const setStudioCameraPosition = useCallback((position: StudioCameraPosition) => {
    cameraLayoutCustomizedRef.current = true
    const next = updateStudio((current) => {
      const placement = placementForPosition(position, current.camera.size)
      return { ...current, camera: { ...current.camera, ...placement, position } }
    })
    const { camera } = next
    void emitStudioEvent({
      source: 'camera', kind: 'transform', x: camera.x, y: camera.y,
      size: camera.size, shape: camera.shape,
    })
  }, [emitStudioEvent, updateStudio])

  const setStudioCameraShape = useCallback((shape: StudioCameraShape) => {
    cameraLayoutCustomizedRef.current = true
    const next = updateStudio((current) => ({
      ...current,
      camera: { ...current.camera, shape },
    }))
    const { camera } = next
    void emitStudioEvent({
      source: 'camera', kind: 'transform', x: camera.x, y: camera.y,
      size: camera.size, shape: camera.shape,
    })
  }, [emitStudioEvent, updateStudio])

  const setStudioCameraSize = useCallback((value: number) => {
    cameraLayoutCustomizedRef.current = true
    const next = updateStudio((current) => {
      const size = clampCameraSize(value)
      const placement = placementForPosition(current.camera.position, size)
      return { ...current, camera: { ...current.camera, ...placement, size } }
    })
    const { camera } = next
    void emitStudioEvent({
      source: 'camera', kind: 'transform', x: camera.x, y: camera.y,
      size: camera.size, shape: camera.shape,
    })
  }, [emitStudioEvent, updateStudio])

  const resetStudioCamera = useCallback(() => {
    cameraLayoutCustomizedRef.current = false
    updateStudio((current) => resetCameraLayoutForRecordingScene(selectedSceneRef.current, current))
    void emitStudioEvent({ source: 'camera', kind: 'reset' })
  }, [emitStudioEvent, updateStudio])

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

  const probe = useCallback(async (refreshSequence?: number) => {
    const explicitSequenceAtProbeStart = captureSetupRefreshSequenceRef.current
    const explicitRefreshAtProbeStart = activeCaptureSetupRefreshRef.current
    const r = await callVerb('screen_record.doctor', {})
    if (refreshSequence !== undefined) {
      // A later explicit refresh owns admission even if its own response has
      // already settled. Never let this older request restore stale sources.
      if (refreshSequence !== captureSetupRefreshSequenceRef.current
        || activeCaptureSetupRefreshRef.current !== refreshSequence) return true
    } else {
      // Polling and device-change probes are advisory. Reject one that began
      // while an explicit refresh was active, or before a newer explicit
      // refresh started, even after that owner has finished.
      if (explicitRefreshAtProbeStart !== null
        || explicitSequenceAtProbeStart !== captureSetupRefreshSequenceRef.current
        || activeCaptureSetupRefreshRef.current !== null
        || failedCaptureSetupRefreshRef.current === captureSetupRefreshSequenceRef.current) {
        return Boolean(r.ok && r.result)
      }
    }
    if (r.ok && r.result) {
      const res = r.result as { cards: RecordCard[]; ready: boolean; start_allowed?: boolean; monitors?: MonitorInfo[]; windows?: WindowInfo[]; window_capture_supported?: boolean; camera?: CameraCapability; quality?: unknown; scenes?: unknown; pause?: unknown }
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
      // Older Doctors omit the field. Preserve their Windows/macOS picker while
      // current Linux Doctor explicitly closes the unsupported source route.
      setWindowCaptureSupported(res.window_capture_supported !== false)
      const camera = res.camera ?? NO_CAMERA_CAPABILITY
      setCameraCapability(camera)
      setCameraDeviceId((previous) => previous ?? (camera.devices[0]?.id ?? null))
      setQualityCapability(res.quality)
      setDoctorSceneCapability(res.scenes)
      setPauseDoctorCapability(res.pause)
      if (refreshSequence !== undefined
        && refreshSequence === captureSetupRefreshSequenceRef.current) {
        failedCaptureSetupRefreshRef.current = null
      }
      // Default the picker to the primary display (else the first), so the chosen
      // index is explicit once there's a list. Empty list ⇒ null (engine primary).
      setMonitorIdx((prev) => {
        if (mons.length === 0) return null
        if (prev !== null) return prev
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
    return Boolean(r.ok && r.result)
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

  const finalize = useCallback(async (_captureId: string, _source: string | null) => {
    await session.stop()
  }, [session])

  useEffect(() => {
    const captureEndedDetail = recordingAudioMeters.captureEndDetail
    if (phase !== 'recording' || stopRetryRequired || !sceneCaptureId || !captureEndedDetail) return
    // `screen_record.stop` is recovery-aware: even when the native owner
    // ended first, it can finalize the retained capture directory. A rejected
    // Stop remains retryable only by an explicit user action, never a status loop.
    void finalize(sceneCaptureId, null)
    setNote(captureEndedDetail)
  }, [finalize, phase, recordingAudioMeters.captureEndDetail, sceneCaptureId, stopRetryRequired])

  // RAW mode: import the stopped file and verify actual timeline placement.
  // A prior polished take can already occupy v1, so first-import auto-place
  // alone is not enough for the visible Add to timeline promise.
  const addRawToTimeline = useCallback(async () => {
    if (!lastRaw) return
    setExportNote('Adding to the timeline…')
    const added = await addRawRecordingToTimeline(lastRaw.path)
    if (!added.ok) { setExportNote(`add failed: ${added.reason}`); return }
    setExportNote('Added to the timeline')
    onClipAdded?.()
    onOpenEdit?.()
  }, [lastRaw, onClipAdded, onOpenEdit])

  // Start a capture. Open-ended by default (no duration_ms); a chosen cap is passed
  // as an upper bound AND drives a local countdown that auto-finalizes at the bound.
  const buildPreset = useCallback((): RecordingPreset | null => {
    if (sourceKind === 'region' || (sourceKind === 'window' && !windowCaptureSupported)) return null
    const portalDisplay = doctorAllowsPortalDisplay({ start_allowed: startAllowed, monitors, cards })
    const source = sourceKind === 'window'
      ? (windowTargetId ? { kind: 'window' as const, windowId: windowTargetId } : null)
      : selectedMonitorCurrent && monitorTargetId
        ? { kind: 'display' as const, monitorId: monitorTargetId }
        : portalDisplay ? { kind: 'portal_display' as const } : null
    if (!source) return null
    return {
      schema: 'shellx-cut/recording-preset/1', source, fps, durationMs: capMs,
      startCountdownSeconds: session.countdownSeconds,
      audio, systemAudio, keys: !rawCapture && !recordingPause.enabled && keys,
      raw: rawCapture,
      ...(qualityRequest && !recordingPause.enabled ? { quality: qualityRequest } : {}),
      ...(studio.camera.enabled && cameraDeviceId && !rawCapture && !recordingPause.enabled ? { cameraId: cameraDeviceId } : {}),
      ...(sceneStartConfig && !recordingPause.enabled ? { scenes: sceneStartConfig } : {}),
      ...(recordingPause.enabled ? { pause: { mode: 'enabled' as const } } : {}),
      studio: {
        background: studio.background,
        ...(studio.camera.enabled && cameraDeviceId && !rawCapture && !recordingPause.enabled
          ? { camera: { x: studio.camera.x, y: studio.camera.y, size: studio.camera.size, shape: studio.camera.shape } }
          : {}),
      },
    }
  }, [sourceKind, windowCaptureSupported, monitorIdx, monitors, cards, startAllowed, monitorTargetId, selectedMonitorCurrent, windowTargetId, fps, capMs, audio, systemAudio, keys, rawCapture, recordingPause.enabled, qualityRequest, studio, cameraDeviceId, sceneStartConfig, session.countdownSeconds])

  useEffect(() => {
    session.setDraftProvider(buildPreset)
    return () => session.setDraftProvider(null)
  }, [buildPreset, session.setDraftProvider])

  const preflightStartError = useCallback(() => {
    if (startAdmissionUnknown) return UNKNOWN_START_ADMISSION
    const frameRateError = recordingFrameRateDraftError(customFps, fps)
    if (frameRateError) return frameRateError
    if (!project?.project_identity) return 'Open a current project before recording.'
    if (startAllowed === null) return 'Wait for source checks to finish.'
    if (startAllowed === false) return 'Screen capture is not ready on this machine.'
    if (sourceKind === 'window' && !windowCaptureSupported) return 'Window capture is unavailable on this machine. Choose Display.'
    if (selectedWindowMissing) return 'The selected window is no longer available. Choose another source before recording.'
    if (sourceKind === 'window' && !windowTargetId) return 'Choose an application window before recording.'
    if (sourceKind === 'display' && !selectedMonitorCurrent && !doctorAllowsPortalDisplay({ start_allowed: startAllowed, monitors, cards })) return 'The selected display changed or disappeared. Choose a current exact display before recording.'
    if (sourceKind === 'region') {
      return regionPickerCapability.availability === 'unavailable'
        ? regionPickerCapability.reason
        : 'Region capture is not connected to the native recorder in this build.'
    }
    if (studio.camera.enabled && selectedScene.layout.kind !== 'presenter_pip') return 'Wait for the camera scene to be selected before recording.'
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
  }, [cameraCapability, cameraDeviceId, cards, customFps, fps, monitorIdx, monitors, project?.project_identity, rawCapture, recordingPause, regionPickerCapability, selectedMonitorCurrent, selectedScene.layout.kind, selectedWindowMissing, sourceKind, startAdmissionUnknown, startAllowed, studio.camera.enabled, windowCaptureSupported, windowTargetId])
  useEffect(() => {
    session.setStartGuard(preflightStartError)
    return () => session.setStartGuard(null)
  }, [preflightStartError, session.setStartGuard])
  const countdown = {
    seconds: session.countdownSeconds,
    setSeconds: session.setCountdownSeconds,
    remaining: session.state.countdownRemaining,
    active: session.state.phase === 'countdown',
    starting: session.state.phase === 'starting',
    requestStart: () => {
      const error = preflightStartError()
      if (error) { setErr(error); return }
      const preset = buildPreset()
      if (!preset) { setErr('Choose a current exact display or window before recording.'); return }
      session.requestStart(preset)
    },
    cancel: session.cancelCountdown,
  }

  const stop = useCallback(() => { void session.stop() }, [session])

  // The app-level session owns the native and focused-window F9 routes.
  const toggle = useCallback(() => session.toggle(), [session])

  // Capability is displayed as status only. Registration is default-on in the
  // desktop shell and the Record page does not expose an activation switch.
  useEffect(() => {
    void getRecordHotkeyCapability().then((capability) => {
      if (capability) setRecordHotkeyCapability(capability)
    })
    return onRecordHotkeyCapability(setRecordHotkeyCapability)
  }, [])


  const busy = countdown.active || countdown.starting || phase === 'recording' || phase === 'finalizing' || audioProbeRunning || micTestRunning
  const sceneControlsDisabled = countdown.active || countdown.starting || phase === 'finalizing' || audioProbeRunning || micTestRunning
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.repeat || shouldIgnoreGlobalShortcut(event)) return
      if (matchesFixedAction(event, 'recording.marker')) {
        if (phase !== 'recording') return
        event.preventDefault()
        addRecordingMarker()
      } else if (matchesFixedAction(event, 'recording.cameraVisible')) {
        if (phase !== 'idle') return
        event.preventDefault()
        if (rawCapture || !cameraCapability.supported || cameraCapability.devices.length === 0) {
          setNote(rawCapture ? 'Camera is unavailable in Raw mode.' : cameraCapability.detail)
          return
        }
        setStudioCameraEnabled(!studio.camera.enabled)
      } else if (matchesFixedAction(event, 'recording.cameraPosition')
        || matchesFixedAction(event, 'recording.cameraPositionReverse')) {
        if (sceneControlsDisabled || !studio.camera.enabled || rawCapture) return
        event.preventDefault()
        const reverse = matchesFixedAction(event, 'recording.cameraPositionReverse')
        const current = STUDIO_POSITIONS.indexOf(studio.camera.position)
        const next = (current + (reverse ? STUDIO_POSITIONS.length - 1 : 1)) % STUDIO_POSITIONS.length
        setStudioCameraPosition(STUDIO_POSITIONS[next])
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [addRecordingMarker, cameraCapability, phase, rawCapture, sceneControlsDisabled, setStudioCameraEnabled, setStudioCameraPosition, studio.camera.enabled, studio.camera.position])
  const pauseCameraCapability = recordingPause.enabled
    ? { ...NO_CAMERA_CAPABILITY, detail: 'Camera is unavailable while Pause & resume is enabled.' }
    : cameraCapability

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
  const refreshCaptureSetup = useCallback(async () => {
    const refreshSequence = ++captureSetupRefreshSequenceRef.current
    activeCaptureSetupRefreshRef.current = refreshSequence
    setStartAllowed(null)
    try {
      const refreshed = await probe(refreshSequence)
      if (refreshSequence === captureSetupRefreshSequenceRef.current && !refreshed) {
        failedCaptureSetupRefreshRef.current = refreshSequence
        setStartAllowed(false)
      }
      return refreshed
    } catch (error) {
      if (refreshSequence === captureSetupRefreshSequenceRef.current) {
        failedCaptureSetupRefreshRef.current = refreshSequence
        setStartAllowed(false)
      }
      throw error
    } finally {
      if (activeCaptureSetupRefreshRef.current === refreshSequence) {
        activeCaptureSetupRefreshRef.current = null
      }
    }
  }, [probe])
  const displayPhase = countdown.active ? 'countdown' : countdown.starting ? 'starting' : phase
  const setupState = startAllowed === false ? 'attention'
    : ready === false ? 'pending'
      : ready === true && startAllowed === true ? 'ready'
        : 'unknown'
  const studioElapsed = phase === 'recording'
    ? fmtElapsed(elapsed)
    : countdown.active
      ? `${countdown.remaining}`
    : phase === 'finalizing'
      ? `${finalizeSec}s`
      : '0:00'
  const videoTimerControl = (
    <>
      <RecordingVideoTimerControl
        timer={sceneTimer}
        status={sceneTimerStatus}
        recording={phase === 'recording'}
        disabled={sceneControlsDisabled}
        unavailableReason={recordingPause.enabled ? 'Video timer is unavailable while Pause & resume is enabled.' : null}
        liveSupported={sceneCapability.supported}
        onTimerChange={selectSceneTimer}
        onTimerControl={(action) => { void controlSceneTimer(action) }}
      />
      {sceneRecovery.state === 'error' && (
        <div className="rec__field" data-cut-rec-scene-recovery="error" role="status">
          <p>{sceneRecovery.message}</p>
          <button type="button" className="rec__export-btn rec__export-btn--small" data-cut-action="record-scene-recovery-refresh" onClick={() => { void refreshSceneRecovery() }}>Check recording state</button>
        </div>
      )}
    </>
  )
  const captureTimingControl = (
    <>
      <div className="rec__field rec__field--length">
        <span className="rec__label">Capture length</span>
        <div className="rec__seg" role="group" aria-label="Capture length">
          {DUR_PRESETS.map((d) => (
            <button key={d.label} type="button" className={`rec__seg-btn${capMs === d.ms ? ' rec__seg-btn--on' : ''}`}
              data-cut-rec-dur={d.ms === null ? 'none' : d.ms} aria-pressed={capMs === d.ms}
              disabled={busy} onClick={() => setCapMs(d.ms)}>{d.label}</button>
          ))}
        </div>
      </div>
      <RecordingCountdownControl value={countdown.seconds} disabled={busy} onValueChange={countdown.setSeconds} />
      <div className="rec__field rec__field--pause">
        <span className="rec__label">Pause capture</span>
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
            updateStudio((current) => ({ ...current, camera: { ...current.camera, enabled: false } }))
          }}
          onControl={() => { void recordingPause.control() }}
        />
      </div>
    </>
  )
  const videoQualityControl = (
    <>
      <RecordFrameRateControl value={fps} customValue={customFps} disabled={busy}
        onValueChange={setFps} onCustomValueChange={updateCustomFps} />
      <p className="rec__fps-status" data-cut-rec-cadence>
        {requestedCadenceLabel(captureCadence, fps)} · {probedAverageCadenceLabel(captureCadence)}
      </p>
      {qualityCapability ? (
        <RecordingQualityControl
          capability={qualityCapability} outputSize={outputSize} profile={profile}
          resolution={qualityResolution} cadence={captureCadence} disabled={busy}
          unavailableReason={recordingPause.enabled ? 'Quality is unavailable while Pause & resume is enabled.' : null}
          onOutputSizeChange={setOutputSize} onProfileChange={setProfile}
        />
      ) : <p className="rec__source-note">Video quality choices are unavailable on this machine.</p>}
    </>
  )
  const hasCaptureResult = Boolean(session.state.rawPath) || session.state.phase === 'recovery'
  const recordingLayoutLabel = `${sourceKind === 'window' ? 'Window' : 'Screen'}${studio.camera.enabled && !rawCapture ? ' + camera' : ''}`
  const resultDuration = session.state.startedAt && session.state.endedAt
    ? fmtElapsed(Math.max(0, Math.floor((session.state.endedAt - session.state.startedAt) / 1000)))
    : fmtElapsed(elapsed)
  return (
    <section
      className="rec"
      data-cut-panel="record"
      data-cut-record-mode={rawCapture ? 'quick' : 'studio'}
      data-cut-record-phase={displayPhase}
      data-cut-rec-result-view={hasCaptureResult ? 'true' : 'false'}
      data-cut-rec-start-admission={startAdmissionUnknown ? 'unknown' : 'none'}
      data-cut-rec-scene-recovery-state={sceneRecovery.state}
    >
      <header className="rec__head">
        <div className="rec__head-copy">
          <h1>Recording Studio</h1>
          <p className="rec__sub">
            {rawCapture ? 'Choose what to capture, check sound, then start a raw take.' : 'Choose what to capture, check sound, then start.'}
          </p>
        </div>
        <RecordingReadinessSummary cards={cards} ready={ready} startAllowed={startAllowed} />
      </header>

      {!project && <p className="rec__source-note" data-cut-rec-no-project>Open a project before recording.</p>}

      {(
        <>
          <div className="rec__body">
          <div className="rec__studio" data-cut-rec-studio>
            <div className="rec__preview-column">
            {hasCaptureResult ? (
              <RecordingResultPanel
                outcome={session.state.phase === 'done' ? 'saved' : session.state.phase === 'recovery' ? 'recovery' : 'partial'}
                raw={session.state.raw}
                message={session.state.message}
                duration={resultDuration}
                rawSaved={Boolean(session.state.rawPath)}
                polishedClipSaved={Boolean(session.state.clipId)}
                hasMic={session.state.rawHasMic}
                hasSystem={session.state.rawHasSystem}
                streams={session.state.rawStreams}
                cursorCorrelation={session.state.cursorCorrelation}
                cadence={probedAverageCadenceLabel(session.state.cadence)}
                quality={qualityResolution}
                hotkeyScope={recordHotkeyCapability?.scope === 'global' ? 'Global F9 registered' : 'F9 works while Cut is focused'}
                exportFormat={exportFmt}
                exportRunning={Boolean(exportJob)}
                rawCopyRunning={Boolean(rawCopy.jobId)}
                exportNote={exportNote}
                rawCopyNote={rawCopy.note}
                recoveryAction={session.state.recoveryAction}
                onFormat={setExportFmt}
                onExport={() => { void exportClip() }}
                onCancelExport={() => { void cancelExport() }}
                onSaveRawCopy={() => { void rawCopy.saveCopy() }}
                onCancelRawCopy={() => { void rawCopy.cancelCopy() }}
                onAddRawToTimeline={() => { void addRawToTimeline() }}
                onOpenEdit={onOpenEdit}
                onNewRecording={session.reset}
                onOpenOutputSettings={onOpenOutputSettings}
                onRecovery={() => { void (session.state.recoveryAction === 'retry_stop' ? session.stop() : session.refreshStopStatus()) }}
              />
            ) : (
              <>
            <StudioPreview
              studio={studio}
              phase={displayPhase}
              activeCaptureId={phase === 'recording' ? session.state.captureId : null}
              elapsed={studioElapsed}
              sceneName={recordingLayoutLabel}
              sceneState={sceneStatus.state}
              sourcePreview={sourcePreview.presentation}
              sourcePreviewCanStart={sourcePreview.presentation.available && sourcePreview.target !== null && !busy && !sourcePreview.busy}
              onSourcePreviewStart={() => { void sourcePreview.start() }}
              actions={{
                cameraCapability: pauseCameraCapability,
                rawCapture,
                configurationDisabled: busy,
                liveAdjustDisabled: sceneControlsDisabled,
                onCameraEnabled: setStudioCameraEnabled,
                onCameraPosition: setStudioCameraPosition,
                onCameraShape: setStudioCameraShape,
                cameraDeviceId,
                onCameraReset: resetStudioCamera,
                onBackground: setStudioBackground,
              }}
            />
            <div className="rec__preview-tools" data-cut-rec-preview-tools>
              <h2>Preview and rehearsal</h2>
              <RecordingSourcePreview
                preview={sourcePreview.presentation}
                status={sourcePreview.status}
                target={sourcePreview.target}
                statusError={sourcePreview.statusError}
                busy={busy || sourcePreview.busy}
                onPause={() => { void sourcePreview.pause() }}
                onResume={() => { void sourcePreview.resume() }}
                onHide={() => { void sourcePreview.hide() }}
                onStop={() => { void sourcePreview.stop() }}
              />
              <RecordingRehearsal
                disabled={busy || startAdmissionUnknown}
                sourceKind={sourceKind}
                fps={fps}
                monitor={monitors.length >= 2 ? monitorIdx : null}
                monitorId={monitorIdx === null ? null : monitors.find((monitor) => monitor.index === monitorIdx)?.id ?? null}
                windowId={windowTargetId}
                startAllowed={startAllowed}
                startError={startAdmissionUnknown ? 'Recording is locked until ShellX Cut restarts.' : preflightStartError()}
                onRefresh={refreshCaptureSetup}
              />
            </div>
              </>
            )}
            </div>
            <StudioControls
              videoTimerControl={videoTimerControl}
              captureTimingControl={captureTimingControl}
              videoQualityControl={videoQualityControl}
              studio={studio}
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
              onCameraReset={resetStudioCamera}
            />

          {/* Screen and sound retain the only source and audio selectors. */}
          <aside className="rec__settings" data-cut-rec-settings data-cut-rec-quality-supported={Boolean(qualityCapability)}>
            <div className="rec__settings-head">
              <div>
                <span className="rec__eyebrow">Capture setup</span>
                <h2 className="rec__settings-title">1 · Screen and sound</h2>
              </div>
              <span className="rec__settings-state" data-cut-rec-setup-state={setupState}>
                {setupState === 'attention' ? 'Check setup'
                  : setupState === 'pending' ? 'Source check remains'
                    : setupState === 'ready' ? 'Ready to configure'
                      : 'Checking setup'}
              </span>
            </div>
            <div className="rec__settings-primary">
              <RecordingSourceSetup
                selection={{ sourceKind, monitors, monitorIdx, windows, windowTargetId, selectedWindowMissing }}
                disabled={busy}
                pauseEnabled={recordingPause.enabled}
                windowCaptureSupported={windowCaptureSupported === true}
                regionCapability={regionPickerCapability}
                onRefresh={refreshCaptureSetup}
                onSourceKindChange={(next) => { setSourceKind(next); setRegionPickerOpen(next === 'region' && regionPickerCapability.availability === 'available') }}
                onMonitorChange={(index) => { setMonitorIdx(index); setMonitorTargetId(monitors.find((monitor) => monitor.index === index)?.id ?? null) }}
                onWindowChange={setWindowTargetId}
              />
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
                  Polished clip in Edit
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
                  Raw MP4 file
                </button>
              </div>
              <p className="rec__source-note" data-cut-rec-mode-note>
                {rawCapture
                  ? 'Save the unchanged MP4 immediately in the default export folder.'
                  : 'Save the unchanged MP4, then add a polished editable clip to this project.'}
              </p>
              {!rawCapture && (
                <label className="rec__toggle rec__toggle--keys" data-cut-rec-keys-toggle title="Keystrokes can reveal passwords — off by default">
                  <input type="checkbox" data-cut-rec-keys-toggle-input checked={recordingPause.enabled ? false : keys} disabled={busy || recordingPause.enabled} onChange={(event) => setKeys(event.target.checked)} /> Show keystrokes
                </label>
              )}
            </div>
            </div>

          </aside>
          </div>
        </div>

        {/* Transport / HUD. It is deliberately a sibling of the scrollable setup
            workspace: capture controls reserve footer space instead of covering
            rehearsal, scene, or recovery controls while the body scrolls. */}
        <div className="rec__transport" data-cut-studio-result={displayPhase} data-cut-rec-primary-transport>
            {countdown.active ? (
              <div className="rec__hud rec__hud--countdown" data-cut-rec-countdown-transport>
                Starting in {countdown.remaining}… Press Escape to cancel.
              </div>
            ) : countdown.starting ? (
              <div className="rec__hud rec__hud--countdown" data-cut-rec-starting-transport aria-live="polite">
                Starting recording…
              </div>
            ) : phase === 'recording' ? (
              <>
                <RecordingLiveControls
                  elapsed={capMs !== null ? `${remaining}s left` : fmtElapsed(elapsed)}
                  sceneName={recordingLayoutLabel}
                  audioMeters={<RecordingAudioMeters {...recordingAudioMeters} />}
                  captureSafety={(
                    <RecordingCaptureSafetyStatus
                      sourceLifecycle={recordingAudioMeters.sourceLifecycle}
                      controllerPlacement={recordingAudioMeters.controllerPlacement}
                    />
                  )}
                  recoveryControls={null}
                  pauseCapability={recordingPause.capability}
                  pauseEnabled={recordingPause.enabled}
                  pauseState={recordingPause.state}
                  pauseMessage={recordingPause.message}
                  markerPending={markerPending}
                  stopRetry={stopRetryRequired}
                  onMarker={addRecordingMarker}
                  onPauseControl={() => { void recordingPause.control() }}
                  onStop={stop}
                />
              </>
            ) : phase === 'finalizing' ? (
              <div className="rec__hud rec__hud--finalize" data-cut-rec-finalizing data-cut-rec-finalize-sec={finalizeSec}>
                {note || 'finalizing…'}{finalizeSec > 0 ? ` (${finalizeSec}s)` : ''}
              </div>
            ) : !hasCaptureResult ? (
              <button
                type="button"
                className="rec__start"
                data-cut-action="record-start"
                disabled={busy || Boolean(preflightStartError())}
                aria-describedby={startAdmissionUnknown ? 'cut-rec-start-admission' : undefined}
                onClick={countdown.requestStart}
              >
                ● Start recording ({SHORTCUT_LABEL})
              </button>
            ) : null}
            {!hasCaptureResult && (err || startAdmissionUnknown) && (
              <p
                id={startAdmissionUnknown ? 'cut-rec-start-admission' : undefined}
                className="rec__err"
                data-cut-rec-error
                data-cut-rec-start-admission-error={startAdmissionUnknown ? 'unknown' : undefined}
                role="alert"
              >
                {startAdmissionUnknown ? UNKNOWN_START_ADMISSION : err}
              </p>
            )}
            {phase === 'recording' && (
              <p className="rec__hint">
                {capMs !== null
                  ? `Stops automatically at the limit, or press Stop / ${SHORTCUT_LABEL} any time.`
                  : `Recording until you stop — press Stop or ${SHORTCUT_LABEL}.`}
                {' '}{recordHotkeyCapability?.scope === 'global'
                  ? SHORTCUT_LABEL + ' works globally — even when another app is focused.'
                  : SHORTCUT_LABEL + ' works while ShellX Cut is focused.'}
                {' '}The first capture on this machine pops a one-time screen-share consent dialog.
              </p>
            )}
          </div>
        </>
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
          onToggle={toggle}
        />
      )}
    </section>
  )
}
