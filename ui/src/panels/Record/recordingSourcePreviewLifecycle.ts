import { useCallback, type Dispatch, type MutableRefObject, type SetStateAction } from 'react'
import { callVerb } from '../../lib/client'
import type { ScreenRecordSourcePreviewFrame, ScreenRecordSourcePreviewStatus } from '../../lib/clientResults'
import {
  recordingSourcePreviewControlArgs,
  recordingSourcePreviewFailedStartStatus,
  recordingSourcePreviewGeneration,
  recordingSourcePreviewLeaseNonce,
  recordingSourcePreviewRetainsLease,
} from './recordingSourcePreviewLease'

export const INITIAL_SOURCE_PREVIEW_STATUS: ScreenRecordSourcePreviewStatus = {
  state: 'idle',
  recursion: 'none',
  has_frame: false,
  generation: null,
}

export const SOURCE_PREVIEW_LIFECYCLE_VERBS = {
  pause: 'screen_record.preview_pause',
  resume: 'screen_record.preview_resume',
  hide: 'screen_record.preview_hide',
  stop: 'screen_record.preview_stop',
} as const

export type SourcePreviewLifecycleAction = keyof typeof SOURCE_PREVIEW_LIFECYCLE_VERBS

export interface SourcePreviewOwnershipRefs {
  mounted: MutableRefObject<boolean>
  commandEpoch: MutableRefObject<number>
  activeSourceKey: MutableRefObject<string | null>
  activeGeneration: MutableRefObject<number | null>
  activeLeaseNonce: MutableRefObject<string | null>
  startingSourceKey: MutableRefObject<string | null>
}

interface PreviewLifecycleSupport {
  refs: SourcePreviewOwnershipRefs
  invalidatePoll: () => void
  startPolling: (sourceKey: string) => void
  forgetReleasedSource: (status: ScreenRecordSourcePreviewStatus) => void
  setBusy: Dispatch<SetStateAction<boolean>>
  setFrame: Dispatch<SetStateAction<ScreenRecordSourcePreviewFrame | null>>
  setStatus: Dispatch<SetStateAction<ScreenRecordSourcePreviewStatus>>
  setStatusError: Dispatch<SetStateAction<string | null>>
}

export function isSourcePreviewPolling(status: ScreenRecordSourcePreviewStatus): boolean {
  return status.state === 'starting' || status.state === 'ready'
}

export function sourcePreviewError(error: unknown, fallback: string): string {
  if (error instanceof TypeError) return `${fallback}: recorder unreachable.`
  if (error && typeof error === 'object' && typeof (error as { message?: unknown }).message === 'string') {
    return `${fallback}: ${(error as { message: string }).message}`
  }
  return fallback
}

function clearActiveOwnership(refs: SourcePreviewOwnershipRefs) {
  refs.activeSourceKey.current = null
  refs.activeGeneration.current = null
  refs.activeLeaseNonce.current = null
}

/** Reconcile a failed start without adopting a lease this panel did not issue. */
export async function reconcileFailedSourcePreviewStart(
  support: Pick<PreviewLifecycleSupport, 'refs' | 'invalidatePoll' | 'setFrame' | 'setStatus'>,
  command: number,
) {
  const { refs, invalidatePoll, setFrame, setStatus } = support
  try {
    const response = await callVerb('screen_record.preview_status', {})
    if (!refs.mounted.current || command !== refs.commandEpoch.current) return
    if (!response.ok || !response.result) {
      clearActiveOwnership(refs)
      refs.startingSourceKey.current = null
      invalidatePoll()
      setFrame(null)
      setStatus(INITIAL_SOURCE_PREVIEW_STATUS)
      return
    }
    setStatus(recordingSourcePreviewFailedStartStatus(response.result))
    setFrame(null)
    clearActiveOwnership(refs)
    refs.startingSourceKey.current = null
    invalidatePoll()
  } catch {
    if (refs.mounted.current && command === refs.commandEpoch.current) {
      clearActiveOwnership(refs)
      refs.startingSourceKey.current = null
      invalidatePoll()
      setFrame(null)
      setStatus(INITIAL_SOURCE_PREVIEW_STATUS)
    }
  }
}

