// statusbar — bottom status bar (status-bar contract).
// Left→right: connection dot+label · live job pills (orange dot, name, %) ·
// spacer · last-receipt chip (PASS green / FAIL red — receipts are first-class
// citizens, never toasts) · playhead/selection readout · build id.
// Self-subscribes to the shared events singleton for connection state and
// job_progress (keeps App.tsx wiring minimal — the bar owns its own jobs map);
// receipts/playhead/selection arrive as props since App already folds them.
// Callers: App.tsx. Dependencies: lib/events, lib/client types, statusbar.css.

import { useEffect, useState } from 'react'
import { Icon } from '../icons'
import { callVerb, type Project, type RenderReceipt } from '../lib/client'
import { envHealthLevel, type DoctorReport } from '../lib/doctor'
import { events, type ConnectionState } from '../lib/events'
import { folderTail, getStoredOutputDir } from '../lib/exportDestination'
import { useTimeDisplay } from '../lib/timedisplay'
import { formatClock } from '../panels/Timeline/layout'
import '../panels/Environment/environment.css'
import './statusbar.css'

export interface StatusBarProps {
  project: Project | null
  receipts: RenderReceipt[]
  playheadMs: number
  selectedClipIds: string[]
  opsCount: number
  clipboardNotice?: string | null
  /** The environment doctor report drives the environment chip's color/label. */
  doctor: DoctorReport | null
  /** Open the Settings>Environment drawer (the chip's click). */
  onOpenEnvironment: (category?: 'overview' | 'general') => void
}

interface JobView {
  job_id: string
  kind: string
  progress: number
}

/** Build identity: vite mode + package version baked at build time. The
 *  version comes from `__APP_VERSION__` (vite `define` reads package.json — see
 *  vite.config.ts), so it can never drift from the package version (H2). */
const BUILD_ID = `cut-ui ${import.meta.env.MODE === 'production' ? `v${__APP_VERSION__}` : import.meta.env.MODE}`

/** Environment chip health: missing essential > any degraded > ok. The SEVERITY
 *  rung comes from the shared envHealthLevel (doctor.ts) so the chip and the
 *  topbar Setup dot can never disagree; the chip layers its own LABEL on top
 *  (incl. the informational judge-rung count when ready). */
function envHealth(doctor: DoctorReport | null): { cls: string; label: string } {
  switch (envHealthLevel(doctor)) {
    case 'unknown':
      return { cls: 'sb-env--degraded', label: 'Checking setup…' }
    case 'missing':
      return { cls: 'sb-env--missing', label: 'Video setup needed' }
    case 'degraded':
      return { cls: 'sb-env--degraded', label: 'Setup needs attention' }
    case 'ok':
      return { cls: 'sb-env--ok', label: 'Setup ready' }
    default:
      return { cls: 'sb-env--degraded', label: 'Checking setup…' }
  }
}

