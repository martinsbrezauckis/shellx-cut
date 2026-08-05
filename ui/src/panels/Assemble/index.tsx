// panels/Assemble — the "Assemble (AI)" drawer: the HUMAN UI for the agent-only
// assemble.* family. Four modes behind one drawer (mirrors Generate's
// provider/kind toggle):
//
//   • Auto-shorts (assemble.shorts)      — a transcribed source → the N best
//     short-worthy RANGES, each ranked by engagement with a suggested reframe +
//     captions. ANALYSIS only: returns candidate ranges to review/jump to; it
//     does NOT materialize a short (use the timeline tools for that). Default mode.
//   • Repurpose  (assemble.repurpose)   — a transcribed source → the N best
//     moments, ranked by the same engagement signal score.clip exposes. ANALYSIS
//     only: it returns candidate segments to review/jump to; it does NOT mutate
//     the timeline. (Distinct from the Clips drawer, which renders a publish pack
//     via clip.candidates → render.bundle.)
//   • From script (assemble.from_script) — paste a script (one line per point) →
//     each line matched to the best transcript span (token-overlap F1). ANALYSIS
//     only; review the matches + jump to them.
//   • B-roll     (assemble.broll)        — fill a timeline slot (query + position
//     + length) with a retrieved clip. This DOES place a clip (the orchestrator
//     runs assets.search/fetch + edit.insert); the new clip arrives via the
//     normal op_applied refresh and is a normal, undoable op.
//
// HONEST degradation mirrors the verbs: repurpose/from_script need the source
// TRANSCRIBED (an un-transcribed asset → the verb's error, shown verbatim); broll
// surfaces the first failing slot's step/error. No fabricated results. Every
// element carries data-cut-* for the debug API + interaction tests.
//
// Callers: App.tsx (activeDrawer === 'assemble'). Deps: lib/client (callVerb +
// Project/Asset types), ../drawer.css (shared cd-* styles).

import { useEffect, useMemo, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { Icon } from '../../icons'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import '../drawer.css'
import './assemble.css'

type Mode = 'shorts' | 'repurpose' | 'from_script' | 'broll'

export interface AssembleDrawerProps {
  project: Project | null
  /** Live playhead (ms) — the default position for a placed b-roll slot. */
  playheadMs?: number
  /** Jump the playhead to a result's start (App publishes it through ui.state). */
  onSeek?: (atMs: number) => void
  onClose: () => void
}

/** The fields we surface from each verb's result (local interfaces, house style —
 *  cf. Generate's GenResult; we read only what we render). */
interface RepurposeClip {
  rank: number
  range_ms: [number, number]
  duration_ms: number
  text: string
  score: number
  reason?: string
}
interface ScriptSegment {
  line_idx: number
  script_line: string
  matched: boolean
  score: number
  range_ms: [number, number] | null
  text: string
}
interface BrollPlaced {
  query: string
  at_ms: number
  duration_ms: number
}
interface ShortsItem {
  rank: number
  range_ms: [number, number]
  duration_ms: number
  score: number
  reason?: string
  title: string
  factors?: Record<string, number>
  reframe?: { aspect: string; crop: { x: number; y: number; w: number; h: number } | null }
  has_captions?: boolean
}

const MODES = [
  { id: 'shorts', label: 'Short ranges', icon: 'effect' },
  { id: 'repurpose', label: 'Best moments', icon: 'split' },
  { id: 'from_script', label: 'From script', icon: 'text' },
  { id: 'broll', label: 'B-roll', icon: 'videoClip' },
] as const

/** ms → m:ss timecode for the result rows. */
function fmtTc(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000))
  const m = Math.floor(s / 60)
  return `${m}:${String(s % 60).padStart(2, '0')}`
}

/** A score that may be 0..1 (factor) or 0..100 — show a clean percent either way. */
function fmtScore(v: number): string {
  return Math.abs(v) <= 1 ? `${Math.round(v * 100)}%` : `${Math.round(v)}`
}

