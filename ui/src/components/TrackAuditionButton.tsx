import { useCallback, useEffect, useRef, useState } from 'react'
import { Icon } from '../icons'
import { callVerb, exportUrl } from '../lib/client'
import { trackAuditionExportError } from './trackAuditionModel'
import './trackAudition.css'

type AuditionState = 'idle' | 'busy' | 'playing' | 'error'
type AuditionSurface = 'timeline' | 'mixer'

interface ActiveAudition {
  token: symbol
  stop: () => void
}

let activeAudition: ActiveAudition | null = null

function playbackErrorMessage(error: unknown): string {
  if (error instanceof DOMException && error.name === 'NotAllowedError') {
    return 'Playback was blocked. Click Listen again to retry the ready track.'
  }
  return error instanceof Error && error.message
    ? `Could not play this track: ${error.message}`
    : 'Could not play this track.'
}

export function TrackAuditionButton({
  trackId,
  revisionKey,
  surface,
}: {
  trackId: string
  /** Project identity + latest edit id. A change makes any rendered stem stale. */
  revisionKey: string
  surface: AuditionSurface
}) {
  const [state, setState] = useState<AuditionState>('idle')
  const [error, setError] = useState<string | null>(null)
  const audioRef = useRef<HTMLAudioElement | null>(null)
  const tokenRef = useRef(Symbol(`track-audition:${surface}:${trackId}`))
  const requestRef = useRef(0)
  const mountedRef = useRef(true)
  const readyRef = useRef<{ revision: string } | null>(null)

  const clearActive = useCallback(() => {
    if (activeAudition?.token === tokenRef.current) activeAudition = null
  }, [])

  const stop = useCallback(() => {
    requestRef.current += 1
    const audio = audioRef.current
    if (audio) {
      audio.onended = null
      audio.onerror = null
      audio.pause()
    }
    clearActive()
    if (mountedRef.current) {
      setState('idle')
      setError(null)
    }
  }, [clearActive])

  const fail = useCallback((message: string, request: number, discardReady = false) => {
    if (!mountedRef.current || request !== requestRef.current) return
    if (discardReady) readyRef.current = null
    clearActive()
    setState('error')
    setError(message)
  }, [clearActive])

  const playReady = useCallback(async (request: number) => {
    const audio = audioRef.current
    if (!audio) return
    audio.currentTime = 0
    audio.onended = () => {
      if (!mountedRef.current || request !== requestRef.current) return
      clearActive()
      setState('idle')
      setError(null)
    }
    audio.onerror = () => fail('The rendered track audio could not be loaded.', request, true)
    try {
      await audio.play()
      if (!mountedRef.current || request !== requestRef.current) {
        audio.pause()
        return
      }
      setState('playing')
      setError(null)
    } catch (playbackError) {
      fail(playbackErrorMessage(playbackError), request)
    }
  }, [clearActive, fail])

  const toggle = useCallback(async () => {
    if (state === 'playing') {
      stop()
      return
    }
    if (state === 'busy') return

    activeAudition?.stop()
    const request = requestRef.current + 1
    requestRef.current = request
    activeAudition = { token: tokenRef.current, stop }
    setState('busy')
    setError(null)

    const cached = readyRef.current
    if (cached?.revision === revisionKey && audioRef.current?.src) {
      await playReady(request)
      return
    }

    try {
      const result = await callVerb('export.audio', {
        format: 'mp3',
        track: trackId,
        rationale: `${surface} per-track listen`,
      })
      if (!mountedRef.current || request !== requestRef.current || activeAudition?.token !== tokenRef.current) return
      const exportError = trackAuditionExportError(result)
      if (exportError) {
        fail(exportError, request, true)
        return
      }
      const path = (result.result as { path: string }).path
      const baseUrl = exportUrl(path)
      const url = `${baseUrl}${baseUrl.includes('?') ? '&' : '?'}v=${encodeURIComponent(revisionKey || '0')}`
      const audio = audioRef.current
      if (!audio) return
      readyRef.current = { revision: revisionKey }
      audio.src = url
      await playReady(request)
    } catch (exportError) {
      const message = exportError instanceof Error && exportError.message
        ? `Could not render this track: ${exportError.message}`
        : 'Could not render this track for listening.'
      fail(message, request, true)
    }
  }, [fail, playReady, revisionKey, state, stop, surface, trackId])

  useEffect(() => {
    requestRef.current += 1
    const audio = audioRef.current
    if (audio) {
      audio.onended = null
      audio.onerror = null
      audio.pause()
      audio.removeAttribute('src')
      audio.load()
    }
    readyRef.current = null
    clearActive()
    setState('idle')
    setError(null)
  }, [clearActive, revisionKey, trackId])

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
      requestRef.current += 1
      const audio = audioRef.current
      if (audio) {
        audio.onended = null
        audio.onerror = null
        audio.pause()
        audio.removeAttribute('src')
      }
      clearActive()
    }
  }, [clearActive])

  const title = state === 'playing'
    ? `Stop listening to ${trackId}`
    : state === 'busy'
      ? `Preparing ${trackId} for listening`
      : error || `Listen to ${trackId} only`
  const className = surface === 'mixer'
    ? `mx-btn mx-btn--listen${state === 'playing' ? ' mx-btn--on' : ''}${state === 'error' ? ' mx-btn--error' : ''}`
    : `tl-listen${state === 'playing' ? ' tl-listen--on' : ''}${state === 'error' ? ' tl-listen--error' : ''}`

  return (
    <>
      <button
        type="button"
        className={className}
        data-cut-action="track-listen"
        data-cut-listen-track={surface === 'timeline' ? trackId : undefined}
        data-cut-mixer-listen={surface === 'mixer' ? trackId : undefined}
        data-cut-audition-state={state}
        data-cut-audition-error={error || undefined}
        aria-label={title}
        aria-pressed={state === 'playing'}
        aria-busy={state === 'busy'}
        disabled={state === 'busy'}
        title={title}
        onMouseDown={surface === 'timeline' ? (event) => event.stopPropagation() : undefined}
        onClick={() => void toggle()}
      >
        <Icon
          name={state === 'error' ? 'warning' : state === 'busy' ? 'spinner' : state === 'playing' ? 'stop' : 'play'}
          size={14}
          className={state === 'busy' ? 'track-audition__spinner' : undefined}
        />
      </button>
      <audio ref={audioRef} hidden preload="none" />
    </>
  )
}
