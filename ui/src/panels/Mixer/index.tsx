// panels/Mixer — the audio MIXER drawer (per-track levels, mute, solo).
//
// Role: a track mixer over the existing per-track gain controls.
// Every AUDIO track gets a fader (edit.gain, dB), a Mute, and a Solo. Imported
// video sound is linked onto its own audio track; video tracks contribute pixels
// only, so exposing audio controls on them would store edits the mix never reads.
//
// MUTE and SOLO are NON-DESTRUCTIVE per-track FLAGS (Track.muted / Track.solo),
// toggled by the `edit.mute` / `edit.solo` verbs. The fader gain (edit.gain, dB) is
// INDEPENDENT — mute/solo never touch it, so a dialed-in level survives mute and a
// reload (this replaced an older mute that wrote -100 dB into the gain and lost the
// level on reload — mute/solo regression). The renderer resolves audibility at mix time
// (Project::audio_track_audible): a track plays iff !muted && (no track soloed ||
// this track soloed). Solo is now MULTI (any number of tracks can be soloed at
// once), derived from server truth — no local solo state. The displayed gain is the
// server's `gain_db`; a fader holds only a transient drag-draft committed on
// release (one edit.gain op per drag, not per tick).
//
// Caller: AppRightRail (the persistent Audio tab). Deps: lib/client (callVerb),
// ../drawer.css (shared embedded-shell styles) + ./mixer.css.
import { useCallback, useEffect, useRef, useState } from 'react'
import { TrackAuditionButton } from '../../components/TrackAuditionButton'
import { callVerb, exportUrl, type Project, type Track } from '../../lib/client'
import { publishUserActionFailure, publishUserActionMessage, runUserVerb } from '../../lib/userActionFeedback'
import { StripMeter } from './StripMeter'
import { mixerLoudnessKey, mixerMeasurementKey, mixerProjectScope, mixerSnapshotMatches } from './mixerIdentity'
import '../drawer.css'
import './mixer.css'

/** Don't decode a stem longer than this for the live meter (memory guard). */
const MAX_METER_MS = 12 * 60 * 1000

const MIN_DB = -60
const MAX_DB = 12

/** Integrated-loudness targets the Measure control offers (verify.loudness).
 *  Mirrors the engine's reference_targets: social -14, long-form/podcast -16,
 *  EBU R128 broadcast -23. -14 is the default (social), matching the verb. */
const LUFS_TARGETS: { value: number; label: string }[] = [
  { value: -14, label: '-14 social' },
  { value: -16, label: '-16 long-form' },
  { value: -23, label: '-23 broadcast' },
]

/** The source-only slice of the verify.loudness receipt shown in the Mixer. */
interface LoudnessReading {
  asset: string
  integrated_lufs: number
  target_lufs: number
}

