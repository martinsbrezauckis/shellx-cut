// topbar/RenderQueueModal — BATCH DELIVERY surface (render.queue), the
// Batch render queue. Role: let the user stack
// N deliveries — each its own output file + quality preset + format/aspect — and
// fire ONE render.queue that runs them SEQUENTIALLY through the same render.final
// path (job + segmented encode + auto verify.checks → RenderReceipt). The queue is
// itself a background job (queue_id); the app owner polls jobs.status{job_id: queue_id} for overall
// progress + per-entry state as each delivery completes. render.queue is a pure
// delivery orchestrator: it records NO op and makes NO timeline mutation, so this is
// a display-only surface (nothing to undo). Honest degradation: a bad entry (unknown
// key / bad enum) fails the WHOLE queue UP FRONT via a per-entry dry_run — we show
// the engine's message verbatim.
//
// Callers: topbar (Export menu → "Render queue / batch…"). Deps: lib/client
// (render.queue + jobs.status), icons, renderqueue.css.

import { useCallback, useEffect, useRef, useState } from 'react'
import { type VerbArgs } from '../lib/client'
import { outputDirectoryForPath } from '../lib/exportDestination'
import { mediaBasename } from '../lib/mediaPath'
import { isTauri, pickRenderOutput } from '../lib/tauri'
import { Icon } from '../icons'
import type { VideoPreflightStatus } from './videoPreflight'
import { useBlockingOverlay } from '../components/overlay/useBlockingOverlay'
import { type RenderQueueOwner, type RenderQueueRow, newRenderQueueRow } from './useRenderQueueOwner'
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
type Row = RenderQueueRow

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

export interface RenderQueueModalProps {
  onClose: () => void
  onPreflight: (actionLabel: string, action: () => Promise<void>, onCancel: () => void, isActive: () => boolean) => Promise<VideoPreflightStatus>
  owner: RenderQueueOwner
}

