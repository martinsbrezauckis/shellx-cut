// panels/Review/QC.tsx — the QC tab: the read-only verify.* receipts that need
// NO render (verify.pacing/captions/delivery/brand), each shown as a pass/fail
// card with its measured numbers, and — where one exists — a one-click FIX verb
// (verify.captions → captions.reflow / captions.shift). This is the product's
// measure→fix loop made visual: run a check, read the numbers, fix, re-check.
// Pure analysis — pacing/delivery/captions take no args; brand needs a target
// aspect (the most common brand constraint), picked inline. Display-only: the
// verify verbs create no op; the fix verbs (reflow/shift) DO and surface as ops.
// Callers: Review/index.tsx. Deps: lib/client (callVerb + the verb types).

import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type BrandCheckResult, type BrandKit, type Project } from '../../lib/client'
import { Icon } from '../../icons'

interface PacingResult { shot_count: number; internal_cuts: number; cuts_per_min: number; mean_shot_ms: number; duration_ms: number }
interface CaptionsResult { cue_count: number; pass: boolean; max_cps: number; mean_cps: number; violations: Record<string, unknown[]>; note?: string }
interface DeliveryResult { word_count: number; pass: boolean; wpm: number; articulation_wpm: number; filler_count: number; fillers_per_min: number; flags: Record<string, boolean>; note?: string }
interface BrandDraft {
  fonts: string
  colors: string
  position: '' | 'bottom' | 'top' | 'center'
  minSize: string
  maxSize: string
  aspect: string
}

/** FLAT verify.judge JOB result — the payload that lands in jobs.status.result.result
 *  (NOT the nested RenderReceipt.judge JudgeEnvelope the Receipts tab reads):
 *    completed → { render_id, status:'completed', verdict:'needs_review'|'pass'|'fail',
 *                  confidence:0.45, issues:1, receipt:'…/render_001.json' }
 *  `issues` here is a COUNT (number), not the per-issue array — the array lives in
 *  the receipt's judge.review.issues (shown, with seek links, on the Receipts tab).
 *  status discriminates completed | not_run | error; the degraded cases carry a
 *  human reason (engine uses `not_run_reason`; the task contract calls it `reason`
 *  — read both, since this surface never fabricates a verdict). */
interface JudgeJobResult {
  render_id?: string
  status?: 'completed' | 'not_run' | 'error' | string
  verdict?: 'pass' | 'fail' | 'needs_review'
  confidence?: number
  issues?: number
  reason?: string
  not_run_reason?: string
  receipt?: string
}

const BRAND_ASPECTS = ['16:9', '9:16', '1:1', '4:5'] as const

function reducedAspect(width = 16, height = 9): string {
  const gcd = (a: number, b: number): number => b === 0 ? a : gcd(b, a % b)
  const divisor = gcd(width, height) || 1
  return `${width / divisor}:${height / divisor}`
}

function brandDraft(brand: BrandKit | undefined, project: Project | null): BrandDraft {
  return {
    fonts: brand?.fonts?.join(', ') ?? '',
    colors: brand?.colors?.join(', ') ?? '',
    position: brand?.position ?? '',
    minSize: brand?.min_size?.toString() ?? '',
    maxSize: brand?.max_size?.toString() ?? '',
    aspect: brand?.aspect ?? reducedAspect(project?.settings.width, project?.settings.height),
  }
}

function splitList(value: string): string[] | undefined {
  const items = value.split(/[\n,]/).map((item) => item.trim()).filter(Boolean)
  return items.length > 0 ? items : undefined
}

function kitFromDraft(draft: BrandDraft): BrandKit {
  const minSize = draft.minSize.trim() === '' ? undefined : Number(draft.minSize)
  const maxSize = draft.maxSize.trim() === '' ? undefined : Number(draft.maxSize)
  return {
    fonts: splitList(draft.fonts),
    colors: splitList(draft.colors),
    position: draft.position || undefined,
    min_size: minSize,
    max_size: maxSize,
    aspect: draft.aspect.trim() || undefined,
  }
}

