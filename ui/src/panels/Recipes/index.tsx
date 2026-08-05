// panels/Recipes — the human UI for recipes (recipe.list /
// recipe.describe / recipe.run). A right-side drawer (the Autopilot/Assemble
// family) that makes Cut's deterministic, op-level, receipt-gated workflows
// human-visible. Every stage is a real, replay-safe operation behind a measured
// gate, and the panel shows that operation list.
//
// FLOW (a small state machine, one drawer):
//   • view='list'   — on open, recipe.list → the named recipes (title + summary
//                     + stage count). Pure read; works with no project open.
//   • view='detail' — pick one → recipe.describe → its description, param inputs
//                     (rendered FROM the manifest's param summaries: enum→select,
//                     integer→number, the `asset` param→an asset picker, else
//                     text), and the ordered stages it will run (verb + why + gate).
//       ↳ Preview plan  — recipe.run{policy:'dry_run'} → the resolved PLAN, returned
//                         DIRECTLY (no checkpoint, nothing dispatched): the exact
//                         op list + per-stage gate the recipe WOULD apply. THE
//                         receipt-legibility preview.
//       ↳ Run recipe    — recipe.run{policy:'run'} → a job; we poll jobs.status for
//                         the clean receipt (summary line + per-stage ok/gate +
//                         what changed + a one-step Restore checkpoint).
//
// HONEST surfacing: recipe.run requires an OPEN PROJECT (the engine fails fast,
// even for dry_run), so both actions are disabled with an inline hint when none
// is open; a missing required param is named by the server and shown verbatim.
// Errors are never swallowed (cd-err). Every element carries data-cut-* for the
// Debug API + the full-coverage gate (secRecipe).
//
// Callers: App.tsx (activeDrawer === 'recipes'). Deps: lib/client (callVerb +
// the recipe result types), ../drawer.css (shared cd-* styles), ./recipes.css.

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  callVerb,
  type Project,
  type RecipeSummary,
  type RecipeManifest,
  type RecipeParam,
  type RecipeGate,
  type RecipeDryRun,
  type RecipeRunHandle,
  type RecipeReport,
} from '../../lib/client'
import { Icon } from '../../icons'
import { recipeNeedsPreview } from './model'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import '../drawer.css'
import './recipes.css'

export interface RecipesDrawerProps {
  project: Project | null
  onProjectSwitched: () => void | Promise<void>
  onClose: () => void
}

type View = 'list' | 'detail'

/** A gate rendered as a compact, readable chip-line ("gate: lufs · cut_on_word"
 *  or "gate: transcript_words gt 0"). Empty/null gate → null (caller hides it). */
function gateLabel(gate: RecipeGate | null | undefined): string | null {
  if (!gate) return null
  const parts: string[] = []
  if (gate.checks?.length) parts.push(...gate.checks)
  if (gate.state?.length) parts.push(...gate.state.map((s) => `${s.fact} ${s.op}${s.value != null ? ` ${s.value}` : ''}`))
  return parts.length ? parts.join(' · ') : null
}

/** Pretty-print a stage's interpolated args for the plan/manifest view (the
 *  op-level receipt). Skips the threaded `rationale` (provenance, not an edit
 *  param) so the line shows only what the verb actually does. */
function argsLabel(args: Record<string, unknown> | undefined): string {
  if (!args) return ''
  const entries = Object.entries(args).filter(([k]) => k !== 'rationale')
  if (entries.length === 0) return ''
  return entries.map(([k, v]) => `${k}: ${typeof v === 'object' ? JSON.stringify(v) : String(v)}`).join('  ')
}

function recipePlanErrorMessage(result: unknown, fallback: string | undefined): string {
  const r = result as { status?: unknown; reason?: unknown; message?: unknown; error?: { message?: unknown } } | null | undefined
  const detail =
    (typeof r?.reason === 'string' && r.reason.trim()) ||
    (typeof r?.error?.message === 'string' && r.error.message.trim()) ||
    (typeof r?.message === 'string' && r.message.trim()) ||
    fallback ||
    'could not plan this recipe'
  return typeof r?.status === 'string' && r.status && r.status !== 'planned'
    ? `${r.status}: ${detail}`
    : detail
}

function cleanStageText(value: string | null | undefined): string {
  const text = String(value ?? '').trim()
  if (!text) return ''
  return text.charAt(0).toUpperCase() + text.slice(1)
}

