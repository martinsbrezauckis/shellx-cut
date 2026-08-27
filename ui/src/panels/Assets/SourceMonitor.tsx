import { useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { callVerb, sourceUrl, type Project } from '../../lib/client'
import {
  overwriteSourceRange,
  overwriteSourceStill,
  placeLinkedAV,
  sourceOverwriteTrackTargets,
  STILL_OVERWRITE_DEFAULT_DURATION_MS,
  STILL_OVERWRITE_MAX_DURATION_MS,
  STILL_OVERWRITE_MIN_DURATION_MS,
} from '../../lib/placement'
import { laidToSharedEditorialPosition } from '../Timeline/layout'
import { revealRegisteredSource } from '../../lib/tauri'
import { requestSourceNavigation } from '../../app/sourceNavigation'
import { Icon } from '../../icons'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import { assetUsesFromSequenceIndex, type AssetUse } from './assetUses'
import SourceWaveform from './SourceWaveform'
import './source-monitor.css'

export interface SourceMonitorAsset {
  id: string
  name: string
  kind: 'video' | 'audio' | 'image'
  durationMs: number
  hasAudio: boolean
  proxy?: string
}

interface SourceMonitorProps {
  asset: SourceMonitorAsset
  project: Project
  playheadMs: number
  initialMs?: number
  onProjectChanged?: () => void | Promise<void>
  onClose: () => void
}

function formatTime(ms: number): string {
  const total = Math.max(0, Math.round(ms))
  const minutes = Math.floor(total / 60_000)
  const seconds = Math.floor((total % 60_000) / 1000)
  const millis = total % 1000
  return `${minutes}:${String(seconds).padStart(2, '0')}.${String(millis).padStart(3, '0')}`
}

export default function SourceMonitor({ asset, project, playheadMs, initialMs = 0, onProjectChanged, onClose }: SourceMonitorProps) {
  const mediaRef = useRef<HTMLMediaElement | null>(null)
  const overwriteButtonRef = useRef<HTMLButtonElement | null>(null)
  const overwriteFocusPending = useRef(false)
  const backdropArmedAt = useRef(Date.now() + 500)
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const initialSeekApplied = useRef(false)
  const [durationMs, setDurationMs] = useState(Math.max(0, asset.durationMs))
  const [stillDurationInput, setStillDurationInput] = useState(String(STILL_OVERWRITE_DEFAULT_DURATION_MS / 1000))
  const [currentMs, setCurrentMs] = useState(Math.max(0, initialMs))
  const [inMs, setInMs] = useState(0)
  const [outMs, setOutMs] = useState(Math.max(0, asset.durationMs))
  const [operation, setOperation] = useState<'insert' | 'overwrite' | null>(null)
  const [playing, setPlaying] = useState(false)
  const [note, setNote] = useState<string | null>(null)
  const [sourceFileBusy, setSourceFileBusy] = useState(false)
  const overwriteTargets = useMemo(() => sourceOverwriteTrackTargets(project), [project])
  const isStill = asset.kind === 'image'
  const sourceHasVideo = asset.kind === 'video' || isStill
  const sourceHasAudio = !isStill && (asset.kind === 'audio' || asset.hasAudio)
  const videoTargets = sourceHasVideo ? overwriteTargets.video : []
  const audioTargets = sourceHasAudio ? overwriteTargets.audio : []
  const [videoTarget, setVideoTarget] = useState<string | null>(() => videoTargets[0] ?? null)
  const [audioTarget, setAudioTarget] = useState<string | null>(() => audioTargets[0] ?? null)
  const [videoTargetTouched, setVideoTargetTouched] = useState(false)
  const [audioTargetTouched, setAudioTargetTouched] = useState(false)
  const busy = operation !== null
  const stillDurationMs = Math.round(Number(stillDurationInput) * 1000)
  const stillDurationValid = Number.isFinite(stillDurationMs)
    && stillDurationMs >= STILL_OVERWRITE_MIN_DURATION_MS
    && stillDurationMs <= STILL_OVERWRITE_MAX_DURATION_MS

  useEffect(() => {
    setVideoTarget((current) => {
      if (current && videoTargets.includes(current)) return current
      return videoTargetTouched ? null : videoTargets[0] ?? null
    })
  }, [videoTargetTouched, videoTargets])

  useEffect(() => {
    setAudioTarget((current) => {
      if (current && audioTargets.includes(current)) return current
      return audioTargetTouched ? null : audioTargets[0] ?? null
    })
  }, [audioTargetTouched, audioTargets])
  const [usesOpen, setUsesOpen] = useState(false)
  const [uses, setUses] = useState<AssetUse[] | null>(null)
  const [usesTruncated, setUsesTruncated] = useState(false)
  const [usesBusy, setUsesBusy] = useState(false)
  const [useBusy, setUseBusy] = useState<string | null>(null)
  const [usesNote, setUsesNote] = useState<string | null>(null)

  const syncDuration = () => {
    const seconds = mediaRef.current?.duration
    if (!seconds || !Number.isFinite(seconds)) return
    const measured = Math.max(0, Math.round(seconds * 1000))
    setDurationMs(measured)
    setOutMs((value) => value > 0 ? Math.min(value, measured) : measured)
    if (!initialSeekApplied.current && mediaRef.current) {
      const seekMs = Math.max(0, Math.min(measured - 1, Math.round(initialMs)))
      mediaRef.current.currentTime = seekMs / 1000
      setCurrentMs(seekMs)
      initialSeekApplied.current = true
    }
  }

  const markIn = () => {
    const next = Math.min(currentMs, Math.max(0, outMs - 1))
    setInMs(next)
    setNote(null)
  }

  const markOut = () => {
    const next = Math.max(currentMs, Math.min(durationMs, inMs + 1))
    setOutMs(next)
    setNote(null)
  }

  const togglePlayback = async () => {
    const media = mediaRef.current
    if (!media) return
    setNote(null)
    if (!media.paused) {
      media.pause()
      return
    }
    try {
      await media.play()
    } catch {
      setPlaying(false)
      setNote('Playback could not start for this source')
    }
  }

  const seekSource = (nextMs: number) => {
    const media = mediaRef.current
    if (!media || durationMs <= 0) return
    const next = Math.max(0, Math.min(durationMs, Math.round(nextMs)))
    media.currentTime = next / 1000
    // The native seek event arrives asynchronously. Update the Source Monitor
    // readout now so a waveform seek is immediate and then remains in sync.
    setCurrentMs(next)
    setNote(null)
  }

  const insert = async () => {
    const sourceIn = Math.max(0, Math.round(inMs))
    const sourceOut = Math.min(durationMs, Math.round(outMs))
    if (busy || sourceOut <= sourceIn) return
    setOperation('insert')
    setNote(null)
    try {
      const result = await placeLinkedAV({
        asset: asset.id,
        kind: asset.kind,
        at_ms: Math.max(0, Math.round(playheadMs)),
        src_range_ms: [sourceIn, sourceOut],
        ripple: true,
        rationale: `insert source range ${sourceIn}-${sourceOut}ms from ${asset.id}`,
        project,
      })
      if (!result.ok) {
        setNote(`Insert failed: ${result.error ?? 'error'}`)
      } else if (asset.kind === 'video' && asset.hasAudio && !result.audioLinked) {
        setNote('Video inserted; linked audio could not be added')
      } else {
        setNote(`Inserted ${formatTime(sourceOut - sourceIn)} at ${formatTime(playheadMs)}`)
      }
    } catch {
      setNote('Insert failed: the editor did not respond')
    } finally {
      setOperation(null)
    }
  }

  const markedRangeReady = durationMs > 0 && outMs > inMs
  const overwriteAudioTarget = isStill ? null : audioTarget
  const hasOverwriteTarget = !!videoTarget || !!overwriteAudioTarget
  const overwritePosition = useMemo(
    () => laidToSharedEditorialPosition(project, playheadMs, [videoTarget, overwriteAudioTarget]),
    [overwriteAudioTarget, playheadMs, project, videoTarget],
  )
  const overwriteState = busy
    ? 'busy'
    : isStill && !stillDurationValid
      ? 'duration-invalid'
      : !isStill && !markedRangeReady
        ? 'range-invalid'
        : !hasOverwriteTarget
          ? 'empty-target'
          : !overwritePosition.ok
            ? 'position-conflict'
            : 'ready'
  const overwriteDisabledReason = isStill && !stillDurationValid
    ? 'Set a still duration from 0.1 to 3,600 seconds'
    : !isStill && !markedRangeReady
    ? 'Mark a source range before overwriting'
    : !hasOverwriteTarget
      ? isStill ? 'Choose a video destination to overwrite' : 'Choose a V or A destination to overwrite'
      : !overwritePosition.ok
        ? overwritePosition.error
      : operation === 'insert'
        ? 'Insert range is running'
        : operation === 'overwrite'
          ? 'Overwrite is running'
          : ''

  useEffect(() => {
    if (operation !== null || !overwriteFocusPending.current) return
    // Effects run after React has committed the enabled button. Focusing from
    // the async handler's finally block can race that commit and silently
    // leave focus on document.body in native WebView2/WKWebView.
    overwriteFocusPending.current = false
    overwriteButtonRef.current?.focus({ preventScroll: true })
  }, [operation])

  const overwrite = async () => {
    const sourceIn = Math.max(0, Math.round(inMs))
    const sourceOut = Math.min(durationMs, Math.round(outMs))
    if (busy || !hasOverwriteTarget || !overwritePosition.ok || (!isStill && sourceOut <= sourceIn) || (isStill && !stillDurationValid)) return
    setOperation('overwrite')
    setNote(null)
    try {
      const result = isStill
        ? await overwriteSourceStill({
          asset: asset.id,
          atMs: overwritePosition.atMs,
          videoTrack: videoTarget,
          durationMs: stillDurationMs,
          rationale: `overwrite still for ${stillDurationMs}ms from ${asset.id}`,
        })
        : await overwriteSourceRange({
          asset: asset.id,
          atMs: overwritePosition.atMs,
          sourceRangeMs: [sourceIn, sourceOut],
          videoTrack: videoTarget,
          audioTrack: overwriteAudioTarget,
          rationale: `overwrite source range ${sourceIn}-${sourceOut}ms from ${asset.id}`,
        })
      if (!result.ok) {
        setNote(`Overwrite failed: ${result.error?.message ?? result.error?.code ?? 'error'}`)
      } else {
        const destinations = [videoTarget && `V ${videoTarget}`, !isStill && audioTarget && `A ${audioTarget}`].filter(Boolean).join(' + ')
        setNote(isStill
          ? `Overwrote ${destinations} for ${(stillDurationMs / 1000).toFixed(1)}s at ${formatTime(playheadMs)}`
          : `Overwrote ${destinations} at ${formatTime(playheadMs)}`)
      }
    } catch {
      setNote('Overwrite failed: the editor did not respond')
    } finally {
      overwriteFocusPending.current = true
      setOperation(null)
    }
  }

  const toggleUses = async () => {
    if (usesBusy) return
    if (usesOpen) {
      setUsesOpen(false)
      return
    }
    setUsesOpen(true)
    setUsesBusy(true)
    setUsesNote(null)
    try {
      // Filter by the stable asset id on the server before applying the bounded
      // result limit. Text search can match sequence/track/id/label collisions
      // that would otherwise crowd this asset's real occurrences out of 500.
      const result = await callVerb('project.sequence_index', { asset: asset.id, kind: 'clip', limit: 500 })
      if (!result.ok || !result.result) {
        setUses(null)
        setUsesTruncated(false)
        setUsesNote(result.error?.message ?? 'Could not load asset uses')
        return
      }
      const next = assetUsesFromSequenceIndex(result.result, asset.id)
      setUses(next.uses)
      setUsesTruncated(next.truncated)
    } catch {
      setUses(null)
      setUsesTruncated(false)
      setUsesNote('Server unreachable')
    } finally {
      setUsesBusy(false)
    }
  }

  const revealInSurface = (destination: 'project' | 'library') => {
    onClose()
    // Finish the dialog-close transaction before moving the app layout so the
    // target card is visible rather than mounted behind this blocking overlay.
    requestAnimationFrame(() => requestSourceNavigation(destination, asset.id))
  }

  const revealSourceFile = async () => {
    if (sourceFileBusy) return
    setSourceFileBusy(true)
    const reply = await revealRegisteredSource(asset.id)
    setSourceFileBusy(false)
    setNote(reply.message)
  }

  const openUse = async (use: AssetUse) => {
    if (useBusy) return
    if (use.offline) {
      setUsesNote('Relink this source before opening an offline occurrence')
      return
    }
    setUseBusy(use.clipId)
    setUsesNote(null)
    try {
      if (use.sequenceId !== (project.active_sequence ?? 'seq1')) {
        const switched = await callVerb('project.sequence_switch', {
          id: use.sequenceId,
          rationale: 'human: open Source Monitor asset use',
        })
        if (!switched.ok) {
          setUsesNote(switched.error?.message ?? 'Could not open that sequence')
          return
        }
        await onProjectChanged?.()
      }
      const jumped = await callVerb('ui.playhead', { at_ms: use.atMs })
      if (!jumped.ok) {
        // The UI command bridge intentionally reports an already-satisfied
        // playhead as a conflict. Verify that state instead of leaving a
        // successfully switched occurrence trapped behind this modal.
        const ui = await callVerb('ui.state', {})
        if (!ui.ok || ui.result?.playhead_ms !== use.atMs) {
          setUsesNote(jumped.error?.message ?? 'Could not move the playhead')
          return
        }
      }
      onClose()
    } catch {
      setUsesNote('Server unreachable')
    } finally {
      setUseBusy(null)
    }
  }

  const mediaProps = {
    ref: (node: HTMLMediaElement | null) => { mediaRef.current = node },
    className: 'source-monitor__media',
    src: asset.proxy ?? sourceUrl(asset.id),
    controls: true,
    preload: 'metadata' as const,
    onLoadedMetadata: syncDuration,
    onDurationChange: syncDuration,
    onTimeUpdate: () => setCurrentMs(Math.max(0, Math.round((mediaRef.current?.currentTime ?? 0) * 1000))),
    onSeeked: () => setCurrentMs(Math.max(0, Math.round((mediaRef.current?.currentTime ?? 0) * 1000))),
    onPlay: () => setPlaying(true),
    onPause: () => setPlaying(false),
    onEnded: () => setPlaying(false),
    onError: () => { setPlaying(false); setNote('This source cannot be played in the monitor') },
  }

  return createPortal(
    <div
      className="source-monitor__backdrop"
      data-cut-source-monitor-backdrop
      onMouseDown={(event) => {
        // A process-bound native menu click can finish after this portal mounts
        // on Linux/Wayland. Ignore only that short opening tail so one click
        // cannot both open and dismiss the monitor; ordinary click-away works
        // immediately afterward and Escape/Close remain active throughout.
        if (event.target === event.currentTarget && Date.now() < backdropArmedAt.current) {
          event.preventDefault()
          return
        }
        overlay.onScrimMouseDown(event)
      }}
    >
      <section
        ref={overlay.dialogRef}
        className="source-monitor"
        data-cut-source-monitor={asset.id}
        data-cut-source-monitor-kind={asset.kind}
        role="dialog"
        aria-modal="true"
        aria-labelledby="source-monitor-title"
        data-cut-blocking-overlay
        tabIndex={-1}
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={overlay.onDialogKeyDown}
      >
        <header className="source-monitor__header">
          <div className="source-monitor__identity">
            <span className="source-monitor__eyebrow">Source</span>
            <h2 id="source-monitor-title" title={asset.name}>{asset.name}</h2>
          </div>
          <button type="button" className="source-monitor__close" data-cut-source-monitor-close onClick={onClose} title="Close source monitor" aria-label="Close source monitor">
            <Icon name="close" size={16} />
          </button>
        </header>

        <div className={`source-monitor__stage source-monitor__stage--${asset.kind}`}>
          {asset.kind === 'video'
            ? <video {...mediaProps} playsInline />
            : asset.kind === 'audio'
              ? <audio {...mediaProps} />
              : <img className="source-monitor__still" data-cut-source-still-preview src={sourceUrl(asset.id)} alt={`Still preview: ${asset.name}`} onError={() => setNote('This still image could not be previewed in the monitor')} />}
        </div>

        {!isStill && <div className="source-monitor__readout" aria-live="polite">
          <span data-cut-source-current>{formatTime(currentMs)}</span>
          <button
            type="button"
            className="source-monitor__transport"
            data-cut-action="source-monitor-play"
            data-cut-source-play
            aria-label={playing ? 'Pause source' : 'Play source'}
            aria-pressed={playing}
            onClick={() => void togglePlayback()}
          >
            <Icon name={playing ? 'pause' : 'play'} size={14} />
            {playing ? 'Pause' : 'Play'}
          </button>
          <span>{formatTime(durationMs)}</span>
        </div>}

        {!isStill && sourceHasAudio && <SourceWaveform
          asset={asset.id}
          durationMs={durationMs}
          currentMs={currentMs}
          inMs={inMs}
          outMs={outMs}
          onSeek={seekSource}
        />}

        {!isStill && <div className="source-monitor__marks">
          <button type="button" data-cut-source-mark-in disabled={busy} onClick={markIn}>Mark In</button>
          <output data-cut-source-in>{formatTime(inMs)}</output>
          <span className="source-monitor__range">{formatTime(Math.max(0, outMs - inMs))}</span>
          <output data-cut-source-out>{formatTime(outMs)}</output>
          <button type="button" data-cut-source-mark-out disabled={busy} onClick={markOut}>Mark Out</button>
        </div>}

        <div className="source-monitor__targets" data-cut-source-targets data-cut-source-overwrite-state={overwriteState} aria-label="Overwrite destinations">
          <span className="source-monitor__targets-label">Overwrite to</span>
          <label className="source-monitor__target-control">
            <span>V</span>
            <select
              data-cut-action="source-target-video"
              data-cut-source-video-target
              data-cut-source-video-target-state={!sourceHasVideo ? 'source-unavailable' : videoTargets.length === 0 ? 'unavailable' : videoTarget ? 'targeted' : 'off'}
              value={videoTarget ?? ''}
              disabled={!sourceHasVideo || videoTargets.length === 0 || busy}
              aria-label="Video overwrite target"
              onChange={(event) => {
                setVideoTarget(event.target.value || null)
                setVideoTargetTouched(true)
                setNote(null)
              }}
            >
              <option value="">{!sourceHasVideo ? 'No source video' : videoTargets.length === 0 ? 'No unlocked V track' : 'Off'}</option>
              {videoTargets.map((track) => <option key={track} value={track}>{track}</option>)}
            </select>
          </label>
          {!isStill && <label className="source-monitor__target-control">
            <span>A</span>
            <select
              data-cut-action="source-target-audio"
              data-cut-source-audio-target
              data-cut-source-audio-target-state={!sourceHasAudio ? 'source-unavailable' : audioTargets.length === 0 ? 'unavailable' : audioTarget ? 'targeted' : 'off'}
              value={audioTarget ?? ''}
              disabled={!sourceHasAudio || audioTargets.length === 0 || busy}
              aria-label="Audio overwrite target"
              onChange={(event) => {
                setAudioTarget(event.target.value || null)
                setAudioTargetTouched(true)
                setNote(null)
              }}
            >
              <option value="">{!sourceHasAudio ? 'No source audio' : audioTargets.length === 0 ? 'No unlocked A track' : 'Off'}</option>
              {audioTargets.map((track) => <option key={track} value={track}>{track}</option>)}
            </select>
          </label>}
          {isStill && <label className="source-monitor__duration-control">
            <span>Duration</span>
            <input
              type="number"
              min={STILL_OVERWRITE_MIN_DURATION_MS / 1000}
              max={STILL_OVERWRITE_MAX_DURATION_MS / 1000}
              step="0.1"
              inputMode="decimal"
              data-cut-action="source-still-duration"
              data-cut-source-still-duration
              data-cut-source-still-duration-state={stillDurationValid ? 'valid' : 'invalid'}
              value={stillDurationInput}
              aria-label="Still overwrite duration in seconds"
              aria-describedby="source-still-duration-help"
              aria-invalid={!stillDurationValid}
              disabled={busy}
              onChange={(event) => { setStillDurationInput(event.target.value); setNote(null) }}
            />
            <span className="source-monitor__duration-unit">sec</span>
            <span id="source-still-duration-help" className="source-monitor__duration-help">0.1–3,600 seconds</span>
          </label>}
          <span className="source-monitor__target-status" data-cut-source-target-status role="status">
            {overwriteState === 'ready'
              ? isStill ? 'Still replaces content at the playhead.' : 'Selected source range replaces content at the playhead.'
              : overwriteDisabledReason}
          </span>
          {isStill && !stillDurationValid && <span className="source-monitor__duration-error" data-cut-source-still-duration-error role="alert">Set a still duration from 0.1 to 3,600 seconds.</span>}
        </div>

        {usesOpen && <section className="source-monitor__uses" data-cut-source-uses aria-label={`All uses of ${asset.name}`}>
          <div className="source-monitor__uses-head">
            <strong>All uses</strong>
            {uses && <span data-cut-source-use-count>{uses.length}</span>}
          </div>
          {usesBusy && <p className="source-monitor__uses-empty" data-cut-source-uses-loading>Loading uses...</p>}
          {!usesBusy && uses?.length === 0 && <p className="source-monitor__uses-empty">Not used in a sequence yet.</p>}
          {!usesBusy && uses?.map((use) => (
            <button
              type="button"
              className="source-monitor__use"
              data-cut-action="source-monitor-use"
              data-cut-source-use={`${use.sequenceId}:${use.trackId}:${use.clipId}`}
              key={`${use.sequenceId}:${use.trackId}:${use.clipId}`}
              disabled={!!useBusy || use.offline}
              title={use.offline
                ? 'Relink this source before opening an offline occurrence'
                : `Open ${use.sequenceName}, ${use.trackId}, at ${formatTime(use.atMs)}`}
              aria-description={use.offline ? 'Relink this source before opening an offline occurrence' : undefined}
              onClick={() => void openUse(use)}
            >
              <span>{use.sequenceName}{use.active ? ' · current' : ''}</span>
              <span>{use.trackId} · {formatTime(use.atMs)}</span>
              {useBusy === use.clipId && <span>Opening...</span>}
            </button>
          ))}
          {usesTruncated && <p className="source-monitor__uses-note" data-cut-source-uses-truncated>Only the first 500 matching clips were checked.</p>}
        </section>}

        <footer className="source-monitor__footer">
          <span className="source-monitor__target">Playhead {formatTime(playheadMs)}</span>
          {(note ?? usesNote) && <span className="source-monitor__note" data-cut-source-note>{note ?? usesNote}</span>}
          <button
            type="button"
            className="source-monitor__uses-button"
            data-cut-action="source-monitor-all-uses"
            data-cut-source-all-uses
            aria-expanded={usesOpen}
            disabled={usesBusy}
            onClick={() => void toggleUses()}
          >
            {usesBusy ? 'Loading...' : usesOpen ? 'Hide uses' : 'All uses'}
          </button>
          <div className="source-monitor__reveal-actions" role="group" aria-label="Reveal registered source">
            <button
              type="button"
              className="source-monitor__reveal"
              data-cut-action="reveal-source-project"
              data-cut-source-reveal-project={asset.id}
              title="Reveal this exact registered asset in Project Assets"
              onClick={() => revealInSurface('project')}
            >
              <Icon name="projectOpen" size={14} /> Project
            </button>
            <button
              type="button"
              className="source-monitor__reveal"
              data-cut-action="reveal-source-library"
              data-cut-source-reveal-library={asset.id}
              title="Reveal this exact registered asset in the cross-project Library"
              onClick={() => revealInSurface('library')}
            >
              <Icon name="folder" size={14} /> Library
            </button>
            <button
              type="button"
              className="source-monitor__reveal"
              data-cut-action="reveal-source-file"
              data-cut-source-reveal-file={asset.id}
              disabled={sourceFileBusy}
              title="Reveal this registered local source file in the desktop file manager"
              onClick={() => void revealSourceFile()}
            >
              <Icon name="file" size={14} /> {sourceFileBusy ? 'Revealing…' : 'Source file'}
            </button>
          </div>
          <div className="source-monitor__actions">
            {!isStill && <button
              type="button"
              className="source-monitor__insert"
              data-cut-source-insert
              disabled={busy || !markedRangeReady}
              onClick={() => void insert()}
            >
              <Icon name="plus" size={14} />
              {operation === 'insert' ? 'Inserting...' : 'Insert range'}
            </button>}
            <button
              ref={overwriteButtonRef}
              type="button"
              className="source-monitor__overwrite"
              data-cut-action="source-overwrite"
              data-cut-source-overwrite
              data-cut-source-overwrite-state={overwriteState}
              disabled={overwriteState !== 'ready'}
              title={overwriteDisabledReason || 'Replace the selected destinations at the playhead'}
              onClick={() => void overwrite()}
            >
              {operation === 'overwrite' ? 'Overwriting...' : isStill ? 'Overwrite still' : 'Overwrite range'}
            </button>
          </div>
        </footer>
      </section>
    </div>,
    document.body,
  )
}
