import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Icon } from '../../icons'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import { clampPopover } from './TrimPopover'
import { callVerb, type VerbArgs } from '../../lib/client'
import {
  activeVoiceoverOwner,
  clearVoiceoverOwner,
  persistVoiceoverReattach,
  readVoiceoverReattach,
  rememberVoiceoverOwner,
  type VoiceoverOwnerClaim,
  type VoiceoverReattachIdentity,
} from '../../app/voiceoverOwnerRuntime'
import { microphoneProblemText, voiceoverFailureText } from '../../app/microphoneMessages'
import {
  selectVoiceoverStartIdentity,
  voiceoverStartRetryWasRejected,
} from '../../app/voiceoverStartIdentity'

interface PublicMicrophone { token: string; label: string }
interface MicrophoneSelection {
  mode: 'system_default' | 'selected'
  label?: string
  status: 'ready' | 'unavailable' | 'system_default_only' | 'recovered_corruption'
}
interface VoiceoverStatus {
  phase: 'waiting_for_microphone' | 'countdown' | 'starting_playback' | 'recording' | 'finishing' | 'finished' | 'placed'
  request_id: string
  accepted_revision: string
  audio_track: string
  start_ms: number
  out_ms: number | null
  countdown_remaining_ms?: number | null
  owner_claim?: VoiceoverOwnerClaim
  terminal?: string | null
  /** Why a take failed: what is missing and what to do about it. */
  failure?: { reason?: string; message?: string; suggested_action?: string; detail?: string }
  placement?: { clip_id: string }
}

interface VoiceoverTrackControlProps {
  trackId: string
  locked: boolean
  projectRevision: string
  playheadMs: number
  exportRange: [number, number] | null
}

const pollMs = 250
let requestSequence = 0

function newRequestId(): string {
  requestSequence += 1
  const suffix = typeof globalThis.crypto?.randomUUID === 'function'
    ? globalThis.crypto.randomUUID()
    : `${Date.now()}-${requestSequence}`
  return `voiceover-${suffix}`
}

function describe(status: VoiceoverStatus): string {
  switch (status.phase) {
    case 'waiting_for_microphone': return 'Waiting for microphone readiness'
    case 'countdown': return `${Math.max(0, Math.ceil((status.countdown_remaining_ms ?? 0) / 1000))} second count-in`
    case 'starting_playback': return 'Starting timeline playback'
    case 'recording': return 'Recording voiceover'
    case 'finishing': return 'Finishing voiceover'
    case 'placed': return 'Voiceover added to the timeline'
    case 'finished':
      if (status.terminal === 'cancelled') return 'Voiceover cancelled'
      if (status.terminal === 'zero_samples') return 'Voiceover ended with no samples — no clip was added.'
      if (status.terminal === 'device_lost_no_samples') return 'Microphone was lost before samples — no clip was added.'
      if (status.terminal === 'failed') return voiceoverFailureText(status.failure)
      if (status.terminal === 'cancel_cleanup_failed') return 'Voiceover cancellation cleanup failed; its private recovery state is retained.'
      return 'Voiceover finished'
  }
}

function releaseTerminal(status: VoiceoverStatus): boolean {
  return status.phase === 'placed'
    || (status.phase === 'finished'
      && ['cancelled', 'zero_samples', 'device_lost_no_samples'].includes(status.terminal ?? ''))
}

/** A refusal names a missing prerequisite; show what is needed and what to do. */
const explainRefusal = microphoneProblemText

function terminalStatus(status: VoiceoverStatus): boolean {
  return status.phase === 'placed' || status.phase === 'finished'
}

/** Dense Timeline disclosure. It needs only the microphone: it reads microphone
 * readiness when opened and names any missing permission or device; browser
 * APIs never make this recorder look available on their own. */
