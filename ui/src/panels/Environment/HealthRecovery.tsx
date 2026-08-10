import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import type { DoctorReport } from '../../lib/doctor'
import type { JobsListResult } from '../../lib/clientResults'
import {
  loadCaptureRecovery,
  type CaptureRecoveryInventory,
} from './captureRecoveryModel'
import {
  healthRecoveryRows,
  loadProjectHealth,
  type AggregatedProjectHealth,
  type HealthAction,
  type HealthRow,
  type RecorderDoctor,
} from './healthRecoveryModel'

interface HealthRecoveryProps {
  hasProject: boolean
  projectSession: number
  onRefreshDoctor: () => Promise<DoctorReport | null>
  onOpenAssets: () => void
  onOpenRecording: () => void
  onOpenToolchain: () => void
}

function stateLabel(state: HealthRow['state']): string {
  switch (state) {
    case 'healthy': return 'Healthy'
    case 'attention': return 'Needs attention'
    case 'recoverable': return 'Recoverable'
    case 'active-recovery': return 'Recovery active'
    case 'unrecoverable': return 'Could not recover'
    case 'unsupported': return 'Not reported'
    case 'checking': return 'Checking'
  }
}

function actionLabel(action: HealthAction): string | null {
  switch (action) {
    case 'assets': return 'Open Assets'
    case 'recording': return 'Open Record'
    case 'toolchain': return 'Open toolchain settings'
    case null: return null
  }
}

function ActionButton({ action, onRun }: { action: Exclude<HealthAction, null>; onRun: () => void }) {
  const label = actionLabel(action)
  if (action === 'assets') return <button type="button" className="env-btn env-btn--ghost" data-cut-health-open-assets onClick={onRun}>{label}</button>
  if (action === 'recording') return <button type="button" className="env-btn env-btn--ghost" data-cut-health-open-recording onClick={onRun}>{label}</button>
  return <button type="button" className="env-btn env-btn--ghost" data-cut-health-open-toolchain onClick={onRun}>{label}</button>
}

type Attempt<T> = { ok: true; value: T } | { ok: false }

async function attempt<T>(request: Promise<T>): Promise<Attempt<T>> {
  try {
    return { ok: true, value: await request }
  } catch {
    return { ok: false }
  }
}

