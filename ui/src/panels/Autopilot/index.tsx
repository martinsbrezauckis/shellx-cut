// panels/Autopilot — the Receipted Autopilot drawer for autopilot.run.
//
// Role: a right-side drawer (the MusicBed/Grade/Clips family) that runs the
// autopilot — render → verify → MECHANICALLY self-fix from the receipt's
// fix_actions → re-verify, under one auto-checkpoint. The user picks a policy
// (Preview = plan only, approve first; Auto-fix = apply the mechanical fixes),
// optionally types a goal, and hits Run; the drawer polls the job and shows a
// CLEAN report.
//
// RECEIPT PHILOSOPHY: the result is ONE summary line + a compact
// pass badge + the short fix list — NEVER an ocean of receipts. The full
// receipts/op-log live in the Inspect rail ("Open Inspect" link). A Restore
// button reverts the whole run in one step (project.revert to the checkpoint).
//
// Callers: App.tsx (mounted when activeDrawer === 'autopilot'). Deps: lib/client,
// ../drawer.css, ./autopilot.css.

import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type AutopilotReport } from '../../lib/client'
import { Icon } from '../../icons'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import '../drawer.css'
import './autopilot.css'

export interface AutopilotDrawerProps {
  onClose: () => void
}

type Policy = 'preview' | 'auto_low_risk'

function actionLabel(verb: string): string {
  const labels: Record<string, string> = {
    'render.final': 'Render again',
    'edit.trim': 'Trim clip',
    'edit.trim_edges': 'Trim clip edges',
    'captions.reflow': 'Improve caption timing',
    'captions.generate': 'Create captions',
    '(manual)': 'Needs your review',
  }
  return labels[verb] ?? verb.replaceAll('.', ' ').replaceAll('_', ' ')
}