function recipeOptionLabel(paramName: string, option: string): string {
  if ((paramName === 'intensity' || paramName === 'aggressiveness') && option === 'jumpy') return 'Tight'
  return cleanStageText(option.replaceAll('_', ' '))
}

function stageTitle(stage: { id?: string; verb?: string; rationale?: string }): string {
  switch (stage.id) {
    case 'transcribe': return 'Create transcript'
    case 'analyze': return 'Find pauses'
    case 'retakes': return 'Remove repeated takes'
    case 'tighten': return 'Remove long pauses'
    case 'fillers': return 'Remove filler words'
    case 'voice': return 'Clean voice audio'
    case 'captions': return 'Add captions'
    case 'render': return 'Render final video'
    case 'trim_edges': return 'Trim start and end'
    case 'bundle': return 'Make social versions'
    case 'mask': return 'Mask selected area'
    case 'publish': return 'Export for platform'
    default: break
  }
  switch (stage.verb) {
    case 'media.transcribe': return 'Create transcript'
    case 'media.perception': return 'Find pauses'
    case 'transcript.remove_retakes': return 'Remove repeated takes'
    case 'transcript.remove_silences': return 'Remove long pauses'
    case 'transcript.remove_fillers': return 'Remove filler words'
    case 'audio.cleanup_voice': return 'Clean voice audio'
    case 'captions.generate': return 'Add captions'
    case 'render.final': return 'Render final video'
    case 'edit.trim_edges': return 'Trim start and end'
    case 'render.bundle': return 'Make social versions'
    case 'edit.add_mask': return 'Mask selected area'
    case 'export.publish': return 'Export for platform'
    default: return cleanStageText(stage.rationale) || 'Apply workflow step'
  }
}

