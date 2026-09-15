// panels/Assemble — the "Assemble (AI)" drawer: the HUMAN UI for the agent-only
// assemble.* family. Four modes behind one drawer (mirrors Generate's
// provider/kind toggle):
//
//   • Auto-shorts (assemble.shorts)      — a transcribed source → ranked short
//     ranges with a planned source crop and transcript captions. Review first,
//     then explicitly add the eligible plan as one editable Undo action.
//   • Repurpose  (assemble.repurpose)   — a transcribed source → the N best
//     moments, ranked by the same engagement signal score.clip exposes. Review
//     first, then explicitly add the reviewed ranges as one editable Undo action.
//   • From script (assemble.from_script) — paste a script (one line per point) →
//     each line matched to the best transcript span (token-overlap F1). Unmatched
//     lines stay visible; apply adds only the reviewed matches in one Undo action.
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
import NativeFolderPicker from '../../components/NativeFolderPicker'
import { Icon } from '../../icons'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import { AssemblePlanControls } from './AssemblePlanControls'
import { AssembleResults } from './AssembleResults'
import { acceptAppliedPlan, acceptPlanBinding } from './assemblePlanAcceptance'
import {
  assembleProjectIdentityScope, assembleProjectRevision, assembleRequestId,
  type ApplyState, type AssembleMode, type BrollPlaced, type RepurposeClip,
  type RepurposeRequest, type ReviewedPlan, type ScriptRequest, type ScriptSegment,
  type ShortsItem, type ShortsMaterialization, type ShortsRequest,
} from './assemblePlanModel'
import { type AssembleApplyCompletion, useAssembleRequestEpoch } from './useAssembleRequestEpoch'
import '../drawer.css'
import './assemble.css'

type Mode = AssembleMode

export interface AssembleDrawerProps {
  project: Project | null
  /** Live playhead (ms) — the default position for a placed b-roll slot. */
  playheadMs?: number
  /** Jump the playhead to a result's start (App publishes it through ui.state). */
  onSeek?: (atMs: number) => void
  onClose: () => void
}

