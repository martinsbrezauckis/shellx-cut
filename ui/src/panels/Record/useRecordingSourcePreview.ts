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
import {
  recordingSourcePreviewControlArgs,
  recordingSourcePreviewGeneration,
  recordingSourcePreviewLease,
  recordingSourcePreviewLeaseNonce,
} from './recordingSourcePreviewLease'
import {
  INITIAL_SOURCE_PREVIEW_STATUS,
  isSourcePreviewPolling,
  reconcileFailedSourcePreviewStart,
  sourcePreviewError,
  useRecordingSourcePreviewLifecycleAction,
} from './recordingSourcePreviewLifecycle'
import { useRecordingSourcePreviewPoller } from './useRecordingSourcePreviewPoller'

interface SourcePreviewInput {
  sourceKind: RecordingSourceKind
  monitors: readonly MonitorInfo[]
  monitorIdx: number | null
  windows: readonly WindowInfo[]
  windowTargetId: string | null
  recording: boolean
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
  const [status, setStatus] = useState<ScreenRecordSourcePreviewStatus>(INITIAL_SOURCE_PREVIEW_STATUS)
  const [frame, setFrame] = useState<ScreenRecordSourcePreviewFrame | null>(null)
  const [statusError, setStatusError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const mountedRef = useRef(false)
  const commandEpochRef = useRef(0)
  const activeSourceKeyRef = useRef<string | null>(null)
  const activeGenerationRef = useRef<number | null>(null)
  const activeLeaseNonceRef = useRef<string | null>(null)
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
  const ownershipRefs = useMemo(() => ({
    mounted: mountedRef,
    commandEpoch: commandEpochRef,
    activeSourceKey: activeSourceKeyRef,
    activeGeneration: activeGenerationRef,
    activeLeaseNonce: activeLeaseNonceRef,
    startingSourceKey: startingSourceKeyRef,
  }), [])
  // A new selector value or record admission hides an old frame in the render
  // before its release effect runs, so a prior source cannot flash as current.
  const visibleFrame = !recording && activeSourceKeyRef.current === targetKey ? frame : null
  const presentation: RecordingSourcePreviewPresentation = recordingSourcePreviewPresentation(capability, status, visibleFrame)

  const forgetReleasedSource = useCallback((nextStatus: ScreenRecordSourcePreviewStatus) => {
    const lease = recordingSourcePreviewLease(activeSourceKeyRef.current, nextStatus)
    activeSourceKeyRef.current = lease?.sourceKey ?? null
    activeGenerationRef.current = lease?.generation ?? null
    activeLeaseNonceRef.current = lease?.leaseNonce ?? null
  }, [])

  const { cancelPolling, invalidatePoll, startPolling } = useRecordingSourcePreviewPoller({
    refs: ownershipRefs,
    forgetReleasedSource,
    setFrame,
    setStatus,
    setStatusError,
  })
  const runLifecycleAction = useRecordingSourcePreviewLifecycleAction({
    refs: ownershipRefs,
    forgetReleasedSource,
    invalidatePoll,
    startPolling,
    setBusy,
    setFrame,
    setStatus,
    setStatusError,
  })

  useEffect(() => {
    mountedRef.current = true
    let stale = false
    void callVerb('screen_record.preview_capability', {}).then((response) => {
      if (stale || !mountedRef.current) return
      if (!response.ok || !response.result) {
        setCapability(recordingSourcePreviewCapability(undefined))
        setStatusError(sourcePreviewError(response.error, 'Native source-preview availability could not be read'))
        return
      }
      setCapability(recordingSourcePreviewCapability(response.result))
    }).catch((error) => {
      if (!stale && mountedRef.current) {
        setCapability(recordingSourcePreviewCapability(undefined))
        setStatusError(sourcePreviewError(error, 'Native source-preview availability could not be read'))
      }
    })
    return () => {
      stale = true
      mountedRef.current = false
      commandEpochRef.current += 1
      cancelPolling()
      const hadPreview = activeSourceKeyRef.current !== null || startingSourceKeyRef.current !== null
      const activeGeneration = activeGenerationRef.current
      const activeLeaseNonce = activeLeaseNonceRef.current
      activeSourceKeyRef.current = null
      activeGenerationRef.current = null
      activeLeaseNonceRef.current = null
      startingSourceKeyRef.current = null
      const controlArgs = recordingSourcePreviewControlArgs(activeGeneration, activeLeaseNonce)
      if (hadPreview && controlArgs) {
        void callVerb('screen_record.preview_stop', controlArgs)
      }
    }
  }, [cancelPolling])

  const stop = useCallback(() => {
    if (activeSourceKeyRef.current === null && startingSourceKeyRef.current === null) return
    if (activeGenerationRef.current === null || activeLeaseNonceRef.current === null) {
      // An in-flight start has no issued generation yet. Invalidate its local
      // ownership; its successful stale response will release only the
      // generation it returns, never a later preview.
      commandEpochRef.current += 1
      activeSourceKeyRef.current = null
      activeGenerationRef.current = null
      activeLeaseNonceRef.current = null
      startingSourceKeyRef.current = null
      invalidatePoll()
      setFrame(null)
      setBusy(false)
      return
    }
    return runLifecycleAction('stop')
  }, [invalidatePoll, runLifecycleAction])

  const hide = useCallback(() => {
    if (activeSourceKeyRef.current === null && startingSourceKeyRef.current === null) return
    return runLifecycleAction('hide')
  }, [runLifecycleAction])

  const reconcileFailedStart = useCallback(
    (command: number) => reconcileFailedSourcePreviewStart({
      refs: ownershipRefs,
      invalidatePoll,
      setFrame,
      setStatus,
    }, command),
    [invalidatePoll, ownershipRefs],
  )

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
        if (startingSourceKeyRef.current === targetKey) startingSourceKeyRef.current = null
        const staleGeneration = response.ok && response.result
          ? recordingSourcePreviewGeneration(response.result.status)
          : null
        const staleLeaseNonce = response.ok && response.result
          ? recordingSourcePreviewLeaseNonce(response.result.status)
          : null
        const controlArgs = recordingSourcePreviewControlArgs(staleGeneration, staleLeaseNonce)
        if (controlArgs) {
          void callVerb('screen_record.preview_stop', controlArgs)
        }
        return
      }
      if (!response.ok || !response.result) {
        startingSourceKeyRef.current = null
        setStatusError(sourcePreviewError(response.error, 'Could not start native source preview'))
        await reconcileFailedStart(command)
        return
      }
      const nextStatus = response.result.status
      const lease = recordingSourcePreviewLease(targetKey, nextStatus)
      startingSourceKeyRef.current = null
      if (!isSourcePreviewPolling(nextStatus) || lease === null) {
        activeSourceKeyRef.current = null
        activeGenerationRef.current = null
        activeLeaseNonceRef.current = null
        setStatus(nextStatus)
        setStatusError('Native source preview did not return an active lease.')
        return
      }
      activeSourceKeyRef.current = lease.sourceKey
      activeGenerationRef.current = lease.generation
      activeLeaseNonceRef.current = lease.leaseNonce
      setStatus(nextStatus)
      if (isSourcePreviewPolling(nextStatus)) startPolling(targetKey)
    } catch (error) {
      if (mountedRef.current && command === commandEpochRef.current) {
        startingSourceKeyRef.current = null
        setStatusError(sourcePreviewError(error, 'Could not start native source preview'))
        await reconcileFailedStart(command)
      }
    } finally {
      if (mountedRef.current && command === commandEpochRef.current) setBusy(false)
    }
  }, [capability, invalidatePoll, reconcileFailedStart, startPolling, target, targetKey])

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