export default function AssembleDrawer({ project, playheadMs = 0, onSeek, onClose }: AssembleDrawerProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const [mode, setMode] = useState<Mode>('shorts')
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [note, setNote] = useState<string | null>(null)

  // Picked source asset (repurpose / from_script). Default to the first
  // transcribed asset so the common case is one click.
  const assetOptions = useMemo(() => {
    const a = project?.assets ?? {}
    return Object.entries(a).map(([id, asset]) => ({
      id,
      name: asset.path.split(/[\\/]/).pop() || id,
      transcribed: !!asset.transcript,
    }))
  }, [project])
  const [asset, setAsset] = useState<string>('')
  const effectiveAsset = asset || assetOptions.find((o) => o.transcribed)?.id || assetOptions[0]?.id || ''

  // Repurpose params.
  const [count, setCount] = useState(5)
  const [targetS, setTargetS] = useState(30)
  const [prompt, setPrompt] = useState('')
  const [repurposeClips, setRepurposeClips] = useState<RepurposeClip[] | null>(null)

  // From-script params.
  const [script, setScript] = useState('')
  const [minScore, setMinScore] = useState(0.35)
  const [segments, setSegments] = useState<ScriptSegment[] | null>(null)

  // B-roll params.
  const [query, setQuery] = useState('')
  const [brollDir, setBrollDir] = useState('')
  const [brollAtS, setBrollAtS] = useState(Math.round(playheadMs / 1000))
  const [brollAtTouched, setBrollAtTouched] = useState(false)
  const [brollDurS, setBrollDurS] = useState(5)
  const [placed, setPlaced] = useState<BrollPlaced[] | null>(null)

  // Auto-shorts params.
  const [aspect, setAspect] = useState<'9:16' | '1:1' | '4:5' | '16:9'>('9:16')
  const [shorts, setShorts] = useState<ShortsItem[] | null>(null)

  const reset = () => {
    setErr(null)
    setNote(null)
    setRepurposeClips(null)
    setSegments(null)
    setPlaced(null)
    setShorts(null)
  }

  const switchMode = (m: Mode) => {
    setMode(m)
    reset()
  }

  useEffect(() => {
    if (!brollAtTouched) setBrollAtS(Math.max(0, Math.round(playheadMs / 1000)))
  }, [brollAtTouched, playheadMs])

  const jump = (range: [number, number] | null) => {
    if (range && onSeek) onSeek(range[0])
  }

  const runShorts = async () => {
    if (!effectiveAsset) { setErr('Import + transcribe a clip first — auto-shorts reads the transcript.'); return }
    setBusy(true); reset()
    try {
      const r = await callVerb('assemble.shorts', {
        asset: effectiveAsset,
        count,
        target_ms: Math.max(3000, targetS * 1000),
        aspect,
      })
      if (r.ok && r.result) {
        const res = (r.result as { shorts?: ShortsItem[] }).shorts ?? []
        setShorts(res)
        if (res.length === 0) setNote('No strong moments found in this source.')
      } else {
        setErr(r.error?.message ?? 'could not build shorts (is the source transcribed?)')
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }

  const runRepurpose = async () => {
    if (!effectiveAsset) { setErr('Import + transcribe a clip first — repurpose reads the transcript.'); return }
    setBusy(true); reset()
    try {
      const r = await callVerb('assemble.repurpose', {
        asset: effectiveAsset,
        count,
        target_ms: Math.max(3000, targetS * 1000),
        ...(prompt.trim() ? { prompt: prompt.trim() } : {}),
      })
      if (r.ok && r.result) {
        const clips = (r.result as { clips?: RepurposeClip[] }).clips ?? []
        setRepurposeClips(clips)
        if (clips.length === 0) setNote('No strong moments found in this source.')
      } else {
        setErr(r.error?.message ?? 'could not find moments (is the source transcribed?)')
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }

  const runFromScript = async () => {
    if (!effectiveAsset) { setErr('Import + transcribe a clip first — script matching reads the transcript.'); return }
    if (!script.trim()) { setErr('Paste a script first (one talking point per line).'); return }
    setBusy(true); reset()
    try {
      const r = await callVerb('assemble.from_script', {
        asset: effectiveAsset,
        script: script.trim(),
        min_score: minScore,
      })
      if (r.ok && r.result) {
        const res = r.result as { segments?: ScriptSegment[]; matched?: number; total_lines?: number }
        setSegments(res.segments ?? [])
        setNote(`Matched ${res.matched ?? 0} / ${res.total_lines ?? 0} lines.`)
      } else {
        setErr(r.error?.message ?? 'could not match the script (is the source transcribed?)')
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }

  const runBroll = async () => {
    if (!query.trim()) { setErr('Describe the b-roll to find (e.g. "city traffic at night").'); return }
    if (!brollDir.trim()) { setErr('Choose the folder to search for b-roll.'); return }
    if (!project) { setErr('Open a project first — the b-roll is placed on its timeline.'); return }
    setBusy(true); reset()
    try {
      const r = await callVerb('assemble.broll', {
        slots: [{ query: query.trim(), at_ms: Math.max(0, brollAtS * 1000), duration_ms: Math.max(1000, brollDurS * 1000) }],
        provider: 'local_folder',
        dir: brollDir.trim(),
        rationale: `human: fill b-roll slot "${query.trim()}"`,
      })
      if (r.ok && r.result) {
        const res = r.result as { status?: string; placed?: BrollPlaced[]; failed_step?: string; error?: string }
        if (res.status === 'failed') {
          setErr(`b-roll failed at ${res.failed_step ?? 'a step'}: ${res.error ?? 'unknown error'}`)
          setPlaced(res.placed ?? null)
        } else {
          setPlaced(res.placed ?? [])
          setNote(`Placed ${res.placed?.length ?? 0} b-roll clip(s). It's on the timeline (undoable).`)
        }
      } else {
        setErr(r.error?.message ?? 'b-roll search/fetch failed')
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }

  const needsAsset = mode === 'shorts' || mode === 'repurpose' || mode === 'from_script'

  return (
    <div className="cd-scrim" data-cut-assemble-scrim onMouseDown={overlay.onScrimMouseDown}>
      <aside
        ref={overlay.dialogRef}
        className="cd-drawer"
        data-cut-assemble
        data-cut-assemble-open="true"
        data-cut-assemble-mode={mode}
        role="dialog"
        aria-modal="true"
        aria-label="Assemble (AI)"
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="cd-head">
          <div>
            <h2 className="cd-title">Assemble (AI)</h2>
            <p className="cd-sub">
              Find short-worthy ranges, surface the best moments, match a script to the
              footage, or fill a slot with b-roll
              (assemble.shorts / repurpose / from_script / broll).
            </p>
          </div>
          <button className="cd-btn cd-btn--ghost" data-cut-assemble-close onClick={onClose}>Close</button>
        </header>

        <div className="cd-body" data-cut-assemble-body>
          {/* mode toggle */}
          <div className="cd-seg" role="tablist" data-cut-assemble-modes>
            {MODES.map((m) => (
              <button
                key={m.id}
                role="tab"
                aria-selected={mode === m.id}
                className={`cd-seg-btn ${mode === m.id ? 'cd-seg-btn--on' : ''}`}
                data-cut-assemble-mode-opt={m.id}
                onClick={() => switchMode(m.id)}
              >
                <Icon name={m.icon} size={14} tone="brand" /> {m.label}
              </button>
            ))}
          </div>

          {/* source asset (repurpose / from_script) */}
          {needsAsset && (
            <label className="cd-field">
              <span className="cd-field-label">Source (transcribed)</span>
              <select
                className="cd-sel"
                data-cut-assemble-asset
                value={effectiveAsset}
                disabled={assetOptions.length === 0}
                onChange={(e) => { setAsset(e.target.value); reset() }}
              >
                {assetOptions.length === 0 && <option value="">No assets — import + transcribe a clip</option>}
                {assetOptions.map((o) => (
                  <option key={o.id} value={o.id}>
                    {o.name}{o.transcribed ? '' : ' — (transcribe first)'}
                  </option>
                ))}
              </select>
            </label>
          )}

          {/* ── AUTO-SHORTS ───────────────────────────────────────────── */}
          {mode === 'shorts' && (
            <>
              <div className="cd-row">
                <label className="cd-field cd-field--inline">
                  <span className="cd-field-label">How many</span>
                  <input className="cd-input cd-input--num" type="number" min={1} max={50}
                    data-cut-assemble-count value={count}
                    onChange={(e) => setCount(Math.max(1, Math.min(50, Number(e.target.value) || 5)))} />
                </label>
                <label className="cd-field cd-field--inline">
                  <span className="cd-field-label">Length (s)</span>
                  <input className="cd-input cd-input--num" type="number" min={3} max={600}
                    data-cut-assemble-target value={targetS}
                    onChange={(e) => setTargetS(Math.max(3, Math.min(600, Number(e.target.value) || 30)))} />
                </label>
                <label className="cd-field cd-field--inline">
                  <span className="cd-field-label">Aspect</span>
                  <select className="cd-sel cd-sel--sm" data-cut-assemble-aspect value={aspect}
                    onChange={(e) => setAspect(e.target.value as typeof aspect)}>
                    <option value="9:16">9:16</option>
                    <option value="1:1">1:1</option>
                    <option value="4:5">4:5</option>
                    <option value="16:9">16:9</option>
                  </select>
                </label>
              </div>
              <button className="cd-btn cd-btn--primary" data-cut-assemble-run disabled={busy || !effectiveAsset}
                onClick={() => void runShorts()}>
                {busy ? 'Finding ranges…' : <><Icon name="effect" size={14} tone="brand" /> Find short-worthy ranges</>}
              </button>
              <p className="cd-note">Ranks the best moments by engagement and suggests a {aspect} reframe + captions for each. Materialize a short from its range with the timeline tools.</p>
            </>
          )}

          {/* ── REPURPOSE ─────────────────────────────────────────────── */}
          {mode === 'repurpose' && (
            <>
              <div className="cd-row">
                <label className="cd-field cd-field--inline">
                  <span className="cd-field-label">How many</span>
                  <input className="cd-input cd-input--num" type="number" min={1} max={50}
                    data-cut-assemble-count value={count}
                    onChange={(e) => setCount(Math.max(1, Math.min(50, Number(e.target.value) || 5)))} />
                </label>
                <label className="cd-field cd-field--inline">
                  <span className="cd-field-label">Target length (s)</span>
                  <input className="cd-input cd-input--num" type="number" min={3} max={600}
                    data-cut-assemble-target value={targetS}
                    onChange={(e) => setTargetS(Math.max(3, Math.min(600, Number(e.target.value) || 30)))} />
                </label>
              </div>
              <label className="cd-field">
                <span className="cd-field-label">Theme / keywords (optional)</span>
                <input className="cd-input" data-cut-assemble-prompt placeholder="e.g. product demo highlights"
                  value={prompt} onChange={(e) => setPrompt(e.target.value)} />
              </label>
              <button className="cd-btn cd-btn--primary" data-cut-assemble-run disabled={busy || !effectiveAsset}
                onClick={() => void runRepurpose()}>
                {busy ? 'Finding…' : <><Icon name="split" size={14} tone="brand" /> Find best moments</>}
              </button>
            </>
          )}

          {/* ── FROM SCRIPT ───────────────────────────────────────────── */}
          {mode === 'from_script' && (
            <>
              <label className="cd-field">
                <span className="cd-field-label">Script (one talking point per line)</span>
                <textarea className="cd-input cd-textarea" rows={5} data-cut-assemble-script
                  placeholder={'Welcome to the demo\nHere is the main feature\nAnd how to get started'}
                  value={script} onChange={(e) => setScript(e.target.value)} />
              </label>
              <label className="cd-field cd-field--inline">
                <span className="cd-field-label">Min match (0–1)</span>
                <input className="cd-input cd-input--num" type="number" min={0} max={1} step={0.05}
                  data-cut-assemble-minscore value={minScore}
                  onChange={(e) => setMinScore(Math.max(0, Math.min(1, Number(e.target.value) || 0.35)))} />
              </label>
              <button className="cd-btn cd-btn--primary" data-cut-assemble-run disabled={busy || !effectiveAsset}
                onClick={() => void runFromScript()}>
                {busy ? 'Matching…' : <><Icon name="text" size={14} tone="brand" /> Match script to footage</>}
              </button>
            </>
          )}

          {/* ── B-ROLL ────────────────────────────────────────────────── */}
          {mode === 'broll' && (
            <>
              <label className="cd-field">
                <span className="cd-field-label">What b-roll to find</span>
                <input className="cd-input" data-cut-assemble-query placeholder="e.g. city traffic at night"
                  value={query} onChange={(e) => setQuery(e.target.value)} />
              </label>
              <label className="cd-field">
                <span className="cd-field-label">Media folder</span>
                <input className="cd-input cd-input--mono" data-cut-assemble-dir placeholder="/path/to/video clips"
                  value={brollDir} onChange={(e) => setBrollDir(e.target.value)} />
              </label>
              <div className="cd-row">
                <label className="cd-field cd-field--inline">
                  <span className="cd-field-label">Place at (s)</span>
                  <input className="cd-input cd-input--num" type="number" min={0}
                    data-cut-assemble-at value={brollAtS}
                    onChange={(e) => {
                      setBrollAtTouched(true)
                      setBrollAtS(Math.max(0, Number(e.target.value) || 0))
                    }} />
                </label>
                <label className="cd-field cd-field--inline">
                  <span className="cd-field-label">Length (s)</span>
                  <input className="cd-input cd-input--num" type="number" min={1} max={120}
                    data-cut-assemble-dur value={brollDurS}
                    onChange={(e) => setBrollDurS(Math.max(1, Math.min(120, Number(e.target.value) || 5)))} />
                </label>
              </div>
              <button className="cd-btn cd-btn--primary" data-cut-assemble-run disabled={busy || !project}
                onClick={() => void runBroll()}>
                {busy ? 'Fetching…' : <><Icon name="videoClip" size={14} tone="media" /> Fill with b-roll</>}
              </button>
              <p className="cd-note">Searches that folder and inserts the matching clip at the chosen spot — a normal, undoable edit.</p>
            </>
          )}

          {err && <div className="cd-err" data-cut-assemble-error role="alert">{err}</div>}
          {note && <p className="cd-note" data-cut-assemble-note>{note}</p>}

          {/* ── RESULTS ───────────────────────────────────────────────── */}
          {shorts && shorts.length > 0 && (
            <div className="cd-results" data-cut-assemble-results="shorts">
              {shorts.map((s) => (
                <div className="cd-result-row" key={s.rank} data-cut-assemble-result={s.rank}>
                  <div className="cd-result-head">
                    <span className="cd-result-rank">#{s.rank}</span>
                    <span className="cd-result-tc">{fmtTc(s.range_ms[0])}–{fmtTc(s.range_ms[1])}</span>
                    <span className="cd-result-score">{fmtScore(s.score)}</span>
                    {onSeek && (
                      <button className="cd-btn cd-btn--ghost cd-btn--xs" data-cut-assemble-jump={s.rank}
                        onClick={() => jump(s.range_ms)}>Jump</button>
                    )}
                  </div>
                  <p className="cd-result-text">{s.title}</p>
                  <p className="cd-result-reason">
                    {Math.round(s.duration_ms / 1000)}s · {s.reframe?.aspect ?? aspect}
                    {s.has_captions ? ' · captions' : ''}
                    {s.factors ? ` · ${Object.entries(s.factors).map(([k, v]) => `${k.split('_')[0]} ${Math.round(v * 100)}`).join(' / ')}` : ''}
                  </p>
                </div>
              ))}
            </div>
          )}

          {repurposeClips && repurposeClips.length > 0 && (
            <div className="cd-results" data-cut-assemble-results="repurpose">
              {repurposeClips.map((c) => (
                <div className="cd-result-row" key={c.rank} data-cut-assemble-result={c.rank}>
                  <div className="cd-result-head">
                    <span className="cd-result-rank">#{c.rank}</span>
                    <span className="cd-result-tc">{fmtTc(c.range_ms[0])}–{fmtTc(c.range_ms[1])}</span>
                    <span className="cd-result-score">{fmtScore(c.score)}</span>
                    {onSeek && (
                      <button className="cd-btn cd-btn--ghost cd-btn--xs" data-cut-assemble-jump={c.rank}
                        onClick={() => jump(c.range_ms)}>Jump</button>
                    )}
                  </div>
                  <p className="cd-result-text">{c.text}</p>
                  {c.reason && <p className="cd-result-reason">{c.reason}</p>}
                </div>
              ))}
            </div>
          )}

          {segments && segments.length > 0 && (
            <div className="cd-results" data-cut-assemble-results="from_script">
              {segments.map((s) => (
                <div className={`cd-result-row ${s.matched ? '' : 'cd-result-row--unmatched'}`} key={s.line_idx}
                  data-cut-assemble-result={s.line_idx} data-cut-assemble-matched={String(s.matched)}>
                  <div className="cd-result-head">
                    <span className="cd-result-rank">{s.matched ? <Icon name="check" size={14} tone="success" /> : '—'}</span>
                    <span className="cd-result-line">{s.script_line}</span>
                    {s.matched && <span className="cd-result-score">{fmtScore(s.score)}</span>}
                    {s.matched && s.range_ms && onSeek && (
                      <button className="cd-btn cd-btn--ghost cd-btn--xs" data-cut-assemble-jump={s.line_idx}
                        onClick={() => jump(s.range_ms)}>Jump</button>
                    )}
                  </div>
                  {s.matched && s.text && <p className="cd-result-text">{s.text}</p>}
                </div>
              ))}
            </div>
          )}

          {placed && placed.length > 0 && (
            <div className="cd-results" data-cut-assemble-results="broll">
              {placed.map((p, i) => (
                <div className="cd-result-row" key={i} data-cut-assemble-result={i}>
                  <div className="cd-result-head">
                    <span className="cd-result-tc">@ {fmtTc(p.at_ms)}</span>
                    <span className="cd-result-line">{p.query}</span>
                    {onSeek && (
                      <button className="cd-btn cd-btn--ghost cd-btn--xs" data-cut-assemble-jump={i}
                        onClick={() => onSeek(p.at_ms)}>Jump</button>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      </aside>
    </div>
  )
}
