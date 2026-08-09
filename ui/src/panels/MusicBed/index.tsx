// panels/MusicBed — the music-bed drawer.
// Role: a small settings DRAWER (the Environment-drawer family pattern) that
// drives ONE verb — audio.add_music. The user picks an already-imported audio
// asset, sets the bed gain, toggles auto-duck + its depth, and fires the verb;
// the engine places the bed on a dedicated track, auto-ducks it under the base
// track's speech (windowed gain RECORDED on the op), and surfaces beat markers
// from the bed's perception beat grid.
//
// The UI renders confirmed truth: the
// duck windows and beat markers this verb produces are RECORDED-ON-OP facts —
// the timeline already renders them verbatim (DuckStrip dimmed spans on the
// music lane + beat markers on the ruler) the moment op_applied lands. This
// drawer does NOT recompute or preview them; it fires the verb and POINTS at
// where the recorded result appears, then closes. After firing, it shows a
// short receipt of what the op recorded (ducked windows + beats marked) read
// straight from the verb's result — the same numbers the timeline draws.
//
// Placement decision: the grid is fixed at 4 panels + bars.
// Like the environment surfaces, this is a drawer launched from a top-bar button
// and relay-drivable is NOT claimed (audio.add_music has no ui.* relay; the
// drawer is a human convenience over a verb an agent calls directly). The
// verb is the contract; this is one client of it.
//
// Callers: App.tsx (mounted when open). Deps: lib/client (verbs); shares the
// unified `.cd-*` drawer shell (../drawer.css) — no bespoke .mb-* CSS.