/** A run state: undefined = not run, null = errored, else the result. */
type Cell<T> = T | null | undefined

const JUDGE_JOB_STORAGE_KEY = 'shellx-cut:qc:judge-job'

export default function QC({ project }: { project: Project | null }) {
  const [pacing, setPacing] = useState<Cell<PacingResult>>(undefined)
  const [captions, setCaptions] = useState<Cell<CaptionsResult>>(undefined)
  const [delivery, setDelivery] = useState<Cell<DeliveryResult>>(undefined)
  const [brand, setBrand] = useState<Cell<BrandCheckResult>>(undefined)
  const [brandForm, setBrandForm] = useState<BrandDraft>(() => brandDraft(project?.brand, project))
  const [brandBusy, setBrandBusy] = useState(false)
  const [brandError, setBrandError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [shiftMs, setShiftMs] = useState(100)
  const [note, setNote] = useState<string | null>(null)
  // AI review (verify.judge) — async job: undefined = not run, null = no/garbled
  // response, else the flat job result. `judging` gates the button + busy card.
  const [judge, setJudge] = useState<Cell<JudgeJobResult>>(undefined)
  const [judging, setJudging] = useState(false)
  const judgeTimer = useRef<number | null>(null)

  const flash = (t: string) => { setNote(t); setTimeout(() => setNote(null), 4000) }

  const pollJudgeJob = useCallback((jobId: string, delayMs = 1000) => {
    if (judgeTimer.current) window.clearTimeout(judgeTimer.current)
    const poll = async () => {
      const j = await callVerb('jobs.status', { job_id: jobId })
      if (j.ok && j.result) {
        const st = j.result.state
        if (st === 'done') {
          setJudge((j.result.result as JudgeJobResult) ?? null)
          setJudging(false)
          localStorage.removeItem(JUDGE_JOB_STORAGE_KEY)
          return
        }
        if (st === 'failed') {
          setJudge({ status: 'error', reason: j.result.error?.message ?? 'judge job failed' })
          setJudging(false)
          localStorage.removeItem(JUDGE_JOB_STORAGE_KEY)
          return
        }
      }
      judgeTimer.current = window.setTimeout(() => void poll(), 1200)
    }
    judgeTimer.current = window.setTimeout(() => void poll(), delayMs)
  }, [])

  const resumeJudgeJob = useCallback((jobId: string) => {
    setJudging(true)
    setJudge(undefined)
    pollJudgeJob(jobId, 0)
  }, [pollJudgeJob])

  // Clear only the UI timer on unmount. The job id stays in localStorage so a
  // remounted QC tab resumes the subscription-CLI result instead of abandoning it.
  useEffect(() => {
    const jobId = localStorage.getItem(JUDGE_JOB_STORAGE_KEY)
    if (jobId) resumeJudgeJob(jobId)
    return () => { if (judgeTimer.current) window.clearTimeout(judgeTimer.current) }
  }, [resumeJudgeJob])

  // verify.judge → poll jobs.status (mirror Autopilot's loop exactly). Explicit
  // click only — NEVER auto-run: the judge spends a real subscription-CLI call.
  const runJudge = useCallback(async () => {
    if (judging) return
    setJudging(true)
    setJudge(undefined) // drop any prior verdict so the busy card shows
    const r = await callVerb('verify.judge', {})
    if (!r.ok || !r.result) {
      // Synchronous precondition (e.g. "no render receipts exist yet") — an honest
      // skip, not a verdict. Surface the engine's message + next step VERBATIM.
      const msg = r.error?.message ?? 'verify.judge failed'
      const sa = r.error?.suggested_action
      setJudge({ status: 'not_run', reason: sa ? `${msg} — ${sa}` : msg })
      setJudging(false)
      localStorage.removeItem(JUDGE_JOB_STORAGE_KEY)
      return
    }
    const jobId = (r.result as { job_id: string }).job_id
    localStorage.setItem(JUDGE_JOB_STORAGE_KEY, jobId)
    pollJudgeJob(jobId, 1000)
  }, [judging, pollJudgeJob])

  // Run the three no-arg receipts together (each read-only, cheap, no render).
  const runChecks = useCallback(async () => {
    setBusy(true)
    try {
      const [p, c, d] = await Promise.all([
        callVerb('verify.pacing', {}),
        callVerb('verify.captions', {}),
        callVerb('verify.delivery', {}),
      ])
      setPacing(p.ok ? (p.result as PacingResult) : null)
      setCaptions(c.ok ? (c.result as CaptionsResult) : null)
      setDelivery(d.ok ? (d.result as DeliveryResult) : null)
    } catch {
      setPacing(null); setCaptions(null); setDelivery(null)
    } finally {
      setBusy(false)
    }
  }, [])

  // Auto-run once when the tab mounts.
  useEffect(() => { void runChecks() }, [runChecks])

  const runBrand = useCallback(async () => {
    setBrandError(null)
    const r = await callVerb('verify.brand', {})
    if (r.ok) setBrand(r.result)
    else {
      setBrand(null)
      setBrandError([r.error?.message, r.error?.suggested_action].filter(Boolean).join(' · ') || 'Brand check failed')
    }
  }, [])

  const savedBrandKey = JSON.stringify(project?.brand ?? null)
  useEffect(() => {
    setBrandForm(brandDraft(project?.brand, project))
    if (project?.brand) void runBrand()
    else { setBrand(undefined); setBrandError(null) }
  }, [project?.name, project?.settings.width, project?.settings.height, savedBrandKey, runBrand])

  const updateBrand = <K extends keyof BrandDraft>(key: K, value: BrandDraft[K]) => {
    setBrandForm((current) => ({ ...current, [key]: value }))
  }

  const saveBrand = useCallback(async () => {
    if (!project || brandBusy) return
    setBrandBusy(true)
    setBrandError(null)
    try {
      const r = await callVerb('project.brand', {
        brand: kitFromDraft(brandForm),
        rationale: 'save project brand kit from Review QC',
      })
      if (!r.ok) {
        setBrandError([r.error?.message, r.error?.cause].filter(Boolean).join(' · ') || 'Could not save brand kit')
        return
      }
      const saved = r.result?.brand
      setBrandForm(brandDraft(saved ?? undefined, project))
      flash('Brand kit saved')
      await runBrand()
    } finally {
      setBrandBusy(false)
    }
  }, [brandBusy, brandForm, project, runBrand])

  const clearBrand = useCallback(async () => {
    if (!project || brandBusy) return
    setBrandBusy(true)
    setBrandError(null)
    try {
      const r = await callVerb('project.brand', { clear: true, rationale: 'clear project brand kit from Review QC' })
      if (!r.ok) {
        setBrandError([r.error?.message, r.error?.cause].filter(Boolean).join(' · ') || 'Could not clear brand kit')
        return
      }
      setBrand(undefined)
      setBrandForm(brandDraft(undefined, project))
      flash('Brand kit cleared')
    } finally {
      setBrandBusy(false)
    }
  }, [brandBusy, project])

  // verify.captions → captions.reflow (the fix), then re-check.
  const reflow = useCallback(async () => {
    const r = await callVerb('captions.reflow', {})
    if (r.ok) { const s = r.result as { extended: number; split: number; still_too_fast: number }; flash(`reflow: split ${s.split}, extended ${s.extended}, ${s.still_too_fast} still fast`) }
    else flash(`reflow: ${r.error?.code ?? 'failed'}`)
    await runChecks()
  }, [runChecks])

  const shift = useCallback(async () => {
    const r = await callVerb('captions.shift', { offset_ms: shiftMs })
    flash(r.ok ? `shifted captions ${shiftMs > 0 ? '+' : ''}${shiftMs}ms` : `shift: ${r.error?.code ?? 'failed'}`)
    await runChecks()
  }, [shiftMs, runChecks])

  const vCount = (v?: Record<string, unknown[]>) => v ? Object.values(v).reduce((n, a) => n + (Array.isArray(a) ? a.length : 0), 0) : 0

  return (
    <div className="qc" data-cut-qc>
      <div className="qc__bar">
        <button className="qc__run" data-cut-action="qc-run" disabled={busy} onClick={() => void runChecks()}>
          {busy ? 'checking…' : <><Icon name="reset" size={14} /> Run QC</>}
        </button>
        {/* AI perceptual review of the latest render (verify.judge). Async — the
            engine runs a subscription CLI on sampled frames, so it can take ~1 min. */}
        <button className="qc__run" data-cut-action="judge-run" disabled={judging} onClick={() => void runJudge()}>
          {judging ? 'reviewing…' : <><Icon name="agent" size={14} /> Get AI review</>}
        </button>
        {note && <span className="qc__note" data-cut-qc-note>{note}</span>}
      </div>

      {/* AI REVIEW card — verify.judge verdict, mirroring the Receipts JudgeSection
          display (verdict badge + confidence + issue count). Honest not_run/error
          states show the engine's reason verbatim; never a fabricated verdict. */}
      {(judging || judge !== undefined) && <JudgeCard judge={judge} busy={judging} />}

      {/* PACING — visual shot rhythm (measurement only) */}
      <Card title="Pacing" tag="verify.pacing" pass={undefined} data="pacing">
        {pacing === null ? <Err /> : pacing === undefined ? <Dim /> : (
          <ul className="qc__metrics">
            <li>{pacing.shot_count} shots · {pacing.internal_cuts} cuts</li>
            <li>{pacing.cuts_per_min}/min · mean shot {(pacing.mean_shot_ms / 1000).toFixed(1)}s</li>
          </ul>
        )}
      </Card>

      {/* CAPTIONS — QC vs timed-text standards + the reflow/shift fixes */}
      <Card title="Captions" tag="verify.captions" pass={captions?.pass} data="captions">
        {captions === null ? <Err /> : captions === undefined ? <Dim /> : (
          <>
            <ul className="qc__metrics">
              <li>{captions.cue_count} cues · max {captions.max_cps} CPS · mean {captions.mean_cps}</li>
              <li>{vCount(captions.violations)} violations{captions.note ? ` · ${captions.note}` : ''}</li>
            </ul>
            <div className="qc__fix">
              <button className="qc__fixbtn" data-cut-action="qc-reflow" onClick={() => void reflow()} title="Split long captions and extend cues that pass too quickly">Reflow ✓</button>
              <span className="qc__shift">
                shift
                <input type="number" step={50} value={shiftMs} data-cut-qc-shift aria-label="Caption shift in milliseconds" onChange={(e) => setShiftMs(parseInt(e.target.value || '0', 10))} />
                ms
                <button className="qc__fixbtn" data-cut-action="qc-shift" onClick={() => void shift()} title="Move every caption by this sync offset">apply</button>
              </span>
            </div>
          </>
        )}
      </Card>

      {/* DELIVERY — verbal pacing (WPM + fillers); fillers fix = remove_fillers */}
      <Card title="Delivery" tag="verify.delivery" pass={delivery?.pass} data="delivery">
        {delivery === null ? <Err /> : delivery === undefined ? <Dim /> : (
          <ul className="qc__metrics">
            <li>{delivery.wpm} WPM (articulation {delivery.articulation_wpm}) · {delivery.word_count} words</li>
            <li>{delivery.filler_count} fillers · {delivery.fillers_per_min}/min{delivery.note ? ` · ${delivery.note}` : ''}</li>
          </ul>
        )}
      </Card>

      {/* BRAND — durable project constraints, shared with publish packages. */}
      <Card title="Brand" tag="verify.brand" pass={brand?.pass} data="brand">
        <div className="qc__fix">
          <span className="qc__brand-status" data-cut-qc-brand-status={project?.brand ? 'saved' : 'not-saved'}>
            {project?.brand ? 'Saved for this project' : 'Not saved'}
          </span>
          <button className="qc__fixbtn" data-cut-action="qc-brand" disabled={!project || brandBusy} onClick={() => void runBrand()} title="Check caption styles and output geometry against the saved brand kit">
            <Icon name="check" size={14} /> Check
          </button>
        </div>
        <details className="qc__brand-editor" data-cut-qc-brand-editor>
          <summary data-cut-qc-brand-editor-toggle>Edit brand kit</summary>
          <div className="qc__brand-grid">
            <label className="qc__brand-wide">Fonts
              <input value={brandForm.fonts} data-cut-qc-brand-fonts onChange={(event) => updateBrand('fonts', event.target.value)} placeholder="Inter, Arial" />
            </label>
            <label className="qc__brand-wide">Palette
              <input value={brandForm.colors} data-cut-qc-brand-colors onChange={(event) => updateBrand('colors', event.target.value)} placeholder="#ffffff, #101820" />
            </label>
            {splitList(brandForm.colors)?.some((color) => /^#[0-9a-f]{3,8}$/i.test(color)) && (
              <div className="qc__brand-swatches" aria-label="Brand palette preview">
                {splitList(brandForm.colors)?.filter((color) => /^#[0-9a-f]{3,8}$/i.test(color)).map((color) => (
                  <span key={color} title={color} style={{ backgroundColor: color }} />
                ))}
              </div>
            )}
            <label>Position
              <select value={brandForm.position} data-cut-qc-brand-position onChange={(event) => updateBrand('position', event.target.value as BrandDraft['position'])}>
                <option value="">Any</option>
                <option value="bottom">Bottom</option>
                <option value="top">Top</option>
                <option value="center">Center</option>
              </select>
            </label>
            <label>Aspect
              <select value={brandForm.aspect} data-cut-qc-brand-aspect onChange={(event) => updateBrand('aspect', event.target.value)}>
                {!BRAND_ASPECTS.includes(brandForm.aspect as (typeof BRAND_ASPECTS)[number]) && <option value={brandForm.aspect}>{brandForm.aspect}</option>}
                {BRAND_ASPECTS.map((aspect) => <option key={aspect} value={aspect}>{aspect}</option>)}
              </select>
            </label>
            <label>Min size
              <input type="number" min={1} max={512} value={brandForm.minSize} data-cut-qc-brand-min-size onChange={(event) => updateBrand('minSize', event.target.value)} placeholder="24" />
            </label>
            <label>Max size
              <input type="number" min={1} max={512} value={brandForm.maxSize} data-cut-qc-brand-max-size onChange={(event) => updateBrand('maxSize', event.target.value)} placeholder="72" />
            </label>
          </div>
          <div className="qc__brand-actions">
            <button className="qc__fixbtn" data-cut-action="qc-brand-save" disabled={!project || brandBusy} onClick={() => void saveBrand()} title="Save these constraints in the project">
              <Icon name="save" size={14} /> {brandBusy ? 'Saving…' : 'Save'}
            </button>
            <button className="qc__fixbtn qc__fixbtn--muted" data-cut-action="qc-brand-clear" disabled={!project?.brand || brandBusy} onClick={() => void clearBrand()} title="Remove the saved brand kit">
              <Icon name="trash" size={14} /> Clear
            </button>
          </div>
        </details>
        {brandError && <div className="qc__err" data-cut-qc-brand-error>{brandError}</div>}
        {brand === null ? <Err /> : brand === undefined ? null : (
          <ul className="qc__metrics">
            <li>{brand.styles_checked} styles · {brand.source} kit{brand.note ? ` · ${brand.note}` : ''}</li>
          </ul>
        )}
      </Card>
    </div>
  )
}

