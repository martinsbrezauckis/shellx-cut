import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import type {
  ScreenRecordSourcePreviewCapability,
  ScreenRecordSourcePreviewFrame,
  ScreenRecordSourcePreviewStatus,
} from '../../lib/clientResults'
import type { MonitorInfo, WindowInfo } from './RecordingSourceControl'
import type { RecordingSourceKind } from './regionPickerModel'
import {
  recordingSourcePreviewCapability,
  recordingSourcePreviewPresentation,
  recordingSourcePreviewSelectionKey,
  recordingSourcePreviewTarget,
  type RecordingSourcePreviewPresentation,
} from './recordingNativeSourcePreview'

const SOURCE_PREVIEW_POLL_MS = 250
const MAX_SOURCE_PREVIEW_READ_FAILURES = 3
const INITIAL_STATUS: ScreenRecordSourcePreviewStatus = {
  state: 'idle',
  recursion: 'none',
  has_frame: false,
  generation: null,
}

interface SourcePreviewInput {
  sourceKind: RecordingSourceKind
  monitors: readonly MonitorInfo[]
  monitorIdx: number | null
  windows: readonly WindowInfo[]
  windowTargetId: string | null
  recording: boolean
}

interface SourcePreviewPollSession {
  epoch: number
  sourceKey: string
}

const LIFECYCLE_VERBS = {
  pause: 'screen_record.preview_pause',
  resume: 'screen_record.preview_resume',
  hide: 'screen_record.preview_hide',
  stop: 'screen_record.preview_stop',
} as const

type LifecycleAction = keyof typeof LIFECYCLE_VERBS

function isPollingState(status: ScreenRecordSourcePreviewStatus): boolean {
  return status.state === 'starting' || status.state === 'ready'
}

function retainsSelectedSource(status: ScreenRecordSourcePreviewStatus): boolean {
  return status.state === 'starting' || status.state === 'ready' || status.state === 'paused'
}

function previewError(error: unknown, fallback: string): string {
  if (error instanceof TypeError) return `${fallback}: recorder unreachable.`
  if (error && typeof error === 'object' && typeof (error as { message?: unknown }).message === 'string') {
    return `${fallback}: ${(error as { message: string }).message}`
  }
  return fallback
}

/**
 * Owns one process-local native preview lease. Polling is bounded and late
 * responses are rejected by both command and source/session generations.
 */
