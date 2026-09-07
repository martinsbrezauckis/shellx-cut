import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import {
  NO_RECORDING_SCENE_CAPABILITY,
  recordingSceneAcknowledged,
  recordingSceneById,
  recordingSceneCapability,
  recordingSceneTimerAcknowledged,
  recordingSceneStartConfig,
  RecordingSceneControlLifetime,
  type RecordingSceneTimer,
  type RecordingSceneCapability,
  type RecordingSceneTimerAction,
} from './recordingScenes'

export type RecordingSceneSaveState = 'draft' | 'ready' | 'switching' | 'saved' | 'active_unsaved' | 'error' | 'unavailable'

export interface RecordingSceneStatus {
  state: RecordingSceneSaveState
  message: string
}

export type RecordingSceneTimerState = 'not_started' | 'awaiting_ack' | 'running' | 'paused' | 'ended' | 'switching' | 'error' | 'unavailable'

export interface RecordingSceneTimerStatus {
  state: RecordingSceneTimerState
  message: string
}

export interface RecordingRecoveryRow {
  capture_id: string
  state: 'complete' | 'recovered' | 'quarantined' | 'interrupted' | 'owner_ambiguous' | 'torn_journal' | 'corrupt'
  receipt?: { state?: string; recovered_segments?: number; lost_tail_ms?: number | null }
}

export interface RecordingRecoveryState {
  state: 'unavailable' | 'checking' | 'clear' | 'attention' | 'error'
  message: string
}

function recoveryMessage(captures: RecordingRecoveryRow[]): RecordingRecoveryState {
  const attention = captures.find((capture) => capture.state !== 'complete')
  if (!attention) return { state: 'clear', message: 'No interrupted recordings are reported for this project.' }
  if (attention.state === 'recovered') {
    const segments = attention.receipt?.recovered_segments
    return {
      state: 'attention',
      message: `A prior recording was recovered${typeof segments === 'number' ? ` (${segments} saved segment${segments === 1 ? '' : 's'})` : ''}. Review it before relying on the final output.`,
    }
  }
  return {
    state: 'attention',
    message: `A prior recording needs attention (${attention.state.replaceAll('_', ' ')}). No repair has been run here.`,
  }
}

function timerMessageAfter(action: RecordingSceneTimerAction, state: RecordingSceneTimerState): string {
  const actionName = action === 'end' ? 'ended' : action === 'pause' ? 'paused' : action === 'resume' ? 'resumed' : action === 'reset' ? 'reset' : 'restarted'
  if (state === 'paused') return `Timer ${actionName}; the recorder acknowledged the change.`
  if (state === 'ended') return `Timer ${actionName}; recording continues until you stop it.`
  return `Timer ${actionName}; the recorder acknowledged the change.`
}

/**
 * Keeps scene and timer mutations fail-closed. Idle selections are drafts;
 * live controls only change after their dedicated recorder verb acknowledges.
 */