function Card({ title, tag, pass, data, children }: { title: string; tag: string; pass?: boolean; data: string; children: React.ReactNode }) {
  const verdict = pass === undefined ? '' : pass ? 'qc__card--pass' : 'qc__card--fail'
  return (
    <div className={`qc__card ${verdict}`} data-cut-qc-card={data}>
      <div className="qc__card-head">
        <span className="qc__card-title">{title}</span>
        {pass !== undefined && <span className="qc__verdict" data-cut-qc-verdict={pass ? 'pass' : 'fail'}>{pass ? 'PASS' : 'FAIL'}</span>}
        <span className="qc__card-tag">{tag}</span>
      </div>
      {children}
    </div>
  )
}

const Dim = () => <div className="qc__dim">not run</div>
const Err = () => <div className="qc__err">no data (need a project + transcript/captions)</div>

/** AI-review card for verify.judge. Uses the QC card chrome (qc__card*) for visual
 *  consistency with the checks above, and the Receipts judge badge (rr-judge__badge,
 *  available because Review/index.tsx loads review.css) for the verdict. Honest
 *  states only: completed → real verdict; not_run → reason stub (never pass-like);
 *  error → reason line. `data-cut-judge` keys off the live status for the debug API. */
function JudgeHead({ badge }: { badge?: React.ReactNode }) {
  return (
    <div className="qc__card-head">
      <span className="qc__card-title">AI Review</span>
      {badge}
      <span className="qc__card-tag">Visual review</span>
    </div>
  )
}