export default function RenderQueueModal({ onClose, onPreflight, owner }: RenderQueueModalProps) {
  const overlay = useBlockingOverlay<HTMLDivElement>(onClose)
  const { rows, phase, error: err, admitted, progress, result: queue } = owner.state
  const queueId = admitted?.id ?? null
  const [pickerNote, setPickerNote] = useState<string | null>(null)
  const cancelled = useRef(false)
  const submitting = useRef(false)
  const [checking, setChecking] = useState(false)
  const [awaitingWarning, setAwaitingWarning] = useState(false)
  const formLocked = checking || awaitingWarning

  useEffect(() => () => { cancelled.current = true }, [])

  const releasePreflight = useCallback(() => {
    submitting.current = false
    if (cancelled.current) return
    setChecking(false)
    setAwaitingWarning(false)
  }, [])

  const setRow = (i: number, patch: Partial<Row>) => {
    if (submitting.current) return
    owner.setRows((rs) => rs.map((r, k) => (k === i ? { ...r, ...patch } : r)))
  }
  const addRow = () => {
    if (submitting.current) return
    owner.setRows((rs) => [...rs, newRenderQueueRow()])
  }
  const removeRow = (i: number) => {
    if (submitting.current) return
    owner.setRows((rs) => (rs.length <= 1 ? rs : rs.filter((_, k) => k !== i)))
  }
  const chooseOutput = async (i: number) => {
    if (submitting.current) return
    setPickerNote(null)
    if (!isTauri()) {
      setPickerNote('Open the desktop app to choose an output file.')
      return
    }
    const epoch = owner.intentEpoch()
    try {
      const path = await pickRenderOutput()
      if (cancelled.current || !owner.intentCurrent(epoch) || submitting.current) return
      if (path) setRow(i, { output: path })
    } catch {
      if (!cancelled.current && owner.intentCurrent(epoch)) setPickerNote('Could not choose an output file. Try again.')
    }
  }

  // Map the form rows → render.final arg subsets. 'project' aspect omits the arg (a
  // normal full render); a non-default output sets render.final's path (via `output`).
  const buildJobs = useCallback((): VerbArgs['render.queue']['jobs'] =>
    rows.map((r) => ({
      preset: r.preset,
      ...(r.aspect !== 'project' ? { aspect: r.aspect } : {}),
      ...(r.output.trim() ? { output: r.output.trim() } : {}),
    })), [rows])

  const submit = useCallback(async () => {
    if (submitting.current || phase !== 'form') return
    owner.setFormError(null)
    try {
      const duplicate = duplicateOutputPaths(rows)
      if (duplicate) { owner.setFormError('Each queued delivery must use a different output file.'); return }
      const outputDirs = [...new Set(rows
        .map((row) => row.output.trim())
        .filter(Boolean)
        .map(outputDirectoryForPath)
        .filter((dir): dir is string => !!dir)
        .map((dir) => dir.replace(/\\/g, '/').toLowerCase()))]
      const explicitOutputCount = rows.filter((row) => row.output.trim()).length
      if (explicitOutputCount > 0 && explicitOutputCount < rows.length) {
        owner.setFormError('Choose output files for every queued render, or use the default exports folder for every row.')
        return
      }
      if (outputDirs.length > 1) {
        owner.setFormError('Choose all queue outputs in one folder, or use the default exports folder for every row.')
        return
      }
      const jobs = buildJobs()
      const explicitPath = rows.find((row) => row.output.trim())?.output.trim()
      const epoch = owner.intentEpoch()
      submitting.current = true
      setChecking(true)
      const preflightStatus = await onPreflight('rendering queued deliveries', async () => {
        if (cancelled.current || !owner.intentCurrent(epoch)) return
        setChecking(false)
        setAwaitingWarning(false)
        await owner.submit(jobs, explicitPath, epoch)
        releasePreflight()
      }, releasePreflight, () => !cancelled.current)
      if (preflightStatus === 'warning') {
        if (!cancelled.current && submitting.current) { setChecking(false); setAwaitingWarning(true) }
      } else {
        releasePreflight()
        if (preflightStatus === 'blocked') owner.setFormError('Install FFmpeg before rendering queued deliveries.')
      }
    } catch (e) {
      releasePreflight()
      owner.setFormError(e instanceof Error ? e.message : String(e))
    }
  }, [buildJobs, onPreflight, owner, phase, releasePreflight, rows])

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

        {owner.foreign && <div className="rq-error" data-cut-render-queue-owner-other>
          <div className="rq-error-msg">A previous project's render queue is still owned by that project. Reopen it to check the exact queue status.</div>
        </div>}

        {!owner.foreign && phase === 'form' && (
          <div className="rq-form" data-cut-render-queue-form>
            {formLocked && <p className="rq-note" data-cut-render-queue-preflight-status>
              {checking ? 'Checking export before queueing…' : 'Review the preflight warning to continue or cancel.'}
            </p>}
            <fieldset className="rq-fields" disabled={formLocked} data-cut-render-queue-fields>
            <div className="rq-rows">
              {rows.map((r, i) => (
                <div className="rq-row" data-cut-render-queue-row={i} key={i}>
                  <span className="rq-row-n">{i + 1}</span>
                  <span
                    className="rq-out"
                    data-cut-render-queue-output={i}
                  >{r.output ? mediaBasename(r.output) : 'Default exports folder'}</span>
                  <button
                    className="rq-pick"
                    data-cut-render-queue-output-pick={i}
                    title={r.output ? 'Change output file' : 'Choose output file'}
                    aria-label={`${r.output ? 'Change' : 'Choose'} output file for delivery ${i + 1}`}
                    onClick={() => void chooseOutput(i)}
                  ><Icon name="save" size={14} label="Choose output file" /></button>
                  {r.output && <button
                    className="rq-row-x"
                    data-cut-render-queue-output-clear={i}
                    title="Use the default exports folder"
                    aria-label={`Use the default exports folder for delivery ${i + 1}`}
                    onClick={() => setRow(i, { output: '' })}
                  ><Icon name="close" size={14} label="clear output" /></button>}
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
            {err && <div className="rq-error-msg rq-form-error" role="alert" data-cut-render-queue-form-error>
              <Icon name="warning" size={16} tone="warn" /> {err}
            </div>}
            <button className="rq-add" data-cut-render-queue-add onClick={addRow}>
              <Icon name="plus" size={14} /> Add a delivery
            </button>
            <div className="rq-actions">
              <span className="rq-count" data-cut-render-queue-count={rows.length}>{rows.length} deliveries · runs sequentially</span>
              <button className="rq-btn rq-btn--primary" data-cut-render-queue-start onClick={() => void submit()}>
                Render queue
              </button>
            </div>
            </fieldset>
          </div>
        )}

        {!owner.foreign && (phase === 'running' || phase === 'status_unknown' || phase === 'submitting' || phase === 'done') && (
          <div className="rq-progress" data-cut-render-queue-progress={phase}>
            <div className="rq-overall">
              <span>{phase === 'done' ? 'Queue complete' : phase === 'status_unknown' ? 'Queue status unknown; checking again…'
                : phase === 'submitting' ? 'Dispatching the render queue…' : 'Rendering deliveries…'}</span>
              <span className="rq-pct" data-cut-render-queue-pct={pct}>{pct}%</span>
            </div>
            <div className="rq-bar"><div className="rq-bar-fill" style={{ width: `${Math.max(4, pct)}%` }} /></div>
            <ul className="rq-list" data-cut-render-queue-list>
              {entries.length > 0
                ? entries.map((e, k) => (
                    <li className="rq-item" data-cut-render-queue-item={e.idx ?? k} key={e.idx ?? k}>
                      <span className="rq-item-n">{(e.idx ?? k) + 1}</span>
                      <span className="rq-item-out">{e.output ? mediaBasename(e.output) : `exports/${e.job_id ?? 'pending'}`}</span>
                      <span className={`rq-item-state rq-item-state--${e.state ?? 'pending'}`}>{e.state ?? (phase === 'done' ? 'done' : 'pending')}</span>
                    </li>
                  ))
                : <li className="rq-item rq-item--empty">{queueId ? `queue ${queueId} dispatched…` : 'dispatching…'}</li>}
            </ul>
            {phase === 'done' && (
              <div className="rq-actions">
                <span className="rq-count" data-cut-render-queue-done>Find the files in the Review tab.</span>
                <button className="rq-btn rq-btn--primary" data-cut-render-queue-done-close onClick={() => { owner.acknowledge(); onClose() }}>Done</button>
              </div>
            )}
          </div>
        )}

        {!owner.foreign && (phase === 'error' || phase === 'submit_unknown') && (
          <div className="rq-error" data-cut-render-queue-error>
            <div className="rq-error-msg"><Icon name="warning" size={16} tone="warn" /> {err}</div>
            <div className="rq-actions">
              <button className="rq-btn" data-cut-render-queue-error-back onClick={owner.acknowledge}>{phase === 'submit_unknown' ? 'Start another queue' : 'Back'}</button>
              <button className="rq-btn rq-btn--primary" data-cut-render-queue-error-close onClick={onClose}>Close</button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
