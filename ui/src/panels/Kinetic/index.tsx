// panels/Kinetic — the kinetic-captions drawer (0.5.0 UI for captions.kinetic).
// Role: a right-side drawer (the MusicBed drawer family) that drives ONE verb —
// captions.kinetic — to animate the existing transcript caption cues (each line
// pops in / fades out in sync with speech) as a native title overlay.
//
// PLACEMENT: captions are a promoted, near-one-tap action, and here the transcript IS
// the caption source. So the launch point is a button in the TRANSCRIPT panel
// header, right beside "Generate captions" (it dispatches cut:open-kinetic).
// This is the highest-frequency casual caption action, in its natural home.
//
// THE OVERLAP (honest): captions.kinetic READS the cap1 static cues to animate
// them, so without intervention the static burn-in AND the animated overlay
// both render = doubled captions. The "Replace static captions" toggle (default
// ON) passes replace_static:true so the engine clears the animated cap1 cues —
// kinetic shows ALONE. Off = the agent's kinetic-over-static behavior.
//
// TRUST STORY: no preview here. Fires the verb; the kinetic overlay composites
// through the existing overlay pipeline (a title track appears; the preview
// shows it). Receipt = the verb result (cue_count, cleared_static). Relay NOT
// claimed — one human client of an agent-callable verb.
//
// Callers: App.tsx (mounted when open). Deps: lib/client, ../drawer.css.

