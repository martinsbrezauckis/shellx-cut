import { watchPolishProgress } from './polishProgress'
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
import { recordingPolishArgs, firstUseRecordingPreset, loadRecordingPreset, normalizeRecordingPresetForStart, recordingReadinessFailure, saveRecordingPreset, sameProjectIdentity, validateRecordingPreset, type RecordingPreset } from './recordingPreset'
import { recordingInputHook, UNOBSERVED_INPUT_HOOK, type RecordingInputHook } from '../panels/Record/recordingInputHook'
import { recordingSessionNotice } from '../panels/Record/recordingUiModel'
import { RecordingToggleGate, classifyStopFailure, isNothingRecorded, nothingRecordedText, refusalText, savingProgressText, stopArgs, withSavedNotes } from './recordingSessionModel'
import { waitForFirstFrame } from './recordingFirstFrame'
import { resolveRecordingStopJob } from './recordingStopJob'
import { recordingDoctorArgs } from './recordingDoctorArgs'
import { SAVING_BEFORE_CLOSE, useRecordingCloseRequest } from './useRecordingCloseRequest'
import { useActiveRecordingAdoption, type ActiveRecording } from './useActiveRecordingAdoption'

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
  cameraSeparate?: boolean
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
const STARTING_MESSAGE = 'Starting recording… waiting for the first screen frame.'
const ADOPTED_MESSAGE = 'Recording started outside this window (for example by an agent). Stop saves it as an unedited MP4.'
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
  const recordingPause = useRecordingPause(Boolean(presetRef.current?.pause), state.phase === 'recording' ? state.captureId : null)
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
    if (patch.message !== undefined) next.message = recordingSessionNotice(patch.message, next.phase)
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
    const blocked = startGuardRef.current?.()
    if (blocked) { showSetup(blocked); return }
    let preset = draft === undefined ? (draftRef.current === undefined ? presetRef.current : draftRef.current) : draft
    if (preset) preset = normalizeRecordingPresetForStart(preset)
    if (draft === null) { showSetup('Choose a current source and valid recording setup first.'); return }
    const initialProjectIdentity = projectRef.current?.project_identity
    busyRef.current = true
    toggleGateRef.current.setInFlight(true)
    try {
      if (!preset) {
        publish({ phase: 'starting', message: 'Checking the primary display for your first recording…' })
        const discovery = await callVerb('screen_record.doctor', {})
        if (!discovery.ok) { showSetup(`Recorder checks failed: ${refusalText(discovery.error, 'unavailable')}`); return }
        preset = firstUseRecordingPreset(discovery.result)
        if (!preset) { showSetup(recordingReadinessFailure(discovery.result) ?? 'No current screen source is available. Open Record to check capture setup.'); return }
      }
      // Preflight refusals create nothing. A project created after these
      // checks stays open if native Start refuses, and the next attempt reuses it.
      const guardFailure = startGuardRef.current?.()
      if (guardFailure) { showSetup(guardFailure); return }
      publish({ phase: 'starting', message: 'Checking current source, devices, and permissions…', raw: preset.raw,
        rawPath: null, source: null, plan: null, resultCaptureId: null, resultProjectIdentity: null,
        clipId: null, endedAt: null, recoveryAction: null, initialStudioWarning: null, startResult: null,
        rawStreams: null, inputHook: UNOBSERVED_INPUT_HOOK, cursorCorrelation: null, cadence: null, quality: null })
      const doctor = await callVerb('screen_record.doctor', recordingDoctorArgs({
        fps: preset.fps,
        sourceKind: preset.source.kind === 'window' ? 'window' : 'display',
        monitor: null,
        monitorId: preset.source.kind === 'display' ? preset.source.monitorId : null,
        windowId: preset.source.kind === 'window' ? preset.source.windowId : null,
      }))
      if (!doctor.ok) { showSetup(`Recorder checks failed: ${refusalText(doctor.error, 'unavailable')}`); return }
      const invalid = validateRecordingPreset(preset, doctor.result)
      if (invalid) { showSetup(invalid); return }
      const projectChanged = initialProjectIdentity
        ? !sameProjectIdentity(initialProjectIdentity, projectRef.current?.project_identity)
        : Boolean(projectRef.current?.project_identity)
      if (projectChanged) { showSetup('The open project changed during recording checks. Try again.'); return }
      const currentProject = await ensureCurrentProject()
      if (!currentProject) { showSetup('Could not create a project for this recording. Try F9 again.'); return }
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
      if (!reply.ok) { showSetup(`Could not start recording: ${refusalText(reply.error)}`); return }
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
      const recordingMessage = (sameProjectIdentity(currentProject.project_identity, projectRef.current?.project_identity)
        ? 'Recording' : 'Recording started, but the open project changed. Stop this capture before continuing.') + presetStorageWarning
        + (initialStudioWarning ? ` ${initialStudioWarning}` : '')
      // "Starting…" until the first screen frame is captured; the timer then
      // counts from that frame. Stop (button or F9) works during this wait.
      publish({ phase: 'starting', captureId: admitted.captureId, startResult: admitted, cadence: admitted.cadence, startedAt: null, projectName: currentProject.name,
        message: STARTING_MESSAGE, initialStudioWarning, indicatorWarning: warning })
      busyRef.current = false
      toggleGateRef.current.setInFlight(false)
      const firstFrame = await waitForFirstFrame(admitted.captureId,
        () => captureRef.current === admitted.captureId && stateRef.current.phase === 'starting', 10 * 60_000)
      if (firstFrame.state === 'ready') {
        publish({ phase: 'recording', startedAt: Date.now() - firstFrame.sinceFirstFrameMs, message: recordingMessage })
      } else if (firstFrame.state === 'ended' && captureRef.current === admitted.captureId) {
        // The capture ended before any frame: Stop reports what happened.
        void stopRef.current()
      }
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
    const blocked = startGuardRef.current?.()
    if (blocked) { showSetup(blocked); return }
    if (draft === null) { showSetup('Choose a current source and valid recording setup first.'); return }
    // With no saved setup, F9 uses a freshly checked primary screen. There is
    // no prior countdown preference, and Start owns project creation/admission.
    if (draft === undefined && draftRef.current === undefined && !presetRef.current) { void start(); return }
    const requestedProjectIdentity = projectRef.current?.project_identity
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
        const projectChanged = requestedProjectIdentity
          ? !sameProjectIdentity(requestedProjectIdentity, projectRef.current?.project_identity)
          : Boolean(projectRef.current?.project_identity)
        if (projectChanged) {
          showSetup('The open project changed during the countdown. Review the recording setup and start again.')
          return
        }
        const blockedAtHandoff = startGuardRef.current?.()
        if (blockedAtHandoff) { showSetup(blockedAtHandoff); return }
        void start(draft)
      })
    }, 100)
  }, [countdownSeconds, publish, showSetup, start])

  useEffect(() => () => {
    countdownGuardRef.current.cancel()
    if (countdownRef.current !== null) window.clearInterval(countdownRef.current)
  }, [])

  // `endedMessage` replaces the whole text when the capture turns out to have
  // ended (an early Stop: nothing to recover, so no recovery hint).
  const checkCaptureStatus = useCallback(async (id: string, stopError = 'Stop did not produce a confirmed result.', endedMessage?: string) => {
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
        message: endedMessage ?? `${stopError} ${disposition.detail}` })
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

  const stop = useCallback(async (reason?: 'app_exit' | 'capture_ended') => {
    const id = captureRef.current
    const preset = activePresetRef.current
    if (reason === 'capture_ended' && adoptedRef.current === id) return
    if (!id || !preset || busyRef.current || (stateRef.current.phase === 'recovery' && stateRef.current.recoveryAction !== 'retry_stop')) return
    busyRef.current = true
    toggleGateRef.current.setInFlight(true)
    const closing = reason === 'app_exit'
    publish({ phase: 'finalizing', message: closing ? SAVING_BEFORE_CLOSE : 'Stopping and saving the raw MP4…' })
    let terminal = false
    try {
      // Stop can take a few seconds for a long take: show which part is being
      // saved while it runs (screen_record.status reports it).
      const plain = closing ? SAVING_BEFORE_CLOSE : 'Stopping and saving the raw MP4…'
      const progress = setInterval(() => {
        void callVerb('screen_record.status', { capture_id: id }).then((status) => {
          const saving = (status.result as { saving?: unknown } | undefined)?.saving
          if (saving && stateRef.current.phase === 'finalizing') publish({ message: savingProgressText(saving, plain) })
        }).catch(() => {})
      }, 1000)
      const reply = await callVerb('screen_record.stop', stopArgs(id, preset.raw, closing ? 'app_exit' : undefined))
        .then((started) => resolveRecordingStopJob(started, id,
          (jobId) => callVerb('jobs.status', { job_id: jobId })))
        .finally(() => clearInterval(progress))
      if (!reply.ok) {
        await checkCaptureStatus(id, `Stop did not complete: ${refusalText(reply.error)}`,
          isNothingRecorded(reply.error) ? nothingRecordedText(reply.error) : undefined)
        return
      }
      terminal = true
      const result = reply.result as { capture_id?: string; raw_path?: string | null; source?: string; plan?: string; raw_streams?: StudioRawStreams; input_hook?: unknown; cursor_correlation?: CursorCorrelation; cadence?: RecordingCadence; quality?: unknown; raw_has_mic?: boolean; raw_has_system?: boolean; notes?: unknown }
      const notes = result.notes
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
        publish({ phase: 'done', message: withSavedNotes('Raw MP4 saved to the recording project’s default export folder.', notes) })
        resultRef.current()
        return
      }
      if (stateRef.current.initialStudioWarning) {
        publish({ phase: 'error', message: withSavedNotes(`Raw MP4 saved. ${stateRef.current.initialStudioWarning}`, notes) })
        return
      }
      if (!result.source || !result.plan) {
        publish({ phase: 'error', message: withSavedNotes('Raw MP4 saved, but the editable plan was not returned. The clip was not added.', notes) })
        return
      }
      if (!sameProjectIdentity(projectIdentityRef.current, projectRef.current?.project_identity)) {
        publish({ phase: 'error', message: withSavedNotes('Raw MP4 saved, but the open project changed before polish. Reopen the recording project to continue.', notes) })
        return
      }
      const polishing = closing ? `${SAVING_BEFORE_CLOSE} Adding the editable clip…` : 'Raw MP4 saved. Polishing and adding the editable clip…'
      publish({ phase: 'finalizing', message: polishing })
      // The bake is a screen_record_polish job: show its progress while the
      // verb runs (a 4K take takes tens of seconds).
      const stopWatching = watchPolishProgress((percent) => publish({ message: `${polishing} ${percent}%` }))
      const polished = await callVerb('screen_record.polish', recordingPolishArgs(result.source, result.plan, preset))
        .finally(stopWatching)
      if (!polished.ok) { publish({ phase: 'error', message: withSavedNotes(`Raw MP4 saved; polish failed: ${polished.error?.message ?? 'unknown error'}`, notes) }); return }
      const placement = polished.result as { placed?: boolean; message?: string } | undefined
      // The recording's project was no longer open when the clip was ready:
      // nothing was added anywhere, and the reply says where it waits.
      if (placement?.placed === false) { publish({ phase: 'error', message: withSavedNotes(`Raw MP4 saved. ${placement.message ?? 'The editable clip was not added.'}`, notes) }); return }
      const clipId = (polished.result as { clip_id?: string })?.clip_id
      if (!clipId) { publish({ phase: 'error', message: withSavedNotes('Raw MP4 saved; polish returned no confirmed editable clip.', notes) }); return }
      publish({ phase: 'done', clipId, message: withSavedNotes('Raw MP4 saved and polished clip added to the current project.', notes) })
      resultRef.current()
    } catch (error) {
      if (terminal) publish({ phase: 'error', recoveryAction: null, message: `Could not complete Stop: ${error instanceof Error ? error.message : 'server unavailable'}. The capture ended; inspect the saved result.` })
      else await checkCaptureStatus(id, `Could not complete Stop: ${error instanceof Error ? error.message : 'server unavailable'}.`)
    } finally { busyRef.current = false; toggleGateRef.current.setInFlight(false) }
  }, [checkCaptureStatus, publish])

  const stopRef = useRef<(reason?: 'app_exit') => Promise<void>>(async () => {})
  stopRef.current = stop

  useRecordingCloseRequest({
    isRecording: () => Boolean(captureRef.current),
    stopForClose: () => stopRef.current('app_exit'),
    announce: (message) => publish({ message }),
  })

  // A recording adopted from outside is stopped by whoever started it unless
  // the person presses Stop here; when it ends elsewhere, just let it go.
  const adoptedRef = useRef<string | null>(null)
  const releaseAdopted = useCallback((id: string) => {
    if (captureRef.current !== id) return
    captureRef.current = null
    activePresetRef.current = null
    adoptedRef.current = null
    void indicator('end_recording_indicator', id)
    publish({ ...INITIAL, message: 'The recording started outside this window has ended.' })
  }, [publish])

  const adopt = useCallback((recording: ActiveRecording) => {
    if (busyRef.current || captureRef.current) return
    adoptedRef.current = recording.captureId
    captureRef.current = recording.captureId
    // Only `raw` is read for an adopted capture: Stop saves the unedited MP4.
    activePresetRef.current = { raw: true } as RecordingPreset
    projectIdentityRef.current = projectRef.current?.project_identity ? { ...projectRef.current.project_identity } : undefined
    void indicator('begin_recording_indicator', recording.captureId).then((warning) => publish({ indicatorWarning: warning }))
    publish({ ...INITIAL, raw: true, captureId: recording.captureId, projectName: projectRef.current?.name ?? null,
      phase: recording.ready ? 'recording' : 'starting',
      startedAt: recording.ready ? Date.now() - (recording.sinceFirstFrameMs ?? 0) : null,
      message: recording.ready ? ADOPTED_MESSAGE : STARTING_MESSAGE })
    if (!recording.ready) {
      void waitForFirstFrame(recording.captureId,
        () => captureRef.current === recording.captureId && stateRef.current.phase === 'starting', 10 * 60_000)
        .then((firstFrame) => {
          if (firstFrame.state === 'ready') publish({ phase: 'recording', startedAt: Date.now() - firstFrame.sinceFirstFrameMs, message: ADOPTED_MESSAGE })
          else if (firstFrame.state === 'ended') releaseAdopted(recording.captureId)
        })
    }
  }, [publish, releaseAdopted])
  useActiveRecordingAdoption({
    idle: () => !busyRef.current && !captureRef.current && !unknownStartRef.current
      && countdownRef.current === null && stateRef.current.phase !== 'recovery' && stateRef.current.phase !== 'finalizing'
      && stateRef.current.phase !== 'starting',
    adopt,
  })

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
          // A recording started outside this window ends without a Stop here.
          if (result.terminal === true && adoptedRef.current === id) { releaseAdopted(id); return }
          if (result.capture_id === id) {
            publish({ inputHook: recordingInputHook(result.input_hook) })
            if (result.terminal === true) void stop()
          } else if (result.capture_id !== id) publish({ phase: 'recovery', message: 'Recorder status returned a different capture ID. Inspect Recording Recovery.' })
        } else if (reply.error?.code === 'not_found' && adoptedRef.current === id) releaseAdopted(id)
        else if (reply.error?.code === 'not_found') await checkCaptureStatus(id, 'The native capture ended before Stop returned an output.')
      } finally { checking = false }
    }
    const timer = window.setInterval(() => { void check() }, 1000)
    return () => window.clearInterval(timer)
  }, [checkCaptureStatus, publish, releaseAdopted, state.captureId, state.phase, stop])

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
