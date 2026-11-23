import { recordingNotice } from './recordingUiModel'
import type { ScreenRecordAudioMeter, ScreenRecordAudioMeterState } from '../../lib/clientResults'

export interface RecordingAudioMeterPresentation {
  label: string
  state: ScreenRecordAudioMeterState
  detail: string
  levelText: string | null
  fillPercent: number
  live: boolean
  clipping: boolean
}

const DBFS_FLOOR = -60

function finiteDbfs(value: number | null): number | null {
  if (value === null || !Number.isFinite(value)) return null
  return Math.max(DBFS_FLOOR, Math.min(0, value))
}

function stateDetail(state: ScreenRecordAudioMeterState, detail?: string): string {
  if (detail) return recordingNotice(detail, stateDetail(state))
  switch (state) {
    case 'not_requested': return 'Not enabled for this recording.'
    case 'awaiting_samples': return 'Waiting for sound.'
    case 'stale': return 'The sound level has not updated. Check your audio source.'
    case 'device_lost': return 'The audio device stopped sending sound. Check its connection.'
    case 'stopped': return 'Audio delivery has stopped for this recording.'
    case 'unavailable': return 'Sound levels are unavailable for this recording.'
    case 'live': return 'Receiving sound.'
  }
}

/**
 * Maps only current server snapshots to a compact meter. A stale, terminal, or
 * malformed reading always becomes a zero-fill non-live presentation.
 */
export function recordingAudioMeterPresentation(
  label: string,
  meter: ScreenRecordAudioMeter,
): RecordingAudioMeterPresentation {
  const rmsDbfs = finiteDbfs(meter.rms_dbfs)
  const live = meter.state === 'live' && !meter.stale && rmsDbfs !== null
  const state = live ? 'live' : meter.state === 'live' ? 'stale' : meter.state
  const fillPercent = live ? Math.round(((rmsDbfs - DBFS_FLOOR) / -DBFS_FLOOR) * 100) : 0
  return {
    label,
    state,
    detail: stateDetail(state, meter.detail),
    levelText: live ? `${Math.round(rmsDbfs)} dBFS` : null,
    fillPercent,
    live,
    clipping: live && meter.clipping,
  }
}