export default function RecipesDrawer({ project, onProjectSwitched, onClose }: RecipesDrawerProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const [view, setView] = useState<View>('list')
  const [recipes, setRecipes] = useState<RecipeSummary[] | null>(null)
  const [manifest, setManifest] = useState<RecipeManifest | null>(null)
  // Param field values are kept as strings (the input model); coerced to the
  // declared type when the args object is built for recipe.run.
  const [paramValues, setParamValues] = useState<Record<string, string>>({})
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  // The two result surfaces — a dry-run PLAN, or a finished run REPORT (+ the
  // live phase string while the run job is polling).
  const [plan, setPlan] = useState<RecipeDryRun | null>(null)
  const [report, setReport] = useState<RecipeReport | null>(null)
  const [phase, setPhase] = useState<string>('')
  const [restored, setRestored] = useState(false)
  const pollTimer = useRef<number | null>(null)
  const activeRunJobRef = useRef<string | null>(null)

  const clearRecipePoll = useCallback(() => {
    if (pollTimer.current) {
      window.clearTimeout(pollTimer.current)
      pollTimer.current = null
    }
    activeRunJobRef.current = null
  }, [])

  const hasProject = !!project
  // Assets in the open project — the picker source for an `asset` param.
  const assetOptions = useMemo(() => {
    const a = project?.assets ?? {}
    return Object.entries(a).map(([id, asset]) => ({
      id,
      name: asset.path.split(/[\\/]/).pop() || id,
      transcribed: !!asset.transcript,
    }))
  }, [project])

  // Esc closes; clear the run-poll ONLY on unmount (App passes a fresh onClose
  // every render — folding the poll cleanup into the keydown effect would kill
  // it on every re-render, the Autopilot bug).
  useEffect(() => () => clearRecipePoll(), [clearRecipePoll])

  // On open: load the recipe catalog (pure read — no project needed).
  const loadList = useCallback(async () => {
    setBusy(true); setErr(null)
    try {
      const r = await callVerb('recipe.list', {})
      if (r.ok && r.result) setRecipes(r.result.recipes ?? [])
      else setErr(r.error?.message ?? 'could not list recipes')
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }, [])
  useEffect(() => { void loadList() }, [loadList])

  // Reset the per-recipe result/error surfaces (on select + before each action).
  const resetResults = () => { setErr(null); setPlan(null); setReport(null); setPhase(''); setRestored(false) }

  // Pick a recipe → describe it → seed the param fields from defaults (the
  // `asset` param defaults to the first transcribed asset so the common case is
  // one click). Switches to the detail view.
  const selectRecipe = useCallback(async (name: string) => {
    clearRecipePoll()
    setBusy(true); resetResults(); setManifest(null)
    try {
      const r = await callVerb('recipe.describe', { name })
      if (r.ok && r.result) {
        const m = r.result
        setManifest(m)
        const seed: Record<string, string> = {}
        for (const p of m.params) {
          if (p.name === 'asset') {
            const pick = assetOptions.find((o) => o.transcribed)?.id || assetOptions[0]?.id || ''
            seed[p.name] = pick
          } else if (p.default != null) {
            seed[p.name] = String(p.default)
          } else {
            seed[p.name] = ''
          }
        }
        setParamValues(seed)
        setView('detail')
      } else {
        setErr(r.error?.message ?? `could not describe "${name}"`)
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }, [assetOptions, clearRecipePoll])

  const backToList = () => {
    clearRecipePoll()
    setView('list'); setManifest(null); resetResults()
  }

  const setParam = (name: string, value: string) => {
    setParamValues((cur) => ({ ...cur, [name]: value }))
    setErr(null)
    setPlan(null)
    setReport(null)
    setPhase('')
    setRestored(false)
  }

  // Required params with an empty value block the action buttons (the server
  // would reject them; we name the gap up front instead).
  const missingRequired = useMemo(() => {
    if (!manifest) return [] as string[]
    return manifest.params.filter((p) => p.required && !String(paramValues[p.name] ?? '').trim()).map((p) => p.name)
  }, [manifest, paramValues])

  // Build the recipe.run `args` override object: every non-empty field, numeric
  // params coerced to Number (the engine validates the rest).
  const buildArgs = useCallback((): Record<string, unknown> => {
    if (!manifest) return {}
    const out: Record<string, unknown> = {}
    for (const p of manifest.params) {
      const raw = String(paramValues[p.name] ?? '').trim()
      if (!raw) continue
      out[p.name] = (p.type === 'integer' || p.type === 'number') ? Number(raw) : raw
    }
    return out
  }, [manifest, paramValues])

  const openFirstEditSample = useCallback(async () => {
    if (busy || manifest?.name !== 'first-project') return
    setBusy(true); resetResults(); setPhase('Preparing sample...')
    try {
      const listed = await callVerb('project.list', { sort: 'recent' })
      const names = new Set(listed.ok ? (listed.result?.projects ?? []).map((p) => p.name) : [])
      let name = 'First edit sample'
      for (let n = 2; names.has(name); n++) name = `First edit sample ${n}`

      const created = await callVerb('project.create', {
        name,
        settings: { width: 640, height: 360, fps: 24 },
        starter: 'first-edit',
      })
      if (!created.ok || !created.result?.starter_asset_path) {
        throw new Error(created.error?.message ?? 'could not create the sample project')
      }
      await onProjectSwitched()

      const imported = await callVerb('media.import', {
        path: created.result.starter_asset_path,
        proxy: false,
        rationale: 'guided First edit sample',
      })
      const importResult = imported.result as { asset_id?: string; job_id?: string } | undefined
      if (!imported.ok || !importResult?.asset_id || !importResult.job_id) {
        throw new Error(imported.error?.message ?? 'could not import the sample clip')
      }

      const deadline = Date.now() + 45_000
      for (;;) {
        const status = await callVerb('jobs.status', { job_id: importResult.job_id })
        if (!status.ok || !status.result) throw new Error(status.error?.message ?? 'sample import status unavailable')
        if (status.result.state === 'done') break
        if (status.result.state === 'failed') throw new Error(status.result.error?.message ?? 'sample import failed')
        if (Date.now() >= deadline) throw new Error('sample import timed out before it was ready to edit')
        await new Promise((resolve) => window.setTimeout(resolve, 350))
      }

      setParamValues((cur) => ({ ...cur, asset: importResult.asset_id! }))
      await onProjectSwitched()
      setPhase('')
    } catch (error) {
      setErr(error instanceof Error ? error.message : 'could not prepare the sample project')
      setPhase('')
    } finally {
      setBusy(false)
    }
  }, [busy, manifest, onProjectSwitched])

  // Preview plan — recipe.run{dry_run}. Returns the PLAN directly (no job).
  const previewPlan = useCallback(async () => {
    if (!manifest || busy) return
    setBusy(true); resetResults()
    try {
      const r = await callVerb('recipe.run', { name: manifest.name, args: buildArgs(), policy: 'dry_run' })
      if (r.ok && r.result && 'status' in r.result && (r.result as RecipeDryRun).status === 'planned') {
        setPlan(r.result as RecipeDryRun)
      } else {
        setErr(recipePlanErrorMessage(r.result, r.error?.message))
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }, [manifest, busy, buildArgs])

  // Run recipe — recipe.run{run}. Returns {job_id, checkpoint}; poll jobs.status
  // for the clean receipt (RecipeReport) that lands in the job result.
  const runRecipe = useCallback(async () => {
    if (!manifest || busy) return
    setBusy(true); resetResults(); setPhase('starting…')
    try {
      const r = await callVerb('recipe.run', { name: manifest.name, args: buildArgs(), policy: 'run', rationale: `human: run recipe ${manifest.name}` })
      if (!r.ok || !r.result || !('job_id' in r.result)) {
        setErr(r.error?.message ?? 'recipe.run failed')
        setBusy(false); setPhase('')
        return
      }
      const handle = r.result as RecipeRunHandle
      const n = handle.stages?.length ?? 0
      activeRunJobRef.current = handle.job_id
      setPhase(`running ${n} stage${n === 1 ? '' : 's'}…`)
      const poll = async () => {
        if (activeRunJobRef.current !== handle.job_id) return
        const j = await callVerb('jobs.status', { job_id: handle.job_id })
        if (activeRunJobRef.current !== handle.job_id) return
        if (j.ok && j.result) {
          const st = j.result.state
          if (st === 'done') {
            activeRunJobRef.current = null
            setReport((j.result.result as RecipeReport) ?? null)
            setBusy(false); setPhase('')
            return
          }
          if (st === 'failed') {
            activeRunJobRef.current = null
            setErr(`recipe failed: ${j.result.error?.message ?? 'a stage errored'}`)
            setBusy(false); setPhase('')
            return
          }
          if (typeof j.result.progress === 'number') setPhase(`running… ${Math.round(j.result.progress * 100)}%`)
        }
        pollTimer.current = window.setTimeout(() => void poll(), 1200)
      }
      pollTimer.current = window.setTimeout(() => void poll(), 1000)
    } catch {
      setErr('server unreachable')
      setBusy(false); setPhase('')
    }
  }, [manifest, busy, buildArgs])

  // One-step revert of the whole run (project.revert to the run's checkpoint).
  const restore = useCallback(async () => {
    if (!report?.checkpoint) return
    const r = await callVerb('project.revert', { to: report.checkpoint, rationale: 'undo recipe run' })
    if (r.ok) setRestored(true)
    else setErr(`restore failed: ${r.error?.message ?? ''}`)
  }, [report])

  const openInspect = () => document.dispatchEvent(new CustomEvent('cut:open-receipts'))

  const requiresPreview = useMemo(() => manifest ? recipeNeedsPreview(manifest) : false, [manifest])
  const planMatchesCurrentRecipe = !!manifest && !!plan && plan.recipe === manifest.name && plan.status === 'planned'
  const previewRequired = requiresPreview && !planMatchesCurrentRecipe
  const actionsDisabled = busy || !hasProject || missingRequired.length > 0
  const runDisabled = actionsDisabled || previewRequired

  return (
    <div className="cd-scrim" data-cut-recipes-scrim onMouseDown={overlay.onScrimMouseDown}>
      <aside
        ref={overlay.dialogRef}
        className="cd-drawer"
        data-cut-panel="recipes"
        data-cut-recipes
        data-cut-recipes-open="true"
        data-cut-recipes-view={view}
        role="dialog"
        aria-modal="true"
        aria-label={view === 'detail' ? manifest?.title ?? 'Recipe' : 'Recipes'}
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="cd-head">
          <div>
            <h2 className="cd-title">
              {view === 'detail' && (
                <button className="rc-back" data-cut-recipes-back onClick={backToList} aria-label="Back to recipes">
                  <Icon name="chevronLeft" size={16} />
                </button>
              )}
              {view === 'detail' ? manifest?.title ?? 'Recipe' : 'Recipes'}
            </h2>
            <p className="cd-sub">
              {view === 'list'
                ? 'Guided workflows for common edits. Preview the steps first, then run them when the plan looks right.'
                : manifest?.description ?? 'Loading…'}
            </p>
          </div>
          <button className="cd-btn cd-btn--ghost" data-cut-recipes-close onClick={onClose}>Close</button>
        </header>

        <div className="cd-body" data-cut-recipes-body>
          {err && <div className="cd-err" data-cut-recipes-error role="alert">{err}</div>}

          {/* ── LIST ──────────────────────────────────────────────────── */}
          {view === 'list' && (
            <div className="rc-list" data-cut-recipes-list>
              {busy && !recipes && <p className="cd-note" data-cut-recipes-loading>Loading recipes…</p>}
              {recipes && recipes.length === 0 && (
                <div className="cd-empty" data-cut-recipes-empty>No recipes are installed.</div>
              )}
              {recipes?.map((r) => (
                <button
                  key={r.name}
                  className={`rc-card${r.name === 'first-project' ? ' rc-card--starter' : ''}`}
                  data-cut-recipe={r.name}
                  onClick={() => void selectRecipe(r.name)}
                  disabled={busy}
                >
                  <div className="rc-card-head">
                    <Icon name="bolt" size={14} tone="brand" />
                    <span className="rc-card-title">{r.title}</span>
                    {r.name === 'first-project' && (
                      <span className="rc-card-starter" data-cut-recipe-starter>Start here</span>
                    )}
                    <span className="rc-card-stages" data-cut-recipe-stages={r.stage_count}>{r.stage_count} stages</span>
                  </div>
                  <p className="rc-card-desc">{r.description}</p>
                </button>
              ))}
            </div>
          )}

          {/* ── DETAIL ────────────────────────────────────────────────── */}
          {view === 'detail' && manifest && (
            <div className="rc-detail" data-cut-recipe-detail={manifest.name}>
              {!hasProject && (
                <div className="rc-no-project" data-cut-recipes-noproject>
                  <p className="cd-note cd-note--warn">
                    Open a project to run this recipe — it edits the open timeline. You can still read its stages below.
                  </p>
                  {manifest.name === 'first-project' && (
                    <button
                      className="cd-btn cd-btn--primary rc-sample"
                      data-cut-recipe-sample
                      disabled={busy}
                      onClick={() => void openFirstEditSample()}
                    >
                      <Icon name="play" size={14} /> {phase || 'Open sample project'}
                    </button>
                  )}
                </div>
              )}

              {/* PARAMS — rendered from the manifest's param summaries. */}
              {manifest.params.length > 0 && (
                <div className="rc-params" data-cut-recipe-params>
                  {manifest.params.map((p) => (
                    <RecipeParamField
                      key={p.name}
                      param={p}
                      value={paramValues[p.name] ?? ''}
                      onChange={(v) => setParam(p.name, v)}
                      assetOptions={assetOptions}
                    />
                  ))}
                </div>
              )}

              {/* STAGES — default path: what happens in human terms. */}
              <div className="rc-stages" data-cut-recipe-stages-list>
                <div className="rc-stages-head">What will happen</div>
                {manifest.stages.map((s, i) => {
                  const g = gateLabel(s.gate)
                  return (
                    <div className="rc-stage" key={s.id} data-cut-recipe-stage={s.id}>
                      <span className="rc-stage-n">{i + 1}</span>
                      <div className="rc-stage-main">
                        <span className="rc-stage-title">{stageTitle(s)}</span>
                        {s.rationale && <span className="rc-stage-why">{cleanStageText(s.rationale)}</span>}
                        {g && <span className="rc-stage-gate" data-cut-recipe-stage-gate><Icon name="qc" size={14} /> checked after this step</span>}
                      </div>
                    </div>
                  )
                })}
              </div>

              <details className="rc-technical" data-cut-recipe-technical>
                <summary data-cut-recipe-technical-toggle>Technical stages</summary>
                <div className="rc-technical-list">
                  {manifest.stages.map((s, i) => {
                    const g = gateLabel(s.gate)
                    return (
                      <div className="rc-op" key={s.id} data-cut-recipe-technical-stage={s.id}>
                        <span className="rc-op-n">{i + 1}</span>
                        <div className="rc-op-main">
                          <span className="rc-op-verb">{s.verb}</span>
                          {g && <span className="rc-op-gate"><Icon name="qc" size={14} /> {g}</span>}
                        </div>
                      </div>
                    )
                  })}
                </div>
              </details>

              {/* ACTIONS — Preview (dry-run plan) + Run. */}
              <div className="rc-actions">
                <button
                  className="cd-btn cd-btn--ghost"
                  data-cut-recipe-preview
                  disabled={actionsDisabled}
                  onClick={() => void previewPlan()}
                  title="Resolve + validate the recipe and show the planned ops WITHOUT running anything"
                >
                  <Icon name="eye" size={14} /> Preview plan
                </button>
                <button
                  className="cd-btn cd-btn--primary"
                  data-cut-recipe-run
                  disabled={runDisabled}
                  onClick={() => void runRecipe()}
                  title={previewRequired ? 'Preview the exact plan before running this recipe' : 'Run this recipe'}
                >
                  {busy && phase ? phase : <><Icon name="bolt" size={14} tone="brand" /> Run recipe</>}
                </button>
              </div>
              {missingRequired.length > 0 && hasProject && (
                <p className="cd-note" data-cut-recipe-missing>Fill required: {missingRequired.join(', ')}.</p>
              )}
              {previewRequired && hasProject && missingRequired.length === 0 && (
                <p className="cd-note" data-cut-recipe-preview-required>
                  Preview the plan first. This recipe changes the timeline or writes deliverables, and the preview shows the exact steps before anything runs.
                </p>
              )}

              {/* ── DRY-RUN PLAN (the receipt-legibility preview) ───────── */}
              {plan && (
                <div className="rc-plan" data-cut-recipe-plan data-cut-recipe-plan-status={plan.status}>
                  <div className="rc-plan-head" data-cut-recipe-plan-head>
                    <Icon name="pending" size={14} tone="brand" />
                    Plan only — {plan.stages.length} op{plan.stages.length === 1 ? '' : 's'}, nothing applied
                  </div>
                  {plan.stages.map((s, i) => {
                    const g = gateLabel(s.gate)
                    const a = argsLabel(s.args)
                    return (
                      <div className="rc-op" key={s.id} data-cut-recipe-plan-op={s.id}>
                        <span className="rc-op-n">{i + 1}</span>
                        <div className="rc-op-main">
                          <span className="rc-op-title">{stageTitle(s)}</span>
                          {s.rationale && <span className="rc-op-args">{cleanStageText(s.rationale)}</span>}
                          {g && <span className="rc-op-gate"><Icon name="qc" size={14} /> checked after this step</span>}
                          {a && (
                            <details className="rc-op-technical" data-cut-recipe-plan-technical={s.id}>
                              <summary data-cut-recipe-plan-technical-toggle={s.id}>Technical args</summary>
                              <span className="rc-op-args">{s.verb} · {a}</span>
                            </details>
                          )}
                        </div>
                      </div>
                    )
                  })}
                </div>
              )}

              {/* ── RUN RECEIPT (the applied ops + per-stage gates) ─────── */}
              {report && (
                <div className="rc-report" data-cut-recipe-report data-cut-recipe-status={report.status}>
                  <div
                    className={`rc-summary ${report.status === 'completed' ? 'rc-summary--pass' : 'rc-summary--warn'}`}
                    data-cut-recipe-summary
                  >
                    <span className="rc-summary-badge">
                      {report.status === 'completed'
                        ? <Icon name="check" size={16} tone="success" label="completed" />
                        : <Icon name="warning" size={16} tone="warn" label={report.status} />}
                    </span>
                    <span>{report.summary_line}</span>
                  </div>

                  {report.changed && (report.changed.ops ?? 0) > 0 && (
                    <div className="rc-changed" data-cut-recipe-changed>
                      {report.changed.ops} timeline change{report.changed.ops === 1 ? '' : 's'}
                      {report.changed.duration_delta_ms != null && report.changed.duration_delta_ms !== 0 && (
                        <> · {report.changed.duration_delta_ms > 0 ? '+' : ''}{(report.changed.duration_delta_ms / 1000).toFixed(1)}s</>
                      )}
                    </div>
                  )}

                  {/* per-stage applied results + gate verdict */}
                  <div className="rc-stage-results" data-cut-recipe-stage-results>
                    {report.stage_results.map((sr, i) => {
                      const gatePass = sr.gate ? sr.gate.pass : null
                      return (
                        <div
                          className={`rc-op rc-op--result ${sr.ok ? '' : 'rc-op--fail'}`}
                          key={sr.id}
                          data-cut-recipe-result={sr.id}
                          data-cut-recipe-result-ok={String(sr.ok)}
                        >
                          <span className="rc-op-n">
                            {sr.ok
                              ? <Icon name="check" size={14} tone="success" />
                              : <Icon name="error" size={14} tone="danger" />}
                          </span>
                          <div className="rc-op-main">
                            <span className="rc-op-title">{stageTitle(sr)}</span>
                            {sr.op_ids.length > 0 && <span className="rc-op-args">{sr.op_ids.length} timeline change{sr.op_ids.length === 1 ? '' : 's'}</span>}
                            {gatePass != null && (
                              <span className={`rc-op-gate ${gatePass ? 'rc-op-gate--pass' : 'rc-op-gate--fail'}`} data-cut-recipe-result-gate={String(gatePass)}>
                                <Icon name={gatePass ? 'check' : 'warning'} size={14} tone={gatePass ? 'success' : 'warn'} />
                                check {gatePass ? 'passed' : 'failed'}
                              </span>
                            )}
                            {sr.error && <span className="rc-op-err">{sr.error.message}</span>}
                          </div>
                          {i === report.stages_run - 1 && !report.status.startsWith('completed') && (
                            <span className="rc-op-stop" title="run stopped here">stopped</span>
                          )}
                        </div>
                      )
                    })}
                  </div>

                  <div className="rc-report-actions">
                    {report.checkpoint && !restored && (
                      <button className="cd-btn cd-btn--ghost" data-cut-recipe-restore onClick={() => void restore()}>
                        Restore (undo run)
                      </button>
                    )}
                    {restored && <span className="rc-restored" data-cut-recipe-restored><Icon name="undo" size={14} /> restored to before the recipe</span>}
                    <button className="cd-btn cd-btn--ghost rc-inspect" data-cut-recipe-inspect onClick={openInspect}>
                      Review details
                    </button>
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </aside>
    </div>
  )
}

/** One param input, rendered from its manifest summary: enum → select, the
 *  `asset` param → an asset picker (project assets), integer/number → number,
 *  else text. The label shows the param name + a (required) marker; the
 *  description (when present) is the field hint. */
function RecipeParamField({
  param,
  value,
  onChange,
  assetOptions,
}: {
  param: RecipeParam
  value: string
  onChange: (v: string) => void
  assetOptions: { id: string; name: string; transcribed: boolean }[]
}) {
  const isAsset = param.name === 'asset'
  const isNum = param.type === 'integer' || param.type === 'number'
  return (
    <label className="cd-field" data-cut-recipe-param={param.name}>
      <span className="cd-field-label">
        {param.name}
        {param.required && <span className="rc-req"> *</span>}
        {!param.required && param.default != null && <span className="cd-val">default {String(param.default)}</span>}
      </span>

      {param.enum && param.enum.length > 0 ? (
        <select className="cd-sel" data-cut-recipe-param-input={param.name} value={value} onChange={(e) => onChange(e.target.value)}>
          {param.enum.map((opt) => <option key={opt} value={opt}>{recipeOptionLabel(param.name, opt)}</option>)}
        </select>
      ) : isAsset ? (
        <select
          className="cd-sel"
          data-cut-recipe-param-input={param.name}
          value={value}
          disabled={assetOptions.length === 0}
          onChange={(e) => onChange(e.target.value)}
        >
          {assetOptions.length === 0 && <option value="">No assets — import a clip first</option>}
          {assetOptions.map((o) => (
            <option key={o.id} value={o.id}>{o.name}{o.transcribed ? '' : ' — (transcribe first)'}</option>
          ))}
        </select>
      ) : isNum ? (
        <input
          className="cd-input cd-input--mono"
          type="number"
          data-cut-recipe-param-input={param.name}
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      ) : (
        <input
          className="cd-input"
          type="text"
          data-cut-recipe-param-input={param.name}
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      )}

      {param.description && <span className="rc-param-hint">{param.description}</span>}
    </label>
  )
}
