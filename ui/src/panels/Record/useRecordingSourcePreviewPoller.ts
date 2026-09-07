import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import { callVerb } from '../../lib/client'
import type { ScreenRecordSourcePreviewFrame, ScreenRecordSourcePreviewStatus } from '../../lib/clientResults'
import {
  recordingSourcePreviewControlArgs,
  recordingSourcePreviewFailedStartStatus,
  recordingSourcePreviewGeneration,
  recordingSourcePreviewLeaseMatches,
} from './recordingSourcePreviewLease'
import {
  isSourcePreviewPolling,
  sourcePreviewError,
  type SourcePreviewOwnershipRefs,
} from './recordingSourcePreviewLifecycle'

const SOURCE_PREVIEW_POLL_MS = 250
const MAX_SOURCE_PREVIEW_READ_FAILURES = 3

interface SourcePreviewPollSession {
  epoch: number
  sourceKey: string
  expectedGeneration: number
  expectedLeaseNonce: string
}

interface PreviewPollerSupport {
  refs: SourcePreviewOwnershipRefs
  setFrame: Dispatch<SetStateAction<ScreenRecordSourcePreviewFrame | null>>
  setStatus: Dispatch<SetStateAction<ScreenRecordSourcePreviewStatus>>
  setStatusError: Dispatch<SetStateAction<string | null>>
  forgetReleasedSource: (status: ScreenRecordSourcePreviewStatus) => void
}

/** Polls one acknowledged preview lease and releases it after unreadable status. */
export function useRecordingSourcePreviewPoller({
  refs,
  forgetReleasedSource,
  setFrame,
  setStatus,
  setStatusError,
}: PreviewPollerSupport) {
  const [pollSession, setPollSession] = useState<SourcePreviewPollSession | null>(null)
  const sessionEpochRef = useRef(0)

  const cancelPolling = useCallback(() => {
    sessionEpochRef.current += 1
  }, [])

  const invalidatePoll = useCallback(() => {
    cancelPolling()
    setPollSession(null)
  }, [cancelPolling])

  const startPolling = useCallback((sourceKey: string) => {
    const expectedGeneration = refs.activeGeneration.current
    const expectedLeaseNonce = refs.activeLeaseNonce.current
    const controlArgs = recordingSourcePreviewControlArgs(expectedGeneration, expectedLeaseNonce)
    if (!controlArgs) return
    const epoch = ++sessionEpochRef.current
    setPollSession({
      epoch,
      sourceKey,
      expectedGeneration: controlArgs.expected_generation,
      expectedLeaseNonce: controlArgs.expected_lease_nonce,
    })
  }, [refs])

  useEffect(() => {
    if (!pollSession) return
    let stale = false
    let inFlight = false
    let readFailures = 0
    let timer: number | null = null
    const current = () => !stale
      && refs.mounted.current
      && sessionEpochRef.current === pollSession.epoch
      && refs.activeSourceKey.current === pollSession.sourceKey
      && refs.activeGeneration.current === pollSession.expectedGeneration
      && refs.activeLeaseNonce.current === pollSession.expectedLeaseNonce
    const stopPolling = () => {
      stale = true
      if (timer !== null) window.clearInterval(timer)
    }
    const abandonUnreadablePreview = () => {
      const activeGeneration = refs.activeGeneration.current
      const activeLeaseNonce = refs.activeLeaseNonce.current
      refs.activeSourceKey.current = null
      refs.activeGeneration.current = null
      refs.activeLeaseNonce.current = null
      refs.startingSourceKey.current = null
      invalidatePoll()
      stopPolling()
      // After repeated read failures this client cannot continue to claim an
      // observed native lease, so it asks the process owner to release it.
      const controlArgs = recordingSourcePreviewControlArgs(activeGeneration, activeLeaseNonce)
      if (controlArgs) void callVerb('screen_record.preview_stop', controlArgs)
    }
    const abandonReplacedPreview = () => {
      // The global preview owner can be replaced by another panel or by a
      // restarted Cut server between polling requests. Its status/frame is
      // evidence about that owner, never an acknowledgement for this panel.
      refs.activeSourceKey.current = null
      refs.activeGeneration.current = null
      refs.activeLeaseNonce.current = null
      refs.startingSourceKey.current = null
      setFrame(null)
      setStatus({ state: 'unavailable', recursion: 'none', has_frame: false, generation: null })
      setStatusError('Native source preview lease changed outside this panel. Start the preview again.')
      invalidatePoll()
      stopPolling()
    }
    const abandonTerminalPreview = (terminalStatus: ScreenRecordSourcePreviewStatus) => {
      // A generation-null status cannot be a replacement lease. Preserve its
      // terminal truth (especially an unresolved native-release reason) while
      // dropping this panel's no-longer-active local ownership.
      refs.activeSourceKey.current = null
      refs.activeGeneration.current = null
      refs.activeLeaseNonce.current = null
      refs.startingSourceKey.current = null
      setFrame(null)
      setStatus(recordingSourcePreviewFailedStartStatus(terminalStatus))
      setStatusError(null)
      invalidatePoll()
      stopPolling()
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
          setStatusError(sourcePreviewError(response.error, 'Native source-preview status could not be read'))
          if (readFailures >= MAX_SOURCE_PREVIEW_READ_FAILURES) abandonUnreadablePreview()
          return
        }
        readFailures = 0
        const snapshot = response.result
        if (recordingSourcePreviewGeneration(snapshot.status) === null) {
          abandonTerminalPreview(snapshot.status)
          return
        }
        if (!recordingSourcePreviewLeaseMatches(
          snapshot.status,
          pollSession.expectedGeneration,
          pollSession.expectedLeaseNonce,
        )) {
          abandonReplacedPreview()
          return
        }
        setStatus(snapshot.status)
        forgetReleasedSource(snapshot.status)
        setFrame(snapshot.frame)
        setStatusError(null)
        if (!isSourcePreviewPolling(snapshot.status)) {
          invalidatePoll()
          stopPolling()
        }
      } catch (error) {
        if (current()) {
          readFailures += 1
          setFrame(null)
          setStatusError(sourcePreviewError(error, 'Native source-preview status could not be read'))
          if (readFailures >= MAX_SOURCE_PREVIEW_READ_FAILURES) abandonUnreadablePreview()
        }
      } finally {
        inFlight = false
      }
    }
    void poll()
    timer = window.setInterval(() => { void poll() }, SOURCE_PREVIEW_POLL_MS)
    return stopPolling
  }, [forgetReleasedSource, invalidatePoll, pollSession, refs, setFrame, setStatus, setStatusError])

  return { cancelPolling, invalidatePoll, startPolling }
}
