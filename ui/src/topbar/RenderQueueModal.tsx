// topbar/RenderQueueModal — BATCH DELIVERY surface (render.queue), the
// Batch render queue. Role: let the user stack
// N deliveries — each its own output file + quality preset + format/aspect — and
// fire ONE render.queue that runs them SEQUENTIALLY through the same render.final
// path (job + segmented encode + auto verify.checks → RenderReceipt). The queue is
// itself a background job (queue_id); we poll jobs.status{queue_id} for overall
// progress + per-entry state as each delivery completes. render.queue is a pure
// delivery orchestrator: it records NO op and makes NO timeline mutation, so this is
// a display-only surface (nothing to undo). Honest degradation: a bad entry (unknown
// key / bad enum) fails the WHOLE queue UP FRONT via a per-entry dry_run — we show
// the engine's message verbatim.
//
// Callers: topbar (Export menu → "Render queue / batch…"). Deps: lib/client
// (render.queue + jobs.status), icons, renderqueue.css.

import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type VerbArgs, type VerbResults } from '../lib/client'
import { outputDirectoryForPath, withAuthorizedOutputPath } from '../lib/exportDestination'
import { isTauri, pickRenderOutput } from '../lib/tauri'
import { Icon } from '../icons'
import { useBlockingOverlay } from '../components/overlay/useBlockingOverlay'
import './renderqueue.css'

/** render.final quality tiers + reframe aspects (schema enums; mirror topbar). */
const PRESETS = ['draft', 'standard', 'high'] as const
type Preset = (typeof PRESETS)[number]
const ASPECTS = ['project', '16:9', '9:16', '1:1', '4:5'] as const
type Aspect = (typeof ASPECTS)[number]

function presetFromInput(value: string, fallback: Preset): Preset {
  for (const option of PRESETS) {
    if (option === value) return option
  }
  return fallback
}

function aspectFromInput(value: string, fallback: Aspect): Aspect {
  for (const option of ASPECTS) {
    if (option === value) return option
  }
  return fallback
}

/** One queue ROW in the form (the editable shape; mapped to a render.final arg
 * subset on submit). output is optional — empty = the engine's default
 * <project>/exports/<render_id> path. */
interface Row {
  output: string
  preset: Preset
  aspect: Aspect
}
const newRow = (): Row => ({ output: '', preset: 'standard', aspect: 'project' })

type Phase = 'form' | 'running' | 'done' | 'error'

function duplicateOutputPaths(rows: Row[]): string | null {
  const seen = new Set<string>()
  for (const row of rows) {
    const raw = row.output.trim()
    if (!raw) continue
    const normalized = raw.replace(/\\/g, '/').replace(/\/+/g, '/').toLowerCase()
    if (seen.has(normalized)) return raw
    seen.add(normalized)
  }
  return null
}

/** The queue job's accruing result — per-entry job ids/outputs/receipts land here
 * (jobs.status{queue_id}.result) as each render completes. Read defensively. */
interface QueueResult {
  queue_id?: string
  count?: number
  jobs?: Array<{ idx?: number; output?: string; job_id?: string; state?: string }>
}

export interface RenderQueueModalProps {
  onClose: () => void
}