import { useEffect, useMemo, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import '../drawer.css'

export interface MusicBedProps {
  project: Project | null
  onClose: () => void
}

/** The verb result we surface as the post-fire receipt (audio.add_music). */
interface AddMusicResult {
  track_id: string
  bed_clip: string
  bed_gain_db: number
  bed_duration_ms: number
  ducked_windows: number
  beats_marked: number
  created_track: boolean
}

/** An audio asset offered as a bed candidate: id + a short label. The probe
 * (media.probe result, stored on the asset) tells us kind=audio OR kind=video
 * with audio. We offer audio-kind assets (a music file imported for the bed);
 * video assets are the footage, not a bed. */
interface BedCandidate {
  id: string
  label: string
  kind: string
}

function bedCandidates(project: Project | null): BedCandidate[] {
  if (!project) return []
  const out: BedCandidate[] = []
  for (const [id, asset] of Object.entries(project.assets ?? {})) {
    const probe = asset.probe as { kind?: string } | undefined
    const kind = probe?.kind ?? 'unknown'
    // Only audio-kind assets are bed material (a music/loop file). Video and
    // image assets are footage/stills — not a music bed.
    if (kind !== 'audio') continue
    // Label = the file basename (path tail), falling back to the asset id.
    const base = asset.path ? asset.path.split('/').pop() || id : id
    out.push({ id, label: base, kind })
  }
  return out
}

const BED_GAIN_DEFAULT = -18 // the bed sits clearly under the VO (verb default)
const DUCK_DB_DEFAULT = -15 // reduction inside speech windows (verb default)

export default function MusicBed({ project, onClose }: MusicBedProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const candidates = useMemo(() => bedCandidates(project), [project])
  const [assetId, setAssetId] = useState<string>('')
  const [bedGain, setBedGain] = useState<number>(BED_GAIN_DEFAULT)
  const [duckOn, setDuckOn] = useState<boolean>(true)
  const [duckDb, setDuckDb] = useState<number>(DUCK_DB_DEFAULT)
  const [beatMarkers, setBeatMarkers] = useState<boolean>(true)
  const [busy, setBusy] = useState(false)
  const [muteBusy, setMuteBusy] = useState(false)
  const [result, setResult] = useState<AddMusicResult | null>(null)
  const [err, setErr] = useState<string | null>(null)

  // N1 "mute original audio": the original footage audio lives on the base
  // audio track (auto-place puts it on the first audio track, `a1t`). This is a
  // convenience copy of the track mute operation, so it uses the persisted
  // muted flag and preserves any dialed-in gain level.
  const baseAudio = useMemo(
    () => (project?.tracks ?? []).find((t) => t.kind === 'audio'),
    [project],
  )
  const originalMuted = baseAudio?.muted === true
  const toggleOriginalMute = async () => {
    if (muteBusy || !baseAudio) return
    setMuteBusy(true)
    setErr(null)
    try {
      const r = await callVerb('edit.mute', {
        track: baseAudio.id,
        on: !originalMuted,
        rationale: originalMuted ? 'unmute original audio' : 'mute original audio under the music bed',
      })
      if (!r.ok) setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.mute failed'}`)
    } catch {
      setErr('server unreachable')
    } finally {
      setMuteBusy(false)
    }
  }

  // Default-select the first candidate when the list arrives.
  useEffect(() => {
    if (!assetId && candidates.length) setAssetId(candidates[0].id)
  }, [candidates, assetId])

  // Esc closes (family convention).

  const fire = async () => {
    if (!assetId) return
    setBusy(true)
    setErr(null)
    setResult(null)
    try {
      const r = await callVerb('audio.add_music', {
        asset: assetId,
        bed_gain_db: bedGain,
        // duck:false skips; an object tunes the depth (against_track default =
        // the base speech track on the engine side).
        duck: duckOn ? { db: duckDb } : false,
        beat_markers: beatMarkers,
        rationale: `user: add music bed ${assetId} @ ${bedGain} dB${duckOn ? `, duck ${duckDb} dB under speech` : ', no duck'}`,
      })
      if (r.ok) {
        setResult(r.result as AddMusicResult)
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'audio.add_music failed'}`)
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="cd-scrim" data-cut-musicbed-scrim onMouseDown={overlay.onScrimMouseDown}>
      <aside
        ref={overlay.dialogRef}
        className="cd-drawer"
        data-cut-musicbed
        data-cut-musicbed-open="true"
        role="dialog"
        aria-modal="true"
        aria-label="Music bed"
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="cd-head">
          <div>
            <h2 className="cd-title">Music bed</h2>
            <p className="cd-sub">
              Place imported music under your video and lower it automatically while someone speaks.
            </p>
          </div>
          <button className="cd-btn cd-btn--ghost" data-cut-musicbed-close onClick={onClose}>
            Close
          </button>
        </header>

        <div className="cd-body">
          {candidates.length === 0 ? (
            // Teach the verb that fills this empty state.
            <div className="cd-empty" data-cut-musicbed-empty>
              No music imported yet.
              <br />
              Open Assets and import an audio file, then return here.
            </div>
          ) : (
            <>
              {/* asset picker */}
              <label className="cd-field">
                <span className="cd-field-label">Music file</span>
                <select
                  className="cd-sel"
                  data-cut-musicbed-asset
                  value={assetId}
                  onChange={(e) => setAssetId(e.target.value)}
                >
                  {candidates.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.label}
                    </option>
                  ))}
                </select>
              </label>

              {/* bed gain */}
              <label className="cd-field">
                <span className="cd-field-label">
                  Music level <span className="cd-val" data-cut-musicbed-bedgain>{bedGain} dB</span>
                </span>
                <input
                  className="cd-range"
                  data-cut-musicbed-bedgain-input
                  type="range"
                  min={-40}
                  max={0}
                  step={1}
                  value={bedGain}
                  onChange={(e) => setBedGain(Number(e.target.value))}
                />
              </label>

              {/* duck toggle + depth */}
              <div className="cd-field">
                <label className="cd-toggle" data-cut-musicbed-duck-toggle>
                  <input
                    type="checkbox"
                    data-cut-musicbed-duck
                    checked={duckOn}
                    onChange={(e) => setDuckOn(e.target.checked)}
                  />
                  <span className="cd-field-label">Lower music under speech</span>
                </label>
                {duckOn && (
                  <label className="cd-duck-depth">
                    <span className="cd-field-label">
                      Reduce music by <span className="cd-val" data-cut-musicbed-duckdb>{Math.abs(duckDb)} dB</span>
                    </span>
                    <input
                      className="cd-range"
                      data-cut-musicbed-duckdb-input
                      type="range"
                      min={-30}
                      max={-1}
                      step={1}
                      value={duckDb}
                      onChange={(e) => setDuckDb(Number(e.target.value))}
                    />
                  </label>
                )}
              </div>

              {/* beat markers toggle */}
              <label className="cd-toggle" data-cut-musicbed-beats-toggle>
                <input
                  type="checkbox"
                  data-cut-musicbed-beats
                  checked={beatMarkers}
                  onChange={(e) => setBeatMarkers(e.target.checked)}
                />
                <span className="cd-field-label">Mark beats on the ruler</span>
              </label>

              {/* N1: mute the original footage audio (its own track) so the
                  bed can stand alone. Fires immediately; reflects server truth
                  so it survives reload without changing the track level. */}
              {baseAudio && (
                <label
                  className="cd-toggle"
                  data-cut-musicbed-mute-original-toggle
                  title={`toggle the original footage audio track (${baseAudio.id})`}
                >
                  <input
                    type="checkbox"
                    data-cut-musicbed-mute-original
                    checked={originalMuted}
                    disabled={muteBusy}
                    onChange={() => void toggleOriginalMute()}
                  />
                  <span className="cd-field-label">
                    Mute original video audio
                  </span>
                </label>
              )}

              {/* the trust note — the recorded result appears on the timeline */}
              <p className="cd-note cd-note--readable">
                Speech reductions and beat markers are saved with the project and shown directly on the timeline.
              </p>

              <button
                className="cd-btn cd-btn--primary"
                data-cut-musicbed-apply
                disabled={busy || !assetId}
                aria-busy={busy}
                onClick={() => void fire()}
              >
                {busy ? 'Placing…' : 'Add music bed'}
              </button>

              {err && (
                <div className="cd-err" data-cut-musicbed-error role="alert">
                  {err}
                </div>
              )}

              {/* post-fire receipt: exactly what the op recorded (the same
                  numbers the timeline now draws — not a separate computation) */}
              {result && (
                <div className="cd-result" data-cut-musicbed-result role="status" aria-live="polite">
                  <div className="cd-result-head">Music placed{result.created_track ? ' on a new track' : ''}</div>
                  <dl className="cd-result-grid">
                    <dt>music level</dt>
                    <dd>{result.bed_gain_db} dB</dd>
                    <dt>duration</dt>
                    <dd>{(result.bed_duration_ms / 1000).toFixed(1)}s</dd>
                    <dt>speech reductions</dt>
                    <dd data-cut-musicbed-result-windows>{result.ducked_windows}</dd>
                    <dt>beat markers</dt>
                    <dd data-cut-musicbed-result-beats>{result.beats_marked}</dd>
                  </dl>
                  <div className="cd-result-foot">
                    The timeline now shows the music track, speech reductions, and beat markers.
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      </aside>
    </div>
  )
}