/** Sends generation-and-nonce-bound lifecycle controls and reconciles rejects. */
export function useRecordingSourcePreviewLifecycleAction(support: PreviewLifecycleSupport) {
  const {
    refs,
    forgetReleasedSource,
    invalidatePoll,
    setBusy,
    setFrame,
    setStatus,
    setStatusError,
    startPolling,
  } = support

  const reconcileFailedLifecycleAction = useCallback(async (
    command: number,
    expectedGeneration: number,
    expectedLeaseNonce: string,
    sourceKey: string | null,
  ) => {
    try {
      const response = await callVerb('screen_record.preview_status', {})
      if (!refs.mounted.current || command !== refs.commandEpoch.current) return
      if (!response.ok || !response.result) {
        clearActiveOwnership(refs)
        invalidatePoll()
        setFrame(null)
        setStatus(INITIAL_SOURCE_PREVIEW_STATUS)
        return
      }
      const nextStatus = response.result
      const generation = recordingSourcePreviewGeneration(nextStatus)
      setFrame(null)
      if (!recordingSourcePreviewRetainsLease(nextStatus) || generation === null) {
        setStatus(nextStatus)
        clearActiveOwnership(refs)
        invalidatePoll()
        return
      }
      if (generation !== expectedGeneration
        || recordingSourcePreviewLeaseNonce(nextStatus) !== expectedLeaseNonce
        || sourceKey === null) {
        // A different process-local lease exists. This panel must not present
        // it as the source it owns merely because a stale cleanup was refused.
        setStatus({ ...INITIAL_SOURCE_PREVIEW_STATUS, state: 'unavailable' })
        clearActiveOwnership(refs)
        invalidatePoll()
        return
      }
      setStatus(nextStatus)
      refs.activeGeneration.current = generation
      refs.activeLeaseNonce.current = expectedLeaseNonce
      if (isSourcePreviewPolling(nextStatus)) startPolling(sourceKey)
    } catch {
      if (refs.mounted.current && command === refs.commandEpoch.current) {
        clearActiveOwnership(refs)
        invalidatePoll()
        setFrame(null)
        setStatus(INITIAL_SOURCE_PREVIEW_STATUS)
      }
    }
  }, [forgetReleasedSource, invalidatePoll, refs, setFrame, setStatus, startPolling])

  return useCallback(async (action: SourcePreviewLifecycleAction) => {
    const expectedGeneration = refs.activeGeneration.current
    const expectedLeaseNonce = refs.activeLeaseNonce.current
    const sourceKey = refs.activeSourceKey.current
    if (expectedGeneration === null || expectedLeaseNonce === null) {
      setStatusError('Native source preview ownership changed before the requested action could be sent.')
      return
    }
    const command = ++refs.commandEpoch.current
    invalidatePoll()
    setBusy(true)
    setFrame(null)
    setStatusError(null)
    try {
      const controlArgs = recordingSourcePreviewControlArgs(expectedGeneration, expectedLeaseNonce)
      if (!controlArgs) return
      const response = await callVerb(SOURCE_PREVIEW_LIFECYCLE_VERBS[action], controlArgs)
      if (!refs.mounted.current || command !== refs.commandEpoch.current) return
      if (!response.ok || !response.result) {
        setStatusError(sourcePreviewError(response.error, `Could not ${action} native source preview`))
        await reconcileFailedLifecycleAction(command, expectedGeneration, expectedLeaseNonce, sourceKey)
        return
      }
      const nextStatus = response.result.status
      setStatus(nextStatus)
      forgetReleasedSource(nextStatus)
      if (action === 'resume' && isSourcePreviewPolling(nextStatus)) {
        const ownedSourceKey = refs.activeSourceKey.current
        if (ownedSourceKey) startPolling(ownedSourceKey)
      }
    } catch (error) {
      if (refs.mounted.current && command === refs.commandEpoch.current) {
        setStatusError(sourcePreviewError(error, `Could not ${action} native source preview`))
        await reconcileFailedLifecycleAction(command, expectedGeneration, expectedLeaseNonce, sourceKey)
      }
    } finally {
      if (refs.mounted.current && command === refs.commandEpoch.current) setBusy(false)
    }
  }, [forgetReleasedSource, invalidatePoll, reconcileFailedLifecycleAction, refs, setBusy, setFrame, setStatus, setStatusError, startPolling])
}