export default function RenderQueueModal({ onClose }: RenderQueueModalProps) {
  const overlay = useBlockingOverlay<HTMLDivElement>(onClose)
  // Start with TWO rows — a batch is ≥2 deliveries; one row would just be Render.
  const [rows, setRows] = useState<Row[]>([newRow(), { ...newRow(), aspect: '9:16' }])
  const [phase, setPhase] = useState<Phase>('form')
  const [err, setErr] = useState<string | null>(null)
  const [queueId, setQueueId] = useState<string | null>(null)
  const [progress, setProgress] = useState(0)
  const [queue, setQueue] = useState<QueueResult | null>(null)
  const [pickerNote, setPickerNote] = useState<string | null>(null)
  const cancelled = useRef(false)

  useEffect(() => () => { cancelled.current = true }, [])

  const setRow = (i: number, patch: Partial<Row>) =>
    setRows((rs) => rs.map((r, k) => (k === i ? { ...r, ...patch } : r)))
  const addRow = () => setRows((rs) => [...rs, newRow()])
  const removeRow = (i: number) => setRows((rs) => (rs.length <= 1 ? rs : rs.filter((_, k) => k !== i)))
  const chooseOutput = async (i: number) => {
    setPickerNote(null)
    if (!isTauri()) {
      setPickerNote('Open the desktop app to choose an output file, or paste a full path.')
      return
    }
    const path = await pickRenderOutput()
    if (path) setRow(i, { output: path })
  }

  // Map the form rows → render.final arg subsets. 'project' aspect omits the arg (a
  // normal full render); a non-default output sets render.final's path (via `output`).
  const buildJobs = useCallback((): VerbArgs['render.queue']['jobs'] =>
    rows.map((r) => ({
      preset: r.preset,
      ...(r.aspect !== 'project' ? { aspect: r.aspect } : {}),
      ...(r.output.trim() ? { output: r.output.trim() } : {}),
    })), [rows])

  // Poll the queue job until it reaches a terminal state, folding overall progress +
  // the accruing per-entry result into view. Mirrors DirectorModal's pollJob.
  const pollQueue = useCallback(async (qid: string) => {
    for (;;) {
      if (cancelled.current) return
      const r = await callVerb('jobs.status', { job_id: qid })
      const rec = r.result as VerbResults['jobs.status'] | undefined
      if (rec) {
        if (typeof rec.progress === 'number') setProgress(rec.progress)
        const res = rec.result as QueueResult | undefined
        if (res) setQueue(res)
        if (rec.state === 'done') { setPhase('done'); return }
        if (rec.state === 'failed') { setErr(rec.error?.message ?? rec.error?.code ?? 'a delivery failed'); setPhase('error'); return }
      } else if (!r.ok) {
        setErr(r.error?.message ?? 'lost the queue job'); setPhase('error'); return
      }
      await new Promise((resolve) => setTimeout(resolve, 700))
    }
  }, [])

  const submit = useCallback(async () => {
    setErr(null)
    setProgress(0)
    setQueue(null)
    setPhase('running')
    try {
      const duplicate = duplicateOutputPaths(rows)
      if (duplicate) { setErr(`Each queued output path must be unique: ${duplicate}`); setPhase('form'); return }
      const outputDirs = [...new Set(rows
        .map((row) => row.output.trim())
        .filter(Boolean)
        .map(outputDirectoryForPath)
        .filter((dir): dir is string => !!dir)
        .map((dir) => dir.replace(/\\/g, '/').toLowerCase()))]
      const explicitOutputCount = rows.filter((row) => row.output.trim()).length
      if (explicitOutputCount > 0 && explicitOutputCount < rows.length) {
        setErr('Choose explicit output files for every queued render, or leave every path empty to use the default export folder.')
        setPhase('form')
        return
      }
      if (outputDirs.length > 1) {
        setErr('Choose explicit queue outputs in one folder, or leave paths empty to use the default export folder.')
        setPhase('form')
        return
      }
      const jobs = buildJobs()
      const explicitPath = rows.find((row) => row.output.trim())?.output.trim()
      const r = await withAuthorizedOutputPath(explicitPath, () =>
        callVerb('render.queue', { jobs, rationale: `batch deliver ${jobs.length} renders` }))
      if (!r.ok) { setErr(r.error?.message ?? r.error?.code ?? 'render.queue rejected'); setPhase('error'); return }
      const res = r.result as QueueResult | undefined
      const qid = res?.queue_id
      setQueue(res ?? null)
      if (!qid) { setErr('render.queue returned no queue id'); setPhase('error'); return }
      setQueueId(qid)
      await pollQueue(qid)
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e))
      setPhase('error')
    }
  }, [buildJobs, pollQueue, rows])

  const pct = Math.round(progress * 100)
  const entries = queue?.jobs ?? []

  return (
    <div className="rq-overlay" data-cut-render-queue onMouseDown={overlay.onScrimMouseDown}>
      <div ref={overlay.dialogRef} className="rq-modal" role="dialog" aria-modal="true" aria-label="Render queue"
        data-cut-blocking-overlay tabIndex={-1} onKeyDown={overlay.onDialogKeyDown}>
        <header className="rq-head">
          <span className="rq-title"><Icon name="render" size={16} tone="brand" /> Render queue <span className="rq-sub">batch deliver</span></span>
          <button className="rq-x" data-cut-render-queue-close onClick={onClose} aria-label="Close">×</button>
        </header>

        {phase === 'form' && (
          <div className="rq-form" data-cut-render-queue-form>
            <div className="rq-rows">
              {rows.map((r, i) => (
                <div className="rq-row" data-cut-render-queue-row={i} key={i}>
                  <span className="rq-row-n">{i + 1}</span>
                  <input
                    className="rq-out"
                    data-cut-render-queue-output={i}
                    placeholder="Choose output file or leave empty for exports"
                    value={r.output}
                    onChange={(e) => setRow(i, { output: e.target.value })}
                  />
                  <button
                    className="rq-pick"
                    data-cut-render-queue-output-pick={i}
                    title="Choose output file"
                    aria-label={`Choose output file for delivery ${i + 1}`}
                    onClick={() => void chooseOutput(i)}
                  ><Icon name="save" size={14} label="Choose output file" /></button>
                  <select
                    className="rq-sel"
                    data-cut-render-queue-preset={i}
                    value={r.preset}
                    title="Quality tier"
                    onChange={(e) => setRow(i, { preset: presetFromInput(e.target.value, r.preset) })}
                  >
                    {PRESETS.map((p) => (
                      <option key={p} value={p}>{p === 'draft' ? 'Draft' : p === 'high' ? 'High' : 'Standard'}</option>
                    ))}
                  </select>
                  <select
                    className="rq-sel"
                    data-cut-render-queue-aspect={i}
                    value={r.aspect}
                    title="Delivery format — non-project values reframe this delivery (subject-aware crop)"
                    onChange={(e) => setRow(i, { aspect: aspectFromInput(e.target.value, r.aspect) })}
                  >
                    {ASPECTS.map((a) => (
                      <option key={a} value={a}>{a === 'project' ? 'Project size' : a}</option>
                    ))}
                  </select>
                  <button
                    className="rq-row-x"
                    data-cut-render-queue-remove={i}
                    disabled={rows.length <= 1}
                    title={rows.length <= 1 ? 'A queue needs at least one delivery' : 'Remove this delivery'}
                    onClick={() => removeRow(i)}
                  ><Icon name="close" size={14} label="remove" /></button>
                </div>
              ))}
            </div>
            {pickerNote && <p className="rq-note" data-cut-render-queue-note>{pickerNote}</p>}
            <button className="rq-add" data-cut-render-queue-add onClick={addRow}>
              <Icon name="plus" size={14} /> Add a delivery
            </button>
            <div className="rq-actions">
              <span className="rq-count" data-cut-render-queue-count={rows.length}>{rows.length} deliveries · runs sequentially</span>
              <button className="rq-btn rq-btn--primary" data-cut-render-queue-start onClick={() => void submit()}>
                Render queue
              </button>
            </div>
          </div>
        )}

        {(phase === 'running' || phase === 'done') && (
          <div className="rq-progress" data-cut-render-queue-progress={phase}>
            <div className="rq-overall">
              <span>{phase === 'done' ? 'Queue complete' : 'Rendering deliveries…'}</span>
              <span className="rq-pct" data-cut-render-queue-pct={pct}>{pct}%</span>
            </div>
            <div className="rq-bar"><div className="rq-bar-fill" style={{ width: `${Math.max(4, pct)}%` }} /></div>
            <ul className="rq-list" data-cut-render-queue-list>
              {entries.length > 0
                ? entries.map((e, k) => (
                    <li className="rq-item" data-cut-render-queue-item={e.idx ?? k} key={e.idx ?? k}>
                      <span className="rq-item-n">{(e.idx ?? k) + 1}</span>
                      <span className="rq-item-out">{e.output || `exports/${e.job_id ?? 'pending'}`}</span>
                      <span className={`rq-item-state rq-item-state--${e.state ?? 'pending'}`}>{e.state ?? (phase === 'done' ? 'done' : 'pending')}</span>
                    </li>
                  ))
                : <li className="rq-item rq-item--empty">{queueId ? `queue ${queueId} dispatched…` : 'dispatching…'}</li>}
            </ul>
            {phase === 'done' && (
              <div className="rq-actions">
                <span className="rq-count" data-cut-render-queue-done>Find the files in the Review tab.</span>
                <button className="rq-btn rq-btn--primary" data-cut-render-queue-done-close onClick={onClose}>Done</button>
              </div>
            )}
          </div>
        )}

        {phase === 'error' && (
          <div className="rq-error" data-cut-render-queue-error>
            <div className="rq-error-msg"><Icon name="warning" size={16} tone="warn" /> {err}</div>
            <div className="rq-actions">
              <button className="rq-btn" data-cut-render-queue-error-back onClick={() => setPhase('form')}>Back</button>
              <button className="rq-btn rq-btn--primary" data-cut-render-queue-error-close onClick={onClose}>Close</button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