const MODES = [
  { id: 'shorts', label: 'Short ranges', icon: 'effect' },
  { id: 'repurpose', label: 'Best moments', icon: 'split' },
  { id: 'from_script', label: 'From script', icon: 'text' },
  { id: 'broll', label: 'B-roll', icon: 'videoClip' },
] as const

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
  const [repurposePlan, setRepurposePlan] = useState<ReviewedPlan<RepurposeRequest> | null>(null)

  // From-script params.
  const [script, setScript] = useState('')
  const [minScore, setMinScore] = useState(0.35)
  const [segments, setSegments] = useState<ScriptSegment[] | null>(null)
  const [scriptPlan, setScriptPlan] = useState<ReviewedPlan<ScriptRequest> | null>(null)

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
  const [shortsPlan, setShortsPlan] = useState<(ReviewedPlan<ShortsRequest> & { materialization: ShortsMaterialization }) | null>(null)
  const [applyState, setApplyState] = useState<ApplyState>('idle')

  const projectIdentityScope = assembleProjectIdentityScope(project)
  const projectRevision = assembleProjectRevision(project)
  const requestEpoch = useAssembleRequestEpoch(projectIdentityScope, projectRevision)

  const reset = () => {
    setErr(null)
    setNote(null)
    setRepurposeClips(null)
    setRepurposePlan(null)
    setSegments(null)
    setScriptPlan(null)
    setPlaced(null)
    setShorts(null)
    setShortsPlan(null)
    setApplyState('idle')
  }

  const invalidateReviewedPlan = () => {
    requestEpoch.invalidate()
    reset()
    setBusy(false)
  }

  const showApplyFailure = (completion: AssembleApplyCompletion, message: string) => {
    if (completion === 'cancelled') return
    if (completion === 'stale') {
      reset()
      setBusy(false)
      setErr('The reviewed plan is no longer current. Review it again before applying.')
      return
    }
    setApplyState('idle')
    setErr(message)
  }

  const switchMode = (next: Mode) => {
    if (busy || next === mode) return
    setMode(next)
    invalidateReviewedPlan()
  }

  // A project switch discards its prior selected asset as well as every plan.
  useEffect(() => {
    requestEpoch.acknowledgeProjectChange('foreign-project')
    setAsset('')
    invalidateReviewedPlan()
  }, [projectIdentityScope])
  // A normal project edit invalidates a review. An Apply's own exact op can
  // publish before its HTTP receipt, so the guard keeps that one completion.
  useEffect(() => {
    if (requestEpoch.projectChange === 'stale-plan') {
      requestEpoch.acknowledgeProjectChange('stale-plan')
      invalidateReviewedPlan()
    }
  }, [projectRevision, requestEpoch.projectChange])
  useEffect(() => {
    if (!brollAtTouched) setBrollAtS(Math.max(0, Math.round(playheadMs / 1000)))
  }, [brollAtTouched, playheadMs])

  const runShorts = async () => {
    if (!effectiveAsset) { setErr('Import + transcribe a clip first — auto-shorts reads the transcript.'); return }
    const token = requestEpoch.start()
    setBusy(true); reset()
    try {
      const request: ShortsRequest = { asset: effectiveAsset, count, target_ms: Math.max(3000, targetS * 1000), aspect }
      const r = await callVerb('assemble.shorts', request)
      if (!requestEpoch.current(token)) return
      const planned = r.result as { shorts?: ShortsItem[]; plan_binding?: unknown; materialization?: ShortsMaterialization } | undefined
      const binding = r.ok ? acceptPlanBinding(planned?.plan_binding, 'assemble.shorts') : null
      if (r.ok && planned && binding && planned.materialization) {
        const shorts = planned.shorts ?? []
        setShorts(shorts)
        setShortsPlan({ request, binding, materialization: planned.materialization })
        if (shorts.length === 0) setNote('No strong moments found in this source.')
      } else setErr(r.ok ? 'The server returned a short plan without a valid review binding. Review the plan again.' : r.error?.message ?? 'could not build shorts (is the source transcribed?)')
    } catch {
      if (requestEpoch.current(token)) setErr('server unreachable')
    } finally {
      if (requestEpoch.current(token)) setBusy(false)
    }
  }

  const runRepurpose = async () => {
    if (!effectiveAsset) { setErr('Import + transcribe a clip first — repurpose reads the transcript.'); return }
    const token = requestEpoch.start()
    setBusy(true); reset()
    try {
      const request: RepurposeRequest = { asset: effectiveAsset, count, target_ms: Math.max(3000, targetS * 1000), ...(prompt.trim() ? { prompt: prompt.trim() } : {}) }
      const r = await callVerb('assemble.repurpose', request)
      if (!requestEpoch.current(token)) return
      const planned = r.result as { clips?: RepurposeClip[]; plan_binding?: unknown } | undefined
      const binding = r.ok ? acceptPlanBinding(planned?.plan_binding, 'assemble.repurpose') : null
      if (r.ok && planned && binding) {
        const clips = planned.clips ?? []
        setRepurposeClips(clips)
        setRepurposePlan({ request, binding })
        if (clips.length === 0) setNote('No strong moments found in this source.')
      } else setErr(r.ok ? 'The server returned a plan without a valid review binding. Review the plan again.' : r.error?.message ?? 'could not find moments (is the source transcribed?)')
    } catch {
      if (requestEpoch.current(token)) setErr('server unreachable')
    } finally {
      if (requestEpoch.current(token)) setBusy(false)
    }
  }

  const runFromScript = async () => {
    if (!effectiveAsset) { setErr('Import + transcribe a clip first — script matching reads the transcript.'); return }
    if (!script.trim()) { setErr('Paste a script first (one talking point per line).'); return }
    const token = requestEpoch.start()
    setBusy(true); reset()
    try {
      const request: ScriptRequest = { asset: effectiveAsset, script: script.trim(), min_score: minScore }
      const r = await callVerb('assemble.from_script', request)
      if (!requestEpoch.current(token)) return
      const planned = r.result as { segments?: ScriptSegment[]; matched?: number; total_lines?: number; plan_binding?: unknown } | undefined
      const binding = r.ok ? acceptPlanBinding(planned?.plan_binding, 'assemble.from_script') : null
      if (r.ok && planned && binding) {
        setSegments(planned.segments ?? [])
        setScriptPlan({ request, binding })
        setNote(`Matched ${planned.matched ?? 0} / ${planned.total_lines ?? 0} lines.`)
      } else setErr(r.ok ? 'The server returned a match plan without a valid review binding. Review the plan again.' : r.error?.message ?? 'could not match the script (is the source transcribed?)')
    } catch {
      if (requestEpoch.current(token)) setErr('server unreachable')
    } finally {
      if (requestEpoch.current(token)) setBusy(false)
    }
  }

  const applyRepurpose = async () => {
    if (!repurposePlan) return
    const token = requestEpoch.startApply(repurposePlan.binding.project_revision)
    if (token === null) {
      invalidateReviewedPlan()
      setErr('The reviewed plan is no longer current. Review it again before applying.')
      return
    }
    setBusy(true); setErr(null); setNote(null); setApplyState('applying')
    try {
      const r = await callVerb('assemble.repurpose', { ...repurposePlan.request, apply: repurposePlan.binding, request_id: assembleRequestId('repurpose'), expected_revision: repurposePlan.binding.project_revision })
      if (!requestEpoch.current(token)) return
      const receipt = acceptAppliedPlan(r, 'assemble.repurpose', repurposePlan.binding.asset)
      const completion = receipt && r.project_revision
        ? requestEpoch.acceptApply(token, r.project_revision)
        : requestEpoch.failApply(token)
      if (completion === 'accepted' && receipt) {
        setApplyState('applied')
        setNote(`Added ${receipt.spans_placed} reviewed range(s) as one editable timeline action.`)
      } else showApplyFailure(completion, r.ok ? 'The server response did not prove one complete timeline action.' : r.error?.message ?? 'could not add the reviewed plan to the timeline')
    } catch { showApplyFailure(requestEpoch.failApply(token), 'server unreachable') }
    finally { if (requestEpoch.current(token)) setBusy(false) }
  }

  const applyShorts = async () => {
    if (!shortsPlan) return
    const token = requestEpoch.startApply(shortsPlan.binding.project_revision)
    if (token === null) {
      invalidateReviewedPlan()
      setErr('The reviewed plan is no longer current. Review it again before applying.')
      return
    }
    setBusy(true); setErr(null); setNote(null); setApplyState('applying')
    try {
      const r = await callVerb('assemble.shorts', { ...shortsPlan.request, apply: shortsPlan.binding, request_id: assembleRequestId('shorts'), expected_revision: shortsPlan.binding.project_revision })
      if (!requestEpoch.current(token)) return
      const receipt = acceptAppliedPlan(r, 'assemble.shorts', shortsPlan.binding.asset)
      const completion = receipt && r.project_revision
        ? requestEpoch.acceptApply(token, r.project_revision)
        : requestEpoch.failApply(token)
      if (completion === 'accepted' && receipt) {
        setApplyState('applied')
        setNote(`Added ${receipt.video_clip_ids.length} editable short(s) and ${receipt.caption_clip_ids.length} transcript caption cue(s) as one timeline action.`)
      } else showApplyFailure(completion, r.ok ? 'The server response did not prove one complete timeline action.' : r.error?.message ?? 'could not add the reviewed short plan to the timeline')
    } catch { showApplyFailure(requestEpoch.failApply(token), 'server unreachable') }
    finally { if (requestEpoch.current(token)) setBusy(false) }
  }

  const applyFromScript = async () => {
    if (!scriptPlan) return
    const token = requestEpoch.startApply(scriptPlan.binding.project_revision)
    if (token === null) {
      invalidateReviewedPlan()
      setErr('The reviewed plan is no longer current. Review it again before applying.')
      return
    }
    setBusy(true); setErr(null); setNote(null); setApplyState('applying')
    try {
      const r = await callVerb('assemble.from_script', { ...scriptPlan.request, apply: scriptPlan.binding, request_id: assembleRequestId('from-script'), expected_revision: scriptPlan.binding.project_revision })
      if (!requestEpoch.current(token)) return
      const receipt = acceptAppliedPlan(r, 'assemble.from_script', scriptPlan.binding.asset)
      const completion = receipt && r.project_revision
        ? requestEpoch.acceptApply(token, r.project_revision)
        : requestEpoch.failApply(token)
      if (completion === 'accepted' && receipt) {
        setApplyState('applied')
        setNote(`Added ${receipt.spans_placed} matched script range(s) as one editable timeline action.`)
      } else showApplyFailure(completion, r.ok ? 'The server response did not prove one complete timeline action.' : r.error?.message ?? 'could not add the matched script plan to the timeline')
    } catch { showApplyFailure(requestEpoch.failApply(token), 'server unreachable') }
    finally { if (requestEpoch.current(token)) setBusy(false) }
  }

  const runBroll = async () => {
    if (!query.trim()) { setErr('Describe the b-roll to find (e.g. "city traffic at night").'); return }
    if (!brollDir.trim()) { setErr('Choose the folder to search for b-roll.'); return }
    if (!project) { setErr('Open a project first — the b-roll is placed on its timeline.'); return }
    const token = requestEpoch.startIdentityRequest()
    if (token === null) return
    setBusy(true); reset()
    try {
      const r = await callVerb('assemble.broll', {
        slots: [{ query: query.trim(), at_ms: Math.max(0, brollAtS * 1000), duration_ms: Math.max(1000, brollDurS * 1000) }],
        provider: 'local_folder',
        dir: brollDir.trim(),
        rationale: `human: fill b-roll slot "${query.trim()}"`,
      })
      if (!requestEpoch.completeIdentityRequest(token)) return
      if (r.ok && r.result) {
        const res = r.result as {
          status?: string
          placed?: BrollPlaced[]
          failed_step?: string
          error?: string | { message?: string; code?: string }
        }
        if (res.status === 'failed') {
          const error = typeof res.error === 'string'
            ? res.error
            : res.error?.message ?? res.error?.code ?? 'unknown error'
          setErr(`b-roll failed at ${res.failed_step ?? 'a step'}: ${error}`)
          setPlaced(res.placed ?? null)
        } else {
          setPlaced(res.placed ?? [])
          setNote(`Placed ${res.placed?.length ?? 0} b-roll clip(s). It's on the timeline (undoable).`)
        }
      } else {
        setErr(r.error?.message ?? 'b-roll search/fetch failed')
      }
    } catch {
      if (requestEpoch.completeIdentityRequest(token)) setErr('server unreachable')
    } finally {
      if (requestEpoch.current(token)) setBusy(false)
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
                disabled={busy}
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
                disabled={busy || assetOptions.length === 0}
                onChange={(e) => { setAsset(e.target.value); invalidateReviewedPlan() }}
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

          {mode !== 'broll' && <AssemblePlanControls
            mode={mode}
            busy={busy}
            effectiveAsset={effectiveAsset}
            count={count}
            targetS={targetS}
            prompt={prompt}
            script={script}
            minScore={minScore}
            aspect={aspect}
            shortsPlan={shortsPlan}
            repurposePlan={repurposePlan}
            scriptPlan={scriptPlan}
            applyState={applyState}
            onCount={(value) => { setCount(value); invalidateReviewedPlan() }}
            onTargetS={(value) => { setTargetS(value); invalidateReviewedPlan() }}
            onPrompt={(value) => { setPrompt(value); invalidateReviewedPlan() }}
            onScript={(value) => { setScript(value); invalidateReviewedPlan() }}
            onMinScore={(value) => { setMinScore(value); invalidateReviewedPlan() }}
            onAspect={(value) => { setAspect(value); invalidateReviewedPlan() }}
            onRunShorts={() => void runShorts()}
            onRunRepurpose={() => void runRepurpose()}
            onRunFromScript={() => void runFromScript()}
            onApplyShorts={() => void applyShorts()}
            onApplyRepurpose={() => void applyRepurpose()}
            onApplyFromScript={() => void applyFromScript()}
          />}

          {/* ── B-ROLL ────────────────────────────────────────────────── */}
          {mode === 'broll' && (
            <>
              <label className="cd-field">
                <span className="cd-field-label">What b-roll to find</span>
                <input className="cd-input" data-cut-assemble-query placeholder="e.g. city traffic at night"
                  value={query} disabled={busy} onChange={(e) => { setQuery(e.target.value); invalidateReviewedPlan() }} />
              </label>
              <NativeFolderPicker kind="assemble" label="Media folder" dialogTitle="Choose b-roll folder — ShellX Cut"
                value={brollDir} disabled={busy} onChooseStart={() => setErr(null)}
                onDesktopRequired={() => setNote('Open the desktop app to choose a local b-roll folder.')}
                onSelected={(selected) => { invalidateReviewedPlan(); setBrollDir(selected); setNote('B-roll folder selected.') }} />
              <div className="cd-row">
                <label className="cd-field cd-field--inline">
                  <span className="cd-field-label">Place at (s)</span>
                  <input className="cd-input cd-input--num" type="number" min={0}
                    data-cut-assemble-at value={brollAtS} disabled={busy}
                    onChange={(e) => {
                      setBrollAtTouched(true)
                      setBrollAtS(Math.max(0, Number(e.target.value) || 0))
                      invalidateReviewedPlan()
                    }} />
                </label>
                <label className="cd-field cd-field--inline">
                  <span className="cd-field-label">Length (s)</span>
                  <input className="cd-input cd-input--num" type="number" min={1} max={120}
                    data-cut-assemble-dur value={brollDurS} disabled={busy}
                    onChange={(e) => { setBrollDurS(Math.max(1, Math.min(120, Number(e.target.value) || 5))); invalidateReviewedPlan() }} />
                </label>
              </div>
              <button className="cd-btn cd-btn--primary" data-cut-assemble-run disabled={busy || !project || !brollDir.trim()}
                onClick={() => void runBroll()}>
                {busy ? 'Fetching…' : <><Icon name="videoClip" size={14} tone="media" /> Fill with b-roll</>}
              </button>
              <p className="cd-note">Searches that folder and inserts the matching clip at the chosen spot — a normal, undoable edit.</p>
            </>
          )}

          {err && <div className="cd-err" data-cut-assemble-error role="alert">{err}</div>}
          {note && <p className="cd-note" data-cut-assemble-note>{note}</p>}

          <AssembleResults
            shorts={shorts}
            repurposeClips={repurposeClips}
            segments={segments}
            placed={placed}
            aspect={aspect}
            onSeek={onSeek}
          />
        </div>
      </aside>
    </div>
  )
}
