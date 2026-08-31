import { useCallback, useState } from 'react'
import { callVerb } from '../../lib/client'
import {
  NO_RECORDING_PAUSE_CAPABILITY,
  recordingPauseAcknowledged,
  recordingPauseCapability,
  type RecordingPauseCapability,
  type RecordingPauseState,
} from './recordingPause'

/** Keeps the live button tied to the durable Pause/Resume acknowledgements. */
export function useRecordingPause() {
  const [capability, setCapability] = useState<RecordingPauseCapability>(NO_RECORDING_PAUSE_CAPABILITY)
  const [enabled, setEnabled] = useState(false)
  const [state, setState] = useState<RecordingPauseState>('idle')
  const [message, setMessage] = useState('Enable Pause & resume before your next recording.')
  const [captureId, setCaptureId] = useState<string | null>(null)

  const setDoctorCapability = useCallback((value: unknown) => {
    const next = recordingPauseCapability(value)
    setCapability(next)
    if (!next.supported) {
      setEnabled(false)
      setState('idle')
      setMessage(next.detail)
    }
  }, [])

  const setPauseEnabled = useCallback((next: boolean) => {
    if (!next) {
      setEnabled(false)
      setMessage('Pause & resume will not be used for the next recording.')
      return
    }
    if (!capability.supported) {
      setMessage(capability.detail)
      return
    }
    setEnabled(true)
    setMessage('Pause & resume will be available after this recording starts.')
  }, [capability])

  const acknowledgeStart = useCallback((id: string, value: unknown) => {
    const admitted = !!value && typeof value === 'object' && (value as Record<string, unknown>).enabled === true
    if (!enabled || !admitted) {
      setCaptureId(null)
      setState('idle')
      if (enabled) setMessage('The recorder did not acknowledge Pause & resume admission; live controls stay unavailable.')
      return
    }
    setCaptureId(id)
    setState('recording')
    setMessage('Recording. Pause only changes state after the recorder seals it.')
  }, [enabled])

  const clearCapture = useCallback(() => {
    setCaptureId(null)
    setState('idle')
  }, [])

  const control = useCallback(async () => {
    if (!captureId || (state !== 'recording' && state !== 'paused')) return
    const action = state === 'paused' ? 'resume' : 'pause'
    setState(action === 'pause' ? 'pausing' : 'resuming')
    setMessage(action === 'pause' ? 'Waiting for the recorder to seal the pause…' : 'Waiting for the recorder to seal the resume…')
    const response = await callVerb(`screen_record.${action}`, { capture_id: captureId })
    const acknowledgement = response.ok ? recordingPauseAcknowledged(response.result, action) : null
    if (!acknowledgement) {
      setState(action === 'pause' ? 'recording' : 'paused')
      setMessage(response.ok
        ? 'The recorder did not return a durable acknowledgement; state is unchanged.'
        : `${response.error?.code ?? 'failed'}: ${response.error?.message ?? 'Pause control failed'}`)
      return
    }
    setState(acknowledgement.state)
    setMessage(acknowledgement.state === 'paused'
      ? `Paused at ${acknowledgement.logical_media_time_ms} ms; the durable journal acknowledged it.`
      : `Resumed at ${acknowledgement.logical_media_time_ms} ms; the durable journal acknowledged it.`)
  }, [captureId, state])

  return {
    capability,
    enabled,
    state,
    message,
    setDoctorCapability,
    setPauseEnabled,
    acknowledgeStart,
    clearCapture,
    control,
  }
}
