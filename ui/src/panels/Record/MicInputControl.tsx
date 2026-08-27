import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'

interface MicWarm {
  live: boolean
  peak_dbfs?: number
  supported: boolean
  selection_unavailable?: boolean
  skipped?: 'recording'
  message?: string
}

interface PublicMicrophone { token: string; label: string }
interface MicrophoneSelection {
  mode: 'system_default' | 'selected'
  label?: string
  status: 'ready' | 'unavailable' | 'system_default_only' | 'recovered_corruption'
}

interface Props {
  audio: boolean
  disabled: boolean
  /** Browser devicechange is allowed to refresh only in this phase. */
  idle: boolean
  onAudioChange: (audio: boolean) => void
  onTestingChange: (testing: boolean) => void
}

type TestState =
  | { kind: 'idle' }
  | { kind: 'testing' }
  | { kind: 'result'; result: MicWarm }
  | { kind: 'error'; detail: string }

function meterPercent(peakDbfs: number | undefined): number {
  if (typeof peakDbfs !== 'number') return 0
  return Math.round(Math.max(0, Math.min(100, ((peakDbfs + 60) / 60) * 100)))
}

function statusFor(state: TestState): string {
  if (state.kind === 'testing') return 'Listening for your voice for a moment…'
  if (state.kind === 'error') return state.detail
  if (state.kind !== 'result') return 'Test opens the selected input or OS System Default briefly. No audio is saved.'
  const { result } = state
  if (!result.supported) return 'This recorder build has no microphone input backend.'
  if (result.selection_unavailable) return 'Selected microphone is unavailable. Choose another microphone or System Default.'
  if (result.skipped === 'recording') return 'Microphone test is unavailable while recording.'
  if (!result.live) return result.message ?? 'No microphone signal arrived. Check access, your OS input, then test again.'
  return typeof result.peak_dbfs === 'number'
    ? `Ready — peak ${result.peak_dbfs} dBFS.`
    : 'Ready — no measurable signal yet.'
}