export default function HealthRecovery({
  hasProject,
  projectSession,
  onRefreshDoctor,
  onOpenAssets,
  onOpenRecording,
  onOpenToolchain,
}: HealthRecoveryProps) {
  const [projectHealth, setProjectHealth] = useState<AggregatedProjectHealth | null>(null)
  const [projectHealthScanFailed, setProjectHealthScanFailed] = useState(false)
  const [jobs, setJobs] = useState<JobsListResult | null>(null)
  const [jobsScanFailed, setJobsScanFailed] = useState(false)
  const [captureDoctor, setCaptureDoctor] = useState<RecorderDoctor | null>(null)
  const [captureDoctorFailed, setCaptureDoctorFailed] = useState(false)
  const [captureRecovery, setCaptureRecovery] = useState<CaptureRecoveryInventory | null>(null)
  const [captureRecoveryFailed, setCaptureRecoveryFailed] = useState(false)
  const [toolchain, setToolchain] = useState<DoctorReport | null>(null)
  const [toolchainScanFailed, setToolchainScanFailed] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [settledRequest, setSettledRequest] = useState(0)
  const request = useRef(0)
  // Environment receives an App callback whose identity can change as a
  // project delta arrives. Keep the latest action without turning ordinary
  // project refreshes into another revision-bound filesystem health scan.
  const doctorRefresh = useRef(onRefreshDoctor)

  useEffect(() => {
    doctorRefresh.current = onRefreshDoctor
  }, [onRefreshDoctor])

  const refresh = useCallback(async () => {
    const id = ++request.current
    setRefreshing(true)
    setProjectHealth(null)
    setProjectHealthScanFailed(false)
    setJobs(null)
    setJobsScanFailed(false)
    setCaptureDoctor(null)
    setCaptureDoctorFailed(false)
    setCaptureRecovery(null)
    setCaptureRecoveryFailed(false)
    setToolchain(null)
    setToolchainScanFailed(false)
    const [jobsRead, doctorRead, toolchainRead, projectHealthRead, recoveryRead] = await Promise.all([
      attempt(callVerb('jobs.list', {})),
      attempt(callVerb('screen_record.doctor', {})),
      attempt(doctorRefresh.current()),
      hasProject
        ? attempt(loadProjectHealth(
          (args) => callVerb('project.health', args),
          (partial) => { if (id === request.current) setProjectHealth(partial) },
        ))
        : Promise.resolve({ ok: true, value: null } satisfies Attempt<AggregatedProjectHealth | null>),
      hasProject
        ? attempt(loadCaptureRecovery((args) => callVerb('screen_record.recovery_status', args)))
        : Promise.resolve({ ok: true, value: null } satisfies Attempt<CaptureRecoveryInventory | null>),
    ])
    if (id !== request.current) return
    if (!jobsRead.ok || !jobsRead.value.ok || !jobsRead.value.result) {
      setJobs(null)
      setJobsScanFailed(true)
    } else {
      setJobs(jobsRead.value.result)
    }
    if (!doctorRead.ok || !doctorRead.value.ok || !doctorRead.value.result) {
      setCaptureDoctor(null)
      setCaptureDoctorFailed(true)
    } else {
      setCaptureDoctor(doctorRead.value.result as RecorderDoctor)
    }
    if (!toolchainRead.ok || !toolchainRead.value) {
      setToolchain(null)
      setToolchainScanFailed(true)
    } else {
      setToolchain(toolchainRead.value)
    }
    if (hasProject && (!projectHealthRead.ok || !projectHealthRead.value)) {
      // A revised or failed page invalidates the partial aggregate. Keep an
      // explicit attention row until the user starts a new first-page scan.
      setProjectHealth(null)
      setProjectHealthScanFailed(true)
    }
    if (hasProject && (!recoveryRead.ok || !recoveryRead.value)) {
      setCaptureRecovery(null)
      setCaptureRecoveryFailed(true)
    } else if (recoveryRead.ok && recoveryRead.value) {
      setCaptureRecovery(recoveryRead.value)
    }
    if (id === request.current) {
      setSettledRequest(id)
      setRefreshing(false)
    }
  }, [hasProject])

  // `projectSession` increments only at a confirmed project switch/close. Do
  // not depend on the project object: normal delta/snapshot refreshes must not
  // restart a full paged filesystem scan while this Settings page stays open.
  useEffect(() => {
    void refresh()
    return () => { request.current += 1 }
  }, [projectSession, refresh])

  const runAction = (action: Exclude<HealthAction, null>) => {
    if (action === 'assets') onOpenAssets()
    else if (action === 'recording') onOpenRecording()
    else onOpenToolchain()
  }
  const rows = healthRecoveryRows({
    hasProject,
    projectHealth,
    projectHealthScanFailed,
    jobs,
    jobsScanFailed,
    captureDoctor,
    captureDoctorFailed,
    captureRecovery,
    captureRecoveryFailed,
    toolchain,
    toolchainScanFailed,
  })

  return (
    <section className="settings-section" aria-labelledby="settings-health-recovery-title" data-cut-settings-section="health-recovery">
      <div className="settings-section-head">
        <p className="settings-eyebrow">Current project & machine</p>
        <h3 id="settings-health-recovery-title">Health &amp; Recovery</h3>
        <p>Recovery evidence is reported by the engine in this check. This page never repairs, deletes, or relinks anything on its own.</p>
      </div>
      <div className="settings-health-toolbar">
        <button type="button" className="env-btn env-btn--ghost" data-cut-health-refresh onClick={() => void refresh()} disabled={refreshing}>
          {refreshing ? 'Checking…' : 'Check again'}
        </button>
        <span data-cut-health-scope>{hasProject ? 'Project checks are read in this check; journal and media pages are revision-bound.' : 'Open a project for journal, media, and capture checks.'}</span>
      </div>
      <div
        className="settings-health-list"
        data-cut-health-list
        data-cut-health-refresh-id={settledRequest}
        data-cut-health-settled={refreshing ? 'false' : 'true'}
        data-cut-health-capture-complete={hasProject ? String(captureRecovery?.complete === true) : 'not-applicable'}
        data-cut-health-capture-count={captureRecovery?.captures.length ?? 0}
      >
        {rows.map((row) => {
          const label = actionLabel(row.action)
          return (
            <article key={row.id} className="settings-health-row" data-cut-health-row={row.id} data-cut-health-state={row.state} data-cut-health-capture={row.id === 'capture' ? 'true' : undefined}>
              <span className={`settings-health-dot settings-health-dot--${row.state}`} aria-hidden="true" />
              <div className="settings-health-copy">
                <div><strong>{row.label}</strong><span className={`settings-health-state settings-health-state--${row.state}`}>{stateLabel(row.state)}</span></div>
                <p>{row.summary}</p>
                {row.detail && <small>{row.detail}</small>}
              </div>
              {row.action && label && <ActionButton action={row.action} onRun={() => runAction(row.action!)} />}
            </article>
          )
        })}
      </div>
      <p className="settings-health-note" data-cut-health-confirmation>Recovery actions with ambiguous or destructive consequences stay in their owning workflow and require your confirmation.</p>
    </section>
  )
}
