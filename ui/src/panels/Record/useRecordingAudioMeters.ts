import { useEffect, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import type { ScreenRecordStatusResult } from '../../lib/clientResults'
import {
  RECORDING_AUDIO_METER_POLL_MS,
  RecordingAudioMeterPollGuard,
  recordingAudioMeterCaptureEndDetail,
  recordingAudioMeterNotFoundEndDetail,
  recordingAudioMeterStatusError,
  recordingAudioMeterStatusIsTerminal,
} from './recordingAudioMeterPolling'

interface RecordingAudioMetersState {
  captureId: string | null
  meters: ScreenRecordStatusResult['audio_meters'] | null
  sourceLifecycle: ScreenRecordStatusResult['source_lifecycle'] | null
  controllerPlacement: ScreenRecordStatusResult['controller_placement'] | null
  statusError: string | null
  captureEndDetail: string | null
}

export interface RecordingAudioMetersProjection {
  meters: ScreenRecordStatusResult['audio_meters'] | null
  sourceLifecycle: ScreenRecordStatusResult['source_lifecycle'] | null
  controllerPlacement: ScreenRecordStatusResult['controller_placement'] | null
  statusError: string | null
  captureEndDetail: string | null
}

const EMPTY_AUDIO_METERS: RecordingAudioMetersState = {
  captureId: null,
  meters: null,
  sourceLifecycle: null,
  controllerPlacement: null,
  statusError: null,
  captureEndDetail: null,
}

const EMPTY_PROJECTION: RecordingAudioMetersProjection = {
  meters: null,
  sourceLifecycle: null,
  controllerPlacement: null,
  statusError: null,
  captureEndDetail: null,
}

/**
 * Reads the process-local meter projection only while this exact capture is
 * live in the UI. It never probes, opens, or otherwise changes audio inputs.
 */
export function useRecordingAudioMeters(captureId: string | null, active: boolean) {
  const [state, setState] = useState<RecordingAudioMetersState>(EMPTY_AUDIO_METERS)
  const pollGuard = useRef(new RecordingAudioMeterPollGuard())

  useEffect(() => {
    if (!active || !captureId) {
      setState(EMPTY_AUDIO_METERS)
      return
    }

    const token = pollGuard.current.begin(captureId)
    let inFlight = false
    let timer: number | null = null
    const stopPolling = () => {
      if (timer !== null) window.clearInterval(timer)
      timer = null
    }
    const stillCurrent = () => pollGuard.current.isCurrent(token)
    const reset = (next: Omit<RecordingAudioMetersState, 'captureId'>) => {
      if (stillCurrent()) setState({ captureId, ...next })
    }

    const poll = async () => {
      if (!stillCurrent() || inFlight) return
      inFlight = true
      try {
        const response = await callVerb('screen_record.status', { capture_id: captureId })
        if (!stillCurrent()) return
        if (!response.ok || !response.result) {
          const captureEndDetail = recordingAudioMeterNotFoundEndDetail(response.error)
          reset({
            meters: null,
            sourceLifecycle: null,
            controllerPlacement: null,
            statusError: recordingAudioMeterStatusError(response.error),
            captureEndDetail,
          })
          if (captureEndDetail) stopPolling()
          return
        }
        const status = response.result
        // A successful envelope still needs the requested capture identity;
        // otherwise its values are not evidence for this recording.
        if (status.capture_id !== captureId) return
        const captureEndDetail = recordingAudioMeterCaptureEndDetail(status)
        reset({
          meters: status.audio_meters,
          sourceLifecycle: status.source_lifecycle,
          controllerPlacement: status.controller_placement,
          statusError: null,
          captureEndDetail,
        })
        if (recordingAudioMeterStatusIsTerminal(status)) stopPolling()
      } catch (error) {
        if (stillCurrent()) {
          reset({
            meters: null,
            sourceLifecycle: null,
            controllerPlacement: null,
            statusError: recordingAudioMeterStatusError(error),
            captureEndDetail: null,
          })
        }
      } finally {
        inFlight = false
      }
    }

    void poll()
    timer = window.setInterval(() => { void poll() }, RECORDING_AUDIO_METER_POLL_MS)
    return () => {
      stopPolling()
      pollGuard.current.cancel(token)
    }
  }, [active, captureId])

  // The identity check prevents an old value appearing even for the render
  // before React runs the replacement effect for a new capture ID.
  if (!active || !captureId || state.captureId !== captureId) return EMPTY_PROJECTION
  return {
    meters: state.meters,
    sourceLifecycle: state.sourceLifecycle,
    controllerPlacement: state.controllerPlacement,
    statusError: state.statusError,
    captureEndDetail: state.captureEndDetail,
  }
}