/** Public Record control: tokens + sanitized labels only, never native identity. */
export function MicInputControl({ audio, disabled, idle, onAudioChange, onTestingChange }: Props) {
  const [state, setState] = useState<TestState>({ kind: 'idle' })
  const [microphones, setMicrophones] = useState<PublicMicrophone[]>([])
  const [selection, setSelection] = useState<MicrophoneSelection>({ mode: 'system_default', status: 'ready' })
  const requestId = useRef(0)
  const previousAudio = useRef(audio)

  const refreshSelection = useCallback(async () => {
    const response = await callVerb('screen_record.doctor', {})
    if (!response.ok || !response.result) return
    const result = response.result as { microphones?: PublicMicrophone[]; microphone_selection?: MicrophoneSelection }
    setMicrophones(result.microphones ?? [])
    setSelection(result.microphone_selection ?? { mode: 'system_default', status: 'ready' })
  }, [])

  useEffect(() => { void refreshSelection() }, [refreshSelection])
  useEffect(() => () => {
    requestId.current += 1
    onTestingChange(false)
  }, [onTestingChange])

  const testMicrophone = useCallback(async () => {
    const id = ++requestId.current
    setState({ kind: 'testing' })
    onTestingChange(true)
    try {
      const response = await callVerb('screen_record.doctor', { warm_mic: true })
      if (id !== requestId.current) return
      const result = response.ok && response.result
        ? (response.result as { mic_warm?: MicWarm }).mic_warm
        : undefined
      if (!result) {
        setState({ kind: 'error', detail: response.error?.suggested_action ?? response.error?.message ?? 'Microphone test did not complete.' })
        return
      }
      setState({ kind: 'result', result })
      await refreshSelection()
    } catch {
      if (id === requestId.current) setState({ kind: 'error', detail: 'Cut could not reach the microphone test. Check the engine and try again.' })
    } finally {
      if (id === requestId.current) onTestingChange(false)
    }
  }, [onTestingChange, refreshSelection])

  // Initial enabled mount stays cold. Turning microphone capture back on is a
  // real user action and therefore gets one bounded test; Start opens capture.
  useEffect(() => {
    const turnedOn = audio && !previousAudio.current
    previousAudio.current = audio
    if (!audio) {
      requestId.current += 1
      setState({ kind: 'idle' })
      onTestingChange(false)
    } else if (turnedOn) {
      void testMicrophone()
    }
  }, [audio, onTestingChange, testMicrophone])

  useEffect(() => {
    if (!idle || !navigator.mediaDevices?.addEventListener) return
    const refresh = () => { void refreshSelection() }
    navigator.mediaDevices.addEventListener('devicechange', refresh)
    return () => navigator.mediaDevices.removeEventListener('devicechange', refresh)
  }, [idle, refreshSelection])

  const changeSelection = useCallback(async (value: string) => {
    const args = value === 'system_default'
      ? { mode: 'system_default' as const }
      : { mode: 'selected' as const, microphone_token: value }
    const response = await callVerb('screen_record.microphone_selection', args)
    if (!response.ok) setState({ kind: 'error', detail: response.error?.suggested_action ?? response.error?.message ?? 'Could not select microphone.' })
    await refreshSelection()
  }, [refreshSelection])

  const result = state.kind === 'result' ? state.result : undefined
  const peak = result?.peak_dbfs
  const meter = meterPercent(peak)
  const statusKind = selection.status === 'unavailable'
    ? 'unavailable'
    : state.kind === 'testing' ? 'testing'
      : state.kind === 'error' ? 'error'
        : result?.live && typeof peak === 'number' ? peak > -60 ? 'signal' : 'quiet'
          : result?.supported === false ? 'unavailable' : 'idle'

  return (
    <div className="rec__mic-input" data-cut-rec-mic-input={statusKind}>
      <label className="rec__toggle rec__toggle--mic" data-cut-rec-audio-toggle>
        <input type="checkbox" data-cut-rec-audio-toggle-input checked={audio} disabled={disabled} onChange={(event) => onAudioChange(event.target.checked)} />
        Capture microphone audio
      </label>
      {audio && (
        <div className="rec__mic-input-detail">
          <div className="rec__mic-input-head">
            <label className="rec__mic-input-device">
              Input
              <select
                data-cut-rec-mic-selection={selection.mode}
                disabled={disabled || selection.status === 'system_default_only'}
                value={selection.mode === 'selected' ? '__selected__' : 'system_default'}
                onChange={(event) => {
                  if (event.target.value !== '__selected__') void changeSelection(event.target.value)
                }}
              >
                <option value="system_default">System Default</option>
                {selection.mode === 'selected' && <option value="__selected__">{selection.label ?? 'Selected microphone unavailable'}</option>}
                {microphones.map((microphone) => <option key={microphone.token} value={microphone.token}>{microphone.label}</option>)}
              </select>
            </label>
            <button type="button" className="rec__export-btn rec__export-btn--small rec__export-btn--ghost" data-cut-action="record-mic-test" disabled={disabled || state.kind === 'testing'} onClick={() => void testMicrophone()}>
              {state.kind === 'testing' ? 'Testing…' : 'Test microphone'}
            </button>
          </div>
          <div className="rec__mic-meter" data-cut-rec-mic-meter={typeof peak === 'number' ? 'measured' : 'none'} role="meter" aria-label="Microphone test peak" aria-valuemin={0} aria-valuemax={100} aria-valuenow={meter} aria-valuetext={typeof peak === 'number' ? `${peak} dBFS peak from the last test` : 'No measurable signal'}>
            <span className="rec__mic-meter-fill" style={{ width: `${meter}%` }} />
          </div>
          <p className="rec__mic-status" data-cut-rec-mic-status={statusKind} aria-live="polite">{selection.status === 'unavailable' ? 'Selected microphone is unavailable. Start is blocked until you choose another input or System Default.' : statusFor(state)}</p>
          <p className="rec__source-note" data-cut-rec-mic-identity-boundary>
            Device choices use expiring opaque tokens and safe display labels. Cut stores the native endpoint privately and never exposes it here.
          </p>
        </div>
      )}
    </div>
  )
}
