import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type Project } from '../lib/client'
import { isBlockingOverlayActive, shouldIgnoreGlobalShortcut } from '../lib/dom'
import { matchesFixedAction } from '../lib/keymap'
import { onRecordHotkey } from '../lib/tauri'
import { recordingStartResult } from '../panels/Record/recordingStartResult'
import { useRecordingPause } from '../panels/Record/useRecordingPause'
import { markRecordingStartAdmissionUnknown } from '../panels/Record/recordingStartAdmission'
import type { RecordingStartResult } from '../panels/Record/recordingStartResult'
import type { RecordingCadence } from '../panels/Record/recordingCadence'
import type { StudioRawStreams, CursorCorrelation, StudioEventPayload, StudioState } from '../panels/Record/studioTypes'
import type { RecordingSourceKind } from '../panels/Record/regionPickerModel'
import type { RecordingOutputSize, RecordingQualityProfile } from '../panels/Record/recordingQuality'
import { RecordingCountdownGuard, countdownRemainingSeconds, type RecordingCountdownSeconds } from '../panels/Record/recordingCountdown'
import { firstUseRecordingPreset, loadRecordingPreset, normalizeRecordingPresetForStart, saveRecordingPreset, sameProjectIdentity, validateRecordingPreset, type RecordingPreset } from './recordingPreset'
import { recordingInputHook, UNOBSERVED_INPUT_HOOK, type RecordingInputHook } from '../panels/Record/recordingInputHook'
import { RecordingToggleGate, classifyStopFailure, stopArgs } from './recordingSessionModel'

export type RecordingPhase = 'idle' | 'countdown' | 'starting' | 'recording' | 'finalizing' | 'recovery' | 'done' | 'error'
export interface RecordingSessionState {
  phase: RecordingPhase
  captureId: string | null
  resultCaptureId: string | null
  resultProjectIdentity: Project['project_identity'] | null
  projectName: string | null
  raw: boolean
  rawPath: string | null
  source: string | null
  plan: string | null
  clipId: string | null
  startedAt: number | null
  endedAt: number | null
  message: string
  indicatorWarning: string | null
  initialStudioWarning: string | null
  recoveryAction: 'retry_stop' | 'check_status' | null
  countdownRemaining: number
  startResult: RecordingStartResult | null
  rawStreams: StudioRawStreams | null
  inputHook: RecordingInputHook
  cursorCorrelation: CursorCorrelation | null
  cadence: RecordingCadence | null
  quality: unknown
  rawHasMic: boolean
  rawHasSystem: boolean
}

/** Visible Record controls kept only for this app session, including invalid choices. */
export interface RecordingDraftView {
  sourceKind: RecordingSourceKind
  monitorIdx: number | null
  monitorTargetId: string | null
  windowTargetId: string | null
  fps: number
  customFps: string
  durationMs: number | null
  audio: boolean
  systemAudio: boolean
  keys: boolean
  raw: boolean
  cameraDeviceId: string | null
  studio: StudioState
  cameraLayoutCustomized: boolean
  sceneId: string | null
  pauseEnabled: boolean
  outputSize: RecordingOutputSize
  qualityProfile: RecordingQualityProfile
}

const INITIAL: RecordingSessionState = {
  phase: 'idle', captureId: null, resultCaptureId: null, resultProjectIdentity: null,
  projectName: null, raw: false, rawPath: null,
  source: null, plan: null, clipId: null, startedAt: null, endedAt: null, message: '', indicatorWarning: null, countdownRemaining: 0,
  recoveryAction: null, initialStudioWarning: null, startResult: null, rawStreams: null, inputHook: UNOBSERVED_INPUT_HOOK, cursorCorrelation: null, cadence: null, quality: null, rawHasMic: false, rawHasSystem: false,
}
const UNKNOWN_START = 'The recorder returned an incomplete Start response. Recording ownership is unknown; restart Cut before another capture.'

async function indicator(command: 'begin_recording_indicator' | 'end_recording_indicator', captureId: string): Promise<string | null> {
  const shell = (window as unknown as { __TAURI__?: { core?: { invoke?: (name: string, args: object) => Promise<unknown> } } }).__TAURI__
  if (!shell?.core?.invoke) return null
  try {
    const result = await shell.core.invoke(command, { captureId }) as { applied?: boolean; reason?: string | null }
    return result.applied === true ? null : result.reason ?? 'The desktop did not confirm its recording indicator.'
  } catch (error) {
    return `Desktop recording indicator failed: ${error instanceof Error ? error.message : 'unknown error'}`
  }
}

