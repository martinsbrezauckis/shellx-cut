// director/DirectorModal — the agent-first DIRECTOR surface for subject-aware
// reframe. Role: drive the foundation-model
// director loop from the UI — render.direct builds a per-scene CONTACT SHEET, the
// user (or the driving agent) picks WHICH subject each shot follows, render.reframe
// executes with that brief, and render.qc reviews the output. This is the human
// face of the same verbs an agent drives headless; the CV does the dense tracking,
// the director decides the editorial "which subject" call the whale editors lack.
// Callers: topbar (Format → "Direct…"). Deps: lib/client (verbs + director types).

import { useCallback, useEffect, useRef, useState } from 'react'
import {
  callVerb,
  type DirectorResult,
  type QcResult,
  type VerbResults,
} from '../lib/client'
import { Icon } from '../icons'
import { useBlockingOverlay } from '../components/overlay/useBlockingOverlay'
import './director.css'

export interface DirectorModalProps {
  /** Target aspect (e.g. "9:16") — the reframe geometry. */
  aspect: string
  /** Subject class preset (talking_head/sports/pets/cars/general). */
  preset: 'talking_head' | 'sports' | 'pets' | 'cars' | 'general'
  onClose: () => void
}

type Phase = 'directing' | 'pick' | 'rendering' | 'done' | 'reviewing' | 'error'
/** Per-scene pick: auto = CV ranker (omit from direction); widen; or a candidate cx. */
type Pick = { kind: 'auto' } | { kind: 'widen' } | { kind: 'cx'; cx: number; label: string }

/** Poll a job to completion; returns the job's `result` payload. */
async function pollJob(
  jobId: string,
  onProgress?: (p: number) => void,
): Promise<unknown> {
  for (;;) {
    const r = await callVerb('jobs.status', { job_id: jobId })
    const res = r.result as VerbResults['jobs.status'] | undefined
    if (onProgress && typeof res?.progress === 'number') onProgress(res.progress)
    if (res?.state === 'done') return (res as { result?: unknown }).result
    if (res?.state === 'failed') {
      throw new Error(res.error?.message ?? res.error?.code ?? 'job failed')
    }
    await new Promise((resolve) => setTimeout(resolve, 700))
  }
}