export function useRecordingSourcePreview({
  sourceKind,
  monitors,
  monitorIdx,
  windows,
  windowTargetId,
  recording,
}: SourcePreviewInput) {
  const [capability, setCapability] = useState<ScreenRecordSourcePreviewCapability | null>(null)
  const [status, setStatus] = useState<ScreenRecordSourcePreviewStatus>(INITIAL_STATUS)
  const [frame, setFrame] = useState<ScreenRecordSourcePreviewFrame | null>(null)
  const [statusError, setStatusError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [pollSession, setPollSession] = useState<SourcePreviewPollSession | null>(null)
  const mountedRef = useRef(false)
  const commandEpochRef = useRef(0)
  const sessionEpochRef = useRef(0)
  const activeSourceKeyRef = useRef<string | null>(null)
  const startingSourceKeyRef = useRef<string | null>(null)
  const recordingRef = useRef(recording)
  recordingRef.current = recording

  const target = useMemo(
    () => recordingSourcePreviewTarget(capability, sourceKind, monitors, monitorIdx, windows, windowTargetId),
    [capability, monitorIdx, monitors, sourceKind, windowTargetId, windows],
  )
  const targetKey = target ? recordingSourcePreviewSelectionKey(target.source) : null
  const targetKeyRef = useRef<string | null>(targetKey)
  targetKeyRef.current = targetKey
  // A new selector value or record admission hides an old frame in the render
  // before its release effect runs, so a prior source cannot flash as current.
  const visibleFrame = !recording && activeSourceKeyRef.current === targetKey ? frame : null
  const presentation: RecordingSourcePreviewPresentation = recordingSourcePreviewPresentation(capability, status, visibleFrame)

  const invalidatePoll = useCallback(() => {
    sessionEpochRef.current += 1
    setPollSession(null)
  }, [])

  const forgetReleasedSource = useCallback((nextStatus: ScreenRecordSourcePreviewStatus) => {
    if (!retainsSelectedSource(nextStatus)) activeSourceKeyRef.current = null
  }, [])

  useEffect(() => {
    mountedRef.current = true
    let stale = false
    void callVerb('screen_record.preview_capability', {}).then((response) => {
      if (stale || !mountedRef.current) return
      if (!response.ok || !response.result) {
        setCapability(recordingSourcePreviewCapability(undefined))
        setStatusError(previewError(response.error, 'Native source-preview availability could not be read'))
        return
      }
      setCapability(recordingSourcePreviewCapability(response.result))
    }).catch((error) => {
      if (!stale && mountedRef.current) {
        setCapability(recordingSourcePreviewCapability(undefined))
        setStatusError(previewError(error, 'Native source-preview availability could not be read'))
      }
    })
    return () => {
      stale = true
      mountedRef.current = false
      commandEpochRef.current += 1
      sessionEpochRef.current += 1
      const hadPreview = activeSourceKeyRef.current !== null || startingSourceKeyRef.current !== null
      activeSourceKeyRef.current = null
      startingSourceKeyRef.current = null
      if (hadPreview) void callVerb('screen_record.preview_stop', {})
    }
  }, [invalidatePoll])

  useEffect(() => {
    if (!pollSession) return
    let stale = false
    let inFlight = false
    let readFailures = 0
    let timer: number | null = null
    const current = () => !stale
      && mountedRef.current
      && sessionEpochRef.current === pollSession.epoch
      && activeSourceKeyRef.current === pollSession.sourceKey
    const stopPolling = () => {
      stale = true
      if (timer !== null) window.clearInterval(timer)
    }
    const abandonUnreadablePreview = () => {
      activeSourceKeyRef.current = null
      startingSourceKeyRef.current = null
      invalidatePoll()
      stopPolling()
      // After repeated read failures this client cannot continue to claim an
      // observed native lease, so it asks the process owner to release it.
      void callVerb('screen_record.preview_stop', {})
    }
    const poll = async () => {
      if (!current() || inFlight) return
      inFlight = true
      try {
        const response = await callVerb('screen_record.preview_frame', {})
        if (!current()) return
        if (!response.ok || !response.result) {
          readFailures += 1
          setFrame(null)
          setStatusError(previewError(response.error, 'Native source-preview status could not be read'))
          if (readFailures >= MAX_SOURCE_PREVIEW_READ_FAILURES) {
            abandonUnreadablePreview()
          }
          return
        }
        readFailures = 0
        const snapshot = response.result
        setStatus(snapshot.status)
        setFrame(snapshot.frame)
        setStatusError(null)
        if (!isPollingState(snapshot.status)) {
          forgetReleasedSource(snapshot.status)
          invalidatePoll()
          stopPolling()
        }
      } catch (error) {
        if (current()) {
          readFailures += 1
          setFrame(null)
          setStatusError(previewError(error, 'Native source-preview status could not be read'))
          if (readFailures >= MAX_SOURCE_PREVIEW_READ_FAILURES) {
            abandonUnreadablePreview()
          }
        }
      } finally {
        inFlight = false
      }
    }
    void poll()
    timer = window.setInterval(() => { void poll() }, SOURCE_PREVIEW_POLL_MS)
    return stopPolling
  }, [forgetReleasedSource, invalidatePoll, pollSession])

  const runLifecycleAction = useCallback(async (action: LifecycleAction) => {
    const command = ++commandEpochRef.current
    invalidatePoll()
    setBusy(true)
    setFrame(null)
    setStatusError(null)
    try {
      const response = await callVerb(LIFECYCLE_VERBS[action], {})
      if (!mountedRef.current || command !== commandEpochRef.current) return
      if (!response.ok || !response.result) {
        setStatusError(previewError(response.error, `Could not ${action} native source preview`))
        return
      }
      const nextStatus = response.result.status
      setStatus(nextStatus)
      forgetReleasedSource(nextStatus)
      if (action === 'resume' && isPollingState(nextStatus)) {
        const sourceKey = activeSourceKeyRef.current
        if (sourceKey) {
          const epoch = ++sessionEpochRef.current
          setPollSession({ epoch, sourceKey })
        }
      }
    } catch (error) {
      if (mountedRef.current && command === commandEpochRef.current) {
        setStatusError(previewError(error, `Could not ${action} native source preview`))
      }
    } finally {
      if (mountedRef.current && command === commandEpochRef.current) setBusy(false)
    }
  }, [forgetReleasedSource, invalidatePoll])

  const stop = useCallback(() => {
    if (activeSourceKeyRef.current === null && startingSourceKeyRef.current === null) return
    return runLifecycleAction('stop')
  }, [runLifecycleAction])

  const hide = useCallback(() => {
    if (activeSourceKeyRef.current === null && startingSourceKeyRef.current === null) return
    return runLifecycleAction('hide')
  }, [runLifecycleAction])

  const start = useCallback(async () => {
    if (recordingRef.current) {
      setStatusError('Native source preview cannot start while this recording owns capture devices.')
      return
    }
    if (!capability || capability.state !== 'available' || !target || !targetKey) {
      setStatusError('Choose a current source after native source-preview support is confirmed.')
      return
    }
    const command = ++commandEpochRef.current
    invalidatePoll()
    startingSourceKeyRef.current = targetKey
    setBusy(true)
    setFrame(null)
    setStatusError(null)
    try {
      const response = await callVerb('screen_record.preview_start', { source: target.source })
      const staleStart = !mountedRef.current
        || command !== commandEpochRef.current
        || targetKeyRef.current !== targetKey
        || recordingRef.current
      if (staleStart) {
        void callVerb('screen_record.preview_stop', {})
        return
      }
      if (!response.ok || !response.result) {
        startingSourceKeyRef.current = null
        setStatusError(previewError(response.error, 'Could not start native source preview'))
        return
      }
      const nextStatus = response.result.status
      startingSourceKeyRef.current = null
      activeSourceKeyRef.current = targetKey
      setStatus(nextStatus)
      if (isPollingState(nextStatus)) {
        const epoch = ++sessionEpochRef.current
        setPollSession({ epoch, sourceKey: targetKey })
      }
    } catch (error) {
      if (mountedRef.current && command === commandEpochRef.current) {
        startingSourceKeyRef.current = null
        setStatusError(previewError(error, 'Could not start native source preview'))
      }
    } finally {
      if (mountedRef.current && command === commandEpochRef.current) setBusy(false)
    }
  }, [capability, invalidatePoll, target, targetKey])

  useEffect(() => {
    const owned = activeSourceKeyRef.current ?? startingSourceKeyRef.current
    if (owned !== null && owned !== targetKey) void stop()
  }, [stop, targetKey])

  useEffect(() => {
    if (recording && (activeSourceKeyRef.current !== null || startingSourceKeyRef.current !== null)) void stop()
  }, [recording, stop])

  useEffect(() => {
    const onVisibilityChange = () => {
      if (document.visibilityState === 'hidden') void hide()
    }
    document.addEventListener('visibilitychange', onVisibilityChange)
    return () => document.removeEventListener('visibilitychange', onVisibilityChange)
  }, [hide])

  return {
    capability,
    status,
    frame,
    presentation,
    target,
    statusError,
    busy,
    start,
    pause: () => runLifecycleAction('pause'),
    resume: () => runLifecycleAction('resume'),
    hide,
    stop,
  }
}