export function useRecordingScenes({
  projectOpen,
  recording,
  captureId,
  onPreviewScene,
}: {
  projectOpen: boolean
  recording: boolean
  captureId: string | null
  onPreviewScene: (sceneId: string) => void
}) {
  const [capability, setCapability] = useState<RecordingSceneCapability>(NO_RECORDING_SCENE_CAPABILITY)
  const [selectedSceneId, setSelectedSceneId] = useState('screen')
  const [timer, setTimer] = useState<RecordingSceneTimer>({ kind: 'elapsed' })
  const [status, setStatus] = useState<RecordingSceneStatus>({
    state: 'draft',
    message: 'Choose a scene for the next recording. It is not saved yet.',
  })
  const [timerStatus, setTimerStatus] = useState<RecordingSceneTimerStatus>({
    state: 'not_started',
    message: 'Choose a timer for the next recording.',
  })
  const [recovery, setRecovery] = useState<RecordingRecoveryState>({
    state: 'unavailable',
    message: 'Open a project to check recording recovery.',
  })
  const controlLifetimeRef = useRef(new RecordingSceneControlLifetime())

  useLayoutEffect(() => {
    const lifetime = controlLifetimeRef.current
    lifetime.mount()
    return () => lifetime.unmount()
  }, [])

  useLayoutEffect(() => {
    if (recording && captureId) controlLifetimeRef.current.replaceCapture(captureId)
    else controlLifetimeRef.current.clearCapture()
  }, [captureId, recording])

  const selectedScene = recordingSceneById(selectedSceneId)
  const startConfig = useMemo(
    () => capability.supported && capability.catalog_revision
      ? recordingSceneStartConfig(capability.catalog_revision, selectedSceneId, timer)
      : undefined,
    [capability.catalog_revision, capability.supported, selectedSceneId, timer],
  )

  const setDoctorCapability = useCallback((value: unknown) => {
    const next = recordingSceneCapability(value)
    setCapability(next)
    setStatus((current) => {
      if (current.state === 'switching' || current.state === 'saved' || current.state === 'active_unsaved' || current.state === 'error') return current
      return next.supported
        ? { state: 'ready', message: 'Selected scene will be sent with the next recording.' }
        : { state: 'draft', message: `${next.detail} Your selection is a local preview only.` }
    })
    if (recording && !next.supported) {
      controlLifetimeRef.current.clearCapture()
      setTimerStatus({ state: 'unavailable', message: 'Live timer controls are unavailable because this recorder has not advertised the scene API.' })
    }
  }, [recording])

  const refreshRecovery = useCallback(async () => {
    if (!projectOpen) {
      setRecovery({ state: 'unavailable', message: 'Open a project to check recording recovery.' })
      return
    }
    setRecovery({ state: 'checking', message: 'Checking recording recovery…' })
    try {
      const response = await callVerb('screen_record.recovery_status', { limit: 6 })
      if (!response.ok) {
        setRecovery({ state: 'error', message: `Recovery check did not complete: ${response.error?.message ?? 'unknown error'}` })
        return
      }
      const result = response.result as { captures?: RecordingRecoveryRow[] } | undefined
      setRecovery(recoveryMessage(result?.captures ?? []))
    } catch {
      setRecovery({ state: 'error', message: 'Recovery check did not complete. Check again before relying on it.' })
    }
  }, [projectOpen])

  useEffect(() => { void refreshRecovery() }, [refreshRecovery])

  const selectScene = useCallback(async (sceneId: string) => {
    const scene = recordingSceneById(sceneId)
    if (!recording) {
      setSelectedSceneId(scene.id)
      onPreviewScene(scene.id)
      setStatus(capability.supported
        ? { state: 'ready', message: 'Selected scene will be sent with the next recording.' }
        : { state: 'draft', message: `${capability.detail} Your selection is a local preview only.` })
      return
    }
    if (!captureId || !capability.supported || !capability.catalog_revision) {
      setStatus({ state: 'unavailable', message: capability.detail })
      return
    }
    const controlLease = controlLifetimeRef.current.begin(captureId, 'scene')
    if (!controlLease) return
    setStatus({ state: 'switching', message: `Switching to ${scene.name}…` })
    try {
      const response = await callVerb('screen_record.scene_activate', {
        capture_id: captureId,
        scene_id: scene.id,
        preset_revision: scene.preset_revision,
      })
      if (!controlLifetimeRef.current.isCurrent(controlLease)) return
      if (!response.ok) {
        setStatus({ state: 'error', message: `Scene switch was not saved: ${response.error?.message ?? 'unknown error'}` })
        return
      }
      if (!recordingSceneAcknowledged(response.result, scene)) {
        setStatus({ state: 'error', message: 'Scene switch was not acknowledged as saved. The preview was not changed.' })
        return
      }
      setSelectedSceneId(scene.id)
      onPreviewScene(scene.id)
      setStatus({ state: 'saved', message: `${scene.name} is live and saved with this recording.` })
    } catch {
      if (!controlLifetimeRef.current.isCurrent(controlLease)) return
      setStatus({ state: 'error', message: 'Scene switch did not reach the recorder. The preview was not changed.' })
    }
  }, [capability, captureId, onPreviewScene, recording])

  const markCaptureStarted = useCallback((startAcknowledgement: unknown) => {
    if (!capability.supported) {
      setTimerStatus({ state: 'unavailable', message: 'This recording started without the scene API, so live timer controls are unavailable.' })
      return
    }
    if (!recordingSceneAcknowledged(startAcknowledgement, selectedScene)) {
      setStatus({
        state: 'error',
        message: 'The recording started, but its selected scene was not acknowledged as saved. The preview is local only.',
      })
      setTimerStatus({ state: 'error', message: 'Timer configuration was not acknowledged by Start.' })
      return
    }
    setStatus({
      state: 'saved',
      message: `${selectedScene.name} is configured and saved for this recording.`,
    })
    setTimerStatus({
      state: 'awaiting_ack',
      message: 'Timer configuration was sent with start. Use a live timer control to confirm a later change.',
    })
  }, [capability.supported, selectedScene.name])

  const selectTimer = useCallback((nextTimer: RecordingSceneTimer) => {
    if (recording) return
    setTimer(nextTimer)
    setTimerStatus({ state: 'not_started', message: 'Timer selection will be sent with the next recording.' })
    setStatus(capability.supported
      ? { state: 'ready', message: 'The selected timer will be sent with the next recording.' }
      : { state: 'draft', message: `${capability.detail} The timer is a local preview only.` })
  }, [capability, recording])

  const controlTimer = useCallback(async (action: RecordingSceneTimerAction) => {
    if (!recording || !captureId || !capability.supported) {
      setTimerStatus({ state: 'unavailable', message: 'Live timer controls are unavailable for this recording.' })
      return
    }
    const controlLease = controlLifetimeRef.current.begin(captureId, 'timer')
    if (!controlLease) return
    setTimerStatus({ state: 'switching', message: `Asking the recorder to ${action} the timer…` })
    try {
      const response = await callVerb('screen_record.scene_timer', { capture_id: captureId, action })
      if (!controlLifetimeRef.current.isCurrent(controlLease)) return
      if (!response.ok) {
        setTimerStatus({ state: 'error', message: `Timer change was not confirmed: ${response.error?.message ?? 'unknown error'}` })
        return
      }
      const timerResult = response.result
      if (!recordingSceneTimerAcknowledged(timerResult, action)) {
        setTimerStatus({ state: 'error', message: 'Timer action was not acknowledged with an exact state. Its state was not changed here.' })
        return
      }
      const nextState = timerResult.state
      setTimerStatus({ state: nextState, message: timerMessageAfter(action, nextState) })
    } catch {
      if (!controlLifetimeRef.current.isCurrent(controlLease)) return
      setTimerStatus({ state: 'error', message: 'Timer change did not reach the recorder. Its state was not changed here.' })
    }
  }, [capability.supported, captureId, recording])

  return {
    capability,
    selectedScene,
    startConfig,
    status,
    timer,
    timerStatus,
    selectTimer,
    controlTimer,
    recovery,
    setDoctorCapability,
    refreshRecovery,
    selectScene,
    markCaptureStarted,
  }
}