function JudgeCard({ judge, busy }: { judge: Cell<JudgeJobResult>; busy: boolean }) {
  // Polling (or a fresh run just cleared the prior verdict).
  if (busy && (judge === undefined || judge === null)) {
    return (
      <div className="qc__card" data-cut-qc-card="judge" data-cut-judge="running">
        <JudgeHead />
        <div className="qc__dim">reviewing… (runs a subscription CLI on the latest render — can take ~1 min)</div>
      </div>
    )
  }
  // null = ok envelope but no usable result (defensive; never pass-like).
  if (judge === null) {
    return (
      <div className="qc__card qc__card--fail" data-cut-qc-card="judge" data-cut-judge="error">
        <JudgeHead />
        <div className="qc__err">judge returned no result</div>
      </div>
    )
  }
  if (judge === undefined) return null

  const { status, verdict, confidence, issues } = judge
  const reason = judge.reason ?? judge.not_run_reason
  const vKind = verdict === 'pass' ? 'pass' : verdict === 'fail' ? 'fail' : 'review'
  // Border only on a measured verdict; needs_review stays neutral (the badge carries it).
  const cardMod = status === 'completed' && verdict === 'pass' ? 'qc__card--pass'
    : status === 'completed' && verdict === 'fail' ? 'qc__card--fail' : ''
  const hook = status === 'completed' ? (verdict ?? 'completed') : (status ?? 'unknown')

  return (
    <div className={`qc__card ${cardMod}`} data-cut-qc-card="judge" data-cut-judge={hook}>
      <JudgeHead badge={status === 'completed' && verdict ? (
        <span className={`rr-judge__badge rr-judge__badge--${vKind}`} data-cut-qc-verdict={verdict}>
          {verdict === 'needs_review' ? 'NEEDS REVIEW' : verdict.toUpperCase()}
        </span>
      ) : undefined} />

      {status === 'completed' ? (
        <ul className="qc__metrics">
          <li>
            {typeof confidence === 'number' ? `${confidence.toFixed(2)} confidence` : 'confidence n/a'}
            {' · '}
            {typeof issues === 'number' ? `${issues} issue${issues === 1 ? '' : 's'}` : 'issues n/a'}
          </li>
          {typeof issues === 'number' && issues > 0 && (
            <li>open the Receipts tab for per-issue detail + seek links</li>
          )}
        </ul>
      ) : status === 'not_run' ? (
        <div className="qc__dim" data-cut-judge-reason>JUDGE NOT RUN{reason ? ` — ${reason}` : ''}</div>
      ) : (
        <div className="qc__err" data-cut-judge-reason>JUDGE ERROR{reason ? ` — ${reason}` : ''}</div>
      )}
    </div>
  )
}
