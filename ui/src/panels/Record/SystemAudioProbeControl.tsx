import { useState } from 'react'
import { callVerb, type ScreenRecordSystemAudioProbeResult } from '../../lib/client'

interface Props {
  disabled: boolean
  onRunningChange: (running: boolean) => void
  /** The engine's operating system from the environment report ('macos', 'windows', 'linux'); null until it is known. */
  hostOs?: string | null
}

/** The hint before a test. Only macOS asks for Audio Capture permission, so the sentence about it is shown there alone. */
export function systemAudioProbeHint(hostOs: string | null | undefined): string {
  return hostOs === 'macos'
    ? 'Play a short sound, then test. macOS may ask for Audio Capture permission.'
    : 'Play a short sound, then test.'
}

type ProbeState =
  | { kind: 'idle' }
  | { kind: 'running' }
  | { kind: 'done'; result: ScreenRecordSystemAudioProbeResult }
  | { kind: 'error'; detail: string }

export function SystemAudioProbeControl({ disabled, onRunningChange, hostOs = null }: Props) {
  const [state, setState] = useState<ProbeState>({ kind: 'idle' })

  const run = async () => {
    setState({ kind: 'running' })
    onRunningChange(true)
    try {
      const response = await callVerb('screen_record.system_audio_probe', { max_ms: 2_500 })
      if (response.ok && response.result) {
        setState({ kind: 'done', result: response.result })
        return
      }
      const error = response.error
      setState({
        kind: 'error',
        detail: error?.suggested_action ?? error?.cause ?? error?.message ?? 'The audio test did not complete.',
      })
    } catch {
      setState({ kind: 'error', detail: 'Cut could not reach the audio test. Check the engine and try again.' })
    } finally {
      onRunningChange(false)
    }
  }

  const status = state.kind === 'running'
    ? 'Listening for system audio for 2.5 seconds…'
    : state.kind === 'done'
      ? state.result.live && state.result.signal_detected
        ? `✓ System audio ready — first packet after ${state.result.first_packet_offset_ms ?? 0} ms.`
        : state.result.detail
      : state.kind === 'error'
        ? state.detail
        : systemAudioProbeHint(hostOs)

  const statusKind = state.kind === 'done' && state.result.live && state.result.signal_detected
    ? 'ready'
    : state.kind === 'done'
      ? state.result.live ? 'silent' : 'no-packets'
      : state.kind === 'error'
        ? 'error'
        : state.kind === 'running'
          ? 'running'
          : 'idle'

  return (
    <div className="rec__audio-probe" data-cut-rec-system-audio-probe={statusKind}>
      <button
        type="button"
        className="rec__export-btn rec__export-btn--small rec__export-btn--ghost"
        data-cut-action="record-system-audio-probe"
        disabled={disabled || state.kind === 'running'}
        onClick={() => void run()}
      >
        {state.kind === 'running' ? 'Testing…' : 'Test system audio'}
      </button>
      <span className="rec__audio-probe-status" aria-live="polite">{status}</span>
    </div>
  )
}