export default function MixerDrawer({
  project,
  playheadMs,
  headOpId,
}: {
  project: Project | null
  /** Live playhead (ms) — the per-track meters sample each stem here. */
  playheadMs: number
  /** Latest applied op id — invalidates the decoded stems on every edit. */
  headOpId: string
}) {
  // The render graph mixes TrackKind::Audio only. Imported video sound is linked
  // to a sibling audio track, so video strips here would be non-functional.
  const tracks = (project?.tracks ?? []).filter(
    (t) => t.kind === 'audio' && t.clips.length > 0,
  )
  const audioTrackIds = tracks.map((t) => t.id)
  const [lufsTarget, setLufsTarget] = useState<number>(-14)
  const measurementKey = mixerMeasurementKey(project, headOpId, audioTrackIds)
  const loudnessKey = mixerLoudnessKey(measurementKey, lufsTarget)
  const currentMeasurementKey = useRef(measurementKey)
  currentMeasurementKey.current = measurementKey
  const currentLoudnessKey = useRef(loudnessKey)
  currentLoudnessKey.current = loudnessKey
  // SOLO is server truth now (Track.solo) — multiple tracks can be soloed. anySolo
  // drives the audibility derivation (a non-soloed track is silent while any solo
  // is active), mirroring the engine's Project::audio_track_audible.
  const anySolo = tracks.some((t) => t.solo)
  // track id → live fader value WHILE dragging (committed on release).
  const [draft, setDraft] = useState<Record<string, number>>({})
  const [trackToggleBusy, setTrackToggleBusy] = useState<Record<string, boolean>>({})
  const trackToggleBusyRef = useRef<Record<string, boolean>>({})
  // --- Source loudness (verify.loudness) -------------------------------------
  // The peak/RMS meters show the edited stem at the playhead. verify.loudness
  // measures one whole source asset; its receipt cannot prove edited-track LUFS.
  // One shared target selector drives every measurement (-14 social default).
  // Track id → source reading, scoped to the current project and revision.
  const [loudness, setLoudness] = useState<{ key: string; readings: Record<string, LoudnessReading> }>({ key: '', readings: {} })
  const [loudBusy, setLoudBusy] = useState<{ key: string; track: string } | null>(null)
  const currentLoudness = loudness.key === loudnessKey ? loudness.readings : {}

  /** The source asset id a track's loudness measures: the first MEDIA clip's
   *  asset (audio clips carry an `asset`; gaps do not). One asset
   *  per track is the honest unit — verify.loudness measures a source file, and a
   *  track is usually one stem / one source. Null = nothing to measure. */
  const trackAsset = useCallback((t: Track): string | null => {
    for (const c of t.clips) if ('asset' in c && c.asset) return c.asset
    return null
  }, [])

  /** Measure one track's first source asset against the selected target. */
  const measureLoudness = useCallback(
    async (t: Track) => {
      const asset = trackAsset(t)
      if (!asset || loudBusy?.key === loudnessKey) return
      const requestKey = loudnessKey
      setLoudBusy({ key: requestKey, track: t.id })
      try {
        const r = await callVerb('verify.loudness', {
          asset,
          target_lufs: lufsTarget,
        })
        if (currentLoudnessKey.current !== requestKey) return
        if (!r.ok) {
          publishUserActionFailure('verify.loudness', r, `Could not measure source loudness for track ${t.id}.`)
          return
        }
        const res = r.result as Partial<LoudnessReading> | undefined
        if (res == null || typeof res.integrated_lufs !== 'number') return
        setLoudness((m) => ({
          key: requestKey,
          readings: { ...(m.key === requestKey ? m.readings : {}), [t.id]: {
            asset,
            integrated_lufs: res.integrated_lufs as number,
            target_lufs: res.target_lufs ?? lufsTarget,
          } },
        }))
      } catch {
        if (currentLoudnessKey.current === requestKey) {
          publishUserActionMessage(`Could not measure source loudness for track ${t.id}: the local engine is unreachable.`)
        }
      } finally {
        setLoudBusy((busy) => busy?.key === requestKey ? null : busy)
      }
    },
    [trackAsset, lufsTarget, loudBusy, loudnessKey],
  )

  // --- per-track meters (v2b) ------------------------------------------------
  // Each AUDIO track's STEM (export.audio{track}) is the track's exact mix
  // contribution (stems sum to the full mix bit-for-bit). Decode each stem ONCE per
  // edit, then sample it at the dead-reckoned playhead — no extra audio playback, so
  // it adds no sound and is fully headless-verifiable. (Video tracks don't feed the
  // engine audio mix, so they get no live meter.)
  const ctxRef = useRef<AudioContext | null>(null)
  const meterMountedRef = useRef(true)
  useEffect(() => {
    meterMountedRef.current = true
    return () => {
      meterMountedRef.current = false
      const ctx = ctxRef.current
      ctxRef.current = null
      if (ctx && ctx.state !== 'closed') void ctx.close().catch(() => {})
    }
  }, [])
  const [stems, setStems] = useState<Map<string, { channels: Float32Array[]; sampleRate: number }>>(
    new Map(),
  )
  const stemsForOp = useRef<string>('__unset__')
  const projectKey = mixerProjectScope(project)

  // Dead-reckoned timeline clock: the playhead prop updates ~10 Hz; between updates
  // we extrapolate while it is advancing so the meters move smoothly at 60 fps.
  const phRef = useRef({ ms: 0, at: 0, playing: false })
  const timeRef = useRef(0)
  useEffect(() => {
    const now = performance.now()
    const prev = phRef.current
    const advancing = playheadMs > prev.ms && now - prev.at < 400
    phRef.current = { ms: playheadMs, at: now, playing: advancing }
    if (!advancing) timeRef.current = playheadMs
  }, [playheadMs])
  useEffect(() => {
    let raf = 0
    const tick = () => {
      const { ms, at, playing } = phRef.current
      timeRef.current = playing ? ms + (performance.now() - at) : ms
      raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(raf)
  }, [])
  const getTimeMs = useCallback(() => timeRef.current, [])

  // Fetch + decode each audio track's stem when the edit (headOpId) or the track
  // set changes. Long timelines are skipped (memory guard) → that track shows no meter.
  const trackKey = audioTrackIds.join(',')
  useEffect(() => {
    const op = headOpId || '0'
    if (stemsForOp.current === measurementKey) return
    let cancelled = false
    const abort = new AbortController()
    const ids = trackKey ? trackKey.split(',') : []
    const run = async () => {
      if (!meterMountedRef.current || cancelled) return
      if (!ids.length) {
        setStems(new Map())
        stemsForOp.current = measurementKey
        return
      }
      if (!ctxRef.current) {
        const Ctx =
          window.AudioContext ||
          (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
        if (!Ctx) return
        ctxRef.current = new Ctx()
      }
      const ctx = ctxRef.current
      const next = new Map<string, { channels: Float32Array[]; sampleRate: number }>()
      for (const id of ids) {
        if (cancelled || !meterMountedRef.current || currentMeasurementKey.current !== measurementKey) return
        try {
          const r = await callVerb('export.audio', {
            format: 'wav',
            track: id,
            rationale: 'mixer per-track meter stem',
          })
          if (cancelled || currentMeasurementKey.current !== measurementKey) return
          if (!r.ok) continue
          const res = r.result as { path?: string; duration_ms?: number }
          if (!res?.path || (res.duration_ms ?? 0) > MAX_METER_MS) continue
          const currentProject = await callVerb('project.state', {})
          const currentSnapshot = currentProject.ok ? (currentProject.result as Project | null) ?? null : null
          if (cancelled || currentMeasurementKey.current !== measurementKey || !currentProject.ok ||
              !mixerSnapshotMatches(project, currentSnapshot)) return
          const url = exportUrl(res.path)
          const ab = await fetch(
            `${url}${url.includes('?') ? '&' : '?'}v=${encodeURIComponent(op)}`,
            { signal: abort.signal },
          ).then((x) => x.arrayBuffer())
          if (cancelled || currentMeasurementKey.current !== measurementKey) return
          const audio = await ctx.decodeAudioData(ab)
          if (cancelled || !meterMountedRef.current || currentMeasurementKey.current !== measurementKey) return
          const channels: Float32Array[] = []
          for (let c = 0; c < audio.numberOfChannels; c++) channels.push(audio.getChannelData(c))
          if (!cancelled && currentMeasurementKey.current === measurementKey) next.set(id, { channels, sampleRate: audio.sampleRate })
        } catch {
          /* stem unavailable / decode failed → this track simply gets no meter */
        }
      }
      if (!cancelled && currentMeasurementKey.current === measurementKey) {
        setStems(next)
        stemsForOp.current = measurementKey
      }
    }
    void run()
    return () => {
      cancelled = true
      abort.abort()
    }
  }, [measurementKey, trackKey])

  const setGain = useCallback((trackId: string, db: number, why: string) => {
    void runUserVerb('edit.gain', { track: trackId, db, rationale: why }, `Could not change the level of track ${trackId}.`)
  }, [])

  // Pan/balance (edit.pan) — commit-on-release like the fader; the slider
  // holds only a transient drag-draft. Double-click snaps back to center.
  const setPan = useCallback((trackId: string, pan: number) => {
    const label = pan === 0 ? 'center' : pan > 0 ? `R${Math.round(pan * 100)}` : `L${Math.round(-pan * 100)}`
    void runUserVerb('edit.pan', { track: trackId, pan, rationale: `set ${trackId} pan ${label}` }, `Could not change the pan of track ${trackId}.`)
  }, [])
  const [panDraft, setPanDraft] = useState<Record<string, number>>({})

  // Server truth — NON-DESTRUCTIVE flags (Track.muted / Track.solo). Mute/solo no
  // longer touch the gain, so the fader level is independent and survives a reload.
  const isMuted = (t: Track) => !!t.muted

  // Audible IN THE MIX (mirror of the engine rule): plays iff not muted AND
  // (nothing soloed OR this track soloed). Drives the meter's active state so a
  // soloed-out / muted strip reads as silenced.
  const isAudible = (t: Track) => !t.muted && (!anySolo || !!t.solo)

  const setTrackToggle = useCallback((key: string, on: boolean) => {
    const next = { ...trackToggleBusyRef.current }
    if (on) next[key] = true
    else delete next[key]
    trackToggleBusyRef.current = next
    setTrackToggleBusy(next)
  }, [])

  const toggleTrackFlag = useCallback(async (verb: 'mute' | 'solo', t: Track) => {
    const key = `${verb}:${t.id}`
    if (trackToggleBusyRef.current[key]) return
    setTrackToggle(key, true)
    try {
      if (verb === 'mute') {
        await runUserVerb('edit.mute', {
          track: t.id,
          on: !t.muted,
          rationale: `${t.muted ? 'unmute' : 'mute'} ${t.id}`,
        }, `Could not ${t.muted ? 'unmute' : 'mute'} track ${t.id}.`)
      } else {
        await runUserVerb('edit.solo', {
          track: t.id,
          on: !t.solo,
          rationale: `${t.solo ? 'un-solo' : 'solo'} ${t.id}`,
        }, `Could not ${t.solo ? 'clear solo on' : 'solo'} track ${t.id}.`)
      }
    } finally {
      setTrackToggle(key, false)
    }
  }, [setTrackToggle])

  // Toggle the mute FLAG (edit.mute). One op; gain untouched.
  const toggleMute = (t: Track) => {
    void toggleTrackFlag('mute', t)
  }

  // Toggle the solo FLAG (edit.solo) — independent per track (multi-solo).
  const toggleSolo = (t: Track) => {
    void toggleTrackFlag('solo', t)
  }

  const body = (
    <>
        <div className="mx-body">
          {tracks.length === 0 && (
            <p className="mx-empty">No audio tracks yet — add one below, or import media.</p>
          )}
          {/* Loudness target — the integrated-LUFS standard the "Measure loudness"
              buttons check each track against (verify.loudness). -14 social is the
              engine default; -16 long-form/podcast; -23 EBU R128 broadcast. */}
          {tracks.length > 0 && (
            <div className="mx-loud-target" data-cut-mixer-loud-target>
              <span className="mx-loud-target-label">Loudness target</span>
              <select
                className="mx-loud-select"
                data-cut-mixer-loud-target-select
                value={lufsTarget}
                title="Choose the target used when measuring loudness"
                onChange={(e) => setLufsTarget(Number(e.target.value))}
              >
                {LUFS_TARGETS.map((t) => (
                  <option key={t.value} value={t.value}>
                    {t.label}
                  </option>
                ))}
              </select>
            </div>
          )}
          {/* Add-audio-track. Video-track
              add already lives in the Layer panel + drop-below-lanes; this is the
              missing audio half (edit.add_track{kind:'audio'}). */}
          <button
            type="button"
            className="mx-addtrack"
            data-cut-mixer-add-audio
            title="Add a new audio track"
            onClick={() => void runUserVerb(
              'edit.add_track',
              { kind: 'audio', rationale: 'user: add audio track (mixer)' },
              'Could not add an audio track.',
            )}
          >
            + Add audio track
          </button>
          {tracks.map((t) => {
            const muted = isMuted(t)
            const soloed = !!t.solo
            const live = draft[t.id]
            // The fader ALWAYS shows the server gain (mute/solo no longer touch it).
            const db = live ?? (t.gain_db ?? 0)
            return (
              <div
                key={t.id}
                className={`mx-strip${muted ? ' mx-strip--muted' : ''}${soloed ? ' mx-strip--solo' : ''}`}
                data-cut-mixer-track={t.id}
                data-cut-muted={muted || undefined}
                data-cut-solo={soloed || undefined}
              >
                <div className="mx-strip-head">
                  <span className="mx-kind" aria-hidden="true">
                    {t.kind === 'audio' ? '♪' : '🎬'}
                  </span>
                  <span className="mx-name">{t.id}</span>
                  <span className="mx-dbval" data-cut-mixer-db={t.id}>
                    {db.toFixed(1)} dB
                  </span>
                  {t.kind === 'audio' && (
                    <StripMeter
                      channels={stemsForOp.current === measurementKey ? stems.get(t.id)?.channels ?? null : null}
                      sampleRate={stemsForOp.current === measurementKey ? stems.get(t.id)?.sampleRate ?? 48000 : 48000}
                      getTimeMs={getTimeMs}
                      active={isAudible(t)}
                    />
                  )}
                </div>
                <input
                  className="mx-fader"
                  data-cut-mixer-fader={t.id}
                  type="range"
                  min={MIN_DB}
                  max={MAX_DB}
                  step={0.5}
                  value={Math.max(MIN_DB, Math.min(MAX_DB, db))}
                  title={`${t.id} level — release to apply`}
                  onChange={(e) => setDraft((d) => ({ ...d, [t.id]: Number(e.target.value) }))}
                  onPointerUp={(e) => {
                    const v = Number((e.target as HTMLInputElement).value)
                    setGain(t.id, v, `set ${t.id} level ${v.toFixed(1)} dB`)
                    setDraft((d) => {
                      const n = { ...d }
                      delete n[t.id]
                      return n
                    })
                  }}
                />
                {/* Pan / balance (edit.pan): L...C...R slider, non-destructive
                    Track.pan flag. Center = unity mix; double-click recenters. */}
                {(() => {
                  const pan = panDraft[t.id] ?? (t.pan ?? 0)
                  const panLabel = pan === 0 ? 'C' : pan > 0 ? `R${Math.round(pan * 100)}` : `L${Math.round(-pan * 100)}`
                  const commitPan = (v: number) => {
                    if (v !== (t.pan ?? 0)) setPan(t.id, v)
                    setPanDraft((d) => {
                      const n = { ...d }
                      delete n[t.id]
                      return n
                    })
                  }
                  return (
                    <div className="mx-pan" data-cut-mixer-pan-row={t.id}>
                      <span className="mx-pan-edge" aria-hidden="true">L</span>
                      <input
                        className="mx-pan-slider"
                        data-cut-mixer-pan={t.id}
                        type="range"
                        min={-1}
                        max={1}
                        step={0.05}
                        value={pan}
                        title={`${t.id} pan or balance — release to apply; double-click to centre`}
                        aria-label={`${t.id} pan`}
                        onChange={(e) => setPanDraft((d) => ({ ...d, [t.id]: Number(e.target.value) }))}
                        onPointerUp={(e) => commitPan(Number((e.target as HTMLInputElement).value))}
                        onDoubleClick={() => commitPan(0)}
                      />
                      <span className="mx-pan-edge" aria-hidden="true">R</span>
                      <span className="mx-pan-val" data-cut-mixer-pan-val={t.id}>{panLabel}</span>
                    </div>
                  )
                })()}
                <div className="mx-btns">
                  <TrackAuditionButton
                    trackId={t.id}
                    revisionKey={`${projectKey}:${headOpId}`}
                    surface="mixer"
                  />
                  <button
                    type="button"
                    className={`mx-btn${muted ? ' mx-btn--on' : ''}`}
                    data-cut-mixer-mute={t.id}
                    aria-pressed={muted}
                    disabled={!!trackToggleBusy[`mute:${t.id}`]}
                    title={muted ? `unmute ${t.id}` : `mute ${t.id}`}
                    onClick={() => toggleMute(t)}
                  >
                    M
                  </button>
                  <button
                    type="button"
                    className={`mx-btn mx-btn--solo${soloed ? ' mx-btn--on' : ''}`}
                    data-cut-mixer-solo={t.id}
                    aria-pressed={soloed}
                    disabled={!!trackToggleBusy[`solo:${t.id}`]}
                    title={soloed ? `un-solo ${t.id}` : `solo ${t.id} (silence non-soloed tracks)`}
                    onClick={() => toggleSolo(t)}
                  >
                    S
                  </button>
                </div>
                {/* verify.loudness measures the first source asset, not the edited stem. */}
                {(() => {
                  const asset = trackAsset(t)
                  const reading = currentLoudness[t.id]
                  const sourceName = asset ? (project?.assets[asset]?.path.split(/[\\/]/).pop() || asset) : null
                  const sourceGap = reading ? reading.integrated_lufs - reading.target_lufs : null
                  const sourceWithin = sourceGap != null && Math.abs(sourceGap) <= 1
                  return (
                    <div className="mx-loud" data-cut-mixer-loud={t.id}>
                      <button
                        type="button"
                        className="mx-loud-btn"
                        data-cut-action="verify-loudness"
                        data-cut-mixer-loud-measure={t.id}
                        disabled={!asset || (loudBusy?.key === loudnessKey && loudBusy.track === t.id)}
                        title={
                          asset
                            ? `Measure first source asset ${sourceName} on ${t.id}; edited-track loudness is not measured`
                            : `${t.id} has no source asset to measure`
                        }
                        onClick={() => void measureLoudness(t)}
                      >
                        {loudBusy?.key === loudnessKey && loudBusy.track === t.id ? 'Measuring…' : 'Measure source LUFS'}
                      </button>
                      {reading ? (
                        <span
                          className={`mx-loud-badge${sourceWithin ? ' mx-loud-badge--ok' : ' mx-loud-badge--off'}`}
                          data-cut-mixer-loudness-lufs={t.id}
                          data-cut-loudness-scope="source"
                          data-cut-loudness-asset={reading.asset}
                          data-cut-loudness-source-lufs={reading.integrated_lufs.toFixed(1)}
                          data-cut-loudness-lufs={reading.integrated_lufs.toFixed(1)}
                          data-cut-loudness-source-within={sourceWithin ? 'true' : 'false'}
                          title={`Source ${sourceName ?? reading.asset} (${reading.asset}) · whole asset, before timeline edits · target ${reading.target_lufs} LUFS`}
                        >
                          {sourceName ?? reading.asset}: {reading.integrated_lufs.toFixed(1)} LUFS source
                          {sourceGap != null && (
                            <span className="mx-loud-gap">
                              {sourceGap >= 0 ? '+' : ''}
                              {sourceGap.toFixed(1)} LU to target
                            </span>
                          )}
                        </span>
                      ) : (
                        <span className="mx-loud-badge mx-loud-badge--empty" data-cut-mixer-loudness-lufs={t.id} data-cut-loudness-lufs="">
                          — LUFS
                        </span>
                      )}
                    </div>
                  )
                })()}
              </div>
            )
          })}
        </div>
    </>
  )

  return (
    <section className="cd-embed mx-drawer" data-cut-mixer data-cut-mixer-open="true" data-cut-mixer-embed aria-label="Audio mixer">
      {body}
    </section>
  )
}
