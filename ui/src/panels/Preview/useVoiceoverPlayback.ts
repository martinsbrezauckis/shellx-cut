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
}: UseVoiceoverPlaybackArgs): VoiceoverPlayback | null {
  const [voiceoverPlayback, setVoiceoverPlayback] = useState<VoiceoverPlayback | null>(null)
  const config = useRef({ durationMs, onSeek })
  config.current = { durationMs, onSeek }

  useEffect(() => {
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
      config.current.onSeek(previewPlaybackClockPosition(Number(startMs), config.current.durationMs, playback).positionMs)
      setVoiceoverPlayback(playback)
      setRate(1)
    }
    const onVoiceoverStop = () => {
      setRate(0)
      setVoiceoverPlayback(null)
    }
    document.addEventListener('cut:voiceover-playback', onVoiceoverPlayback)
    document.addEventListener('cut:voiceover-stop', onVoiceoverStop)
    return () => {
      document.removeEventListener('cut:voiceover-playback', onVoiceoverPlayback)
      document.removeEventListener('cut:voiceover-stop', onVoiceoverStop)
    }
  }, [setRate])

  const outReported = useRef<string | null>(null)
  useEffect(() => {
    if (!voiceoverPlayback || voiceoverPlayback.outMs === null || rate === 0) return
    if (playheadMs < voiceoverPlayback.outMs || outReported.current === voiceoverPlayback.requestId) return
    outReported.current = voiceoverPlayback.requestId
    void (async () => {
      try {
        const response = await callVerb('voiceover.observe_playhead', {
          owner_session_id: voiceoverPlayback.ownerSessionId,
          owner_capability: voiceoverPlayback.ownerCapability,
          request_id: voiceoverPlayback.requestId,
          request_fingerprint: voiceoverPlayback.requestFingerprint,
          bridge_epoch: voiceoverPlayback.bridgeEpoch,
          playhead_ms: playheadMs,
        })
        // A stale Out must never stop another tab's Preview locally. Only the
        // accepted server transition (or its terminal projection) owns that.
        if (response.ok && response.result) {
          const status = response.result as { phase?: string }
          if (['finishing', 'finished', 'placed'].includes(status.phase ?? '')) {
            document.dispatchEvent(new CustomEvent('cut:voiceover-stop'))
          }
        } else {
          outReported.current = null
        }
      } catch {
        outReported.current = null
      }
    })()
  }, [playheadMs, rate, setRate, voiceoverPlayback])

  return voiceoverPlayback
}