import { useMemo, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import '../drawer.css'

export interface KineticDrawerProps {
  project: Project | null
  onClose: () => void
}

const POSITIONS = [
  { id: 'bottom', label: 'Bottom (subtitle)' },
  { id: 'center', label: 'Center' },
  { id: 'top', label: 'Top' },
] as const
type Position = (typeof POSITIONS)[number]['id']

function positionFromInput(value: string, fallback: Position): Position {
  for (const option of POSITIONS) {
    if (option.id === value) return option.id
  }
  return fallback
}

/** The verb result we surface as the post-fire receipt (captions.kinetic). */
interface KineticResult {
  title_track: string
  asset_id: string
  clip_id: string
  cue_count: number
  range_ms: [number, number]
  cleared_static: number
}

function rangeMsFrom(v: unknown): [number, number] | null {
  if (!Array.isArray(v) || v.length !== 2 || typeof v[0] !== 'number' || typeof v[1] !== 'number') return null
  return [v[0], v[1]]
}

function stringField(v: object, name: string): string | null {
  const value = Reflect.get(v, name)
  return typeof value === 'string' ? value : null
}

function numberField(v: object, name: string): number | null {
  const value = Reflect.get(v, name)
  return typeof value === 'number' ? value : null
}

function kineticResultFrom(v: unknown): KineticResult | null {
  if (v === null || typeof v !== 'object') return null
  const titleTrack = stringField(v, 'title_track')
  const assetId = stringField(v, 'asset_id')
  const clipId = stringField(v, 'clip_id')
  const cueCount = numberField(v, 'cue_count')
  const rangeMs = rangeMsFrom(Reflect.get(v, 'range_ms'))
  const clearedStatic = numberField(v, 'cleared_static')
  if (!titleTrack || !assetId || !clipId || cueCount == null || !rangeMs || clearedStatic == null) return null
  return {
    title_track: titleTrack,
    asset_id: assetId,
    clip_id: clipId,
    cue_count: cueCount,
    range_ms: rangeMs,
    cleared_static: clearedStatic,
  }
}

/** Count static caption cues across ALL caption-kind tracks (the cues kinetic
 *  animates). Detect by `kind === 'caption'`, NOT a literal `cap1` id: a caption
 *  track created with any other id still holds animatable cues, and keying on the
 *  hardcoded `cap1` id falsely reported "No captions" and hid the Animate button
 *  (caption-track regression). The engine may treat `cap1` as the first caption track, but detection
 *  here is kind-based so every caption track counts. Returns null when the project
 *  has no caption track at all (drives the "generate captions first" empty state);
 *  otherwise the total cue count across caption tracks. */
function captionCueCount(project: Project | null): number | null {
  if (!project) return null
  const capTracks = project.tracks.filter((t) => t.kind === 'caption')
  if (capTracks.length === 0) return null
  return capTracks.reduce((n, t) => n + t.clips.filter((c) => 'text' in c).length, 0)
}

export default function KineticDrawer({ project, onClose }: KineticDrawerProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const cueCount = useMemo(() => captionCueCount(project), [project])
  const [position, setPosition] = useState<Position>('bottom')
  const [replaceStatic, setReplaceStatic] = useState(true)
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<KineticResult | null>(null)
  const [err, setErr] = useState<string | null>(null)

  // Esc closes (drawer family convention).

  // Kinetic needs caption cues to animate. null = no caption track yet.
  const hasCaptions = cueCount !== null && cueCount > 0

  const fire = async () => {
    if (!hasCaptions || busy) return
    setBusy(true)
    setErr(null)
    setResult(null)
    try {
      const r = await callVerb('captions.kinetic', {
        position,
        replace_static: replaceStatic,
        rationale: `user: kinetic captions (${position}${replaceStatic ? ', replace static' : ''})`,
      })
      const result = r.ok ? kineticResultFrom(r.result) : null
      if (result) {
        setResult(result)
        // Flip the Preview to COMPOSED so the kinetic overlay is visible (the raw
        // proxy never shows overlays) — makes the receipt below true.
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'captions.kinetic failed'}`)
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="cd-scrim" data-cut-kinetic-scrim onMouseDown={overlay.onScrimMouseDown}>
      <aside
        ref={overlay.dialogRef}
        className="cd-drawer"
        data-cut-kinetic
        data-cut-kinetic-open="true"
        role="dialog"
        aria-modal="true"
        aria-label="Kinetic captions"
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="cd-head">
          <div>
            <h2 className="cd-title">Kinetic captions</h2>
            <p className="cd-sub">Animate transcript captions so each line pops in and fades out in sync with speech.</p>
          </div>
          <button className="cd-btn cd-btn--ghost" data-cut-kinetic-close onClick={onClose}>
            Close
          </button>
        </header>

        <div className="cd-body">
          {!hasCaptions ? (
            // Teach the prerequisite verb in the empty state.
            <div className="cd-empty" data-cut-kinetic-empty>
              No captions to animate yet.
              <br />
              Run <code>Generate captions</code> in the transcript panel first — kinetic captions animate those cues.
            </div>
          ) : (
            <>
              <p className="cd-note" data-cut-kinetic-cuecount>
                {cueCount} caption cue{cueCount === 1 ? '' : 's'} ready to animate.
              </p>

              {/* position */}
              <label className="cd-field">
                <span className="cd-field-label">Position</span>
                <select
                  className="cd-sel"
                  data-cut-kinetic-position
                  value={position}
                  onChange={(e) => setPosition(positionFromInput(e.target.value, position))}
                >
                  {POSITIONS.map((p) => (
                    <option key={p.id} value={p.id}>{p.label}</option>
                  ))}
                </select>
              </label>

              {/* replace static — the overlap fix, default ON */}
              <label className="cd-toggle" data-cut-kinetic-replace-toggle>
                <input
                  type="checkbox"
                  data-cut-kinetic-replace
                  checked={replaceStatic}
                  onChange={(e) => setReplaceStatic(e.target.checked)}
                />
                <span className="cd-field-label">Replace the static captions</span>
              </label>
              <p className="cd-note">
                {replaceStatic
                  ? 'The static burn-in is removed so only the animated captions show (recommended).'
                  : 'The static burn-in STAYS — animated captions render ON TOP of it (both visible).'}
              </p>

              <button
                className="cd-btn cd-btn--primary"
                data-cut-kinetic-apply
                disabled={busy || !hasCaptions}
                onClick={() => void fire()}
              >
                {busy ? 'Animating…' : 'Animate captions'}
              </button>

              {err && (
                <div className="cd-err" data-cut-kinetic-error role="alert">{err}</div>
              )}

              {result && (
                <div className="cd-result" data-cut-kinetic-result>
                  <div className="cd-result-head">kinetic captions placed · {result.title_track}</div>
                  <dl className="cd-result-grid">
                    <dt>animated cues</dt>
                    <dd data-cut-kinetic-result-cues>{result.cue_count}</dd>
                    <dt>static cleared</dt>
                    <dd data-cut-kinetic-result-cleared>{result.cleared_static}</dd>
                    <dt>span</dt>
                    <dd>{(result.range_ms[0] / 1000).toFixed(1)}–{(result.range_ms[1] / 1000).toFixed(1)}s</dd>
                  </dl>
                  <div className="cd-result-foot">
                    See the <strong>{result.title_track}</strong> overlay on the timeline; scrub to watch the captions animate.
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
