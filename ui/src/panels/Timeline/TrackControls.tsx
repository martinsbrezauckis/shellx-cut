import { useCallback, useEffect, useState } from 'react'
import { TrackAuditionButton } from '../../components/TrackAuditionButton'
import { Icon } from '../../icons'
import { runUserVerb } from '../../lib/userActionFeedback'
import { trackOrderStatus, trackReorderTargetIndex, type TrackOrderTrack } from './trackControlsModel'

// Editable audio-track gain readout -> edit.gain{track,db}. The committed value
// always reads from server truth; the local draft exists only while focused.
export function GainControl({ trackId, db }: { trackId: string; db: number }) {
  const [draft, setDraft] = useState<string | null>(null)
  const commit = useCallback(
    (raw: string) => {
      const next = Number(raw)
      if (Number.isFinite(next) && Math.round(next * 10) !== Math.round(db * 10)) {
        void runUserVerb(
          'edit.gain',
          { track: trackId, db: next, rationale: `user set ${trackId} gain to ${next.toFixed(1)} dB` },
          `Could not change the level of track ${trackId}.`,
        )
      }
      setDraft(null)
    },
    [trackId, db],
  )
  return (
    <input
      className="tl-gain tl-gain--input"
      data-cut-action="set-gain"
      data-cut-gain-track={trackId}
      type="number"
      step={0.5}
      value={draft ?? db.toFixed(1)}
      title={`Track ${trackId} level — press Enter to apply`}
      onMouseDown={(e) => e.stopPropagation()}
      onFocus={(e) => { setDraft(db.toFixed(1)); e.currentTarget.select() }}
      onChange={(e) => setDraft(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') e.currentTarget.blur()
        else if (e.key === 'Escape') { setDraft(null); e.currentTarget.blur() }
      }}
      onBlur={(e) => commit(e.target.value)}
    />
  )
}

// Per-track audio mute is a non-destructive Track.muted flag (edit.mute), not a
// gain rewrite, so the user's gain level survives mute/unmute and reload.
export function MuteButton({ trackId, muted }: { trackId: string; muted: boolean }) {
  const [draftMuted, setDraftMuted] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    setDraftMuted(null)
    setBusy(false)
  }, [trackId, muted])
  const effectiveMuted = draftMuted ?? muted
  const toggle = useCallback(async () => {
    if (busy) return
    const next = !effectiveMuted
    setDraftMuted(next)
    setBusy(true)
    try {
      const r = await runUserVerb(
        'edit.mute',
        { track: trackId, on: next, rationale: `${next ? 'mute' : 'unmute'} track ${trackId}` },
        `Could not ${next ? 'mute' : 'unmute'} track ${trackId}.`,
      )
      if (!r?.ok) setDraftMuted(muted)
    } catch {
      setDraftMuted(muted)
    } finally {
      setBusy(false)
    }
  }, [busy, effectiveMuted, muted, trackId])
  return (
    <button
      type="button"
      className={`tl-mute${effectiveMuted ? ' tl-mute--on' : ''}`}
      data-cut-action="toggle-mute"
      data-cut-mute-track={trackId}
      data-cut-muted={effectiveMuted ? 'true' : undefined}
      title={effectiveMuted ? `track ${trackId} muted — click to unmute` : `mute track ${trackId}`}
      aria-pressed={effectiveMuted}
      disabled={busy}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={() => void toggle()}
    >
      <MuteIcon muted={effectiveMuted} />
    </button>
  )
}

export function SoloButton({ trackId, solo }: { trackId: string; solo: boolean }) {
  const [draftSolo, setDraftSolo] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    setDraftSolo(null)
    setBusy(false)
  }, [trackId, solo])
  const effectiveSolo = draftSolo ?? solo
  const toggle = useCallback(async () => {
    if (busy) return
    const next = !effectiveSolo
    setDraftSolo(next)
    setBusy(true)
    try {
      const r = await runUserVerb(
        'edit.solo',
        { track: trackId, on: next, rationale: `${next ? 'solo' : 'clear solo'} track ${trackId}` },
        `Could not ${next ? 'solo' : 'clear solo on'} track ${trackId}.`,
      )
      if (!r?.ok) setDraftSolo(solo)
    } catch {
      setDraftSolo(solo)
    } finally {
      setBusy(false)
    }
  }, [busy, effectiveSolo, solo, trackId])
  return (
    <button
      type="button"
      className={`tl-solo${effectiveSolo ? ' tl-solo--on' : ''}`}
      data-cut-action="toggle-solo"
      data-cut-solo-track={trackId}
      data-cut-soloed={effectiveSolo ? 'true' : undefined}
      title={effectiveSolo ? `track ${trackId} soloed — click to clear` : `solo track ${trackId}`}
      aria-pressed={effectiveSolo}
      disabled={busy}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={() => void toggle()}
    >
      S
    </button>
  )
}