export default function AutopilotDrawer({ onClose }: AutopilotDrawerProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const [goal, setGoal] = useState('')
  const [policy, setPolicy] = useState<Policy>('preview')
  const [running, setRunning] = useState(false)
  const [phase, setPhase] = useState<string>('')
  const [report, setReport] = useState<AutopilotReport | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const [restored, setRestored] = useState(false)
  const pollTimer = useRef<number | null>(null)

  // Clear the poll ONLY on unmount — NOT tied to onClose. App passes a fresh
  // closeDrawer ref every render, so folding this into the keydown effect above
  // re-ran its cleanup on every App re-render (and the autopilot's renders emit
  // many WS events → many re-renders), clearing pollTimer and silently killing
  // the job poll so the report never arrived.
  useEffect(() => () => { if (pollTimer.current) window.clearTimeout(pollTimer.current) }, [])

  // `policyArg` lets a caller (the Apply button) run with a policy NOW, without
  // waiting for a setPolicy re-render to rebind this callback. setPolicy only
  // schedules a re-render with a NEW `run`; the click that fired this invocation
  // still holds the OLD closure, so reading `policy` from the closure would send
  // the stale value. Defaulting to the state `policy` keeps the normal Run button
  // (no arg) behaving as before. effPolicy is used for BOTH the verb arg and the
  // phase label so they never diverge from what was actually sent.
  const run = useCallback(async (policyArg?: Policy) => {
    if (running) return
    const effPolicy: Policy = policyArg ?? policy
    setRunning(true)
    setErr(null)
    setReport(null)
    setRestored(false)
    setPhase('planning…')
    const r = await callVerb('autopilot.run', {
      ...(goal.trim() ? { goal: goal.trim() } : {}),
      policy: effPolicy,
      max_fix_iters: 3,
    })
    if (!r.ok || !r.result) {
      setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'autopilot.run failed'}`)
      setRunning(false)
      return
    }
    const jobId = (r.result as { job_id: string }).job_id
    setPhase(effPolicy === 'preview' ? 'render + verify…' : 'render · verify · self-fix…')
    const poll = async () => {
      const j = await callVerb('jobs.status', { job_id: jobId })
      if (j.ok && j.result) {
        const st = j.result.state
        if (st === 'done') {
          setReport((j.result.result as AutopilotReport) ?? null)
          setRunning(false)
          return
        }
        if (st === 'failed') {
          setErr(`autopilot failed: ${j.result.error?.message ?? 'render/verify error'}`)
          setRunning(false)
          return
        }
        // progress message rides on the job (best-effort)
        if (typeof j.result.progress === 'number') {
          setPhase(`working… ${Math.round(j.result.progress * 100)}%`)
        }
      }
      pollTimer.current = window.setTimeout(() => void poll(), 1200)
    }
    pollTimer.current = window.setTimeout(() => void poll(), 1000)
  }, [running, goal, policy])

  // One-step revert of the whole run (project.revert to the auto-checkpoint).
  const restore = useCallback(async () => {
    if (!report?.checkpoint) return
    const r = await callVerb('project.revert', {
      to: report.checkpoint,
      rationale: 'undo autopilot run',
    })
    if (r.ok) setRestored(true)
    else setErr(`restore failed: ${r.error?.message ?? ''}`)
  }, [report])

  const openInspect = () => document.dispatchEvent(new CustomEvent('cut:open-receipts'))

  return (
    <div className="cd-scrim" data-cut-autopilot-scrim onMouseDown={overlay.onScrimMouseDown}>
      <aside
        ref={overlay.dialogRef}
        className="cd-drawer"
        data-cut-autopilot
        data-cut-autopilot-open="true"
        role="dialog"
        aria-modal="true"
        aria-label="Autopilot"
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="cd-head">
          <div>
            <h2 className="cd-title">Autopilot</h2>
            <p className="cd-sub">
              Preview a quality pass, then approve low-risk fixes. Every run keeps one restore point
              and reports only what it actually checked.
            </p>
          </div>
          <button className="cd-btn cd-btn--ghost" data-cut-autopilot-close onClick={onClose}>
            Close
          </button>
        </header>

        <div className="cd-body">
          <label className="cd-field">
            <span className="cd-field-label">Goal (optional)</span>
            <input
              className="cd-input"
              data-cut-autopilot-goal
              placeholder="e.g. clean up the cut for publish"
              value={goal}
              onChange={(e) => setGoal(e.target.value)}
              disabled={running}
            />
          </label>

          <div className="ap-policy" data-cut-autopilot-policy>
            <span className="cd-field-label">Mode:</span>
            <label className={`ap-radio ${policy === 'preview' ? 'ap-radio--on' : ''}`}>
              <input type="radio" name="ap-policy" checked={policy === 'preview'} onChange={() => setPolicy('preview')} disabled={running} data-cut-policy="preview" />
              Preview <span className="ap-radio-hint">plan only, approve first</span>
            </label>
            <label className={`ap-radio ${policy === 'auto_low_risk' ? 'ap-radio--on' : ''}`}>
              <input type="radio" name="ap-policy" checked={policy === 'auto_low_risk'} onChange={() => setPolicy('auto_low_risk')} disabled={running} data-cut-policy="auto_low_risk" />
              Auto-fix <span className="ap-radio-hint">apply low-risk fixes</span>
            </label>
          </div>

          <button
            className="cd-btn cd-btn--primary ap-run"
            data-cut-autopilot-run
            disabled={running}
            aria-busy={running}
            onClick={() => void run()}
          >
            {running ? (phase || 'Working…') : policy === 'preview' ? 'Preview fixes' : 'Run autopilot'}
          </button>

          {err && <div className="cd-err" data-cut-autopilot-error role="alert">{err}</div>}

          {report && (
            <div className="ap-report" data-cut-autopilot-report role="status" aria-live="polite">
              <div className={`ap-summary ${report.checks_pass ? 'ap-summary--pass' : 'ap-summary--warn'}`} data-cut-autopilot-summary data-cut-autopilot-pass={String(report.checks_pass)}>
                <span className="ap-summary-badge">{report.checks_pass
                  ? <Icon name="check" size={16} tone="success" label="all checks pass" />
                  : report.policy === 'preview'
                    ? <Icon name="pending" size={16} tone="brand" label="preview" />
                    : <Icon name="warning" size={16} tone="warn" label="needs attention" />}</span>
                <span>{report.summary_line}</span>
              </div>

              {/* changed (compact, user-facing) */}
              {report.changed && (report.changed.ops ?? 0) > 0 && (
                <div className="ap-changed" data-cut-autopilot-changed>
                  {report.changed.ops} op(s)
                  {report.changed.duration_delta_ms != null && report.changed.duration_delta_ms !== 0 && (
                    <> · {report.changed.duration_delta_ms > 0 ? '+' : ''}{(report.changed.duration_delta_ms / 1000).toFixed(1)}s</>
                  )}
                </div>
              )}

              {/* PREVIEW: the plan of what it WOULD fix */}
              {report.policy === 'preview' && report.plan.length > 0 && (
                <div className="ap-list" data-cut-autopilot-plan>
                  <div className="ap-list-head">Would fix:</div>
                  {report.plan.map((p, i) => (
                    <div className="ap-list-row" key={i}>
                      <span className={`ap-tag ${p.auto_fixable ? 'ap-tag--auto' : 'ap-tag--manual'}`}>{p.auto_fixable ? 'auto' : 'manual'}</span>
                      <span className="ap-check">{p.check}</span>
                      <span className="ap-via">→ {actionLabel(p.fix_verb)}</span>
                    </div>
                  ))}
                  {report.plan.some((p) => p.auto_fixable) && (
                    <button className="cd-btn cd-btn--primary ap-apply" data-cut-autopilot-apply onClick={() => { setPolicy('auto_low_risk'); void run('auto_low_risk') }} disabled={running}>
                      Apply fixes
                    </button>
                  )}
                </div>
              )}

              {/* AUTO-FIX: what it applied */}
              {report.policy === 'auto_low_risk' && report.fixes_applied.length > 0 && (
                <div className="ap-list" data-cut-autopilot-fixes>
                  <div className="ap-list-head">Applied:</div>
                  {report.fixes_applied.map((f, i) => (
                    <div className="ap-list-row" key={i}>
                      <span className={`ap-tag ${f.failed ? 'ap-tag--failed' : 'ap-tag--auto'}`}>{f.failed ? 'failed' : 'fixed'}</span>
                      <span className="ap-check">{f.check}</span>
                      <span className="ap-via">→ {actionLabel(f.via)}</span>
                    </div>
                  ))}
                </div>
              )}

              <div className="ap-actions">
                {report.fixes_applied.length > 0 && !restored && (
                  <button className="cd-btn cd-btn--ghost" data-cut-autopilot-restore onClick={() => void restore()}>
                    Restore (undo run)
                  </button>
                )}
                {restored && <span className="ap-restored" data-cut-autopilot-restored><Icon name="undo" size={14} /> reverted to checkpoint</span>}
                <button className="cd-btn cd-btn--ghost ap-inspect" data-cut-autopilot-inspect onClick={openInspect}>
                  Open Inspect →
                </button>
              </div>
            </div>
          )}
        </div>
      </aside>
    </div>
  )
}
