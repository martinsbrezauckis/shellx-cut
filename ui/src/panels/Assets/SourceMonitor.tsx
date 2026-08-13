import { useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { sourceUrl, type Project } from '../../lib/client'
import { placeLinkedAV } from '../../lib/placement'
import { Icon } from '../../icons'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import './source-monitor.css'

export interface SourceMonitorAsset {
  id: string
  name: string
  kind: 'video' | 'audio'
  durationMs: number
  hasAudio: boolean
  proxy?: string
}

interface SourceMonitorProps {
  asset: SourceMonitorAsset
  project: Project
  playheadMs: number
  initialMs?: number
  onClose: () => void
}

function formatTime(ms: number): string {
  const total = Math.max(0, Math.round(ms))
  const minutes = Math.floor(total / 60_000)
  const seconds = Math.floor((total % 60_000) / 1000)
  const millis = total % 1000
  return `${minutes}:${String(seconds).padStart(2, '0')}.${String(millis).padStart(3, '0')}`
}

export default function SourceMonitor({ asset, project, playheadMs, initialMs = 0, onClose }: SourceMonitorProps) {
  const mediaRef = useRef<HTMLMediaElement | null>(null)
  const backdropArmedAt = useRef(Date.now() + 500)
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const initialSeekApplied = useRef(false)
  const [durationMs, setDurationMs] = useState(Math.max(0, asset.durationMs))
  const [currentMs, setCurrentMs] = useState(Math.max(0, initialMs))
  const [inMs, setInMs] = useState(0)
  const [outMs, setOutMs] = useState(Math.max(0, asset.durationMs))
  const [busy, setBusy] = useState(false)
  const [playing, setPlaying] = useState(false)
  const [note, setNote] = useState<string | null>(null)

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

  const insert = async () => {
    const sourceIn = Math.max(0, Math.round(inMs))
    const sourceOut = Math.min(durationMs, Math.round(outMs))
    if (busy || sourceOut <= sourceIn) return
    setBusy(true)
    setNote(null)
    const result = await placeLinkedAV({
      asset: asset.id,
      kind: asset.kind,
      at_ms: Math.max(0, Math.round(playheadMs)),
      src_range_ms: [sourceIn, sourceOut],
      ripple: true,
      rationale: `insert source range ${sourceIn}-${sourceOut}ms from ${asset.id}`,
      project,
    })
    setBusy(false)
    if (!result.ok) {
      setNote(`Insert failed: ${result.error ?? 'error'}`)
    } else if (asset.kind === 'video' && asset.hasAudio && !result.audioLinked) {
      setNote('Video inserted; linked audio could not be added')
    } else {
      setNote(`Inserted ${formatTime(sourceOut - sourceIn)} at ${formatTime(playheadMs)}`)
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
            : <audio {...mediaProps} />}
        </div>

        <div className="source-monitor__readout" aria-live="polite">
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
        </div>

        <div className="source-monitor__marks">
          <button type="button" data-cut-source-mark-in onClick={markIn}>Mark In</button>
          <output data-cut-source-in>{formatTime(inMs)}</output>
          <span className="source-monitor__range">{formatTime(Math.max(0, outMs - inMs))}</span>
          <output data-cut-source-out>{formatTime(outMs)}</output>
          <button type="button" data-cut-source-mark-out onClick={markOut}>Mark Out</button>
        </div>

        <footer className="source-monitor__footer">
          <span className="source-monitor__target">Playhead {formatTime(playheadMs)}</span>
          {note && <span className="source-monitor__note" data-cut-source-note>{note}</span>}
          <button
            type="button"
            className="source-monitor__insert"
            data-cut-source-insert
            disabled={busy || durationMs <= 0 || outMs <= inMs}
            onClick={() => void insert()}
          >
            <Icon name="plus" size={14} />
            {busy ? 'Inserting...' : 'Insert range'}
          </button>
        </footer>
      </section>
    </div>,
    document.body,
  )
}
