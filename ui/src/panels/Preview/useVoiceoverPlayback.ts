import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import { callVerb } from '../../lib/client'
import type { Rate } from './PreviewTransport'

interface VoiceoverPlayback {
  requestId: string
  requestFingerprint: string
  bridgeEpoch: number
  outMs: number | null
  ownerSessionId: string
  ownerCapability: string
}

/** Clock position shared by Preview playback and the correlated start seek. */
export function previewPlaybackClockPosition(
  positionMs: number,
  durationMs: number,
  voiceover: Pick<VoiceoverPlayback, 'outMs'> | null,
): { positionMs: number; stop: boolean } {
  if (voiceover) {
    // The reserved take owns its clock beyond current program content. At
    // finite Out, publish the boundary and let the server accept Stop; setting
    // rate zero here would prevent the hook from reporting that observation.
    return {
      positionMs: Math.max(0, Math.min(voiceover.outMs ?? Infinity, Math.round(positionMs))),
      stop: false,
    }
  }
  return {
    positionMs: Math.max(0, Math.min(durationMs, Math.round(positionMs))),
    stop: positionMs <= 0 || positionMs >= durationMs,
  }
}

/** Old clock cleanup must not overwrite a newly admitted Voiceover seek. */
export function previewPlaybackCleanupPosition(
  positionMs: number,
  durationMs: number,
  previous: VoiceoverPlayback | null,
  current: VoiceoverPlayback | null,
): number | null {
  if (current && (!previous
    || current.requestId !== previous.requestId
    || current.requestFingerprint !== previous.requestFingerprint
    || current.bridgeEpoch !== previous.bridgeEpoch
    || current.ownerSessionId !== previous.ownerSessionId
    || current.ownerCapability !== previous.ownerCapability
    || current.outMs !== previous.outMs)) return null
  // A true terminal transition still seals the old clock position using that
  // take's horizon, rather than rewinding to the old program content extent.
  return previewPlaybackClockPosition(positionMs, durationMs, previous).positionMs
}

interface UseVoiceoverPlaybackArgs {
  playheadMs: number
  durationMs: number
  rate: Rate
  onSeek: (atMs: number) => void
  setRate: Dispatch<SetStateAction<Rate>>
}

/** Preview-side half of the server-owned voiceover bridge. It starts ordinary
 * program playback only after the controller supplied a correlated request,
 * and reports Out only from the actual Preview playhead. */