async function appendInitialStudioEvent(captureId: string, event: StudioEventPayload): Promise<boolean> {
  try {
    const reply = await callVerb('screen_record.studio_event', { capture_id: captureId, event: { t_ms: 0, ...event } })
    const saved = (reply.result as { last_event?: { source?: string; kind?: string; t_ms?: number } } | undefined)?.last_event
    return reply.ok && saved?.source === event.source && saved?.kind === event.kind && saved?.t_ms === 0
  } catch { return false }
}

/** Exactly one owner for native capture, result, recovery, and both F9 routes. */
export function useRecordingSession({ project, onEnsureProject, onResult }: {
  project: Project | null
  onEnsureProject: () => Promise<Project | null>
  onResult: () => void
}) {
  const [state, setState] = useState<RecordingSessionState>(INITIAL)
  const stateRef = useRef(state)
  stateRef.current = state
  const projectRef = useRef(project)
  projectRef.current = project
  const ensureProjectRef = useRef(onEnsureProject)
  ensureProjectRef.current = onEnsureProject
  const resultRef = useRef(onResult)
  resultRef.current = onResult
  const presetRef = useRef<RecordingPreset | null>(loadRecordingPreset())
  // Live Pause/Resume acknowledgements must outlive Record panel navigation.
  const recordingPause = useRecordingPause(Boolean(presetRef.current?.pause))
  // The mounted Record controls own the current draft. Keep its latest value
  // through an Edit handoff without promoting it to an admitted/saved preset.
  // `null` is an invalid or incomplete choice and must not fall back to a take.
  const draftRef = useRef<RecordingPreset | null | undefined>(undefined)
  const draftViewRef = useRef<RecordingDraftView | null>(null)
  const setDraft = useCallback((draft: RecordingPreset | null, view: RecordingDraftView) => {
    draftRef.current = draft
    draftViewRef.current = view
  }, [])
  const busyRef = useRef(false)
  const unknownStartRef = useRef(false)
  const toggleGateRef = useRef(new RecordingToggleGate())
  const captureRef = useRef<string | null>(null)
  const projectIdentityRef = useRef<Project['project_identity']>(undefined)
  const activePresetRef = useRef<RecordingPreset | null>(null)
  const previewReleaseRef = useRef<(() => Promise<void> | void) | null>(null)
  const setPreviewRelease = useCallback((release: (() => Promise<void> | void) | null) => { previewReleaseRef.current = release }, [])
  const startGuardRef = useRef<(() => string | null) | null>(null)
  const setStartGuard = useCallback((guard: (() => string | null) | null) => { startGuardRef.current = guard }, [])
  const countdownRef = useRef<number | null>(null)
  const countdownGuardRef = useRef(new RecordingCountdownGuard())
  const [countdownSeconds, setCountdownSeconds] = useState<RecordingCountdownSeconds>(presetRef.current?.startCountdownSeconds ?? 3)

  const publish = useCallback((patch: Partial<RecordingSessionState>) => {
    const next = { ...stateRef.current, ...patch }
    stateRef.current = next
    setState(next)
  }, [])

  useEffect(() => {
    if (!state.captureId) recordingPause.clearCapture()
  }, [state.captureId, recordingPause.clearCapture])

  const showSetup = useCallback((message: string) => {
    publish({ phase: 'error', message })
    // The app-level status banner owns background F9 failures. Do not switch
    // workspace or restore a minimized window to present setup errors.
  }, [publish])

  const ensureCurrentProject = useCallback(async (): Promise<Project | null> => {
    const current = projectRef.current
    if (current?.project_identity) return current
    try {
      const created = await ensureProjectRef.current()
      if (created?.project_identity) {
        projectRef.current = created
        return created
      }
    } catch { /* the caller publishes one bounded failure below */ }
    return null
  }, [])

  const start = useCallback(async (draft?: RecordingPreset | null) => {
    if (busyRef.current || captureRef.current || unknownStartRef.current) return
    const blocked = projectRef.current?.project_identity ? startGuardRef.current?.() : null
    if (blocked) { showSetup(blocked); return }
    let preset = draft === undefined ? (draftRef.current === undefined ? presetRef.current : draftRef.current) : draft
    if (preset) preset = normalizeRecordingPresetForStart(preset)
    if (draft === null) { showSetup('Choose a current source and valid recording setup first.'); return }
    busyRef.current = true
    toggleGateRef.current.setInFlight(true)
    try {
      if (!preset) {
        publish({ phase: 'starting', message: 'Checking the primary display for your first recording…' })
        const discovery = await callVerb('screen_record.doctor', {})
        if (!discovery.ok) { showSetup(`Recorder checks failed: ${discovery.error?.message ?? 'unavailable'}`); return }
        preset = firstUseRecordingPreset(discovery.result)
        if (!preset) { showSetup('No current screen source is available. Open Record to check capture setup.'); return }
      }
      const currentProject = await ensureCurrentProject()
      if (!currentProject) { showSetup('Could not create a project for this recording. Try F9 again.'); return }
      // A no-project F9/Start may have created the fallback while the Record
      // panel's source checks were still running. Re-run that product-owned
      // guard before the immediate-start path so it cannot skip a stale source
      // or device validation just because project admission was asynchronous.
      const guardFailure = startGuardRef.current?.()
      if (guardFailure) { showSetup(guardFailure); return }
      publish({ phase: 'starting', message: 'Checking current source, devices, and permissions…', raw: preset.raw,
        rawPath: null, source: null, plan: null, resultCaptureId: null, resultProjectIdentity: null,
        clipId: null, endedAt: null, recoveryAction: null, initialStudioWarning: null, startResult: null,
        rawStreams: null, inputHook: UNOBSERVED_INPUT_HOOK, cursorCorrelation: null, cadence: null, quality: null })
      // A fallback project may take time to create. Recheck the source again
      // immediately before Start, including on first use.
      const doctor = await callVerb('screen_record.doctor', {})
      if (!doctor.ok) { showSetup(`Recorder checks failed: ${doctor.error?.message ?? 'unavailable'}`); return }
      const invalid = validateRecordingPreset(preset, doctor.result)
      if (invalid) { showSetup(invalid); return }
      if (!sameProjectIdentity(currentProject.project_identity, projectRef.current?.project_identity)) { showSetup('The open project changed during recording checks. Try again.'); return }
      // Save only a freshly admitted setup intent. No capture ID, output path,
      // one-use token, or project binding is persisted.
      presetRef.current = preset
      let presetStorageWarning = ''
      try { saveRecordingPreset(preset) } catch { presetStorageWarning = ' Recording setup could not be stored for the next launch.' }
      await previewReleaseRef.current?.()
      const args: Record<string, unknown> = {
        fps: preset.fps, audio: preset.audio, system_audio: preset.systemAudio,
        keys: preset.raw || preset.pause ? false : preset.keys,
        expected_project_identity: currentProject.project_identity,
      }
      if (preset.durationMs !== null) args.duration_ms = preset.durationMs
      if (preset.source.kind === 'display') args.monitor_id = preset.source.monitorId
      else if (preset.source.kind === 'window') args.window = preset.source.windowId
      if (preset.quality && !preset.pause) args.quality = preset.quality
      if (preset.cameraId && !preset.raw && !preset.pause) args.camera_id = preset.cameraId
      if (preset.scenes && !preset.pause) args.scenes = preset.scenes
      if (preset.pause) args.pause = preset.pause
      if (preset.studio) args.studio = preset.studio
      const reply = await callVerb('screen_record.start', args)
      if (!reply.ok) { showSetup(`Could not start recording: ${reply.error?.message ?? 'unknown error'}`); return }
      const admitted = recordingStartResult(reply.result)
      if (!admitted) {
        unknownStartRef.current = true
        markRecordingStartAdmissionUnknown()
        showSetup(UNKNOWN_START)
        return
      }
      captureRef.current = admitted.captureId
      projectIdentityRef.current = currentProject.project_identity ? { ...currentProject.project_identity } : undefined
      activePresetRef.current = preset
      const warning = await indicator('begin_recording_indicator', admitted.captureId)
      let initialStudioWarning: string | null = null
      if (!preset.raw && preset.studio) {
        const events: StudioEventPayload[] = [
          { source: 'background', kind: 'style', background: preset.studio.background },
        ]
        if (preset.cameraId && preset.studio.camera) {
          events.push({ source: 'camera', kind: 'visibility', visible: true })
          events.push({ source: 'camera', kind: 'transform', ...preset.studio.camera })
        }
        for (const event of events) {
          if (!await appendInitialStudioEvent(admitted.captureId, event)) {
            initialStudioWarning = 'Initial recording style was not saved. The raw MP4 remains available, but polished output will not be claimed for this take.'
            break
          }
        }
      }
      publish({ phase: 'recording', captureId: admitted.captureId, startResult: admitted, cadence: admitted.cadence, startedAt: Date.now(), projectName: currentProject.name,
        message: (sameProjectIdentity(currentProject.project_identity, projectRef.current?.project_identity)
          ? 'Recording' : 'Recording started, but the open project changed. Stop this capture before continuing.') + presetStorageWarning
          + (initialStudioWarning ? ` ${initialStudioWarning}` : ''),
        initialStudioWarning, indicatorWarning: warning })
    } catch (error) {
      showSetup(`Recorder start failed: ${error instanceof Error ? error.message : 'server unavailable'}`)
    } finally { busyRef.current = false; toggleGateRef.current.setInFlight(false) }
  }, [ensureCurrentProject, publish, showSetup])

  const cancelCountdown = useCallback(() => {
    if (countdownRef.current === null) return
    countdownGuardRef.current.cancel()
    window.clearInterval(countdownRef.current)
    countdownRef.current = null
    busyRef.current = false
    toggleGateRef.current.setInFlight(false)
    publish({ phase: 'idle', countdownRemaining: 0, message: 'Countdown cancelled. Nothing was recorded.' })
  }, [publish])

  const requestStart = useCallback(async (draft?: RecordingPreset | null, seconds = draft?.startCountdownSeconds ?? countdownSeconds) => {
    if (busyRef.current || captureRef.current || unknownStartRef.current) return
    const blocked = projectRef.current?.project_identity ? startGuardRef.current?.() : null
    if (blocked) { showSetup(blocked); return }
    if (draft === null) { showSetup('Choose a current source and valid recording setup first.'); return }
    // With no saved setup, F9 uses a freshly checked primary screen. There is
    // no prior countdown preference, and Start owns project creation/admission.
    if (draft === undefined && draftRef.current === undefined && !presetRef.current) { void start(); return }
    if (!projectRef.current?.project_identity) {
      busyRef.current = true
      toggleGateRef.current.setInFlight(true)
      const ensured = await ensureCurrentProject()
      busyRef.current = false
      toggleGateRef.current.setInFlight(false)
      if (!ensured) { showSetup('Could not create a project for this recording. Try F9 again.'); return }
    }
    const requestedProjectIdentity = projectRef.current?.project_identity
    if (!requestedProjectIdentity) { showSetup('Could not create a project for this recording. Try F9 again.'); return }
    if (seconds <= 0) { void start(draft); return }
    busyRef.current = true
    toggleGateRef.current.setInFlight(true)
    const generation = countdownGuardRef.current.begin()
    const endAt = Date.now() + seconds * 1000
    publish({ phase: 'countdown', countdownRemaining: seconds, message: `Starting in ${seconds}…` })
    countdownRef.current = window.setInterval(() => {
      if (!countdownGuardRef.current.isCurrent(generation)) return
      const remaining = countdownRemainingSeconds(endAt)
      if (remaining > 0) { publish({ countdownRemaining: remaining, message: `Starting in ${remaining}…` }); return }
      countdownGuardRef.current.handoff(generation, () => {
        if (countdownRef.current !== null) window.clearInterval(countdownRef.current)
        countdownRef.current = null
        busyRef.current = false
        toggleGateRef.current.setInFlight(false)
        if (!sameProjectIdentity(requestedProjectIdentity, projectRef.current?.project_identity)) {
          showSetup('The open project changed during the countdown. Review the recording setup and start again.')
          return
        }
        const blocked = startGuardRef.current?.()
        if (blocked) { showSetup(blocked); return }
        void start(draft)
      })
    }, 100)
  }, [countdownSeconds, ensureCurrentProject, publish, showSetup, start])

  useEffect(() => () => {
    countdownGuardRef.current.cancel()
    if (countdownRef.current !== null) window.clearInterval(countdownRef.current)
  }, [])

  const checkCaptureStatus = useCallback(async (id: string, stopError = 'Stop did not produce a confirmed result.') => {
    let status
    try { status = await callVerb('screen_record.status', { capture_id: id }) }
    catch {
      if (captureRef.current === id) publish({ phase: 'recovery', recoveryAction: 'check_status',
        message: `${stopError} Capture status could not be checked. Check capture status before another action.` })
      return
    }
    if (captureRef.current !== id) return
    const disposition = classifyStopFailure(id, {
      ok: status.ok, code: status.error?.code,
      result: status.result as { capture_id?: string; terminal?: boolean } | undefined,
    })
    if (disposition.state === 'ended') {
      const warning = await indicator('end_recording_indicator', id)
      captureRef.current = null
      activePresetRef.current = null
      publish({ phase: 'error', captureId: null, recoveryAction: null, indicatorWarning: warning,
        message: `${stopError} ${disposition.detail}` })
      return
    }
    publish({ phase: 'recovery', recoveryAction: disposition.state === 'live' ? 'retry_stop' : 'check_status',
      message: `${stopError} ${disposition.detail} ${disposition.state === 'live'
        ? 'Retry Stop for this same capture.' : 'Check capture status before another action.'}` })
  }, [publish])

  const refreshStopStatus = useCallback(async () => {
    const id = captureRef.current
    if (!id || busyRef.current) return
    busyRef.current = true
    try { await checkCaptureStatus(id, 'Checking this capture after an unconfirmed Stop.') }
    finally { busyRef.current = false }
  }, [checkCaptureStatus])

  const reset = useCallback(() => {
    if (busyRef.current || captureRef.current || unknownStartRef.current) return
    publish(INITIAL)
  }, [publish])

  const stop = useCallback(async () => {
    const id = captureRef.current
    const preset = activePresetRef.current
    if (!id || !preset || busyRef.current || (stateRef.current.phase === 'recovery' && stateRef.current.recoveryAction !== 'retry_stop')) return
    busyRef.current = true
    toggleGateRef.current.setInFlight(true)
    publish({ phase: 'finalizing', message: 'Stopping and saving the raw MP4…' })
    let terminal = false
    try {
      const reply = await callVerb('screen_record.stop', stopArgs(id, preset.raw))
      if (!reply.ok) {
        await checkCaptureStatus(id, `Stop did not complete: ${reply.error?.message ?? 'unknown error'}.`)
        return
      }
      terminal = true
      const result = reply.result as { capture_id?: string; raw_path?: string | null; source?: string; plan?: string; raw_streams?: StudioRawStreams; input_hook?: unknown; cursor_correlation?: CursorCorrelation; cadence?: RecordingCadence; quality?: unknown; raw_has_mic?: boolean; raw_has_system?: boolean }
      const warning = await indicator('end_recording_indicator', id)
      captureRef.current = null
      activePresetRef.current = null
      if (result.capture_id && result.capture_id !== id) {
        publish({ phase: 'error', captureId: null, recoveryAction: null, message: 'Stop returned a different capture ID. Inspect Recording Recovery before another take.', indicatorWarning: warning })
        return
      }
      if (!result.raw_path) {
        publish({ phase: 'error', captureId: null, message: 'Capture ended, but no raw MP4 was confirmed.', indicatorWarning: warning })
        return
      }
      publish({ rawPath: result.raw_path, source: result.source ?? null, plan: result.plan ?? null,
        resultCaptureId: id, resultProjectIdentity: projectIdentityRef.current ? { ...projectIdentityRef.current } : null,
        captureId: null, recoveryAction: null, indicatorWarning: warning, endedAt: Date.now(),
        rawStreams: result.raw_streams ?? null, inputHook: recordingInputHook(result.input_hook), cursorCorrelation: result.cursor_correlation ?? null, cadence: result.cadence ?? null, quality: result.quality ?? null,
        rawHasMic: Boolean(result.raw_has_mic), rawHasSystem: Boolean(result.raw_has_system) })
      if (preset.raw) {
        publish({ phase: 'done', message: 'Raw MP4 saved to the recording project’s default export folder.' })
        resultRef.current()
        return
      }
      if (stateRef.current.initialStudioWarning) {
        publish({ phase: 'error', message: `Raw MP4 saved. ${stateRef.current.initialStudioWarning}` })
        return
      }
      if (!result.source || !result.plan) {
        publish({ phase: 'error', message: 'Raw MP4 saved, but the editable plan was not returned. The clip was not added.' })
        return
      }
      if (!sameProjectIdentity(projectIdentityRef.current, projectRef.current?.project_identity)) {
        publish({ phase: 'error', message: 'Raw MP4 saved, but the open project changed before polish. Reopen the recording project to continue.' })
        return
      }
      publish({ phase: 'finalizing', message: 'Raw MP4 saved. Polishing and adding the editable clip…' })
      const polished = await callVerb('screen_record.polish', { source: result.source, plan: result.plan, raw: false })
      if (!polished.ok) { publish({ phase: 'error', message: `Raw MP4 saved; polish failed: ${polished.error?.message ?? 'unknown error'}` }); return }
      const clipId = (polished.result as { clip_id?: string })?.clip_id
      if (!clipId) { publish({ phase: 'error', message: 'Raw MP4 saved; polish returned no confirmed editable clip.' }); return }
      publish({ phase: 'done', clipId, message: 'Raw MP4 saved and polished clip added to the current project.' })
      resultRef.current()
    } catch (error) {
      if (terminal) publish({ phase: 'error', recoveryAction: null, message: `Could not complete Stop: ${error instanceof Error ? error.message : 'server unavailable'}. The capture ended; inspect the saved result.` })
      else await checkCaptureStatus(id, `Could not complete Stop: ${error instanceof Error ? error.message : 'server unavailable'}.`)
    } finally { busyRef.current = false; toggleGateRef.current.setInFlight(false) }
  }, [checkCaptureStatus, publish])

  useEffect(() => {
    if (state.phase !== 'recording' || !state.captureId) return
    const id = state.captureId
    let checking = false
    const check = async () => {
      if (checking || busyRef.current || captureRef.current !== id) return
      checking = true
      try {
        const reply = await callVerb('screen_record.status', { capture_id: id })
        if (captureRef.current !== id || busyRef.current) return
        if (reply.ok) {
          const result = reply.result as { capture_id?: string; terminal?: boolean; input_hook?: unknown }
          if (result.capture_id === id) {
            publish({ inputHook: recordingInputHook(result.input_hook) })
            if (result.terminal === true) void stop()
          } else if (result.capture_id !== id) publish({ phase: 'recovery', message: 'Recorder status returned a different capture ID. Inspect Recording Recovery.' })
        } else if (reply.error?.code === 'not_found') await checkCaptureStatus(id, 'The native capture ended before Stop returned an output.')
      } finally { checking = false }
    }
    const timer = window.setInterval(() => { void check() }, 1000)
    return () => window.clearInterval(timer)
  }, [checkCaptureStatus, publish, state.captureId, state.phase, stop])

  const toggleRef = useRef<() => void>(() => {})
  toggleRef.current = () => {
    if (countdownRef.current !== null) {
      cancelCountdown()
      toggleGateRef.current.claim(Date.now())
      return
    }
    if (!toggleGateRef.current.claim(Date.now())) return
    if (busyRef.current) return
    if (captureRef.current) { void stop(); return }
    if (stateRef.current.phase === 'recovery' || unknownStartRef.current) return
    if (isBlockingOverlayActive()) return
    requestStart(draftRef.current)
  }
  useEffect(() => {
    const off = onRecordHotkey(() => toggleRef.current())
    const onKey = (event: KeyboardEvent) => {
      if (event.repeat) return
      if (event.key === 'Escape' && countdownRef.current !== null) { event.preventDefault(); cancelCountdown(); return }
      if (captureRef.current && matchesFixedAction(event, 'recording.toggle')) {
        event.preventDefault()
        toggleRef.current()
        return
      }
      if (shouldIgnoreGlobalShortcut(event)) return
      if (!matchesFixedAction(event, 'recording.toggle')) return
      event.preventDefault()
      toggleRef.current()
    }
    window.addEventListener('keydown', onKey)
    return () => { off(); window.removeEventListener('keydown', onKey) }
  }, [])

  return { state, start, requestStart, cancelCountdown, countdownSeconds, setCountdownSeconds, stop, reset,
    recordingPause, setPreviewRelease, setDraft, setStartGuard, refreshStopStatus,
    toggle: () => toggleRef.current(), preset: presetRef.current, draft: draftRef.current, draftView: draftViewRef.current }
}

export type RecordingSession = ReturnType<typeof useRecordingSession>