export default function StatusBar({ project, receipts, playheadMs, selectedClipIds, opsCount, clipboardNotice, doctor, onOpenEnvironment }: StatusBarProps) {
  const [connection, setConnection] = useState<ConnectionState>('connecting')
  const [jobs, setJobs] = useState<Record<string, JobView>>({})
  const [jobCancelErrors, setJobCancelErrors] = useState<Record<string, string>>({})
  const [outputDir, setOutputDir] = useState<string | null>(() => getStoredOutputDir())
  const timeMode = useTimeDisplay() // keep the bar's readout in lockstep with the timeline toggle
  const fps = project?.settings.fps ?? 30

  const cancelJob = async (jobId: string) => {
    setJobCancelErrors((previous) => {
      const next = { ...previous }
      delete next[jobId]
      return next
    })
    try {
      const r = await callVerb('jobs.cancel', { job_id: jobId })
      if (!r.ok) {
        setJobCancelErrors((previous) => ({
          ...previous,
          [jobId]: r.error?.message ?? 'Could not cancel this job.',
        }))
        return
      }
      setJobs((previous) => {
        const next = { ...previous }
        delete next[jobId]
        return next
      })
    } catch {
      setJobCancelErrors((previous) => ({
        ...previous,
        [jobId]: 'Server unreachable. Click to retry cancellation.',
      }))
    }
  }

  // Track connection + running jobs straight off the WS singleton.
  useEffect(() => {
    const offStatus = events.onStatus(setConnection)
    const offEvents = events.subscribe((ev) => {
      if (ev.type === 'job_progress') {
        setJobs((prev) => {
          const next = { ...prev }
          if (ev.progress >= 1) delete next[ev.job_id] // done → pill disappears
          else next[ev.job_id] = { job_id: ev.job_id, kind: ev.kind, progress: ev.progress }
          return next
        })
      } else if (ev.type === 'render_done') {
        setJobs((prev) => {
          const next = { ...prev }
          delete next[ev.job_id]
          return next
        })
      }
    })
    return () => { offStatus(); offEvents() }
  }, [])

  useEffect(() => {
    const onExportDir = (event: Event) => setOutputDir((event as CustomEvent<string | null>).detail ?? null)
    window.addEventListener('cut:export-output-dir', onExportDir)
    return () => window.removeEventListener('cut:export-output-dir', onExportDir)
  }, [])

  const lastReceipt = receipts.length ? receipts[receipts.length - 1] : null
  // Use the connected endpoint rather than an assumed default because cutd can
  // bind a caller-selected address.
  const connLabel = connection === 'open' ? 'connected'
    : connection === 'connecting' ? 'reconnecting…' : 'disconnected'

  return (
    <footer className="sb" data-panel="statusbar" data-cut-panel="statusbar">
      <span className="sb-conn" data-cut-connection={connection}>
        <span className={`sb-dot sb-dot--${connection}`} />
        {connLabel}
      </span>

      {/* Environment chip opens Settings > Environment (the same cards as
          the start wizard). Color = env health (missing essential = red). */}
      {(() => {
        const env = envHealth(doctor)
        return (
          <button
            className={`sb-env ${env.cls}`}
            data-cut-env-chip
            data-cut-env-essential-ok={doctor ? doctor.essential_ok : 'unknown'}
            title="Open Settings for video tools, captions, optional services, and app information"
            onClick={() => onOpenEnvironment('overview')}
          >
            <span className="sb-env-dot" />
            {env.label}
          </button>
        )
      })()}

      <button
        className="sb-output"
        data-cut-output-chip
        data-cut-output-dir={outputDir ?? ''}
        title={outputDir ? `Default export folder: ${outputDir}` : 'Default export folder: each project /exports folder'}
        onClick={() => onOpenEnvironment('general')}
      >
        <Icon name="folder" size={14} />
        <span className="sb-output-label">export folder:</span>
        <span className="sb-output-path">{outputDir ? folderTail(outputDir) : 'project exports'}</span>
      </button>

      {Object.values(jobs).map((j) => (
        <span key={j.job_id} className="sb-job" data-cut-job={j.job_id} title={j.job_id}>
          <span className="sb-job-dot" />
          {j.kind}
          <span className="sb-job-pct">{Math.round(j.progress * 100)}%</span>
          <button
            type="button"
            className={`sb-job-cancel${jobCancelErrors[j.job_id] ? ' sb-job-cancel--error' : ''}`}
            data-cut-job-cancel={j.job_id}
            data-cut-job-cancel-error={jobCancelErrors[j.job_id] || undefined}
            title={jobCancelErrors[j.job_id] || `Cancel ${j.kind}`}
            aria-label={jobCancelErrors[j.job_id] ? `Retry cancelling ${j.kind} job` : `Cancel ${j.kind} job`}
            onClick={() => void cancelJob(j.job_id)}
          >
            ×
          </button>
        </span>
      ))}

      <span className="sb-spacer" />

      {lastReceipt ? (() => {
        // Clean one-line "what changed" for normal use (receipt-philosophy
        //: a human verdict + output length, NOT an ocean of checks.
        // The detail (every check + evidence) lives in the Inspect rail, one
        // click away — this chip is the summary + the door to it.
        const dur = (lastReceipt.duration_ms / 1000).toFixed(1)
        // Recognised auto-/manual fixes from the fix_actions contract (falls
        // back to the failing-check count for legacy receipts without it).
        const receiptChecks = lastReceipt.checks.filter((c) => c.name !== 'footage_profile')
        const isUnmeasured = (c: (typeof receiptChecks)[number]) => {
          const d = c.details
          return d !== null && typeof d === 'object'
            && (Reflect.get(d, 'status') === 'unmeasured' || Reflect.get(d, 'measured') === false)
        }
        const unmeasured = receiptChecks.filter(isUnmeasured).length
        const failing = receiptChecks.filter(
          (c) => !c.pass && !isUnmeasured(c),
        ).length
        const fixActions = lastReceipt.fix_actions
        const toFix = failing > 0 && Array.isArray(fixActions) && fixActions.length > 0
          ? fixActions.length
          : failing
        const failSummary = failing > 0 && Array.isArray(fixActions) && fixActions.length > 0
          ? `${fixActions.length} to fix`
          : failing > 0
            ? `${failing} failed${unmeasured > 0 ? ` · ${unmeasured} unmeasured` : ''}`
            : unmeasured > 0
              ? `${unmeasured} unmeasured`
            : 'needs review'
        const summary = lastReceipt.pass
          ? `${dur}s · all checks pass`
          : `${dur}s · ${failSummary}`
        return (
          <button
            className={`sb-receipt ${lastReceipt.pass ? 'sb-receipt--pass' : failing === 0 && unmeasured > 0 ? 'sb-receipt--unmeasured' : 'sb-receipt--fail'}`}
            data-cut-last-receipt={lastReceipt.render_id}
            data-cut-receipt-pass={lastReceipt.pass}
            data-cut-receipt-tofix={toFix}
            title={`render ${lastReceipt.render_id} — open Inspect for the full receipt`}
            // The RECEIPTS tab lives in the review (Inspect) rail; it listens
            // for this DOM event to expand + switch tabs (loose-coupled join).
            onClick={() => document.dispatchEvent(new CustomEvent('cut:open-receipts'))}
          >
            {lastReceipt.pass
              ? <Icon name="success" size={14} tone="success" />
              : <Icon name="warning" size={14} tone="warn" />}
            {' '}{summary}
          </button>
        )
      })() : (
        <span className="sb-receipt sb-receipt--none" data-cut-last-receipt="none">no receipts</span>
      )}

      <span className="sb-pos" data-cut-pos>
        {selectedClipIds.length > 0 ? `${selectedClipIds.length} sel · ` : ''}
        {formatClock(playheadMs, fps, timeMode)}
      </span>
      {clipboardNotice ? <span className="sb-note" data-cut-clipboard-notice>{clipboardNotice}</span> : null}

      <span className="sb-pos">{project ? `${opsCount} ops` : 'no project'}</span>
      <span className="sb-build" data-cut-build>{BUILD_ID}</span>
    </footer>
  )
}