export function useVoiceoverPlayback({
  playheadMs,
  durationMs,
  rate,
  onSeek,
  setRate,
}: UseVoiceoverPlaybackArgs): { playback: VoiceoverPlayback | null; outUnconfirmed: boolean } {
  const [voiceoverPlayback, setVoiceoverPlayback] = useState<VoiceoverPlayback | null>(null)
  const [outUnconfirmed, setOutUnconfirmed] = useState(false)
  const claimRef = useRef<VoiceoverPlayback | null>(null)
  const liveRef = useRef(true)
  const rateRef = useRef(rate)
  const inFlightRef = useRef<{ promise: Promise<unknown>; controller: AbortController; claim: VoiceoverPlayback } | null>(null)
  rateRef.current = rate
  const config = useRef({ durationMs, onSeek })
  config.current = { durationMs, onSeek }

  useEffect(() => {
    liveRef.current = true
    const onVoiceoverPlayback = (event: Event) => {
      const detail = event instanceof CustomEvent && event.detail && typeof event.detail === 'object'
        ? event.detail as Record<string, unknown>
        : null
      const requestId = typeof detail?.request_id === 'string' ? detail.request_id : ''
      const requestFingerprint = typeof detail?.request_fingerprint === 'string' ? detail.request_fingerprint : ''
      const bridgeEpoch = detail?.bridge_epoch
      const startMs = detail?.start_ms
      const outMs = detail?.out_ms
      const ownerSessionId = typeof detail?.owner_session_id === 'string' ? detail.owner_session_id : ''
      const ownerCapability = typeof detail?.owner_capability === 'string' ? detail.owner_capability : ''
      if (!requestId || !/^[a-f0-9]{64}$/i.test(requestFingerprint)
        || !Number.isSafeInteger(bridgeEpoch) || !Number.isSafeInteger(startMs)
        || !ownerSessionId || !ownerCapability) return
      const playback = {
        requestId,
        requestFingerprint,
        bridgeEpoch: Number(bridgeEpoch),
        outMs: Number.isSafeInteger(outMs) ? Number(outMs) : null,
        ownerSessionId,
        ownerCapability,
      }
      inFlightRef.current?.controller.abort()
      claimRef.current = playback
      setOutUnconfirmed(false)
      config.current.onSeek(previewPlaybackClockPosition(Number(startMs), config.current.durationMs, playback).positionMs)
      setVoiceoverPlayback(playback)
      setRate(1)
    }
    const onVoiceoverStop = () => {
      inFlightRef.current?.controller.abort()
      claimRef.current = null
      setOutUnconfirmed(false)
      setRate(0)
      setVoiceoverPlayback(null)
    }
    document.addEventListener('cut:voiceover-playback', onVoiceoverPlayback)
    document.addEventListener('cut:voiceover-stop', onVoiceoverStop)
    return () => {
      inFlightRef.current?.controller.abort()
      liveRef.current = false
      claimRef.current = null
      document.removeEventListener('cut:voiceover-playback', onVoiceoverPlayback)
      document.removeEventListener('cut:voiceover-stop', onVoiceoverStop)
    }
  }, [setRate])

  const atOut = voiceoverPlayback?.outMs !== null && voiceoverPlayback?.outMs !== undefined
    && playheadMs >= voiceoverPlayback.outMs
  useEffect(() => {
    if (!voiceoverPlayback || !atOut || rate === 0) return
    const claim = voiceoverPlayback
    let retired = false
    let timer: number | null = null
    let attempts = 0
    const current = () => !retired && liveRef.current && claimRef.current === claim && rateRef.current !== 0
    const terminal = (result: unknown) => {
      if (!result || typeof result !== 'object') return false
      const status = result as {
        request_id?: string; phase?: string; terminal?: string
        owner_claim?: { session_id?: string; capability?: string } | null
        placement?: { asset_id?: string; clip_id?: string; op_id?: string; already_applied?: boolean }
      }
      if (status.request_id !== claim.requestId) return false
      if (status.owner_claim !== undefined) {
        return status.owner_claim?.session_id === claim.ownerSessionId
          && status.owner_claim?.capability === claim.ownerCapability
          && ['finishing', 'finished'].includes(status.phase ?? '')
      }
      if (status.phase === 'finished') {
        return ['cancelled', 'zero_samples', 'device_lost_no_samples'].includes(status.terminal ?? '')
      }
      return status.phase === 'placed'
        && ['saved', 'device_lost_saved_prefix'].includes(status.terminal ?? '')
        && !!status.placement?.asset_id && !!status.placement.clip_id && !!status.placement.op_id
        && typeof status.placement.already_applied === 'boolean'
    }
    const stopIfCurrent = () => {
      if (current()) document.dispatchEvent(new CustomEvent('cut:voiceover-stop'))
    }
    const serialized = async <T,>(start: (signal: AbortSignal) => Promise<T>): Promise<T | null> => {
      const prior = inFlightRef.current
      if (prior) { try { await prior.promise } catch { /* Its owner handles its own failure. */ } }
      if (!current()) return null
      const controller = new AbortController()
      const pending = start(controller.signal)
      inFlightRef.current = { promise: pending, controller, claim }
      const timeout = window.setTimeout(() => controller.abort(), 2_500)
      try { return await pending } finally {
        window.clearTimeout(timeout)
        if (inFlightRef.current?.promise === pending) inFlightRef.current = null
      }
    }
    const observe = async () => {
      if (!current()) return
      attempts += 1
      try {
        const response = await serialized(signal => callVerb('voiceover.observe_playhead', {
          owner_session_id: claim.ownerSessionId,
          owner_capability: claim.ownerCapability,
          request_id: claim.requestId,
          request_fingerprint: claim.requestFingerprint,
          bridge_epoch: claim.bridgeEpoch,
          playhead_ms: claim.outMs!,
        }, signal))
        if (!current()) return
        if (response?.ok && terminal(response.result)) { stopIfCurrent(); return }
      } catch { /* The server may have accepted Out before the reply was lost. */ }
      if (!current()) return
      try {
        const read = await serialized(signal => callVerb('voiceover.tick', {
          owner_session_id: claim.ownerSessionId, owner_capability: claim.ownerCapability,
        }, signal))
        if (!current()) return
        if (read?.ok && terminal(read.result)) { stopIfCurrent(); return }
      } catch { /* Keep the exact take and retry the observed Out below. */ }
      if (!current()) return
      if (attempts >= 5) { setOutUnconfirmed(true); return }
      timer = window.setTimeout(() => { void observe() }, Math.min(2_000, 250 * 2 ** (attempts - 1)))
    }
    void observe()
    return () => {
      retired = true
      if (timer !== null) window.clearTimeout(timer)
      if (inFlightRef.current?.claim === claim) inFlightRef.current.controller.abort()
    }
  }, [atOut, rate === 0, voiceoverPlayback])

  return { playback: voiceoverPlayback, outUnconfirmed }
}
