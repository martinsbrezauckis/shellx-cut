import { useEffect, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import { acceptRecordingLiveFrame, awaitingRecordingLiveFrame, recordingLiveFramePresentation, type RecordingLiveFramePresentation } from './recordingLiveFramePresentation'

const POLL_MS = 100

/** Read-only, serial polling of the recording owner's frame; owns no second capture. */
export function useRecordingLiveFrame(captureId: string | null): RecordingLiveFramePresentation | null {
  const [snapshot, setSnapshot] = useState<RecordingLiveFramePresentation | null>(null)
  const epochRef = useRef(0)
  const generationRef = useRef<number | null>(null)
  const snapshotRef = useRef<RecordingLiveFramePresentation | null>(null)

  useEffect(() => {
    const epoch = ++epochRef.current
    generationRef.current = null
    snapshotRef.current = captureId ? awaitingRecordingLiveFrame(captureId) : null
    setSnapshot(captureId ? awaitingRecordingLiveFrame(captureId) : null)
    if (!captureId) return
    let timer: number | null = null
    let cancelled = false
    const current = () => !cancelled && epochRef.current === epoch
    const poll = async () => {
      try {
        const response = await callVerb('screen_record.live_frame', { capture_id: captureId })
        if (!current()) return
        if (!response.ok || !response.result) {
          setSnapshot({ captureId, state: 'read_error', detail: response.error?.message || 'Cannot read live recording frames.', frameUrl: null, generation: generationRef.current, capturedAtMs: null })
          return
        }
        const next = recordingLiveFramePresentation(captureId, response.result)
        const accepted = acceptRecordingLiveFrame(snapshotRef.current, next)
        if (accepted === snapshotRef.current) return
        snapshotRef.current = accepted
        generationRef.current = accepted.generation
        setSnapshot(accepted)
      } catch {
        if (current()) setSnapshot({ captureId, state: 'read_error', detail: 'Cannot read live recording frames.', frameUrl: null, generation: generationRef.current, capturedAtMs: null })
      } finally {
        if (current()) timer = window.setTimeout(() => { void poll() }, POLL_MS)
      }
    }
    void poll()
    return () => {
      cancelled = true
      ++epochRef.current
      if (timer !== null) window.clearTimeout(timer)
    }
  }, [captureId])

  // Effects run after render: never expose the previous capture's pixels for one frame.
  return captureId && snapshot?.captureId === captureId ? snapshot
    : captureId ? awaitingRecordingLiveFrame(captureId) : null
}