export function TrackOrderControls({ tracks, trackId }: { tracks: TrackOrderTrack[]; trackId: string }) {
  const status = trackOrderStatus(tracks, trackId)
  if (!status || status.count <= 1) return null
  const move = (direction: 'back' | 'forward') => {
    const index = trackReorderTargetIndex(tracks, trackId, direction)
    if (index == null) return
    void runUserVerb('edit.reorder_track', {
      track: trackId,
      index,
      rationale: direction === 'forward' ? `bring ${trackId} forward in the layer stack` : `send ${trackId} back in the layer stack`,
    }, `Could not move track ${trackId} ${direction}.`)
  }
  return (
    <span className="tl-order-controls" data-cut-track-order={trackId}>
      <button
        type="button"
        className="tl-order"
        data-cut-action="track-send-back"
        data-cut-track-order-direction="back"
        disabled={!status.canMoveBack}
        title="Send this track backward in its stack"
        onMouseDown={(e) => e.stopPropagation()}
        onClick={() => move('back')}
      >
        <Icon name="chevronUp" size={14} />
      </button>
      <button
        type="button"
        className="tl-order"
        data-cut-action="track-bring-forward"
        data-cut-track-order-direction="forward"
        disabled={!status.canMoveForward}
        title="Bring this track forward in its stack"
        onMouseDown={(e) => e.stopPropagation()}
        onClick={() => move('forward')}
      >
        <Icon name="chevronDown" size={14} />
      </button>
    </span>
  )
}

export function TrackVisibilityButton({ trackId, visible }: { trackId: string; visible: boolean }) {
  const toggle = useCallback(() => {
    void runUserVerb('edit.track_visible', {
      track: trackId,
      on: !visible,
      rationale: `${visible ? 'hide' : 'show'} visual track ${trackId}`,
    }, `Could not ${visible ? 'hide' : 'show'} track ${trackId}.`)
  }, [trackId, visible])
  return (
    <button
      type="button"
      className={`tl-visibility${visible ? '' : ' tl-visibility--off'}`}
      data-cut-action="toggle-track-visibility"
      data-cut-visibility-track={trackId}
      data-cut-track-visible={visible ? 'true' : 'false'}
      title={visible ? `hide track ${trackId} in preview/export` : `show track ${trackId}`}
      aria-pressed={visible}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={toggle}
    >
      <Icon name={visible ? 'eye' : 'redact'} size={14} />
    </button>
  )
}

export function TrackLockButton({ trackId, locked }: { trackId: string; locked: boolean }) {
  const toggle = useCallback(() => {
    void runUserVerb('edit.track_lock', {
      track: trackId,
      on: !locked,
      rationale: `${locked ? 'unlock' : 'lock'} track ${trackId}`,
    }, `Could not ${locked ? 'unlock' : 'lock'} track ${trackId}.`)
  }, [trackId, locked])
  return (
    <button
      type="button"
      className={`tl-lock${locked ? ' tl-lock--on' : ''}`}
      data-cut-action="toggle-track-lock"
      data-cut-lock-track={trackId}
      data-cut-locked={locked || undefined}
      title={locked ? `track ${trackId} locked — click to unlock` : `lock track ${trackId}`}
      aria-pressed={locked}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={toggle}
    >
      <Icon name="lock" size={14} />
    </button>
  )
}

const PAN_PRESETS = [
  { value: -1, label: 'L' },
  { value: -0.5, label: 'L50' },
  { value: 0, label: 'C' },
  { value: 0.5, label: 'R50' },
  { value: 1, label: 'R' },
]

function closestPanPreset(pan: number): string {
  const best = PAN_PRESETS.reduce((acc, p) => (
    Math.abs(p.value - pan) < Math.abs(acc.value - pan) ? p : acc
  ), PAN_PRESETS[2])
  return String(best.value)
}

export function PanControl({ trackId, pan }: { trackId: string; pan: number }) {
  const value = closestPanPreset(Number.isFinite(pan) ? pan : 0)
  const commit = useCallback(
    (raw: string) => {
      const next = Number(raw)
      if (Number.isFinite(next) && Math.round(next * 100) !== Math.round((pan ?? 0) * 100)) {
        void runUserVerb('edit.pan', {
          track: trackId,
          pan: next,
          rationale: `user set ${trackId} pan to ${next}`,
        }, `Could not change the pan of track ${trackId}.`)
      }
    },
    [trackId, pan],
  )
  return (
    <select
      className="tl-pan"
      data-cut-action="set-pan"
      data-cut-pan-track={trackId}
      value={value}
      title={`Track ${trackId} pan or balance`}
      aria-label={`Pan ${trackId}`}
      onMouseDown={(e) => e.stopPropagation()}
      onChange={(e) => commit(e.target.value)}
    >
      {PAN_PRESETS.map((p) => (
        <option key={p.value} value={p.value}>{p.label}</option>
      ))}
    </select>
  )
}

// Audition one audio track stem from the timeline header. Read-only: it exports
// a temporary stem and plays it without mutating timeline state.
export function ListenButton({ trackId, revisionKey }: { trackId: string; revisionKey: string }) {
  return <TrackAuditionButton trackId={trackId} revisionKey={revisionKey} surface="timeline" />
}

function MuteIcon({ muted }: { muted: boolean }) {
  return <Icon name={muted ? 'mute' : 'volume'} size={14} />
}

export function KindIcon({ kind }: { kind: string }) {
  if (kind === 'video') return <Icon name="video" size={14} />
  if (kind === 'audio') return <Icon name="audioClip" size={14} />
  return <Icon name="captions" size={14} />
}