export default function DirectorModal({ aspect, preset, onClose }: DirectorModalProps) {
  const overlay = useBlockingOverlay<HTMLDivElement>(onClose)
  const [phase, setPhase] = useState<Phase>('directing')
  const [err, setErr] = useState<string | null>(null)
  const [director, setDirector] = useState<DirectorResult | null>(null)
  const [picks, setPicks] = useState<Record<number, Pick>>({})
  const [reframeId, setReframeId] = useState<string | null>(null)
  const [qc, setQc] = useState<QcResult | null>(null)
  const [progress, setProgress] = useState(0)
  const cancelled = useRef(false)

  // Step 1 — render.direct → the contact sheet. Runs once on open.
  useEffect(() => {
    cancelled.current = false
    void (async () => {
      try {
        const r = await callVerb('render.direct', { preset })
        const jobId = (r.result as { job_id?: string } | undefined)?.job_id
        if (!jobId) throw new Error(r.error?.message ?? r.error?.code ?? 'render.direct rejected')
        const out = (await pollJob(jobId, (p) => !cancelled.current && setProgress(p))) as DirectorResult
        if (cancelled.current) return
        setDirector(out)
        const init: Record<number, Pick> = {}
        out.scenes?.forEach((s) => { init[s.scene] = { kind: 'auto' } })
        setPicks(init)
        setPhase('pick')
      } catch (e) {
        if (!cancelled.current) { setErr(e instanceof Error ? e.message : String(e)); setPhase('error') }
      }
    })()
    return () => { cancelled.current = true }
  }, [preset])

  const setPick = useCallback((scene: number, pick: Pick) => {
    setPicks((prev) => ({ ...prev, [scene]: pick }))
  }, [])

  // Step 2 — render.reframe with the director brief built from non-auto picks.
  const render = useCallback(async () => {
    if (!director) return
    setPhase('rendering'); setProgress(0); setErr(null)
    const direction: Record<string, { cx?: number; mode?: 'widen' }> = {}
    for (const [scene, pick] of Object.entries(picks)) {
      if (pick.kind === 'cx') direction[scene] = { cx: pick.cx }
      else if (pick.kind === 'widen') direction[scene] = { mode: 'widen' }
      // 'auto' → omit → the CV ranker decides that scene.
    }
    try {
      const r = await callVerb('render.reframe', {
        aspect,
        preset,
        ...(Object.keys(direction).length ? { direction } : {}),
        rationale: 'director (UI)',
      })
      const result = r.result as { job_id?: string; reframe_id?: string } | undefined
      if (!result?.job_id) throw new Error(r.error?.message ?? r.error?.code ?? 'reframe rejected')
      await pollJob(result.job_id, (p) => setProgress(p))
      setReframeId(result.reframe_id ?? null)
      setQc(null)
      setPhase('done')
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e)); setPhase('error')
    }
  }, [director, picks, aspect, preset])

  // Step 3 — render.qc reviews the reframed output.
  const review = useCallback(async () => {
    if (!reframeId) return
    setPhase('reviewing'); setProgress(0); setErr(null)
    try {
      const r = await callVerb('render.qc', { reframe_id: reframeId, preset })
      const jobId = (r.result as { job_id?: string } | undefined)?.job_id
      if (!jobId) throw new Error(r.error?.message ?? r.error?.code ?? 'qc rejected')
      const out = (await pollJob(jobId, (p) => setProgress(p))) as QcResult
      setQc(out)
      setPhase('done')
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e)); setPhase('error')
    }
  }, [reframeId, preset])

  const pct = Math.round(progress * 100)

  return (
    <div className="dir-overlay" data-cut-director onMouseDown={overlay.onScrimMouseDown}>
      <div ref={overlay.dialogRef} className="dir-modal" role="dialog" aria-modal="true" aria-label="Smart reframe director"
        data-cut-blocking-overlay tabIndex={-1} onKeyDown={overlay.onDialogKeyDown}>
        <header className="dir-head">
          <span className="dir-title">Smart Reframe → <b>{aspect}</b> <span className="dir-sub">director · {preset}</span></span>
          <button className="dir-x" data-cut-director-close onClick={onClose} aria-label="Close">×</button>
        </header>

        {(phase === 'directing' || phase === 'rendering' || phase === 'reviewing') && (
          <div className="dir-busy" data-cut-director-busy={phase}>
            <div className="dir-spinner" />
            <div className="dir-busy-label">
              {phase === 'directing' ? 'Analyzing scenes…' : phase === 'rendering' ? 'Reframing…' : 'Reviewing output…'}
            </div>
            <div className="dir-bar"><div className="dir-bar-fill" style={{ width: `${Math.max(6, pct)}%` }} /></div>
          </div>
        )}

        {phase === 'error' && (
          <div className="dir-error" data-cut-director-error>
            <div className="dir-error-msg"><Icon name="warning" size={16} tone="warn" /> {err}</div>
            <button className="dir-btn" data-cut-director-error-close onClick={onClose}>Close</button>
          </div>
        )}

        {phase === 'pick' && director && (
          <div className="dir-pick" data-cut-director-pick>
            {director.contact_sheet_url && (
              <img className="dir-sheet" data-cut-director-sheet src={director.contact_sheet_url} alt="per-scene contact sheet" />
            )}
            <div className="dir-hint">Pick who each shot follows. <b>Auto</b> = let the CV ranker decide (active speaker / saliency).</div>
            <div className="dir-scenes">
              {director.scenes.map((s) => {
                const cur = picks[s.scene] ?? { kind: 'auto' as const }
                return (
                  <div className="dir-scene" key={s.scene} data-cut-director-scene={s.scene}>
                    <span className="dir-scene-n">Scene {s.scene}</span>
                    <div className="dir-opts">
                      <button
                        type="button"
                        className={`dir-opt ${cur.kind === 'auto' ? 'dir-opt--on' : ''}`}
                        data-cut-pick="auto"
                        aria-pressed={cur.kind === 'auto'}
                        onClick={() => setPick(s.scene, { kind: 'auto' })}
                      >Auto</button>
                      {s.candidates.map((c) => (
                        <button
                          key={c.label}
                          type="button"
                          className={`dir-opt ${cur.kind === 'cx' && cur.label === c.label ? 'dir-opt--on' : ''}`}
                          data-cut-pick={c.label}
                          aria-pressed={cur.kind === 'cx' && cur.label === c.label}
                          title={`${c.cls}${c.has_face ? ' · face' : ''} @ x=${c.cx.toFixed(2)}`}
                          onClick={() => setPick(s.scene, { kind: 'cx', cx: c.cx, label: c.label })}
                        >{c.label}{c.has_face ? <Icon name="matte" size={14} tone="brand" label="has a face" /> : null}</button>
                      ))}
                      <button
                        type="button"
                        className={`dir-opt ${cur.kind === 'widen' ? 'dir-opt--on' : ''}`}
                        data-cut-pick="widen"
                        aria-pressed={cur.kind === 'widen'}
                        onClick={() => setPick(s.scene, { kind: 'widen' })}
                      >Widen</button>
                    </div>
                  </div>
                )
              })}
            </div>
            <div className="dir-actions">
              <button className="dir-btn dir-btn--primary" data-cut-director-render onClick={() => void render()}>
                Reframe → {aspect}
              </button>
            </div>
          </div>
        )}

        {phase === 'done' && (
          <div className="dir-done" data-cut-director-done>
            <div className="dir-done-msg"><Icon name="check" size={16} tone="success" /> Reframed → <b>{reframeId}</b></div>
            {qc && (
              <div className="dir-qc" data-cut-director-qc>
                <div className={`dir-qc-verdict ${qc.review_count > 0 ? 'dir-qc-verdict--flag' : 'dir-qc-verdict--ok'}`}>
                  QC: {qc.review_count > 0 ? `${qc.review_count}/${qc.scene_count} scene(s) flagged` : `all ${qc.scene_count} scene(s) look good`}
                </div>
                {qc.qc_sheet_url && <img className="dir-sheet" data-cut-director-qc-sheet src={qc.qc_sheet_url} alt="QC review sheet" />}
                {qc.review_count > 0 && (
                  <ul className="dir-qc-list">
                    {qc.scenes.filter((s) => s.needs_review).map((s) => (
                      <li key={s.scene}>Scene {s.scene}: {s.issues.join(', ') || 'review'}</li>
                    ))}
                  </ul>
                )}
              </div>
            )}
            <div className="dir-actions">
              {!qc && <button className="dir-btn" data-cut-director-review onClick={() => void review()}>Review (QC)</button>}
              {qc && qc.review_count > 0 && (
                <button className="dir-btn" data-cut-director-repick onClick={() => setPhase('pick')}>Re-pick & fix</button>
              )}
              <button className="dir-btn dir-btn--primary" data-cut-director-done-close onClick={onClose}>Done</button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