export function VoiceoverTrackControl({
  trackId,
  locked,
  projectRevision,
  playheadMs,
  exportRange,
}: VoiceoverTrackControlProps) {
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const closePopover = useCallback(() => setOpen(false), [])
  const overlay = useBlockingOverlay<HTMLSpanElement>(closePopover, open)
  const placePopover = useCallback(() => {
    const popover = overlay.dialogRef.current
    const trigger = triggerRef.current
    if (!popover || !trigger) return
    const rect = trigger.getBoundingClientRect()
    const height = popover.getBoundingClientRect().height
    const below = rect.bottom + 5
    const y = below + height <= window.innerHeight - 8 ? below : rect.top - height - 5
    clampPopover(popover, rect.left - 77, y)
  }, [overlay.dialogRef])
  useLayoutEffect(() => {
    if (!open) return
    placePopover()
    // Read the live trigger after a resize or any ancestor's scroll. Readiness
    // and recording messages can also change the popup's height while open.
    window.addEventListener('resize', placePopover)
    window.addEventListener('scroll', placePopover, true)
    const observer = new ResizeObserver(placePopover)
    if (overlay.dialogRef.current) observer.observe(overlay.dialogRef.current)
    return () => {
      window.removeEventListener('resize', placePopover)
      window.removeEventListener('scroll', placePopover, true)
      observer.disconnect()
    }
  }, [open, placePopover, overlay.dialogRef])
  const [startAllowed, setStartAllowed] = useState<boolean | null>(null)
  const [microphones, setMicrophones] = useState<PublicMicrophone[]>([])
  const [selection, setSelection] = useState<MicrophoneSelection>({ mode: 'system_default', status: 'ready' })
  const [microphoneBlocked, setMicrophoneBlocked] = useState<string | null>(null)
  const capabilityRequest = useRef(0)
  const [status, setStatus] = useState<VoiceoverStatus | null>(null)
  const [error, setError] = useState<string | null>(null)
  const retryIdentity = useRef<VoiceoverReattachIdentity | null>(readVoiceoverReattach())
  const reattachAttempted = useRef(false)
  // Only outcomes that the server has explicitly discarded are idle again.
  // A failed/cancel-cleanup terminal remains visibly owned until its private
  // recovery path can resolve it; showing Record again here would be a lie.
  const releasedTerminal = status?.phase === 'finished'
    && ['cancelled', 'zero_samples', 'device_lost_no_samples'].includes(status.terminal ?? '')
  const terminalError = status?.phase === 'finished'
    && ['failed', 'cancel_cleanup_failed'].includes(status.terminal ?? '')
  const active = status !== null && status.phase !== 'placed' && !releasedTerminal
  const selectionUnavailable = selection.status === 'unavailable' || selection.status === 'recovered_corruption'
  const recordEnabled = startAllowed === true && !locked && !selectionUnavailable && projectRevision.length > 0 && !active

  const acceptStatus = useCallback((next: VoiceoverStatus) => {
    setStatus(next)
    if (next.owner_claim) {
      const identity: VoiceoverReattachIdentity = {
        request_id: next.request_id,
        expected_revision: next.accepted_revision,
        audio_track: next.audio_track,
        start_ms: next.start_ms,
        out_ms: next.out_ms,
      }
      retryIdentity.current = identity
      rememberVoiceoverOwner(identity, next.owner_claim)
      return
    }
    if (releaseTerminal(next)) {
      retryIdentity.current = null
      clearVoiceoverOwner()
    }
  }, [])

  const refreshCapability = useCallback(async () => {
    const request = ++capabilityRequest.current
    let response
    try {
      // Voiceover records only the microphone, so it reads only microphone
      // readiness: no screen read, list, or capture check.
      response = await callVerb('screen_record.doctor', { mode: 'microphone' })
    } catch {
      if (request === capabilityRequest.current) {
        setStartAllowed(false)
        setMicrophoneBlocked('ShellX Cut could not read microphone readiness. Check that the app is running, then open these controls again.')
      }
      return
    }
    if (request !== capabilityRequest.current) return
    if (!response.ok || !response.result) {
      setStartAllowed(false)
      setMicrophoneBlocked(explainRefusal(response.error) ?? 'ShellX Cut could not read microphone readiness.')
      return
    }
    const result = response.result as {
      start_allowed?: boolean
      microphones?: PublicMicrophone[]
      microphone_selection?: MicrophoneSelection
      microphone_blocked?: { message?: string; suggested_action?: string } | null
    }
    const blocked = result.microphone_blocked
    setMicrophoneBlocked(blocked ? explainRefusal(blocked) : null)
    setStartAllowed(result.start_allowed === true)
    setMicrophones(result.microphones ?? [])
    setSelection(result.microphone_selection ?? { mode: 'system_default', status: 'ready' })
  }, [])

  // Every audio track header renders this control. Read microphone readiness
  // only when the person opens these voiceover controls, or when this tab has
  // a take it started that must reattach after a reload, so opening a project
  // never touches a device.
  const capabilityWanted = open || retryIdentity.current !== null
  useEffect(() => {
    if (!capabilityWanted) return
    void refreshCapability()
    return () => { capabilityRequest.current += 1 }
  }, [capabilityWanted, refreshCapability])

  useEffect(() => {
    if (!active) return
    let cancelled = false
    let timer: number | null = null
    const poll = async () => {
      const owner = activeVoiceoverOwner()
      if (!owner) {
        setError('Voiceover is active but this tab must reattach before it can control the take.')
        return
      }
      const response = await callVerb('voiceover.tick', {
        owner_session_id: owner.owner.session_id,
        owner_capability: owner.owner.capability,
      })
      const stillOwner = activeVoiceoverOwner()
      // A Stop/Cancel may have won while this poll was in flight. Its accepted
      // terminal projection clears the owner; the stale poll must not replace
      // that truth with a retry error or an old active status.
      if (cancelled || stillOwner?.owner.capability !== owner.owner.capability) return
      if (!response.ok || !response.result) {
        setError(response.error?.suggested_action ?? response.error?.message ?? 'Voiceover could not continue.')
        // Keep the active owner visible and retry polling: a transient bridge
        // failure must not make an already-reserved native take look idle.
        timer = window.setTimeout(() => { void poll() }, pollMs)
        return
      }
      const next = response.result as VoiceoverStatus
      acceptStatus(next)
      if (next.phase === 'placed' || next.phase === 'finished') {
        if (next.phase === 'placed') setError(null)
        if (terminalStatus(next)) document.dispatchEvent(new CustomEvent('cut:voiceover-stop'))
        return
      }
      timer = window.setTimeout(() => { void poll() }, pollMs)
    }
    timer = window.setTimeout(() => { void poll() }, pollMs)
    return () => {
      cancelled = true
      if (timer !== null) window.clearTimeout(timer)
    }
  }, [acceptStatus, active])

  const changeMicrophone = useCallback(async (value: string) => {
    const args: VerbArgs['screen_record.microphone_selection'] = value === 'system_default'
      ? { mode: 'system_default' }
      : { mode: 'selected', microphone_token: value }
    const response = await callVerb('screen_record.microphone_selection', args)
    if (!response.ok) setError(response.error?.suggested_action ?? response.error?.message ?? 'Could not change microphone input.')
    await refreshCapability()
  }, [refreshCapability])

  const record = useCallback(async () => {
    setError(null)
    // Resolve persisted A before reading the current UI target. A response-
    // loss retry must not even construct B from a moved playhead/range until
    // the coordinator explicitly proves A was not admitted.
    const reattach = retryIdentity.current
    const identity = reattach ?? selectVoiceoverStartIdentity(null, {
      projectRevision,
      audioTrack: trackId,
      startMs: exportRange ? exportRange[0] : playheadMs,
      outMs: exportRange?.[1] ?? null,
    }, newRequestId)
    retryIdentity.current = identity
    // This happens before the first network send. A reload after a lost
    // response receives the same request/revision/track/range and can ask the
    // server for its still-active memory-only claim.
    persistVoiceoverReattach(identity)
    try {
      const response = await callVerb('voiceover.start', {
        request_id: identity.request_id,
        expected_revision: identity.expected_revision,
        audio_track: identity.audio_track,
        start_ms: identity.start_ms,
        ...(identity.out_ms === null ? {} : { out_ms: identity.out_ms }),
      })
      // A complete envelope can still describe an active take this tab is not
      // allowed to control. Forget A only when the coordinator explicitly
      // proves no matching owner survived pre-admission; all other errors keep
      // A for an exact retry rather than silently replacing it with B.
      if (!response.ok) {
        if (voiceoverStartRetryWasRejected(response.error)) {
          retryIdentity.current = null
          clearVoiceoverOwner()
        }
        setError(explainRefusal(response.error) ?? 'Voiceover start was not confirmed. Record voiceover retries the same request.')
        return
      }
      if (!response.result) {
        setError('Voiceover start was not confirmed. Record voiceover retries the same request.')
        return
      }
      const next = response.result as VoiceoverStatus
      acceptStatus(next)
      if (terminalStatus(next)) document.dispatchEvent(new CustomEvent('cut:voiceover-stop'))
    } catch {
      setError('Voiceover start response was lost. Record voiceover retries the same request.')
    }
  }, [acceptStatus, exportRange, playheadMs, projectRevision, trackId])

  useEffect(() => {
    const identity = retryIdentity.current
    if (reattachAttempted.current || status || startAllowed !== true || !identity) return
    reattachAttempted.current = true
    void record()
  }, [projectRevision, record, startAllowed, status, trackId])

  const terminal = useCallback(async (kind: 'stop' | 'cancel') => {
    const owner = activeVoiceoverOwner()
    if (!owner) {
      setError('Voiceover must reattach to this Timeline tab before it can be stopped or cancelled.')
      return
    }
    try {
      const response = await callVerb(`voiceover.${kind}`, {
        owner_session_id: owner.owner.session_id,
        owner_capability: owner.owner.capability,
      })
      if (!response.ok || !response.result) {
        setError(response.error?.suggested_action ?? response.error?.message ?? `Voiceover ${kind} failed.`)
        return
      }
      const next = response.result as VoiceoverStatus
      acceptStatus(next)
      // Stop Preview only after the server has accepted the terminal intent.
      document.dispatchEvent(new CustomEvent('cut:voiceover-stop'))
    } catch {
      // The server may have accepted the terminal request before its response
      // was lost. Reuse the exact original start identity to read back its
      // active or terminal projection; never stop local Preview speculatively.
      setError(`Voiceover ${kind} response was lost. Reattaching to the same take.`)
      void record()
    }
  }, [acceptStatus, record])

  const unavailableReason = locked
    ? 'Unlock this audio track to record voiceover.'
    : selectionUnavailable
      ? 'Choose an available microphone or System Default first.'
      : startAllowed === false
        ? microphoneBlocked ?? 'The microphone is not ready for voiceover.'
        : !projectRevision
          ? 'Timeline revision is loading.'
          : 'Checking voiceover capability…'
  const rangeLabel = exportRange ? `In–Out ${exportRange[0]}–${exportRange[1]} ms` : `Playhead ${playheadMs} ms`

  return (
    <span className="tl-voiceover" data-cut-voiceover-track={trackId}>
      <button
        type="button"
        className={`tl-voiceover__trigger${active ? ' tl-voiceover__trigger--active' : ''}`}
        data-cut-action="voiceover-controls"
        data-cut-voiceover-control={active ? status?.phase : startAllowed === true ? 'ready' : startAllowed === null ? 'unchecked' : 'unavailable'}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={active ? describe(status!) : 'Record voiceover on this audio track'}
        ref={triggerRef}
        onMouseDown={(event) => event.stopPropagation()}
        onClick={() => setOpen((value) => !value)}
      >
        <Icon name="record" size={14} />
      </button>
      {open && createPortal(
        <span
          ref={overlay.dialogRef}
          className="tl-voiceover__popover"
          data-cut-blocking-overlay
          data-cut-overlay-part
          role="dialog"
          aria-modal="true"
          aria-label={`Voiceover controls for ${trackId}`}
          tabIndex={-1}
          onKeyDown={overlay.onDialogKeyDown}
        >
          <span className="tl-voiceover__title">Voiceover <b>{trackId}</b></span>
          <label className="tl-voiceover__field">
            <span>Mic</span>
            <select
              data-cut-action="voiceover-mic-selection"
              data-cut-voiceover-mic={selection.mode}
              value={selection.mode === 'selected' ? '__selected__' : 'system_default'}
              disabled={active || selection.status === 'system_default_only'}
              onChange={(event) => {
                if (event.target.value !== '__selected__') void changeMicrophone(event.target.value)
              }}
            >
              <option value="system_default">System Default</option>
              {selection.mode === 'selected' && <option value="__selected__">{selection.label ?? 'Selected microphone unavailable'}</option>}
              {microphones.map((microphone) => <option key={microphone.token} value={microphone.token}>{microphone.label}</option>)}
            </select>
          </label>
          <span className="tl-voiceover__range" data-cut-voiceover-range={exportRange ? 'in-out' : 'playhead'}>{rangeLabel}</span>
          <span className="tl-voiceover__countdown" data-cut-voiceover-countdown>3 s count-in</span>
          <span className="tl-voiceover__monitoring" data-cut-voiceover-monitoring="off">Direct monitoring: Off</span>
          {active ? (
            <span className="tl-voiceover__actions">
              <button type="button" data-cut-action="voiceover-stop" onClick={() => void terminal('stop')} disabled={status?.phase === 'finishing'}>Stop</button>
              <button type="button" data-cut-action="voiceover-cancel" onClick={() => void terminal('cancel')} disabled={status?.phase === 'finishing'}>Cancel</button>
            </span>
          ) : (
            <button
              type="button"
              className="tl-voiceover__record"
              data-cut-action="voiceover-record"
              disabled={!recordEnabled}
              title={recordEnabled ? 'Record voiceover' : unavailableReason}
              onClick={() => void record()}
            >
              <Icon name="record" size={14} /> Record voiceover
            </button>
          )}
          <span className={`tl-voiceover__status${error || terminalError ? ' tl-voiceover__status--error' : ''}`} data-cut-voiceover-status={error || terminalError ? 'error' : active ? status?.phase : startAllowed === true ? 'idle' : 'unavailable'} aria-live="polite">
            {error ?? (status ? describe(status) : startAllowed === true ? 'Ready — no direct mic monitoring.' : unavailableReason)}
          </span>
        </span>, document.body,
      )}
    </span>
  )
}
