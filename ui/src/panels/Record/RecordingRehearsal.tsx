import { useCallback, useEffect, useRef, useState } from 'react'
import {
  API_BASE,
  callVerb,
  type ScreenRecordRehearsalStartResult,
} from '../../lib/client'
import type { RecordingSourceKind } from './regionPickerModel'

type RehearsalState =
  | { kind: 'idle' }
  | { kind: 'running' }
  | { kind: 'ready'; take: ScreenRecordRehearsalStartResult }
  | { kind: 'error'; detail: string }

interface RecordingRehearsalProps {
  disabled: boolean
  sourceKind: RecordingSourceKind
  fps: number
  monitor: number | null
  monitorId: string | null
  windowId: string | null
  startAllowed: boolean | null
  startError: string | null
  /** Refreshes Doctor after the native take returns or is cleaned up. */
  onRefresh: () => void
}

/** Every disposable take is bound to this exact current recorder selection. */
export function recordingRehearsalConfigKey({
  sourceKind,
  fps,
  monitor,
  monitorId,
  windowId,
}: Pick<RecordingRehearsalProps, 'sourceKind' | 'fps' | 'monitor' | 'monitorId' | 'windowId'>): string {
  return JSON.stringify([sourceKind, fps, monitor, monitorId, windowId])
}

function sourceLabel(source: RecordingSourceKind): string {
  if (source === 'window') return 'selected window'
  if (source === 'region') return 'selected area'
  return 'selected display'
}

function unavailableReason(startAllowed: boolean | null, startError: string | null): string | null {
  if (startError) return startError
  if (startAllowed === false) return 'Screen capture is not ready on this machine.'
  return null
}

/**
 * A rehearsal is deliberately a real but disposable native video take. The
 * endpoint in its result is an opaque, revocable server capability; it is not
 * a project media path and cannot turn into a recording implicitly.
 */
export function RecordingRehearsal({
  disabled,
  sourceKind,
  fps,
  monitor,
  monitorId,
  windowId,
  startAllowed,
  startError,
  onRefresh,
}: RecordingRehearsalProps) {
  const [state, setState] = useState<RehearsalState>({ kind: 'idle' })
  const requestId = useRef(0)
  const activeHandle = useRef<string | null>(null)
  const configKey = recordingRehearsalConfigKey({ sourceKind, fps, monitor, monitorId, windowId })
  const configKeyRef = useRef(configKey)

  const discard = useCallback(async (handle = activeHandle.current) => {
    if (!handle) return
    if (activeHandle.current === handle) activeHandle.current = null
    try {
      await callVerb('screen_record.rehearsal_discard', { handle })
    } finally {
      onRefresh()
    }
  }, [onRefresh])

  // Closing/navigating away always drops the short-lived capability. If an
  // in-flight native take returns after cleanup begins, `rehearse` below sees
  // the bumped request id and explicitly discards that late result too.
  useEffect(() => () => {
    requestId.current += 1
    void discard()
  }, [discard])

  // A ready or running take is evidence only for the configuration that
  // created it. Replacing any source identity or fps hides it immediately;
  // the increment also makes a late start response discard itself.
  useEffect(() => {
    if (configKeyRef.current === configKey) return
    configKeyRef.current = configKey
    requestId.current += 1
    const priorHandle = activeHandle.current
    activeHandle.current = null
    setState({ kind: 'idle' })
    if (priorHandle) void discard(priorHandle)
  }, [configKey, discard])

  const rehearse = useCallback(async () => {
    const id = ++requestId.current
    await discard()
    if (id !== requestId.current) return
    setState({ kind: 'running' })
    const args: { duration_ms: number; fps: number; monitor?: number; monitor_id?: string; window?: string } = {
      duration_ms: 3_000,
      fps,
    }
    if (sourceKind === 'window' && windowId) args.window = windowId
    else {
      if (monitor !== null) args.monitor = monitor
      if (monitorId) args.monitor_id = monitorId
    }
    try {
      const response = await callVerb('screen_record.rehearsal_start', args)
      const take = response.result
      if (id !== requestId.current) {
        if (response.ok && take) void discard(take.playback_handle)
        return
      }
      if (!response.ok || !take) {
        setState({
          kind: 'error',
          detail: response.error?.suggested_action ?? response.error?.message ?? 'The native rehearsal did not complete.',
        })
        return
      }
      activeHandle.current = take.playback_handle
      setState({ kind: 'ready', take })
      onRefresh()
    } catch {
      if (id === requestId.current) {
        setState({ kind: 'error', detail: 'Cut could not reach the native rehearsal. Check the engine and try again.' })
      }
    }
  }, [discard, fps, monitor, monitorId, onRefresh, sourceKind, windowId])

  const blockReason = unavailableReason(startAllowed, startError)
  const status = state.kind === 'running'
    ? `Capturing a 3-second rehearsal of your ${sourceLabel(sourceKind)}…`
    : state.kind === 'ready'
      ? 'Playback is ready. It is temporary and will be discarded when you leave or start recording.'
      : state.kind === 'error'
        ? state.detail
        : blockReason
          ? blockReason
          : `Optionally take a 3-second rehearsal of your ${sourceLabel(sourceKind)} to check it before recording.`

  return (
    <section className="rec-rehearsal" data-cut-rec-rehearsal data-cut-rec-rehearsal-state={state.kind}>
      <div className="rec-rehearsal__head">
        <div>
          <span className="rec__eyebrow">Optional check</span>
          <strong>Rehearse setup</strong>
        </div>
        <button
          type="button"
          className="rec__export-btn rec__export-btn--small rec__export-btn--ghost"
          data-cut-action="record-rehearse"
          disabled={disabled || Boolean(blockReason) || state.kind === 'running'}
          onClick={() => { void rehearse() }}
        >
          {state.kind === 'running' ? 'Capturing…' : state.kind === 'ready' ? 'Try again' : 'Rehearse'}
        </button>
      </div>
      <p className="rec-rehearsal__status" data-cut-rec-rehearsal-status role="status" aria-live="polite">{status}</p>
      <p className="rec-rehearsal__boundary" data-cut-rec-rehearsal-disposable>
        This is a disposable test take: it does not create a project, recording, recovery item, or timeline media.
      </p>
      <details className="rec-rehearsal__details" data-cut-rec-rehearsal-details>
        <summary data-cut-action="record-rehearsal-details-toggle">What rehearsal checks</summary>
        <p className="rec-rehearsal__boundary" data-cut-rec-rehearsal-audio-boundary>
          Playback verifies the selected picture source. Microphone and system-audio checks remain separate, so this take never claims audio that it did not capture.
        </p>
      </details>
      {state.kind === 'ready' && (
        <div className="rec-rehearsal__playback" data-cut-rec-rehearsal-playback>
          <video
            data-cut-rec-rehearsal-video
            src={`${API_BASE}${state.take.playback_url}`}
            controls
            autoPlay
            playsInline
            onError={() => setState({ kind: 'error', detail: 'The native take completed, but this browser could not play its temporary video.' })}
          />
          <button
            type="button"
            className="rec__text-button"
            data-cut-action="record-rehearsal-discard"
            onClick={() => {
              void discard(state.take.playback_handle)
              setState({ kind: 'idle' })
            }}
          >
            Discard rehearsal
          </button>
        </div>
      )}
    </section>
  )
}
