import { useCallback, useEffect, useRef, useState } from 'react'
import { OPEN_RECORDING_RECOVERY_EVENT } from '../../app/RecordingRecoveryNotice'
import { callVerb } from '../../lib/client'
import type { DoctorReport } from '../../lib/doctor'
import type { JobsListResult } from '../../lib/clientResults'
import CacheLifecycle from './CacheLifecycle'
import JobRetryRecovery from './JobRetryRecovery'
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
  assetPaths?: Readonly<Record<string, string>>
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
    case 'not-checked': return 'Not checked yet'
  }
}

function actionLabel(action: HealthAction): string | null {
  switch (action) {
    case 'assets': return 'Open Assets'
    case 'recording': return 'Open Record'
    case 'toolchain': return 'Open toolchain settings'
    case 'check-capture': return 'Check screen capture'
    case 'recover-recordings': return 'Recover recordings'
    case null: return null
  }
}

export function ActionButton({ action, onRun }: { action: Exclude<HealthAction, null>; onRun: () => void }) {
  const label = actionLabel(action)
  if (action === 'assets') return <button type="button" className="env-btn env-btn--ghost" data-cut-health-open-assets onClick={onRun}>{label}</button>
  if (action === 'recording') return <button type="button" className="env-btn env-btn--ghost" data-cut-health-open-recording onClick={onRun}>{label}</button>
  if (action === 'check-capture') return <button type="button" className="env-btn env-btn--ghost" data-cut-health-check-capture onClick={onRun}>{label}</button>
  if (action === 'recover-recordings') return <button type="button" className="env-btn env-btn--ghost" data-cut-health-recover-recordings onClick={onRun}>{label}</button>
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

/** The full screen-capture check, started by its own button: the status read
 *  on page open never captures. Null when the engine did not answer. */
export async function readCaptureCheck(call: typeof callVerb): Promise<RecorderDoctor | null> {
  const read = await attempt(call('screen_record.doctor', { mode: 'check' }))
  if (!read.ok || !read.value.ok || !read.value.result) return null
  return read.value.result as RecorderDoctor
}

/** What each Health action button does. Recovering recordings asks the
 *  recording banner to list them, then goes to Record where they are handled. */
export function runHealthAction(action: Exclude<HealthAction, null>, to: {
  checkCapture: () => void
  openAssets: () => void
  openRecording: () => void
  openToolchain: () => void
}): void {
  if (action === 'check-capture') to.checkCapture()
  else if (action === 'assets') to.openAssets()
  else if (action === 'recording') to.openRecording()
  else if (action === 'recover-recordings') {
    // The recovery banner lists the recordings with their actions.
    window.dispatchEvent(new Event(OPEN_RECORDING_RECOVERY_EVENT))
    to.openRecording()
  }
  else to.openToolchain()
}

export default function HealthRecovery({
  assetPaths = {},
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
  const captureCheckRequest = useRef(0)
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
      // Opening this page only shows readiness: the status read never starts a
      // capture or a system prompt. The full check runs from its own button.
      attempt(callVerb('screen_record.doctor', { mode: 'status' })),
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

  // The person asked for this check, so it may run the short screen capture
  // (and macOS may show its screen-recording alert).
  const checkCapture = useCallback(async () => {
    const id = ++captureCheckRequest.current
    const page = request.current
    setCaptureDoctor(null)
    setCaptureDoctorFailed(false)
    const read = await readCaptureCheck(callVerb)
    if (id !== captureCheckRequest.current || page !== request.current) return
    if (read === null) setCaptureDoctorFailed(true)
    else setCaptureDoctor(read)
  }, [])

  const runAction = (action: Exclude<HealthAction, null>) => runHealthAction(action, {
    checkCapture: () => void checkCapture(),
    openAssets: onOpenAssets,
    openRecording: onOpenRecording,
    openToolchain: onOpenToolchain,
  })
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
  // A rebuild is admitted only from the completed, revision-bound aggregate.
  // Sorting makes repeated 64-item batches deterministic without exposing the
  // underlying ids to the UI.
  const rebuildAssetIds = projectHealth?.complete && projectHealth.media.status === 'ready'
    ? [...new Set(projectHealth.media.assets
      .filter((asset) => asset.proxy === 'missing' || asset.proxy === 'not_recorded' || asset.filmstrip === 'missing' || asset.filmstrip === 'not_recorded')
      .map((asset) => asset.asset))].sort()
    : null

  return (
    <section className="settings-section" aria-labelledby="settings-health-recovery-title" data-cut-settings-section="health-recovery">
      <div className="settings-section-head">
        <p className="settings-eyebrow">Current project & machine</p>
        <h3 id="settings-health-recovery-title">Health &amp; Recovery</h3>
        <p>Recovery evidence is reported by the engine in this check. This page never repairs or relinks anything on its own; cache cleanup requires a separate preview and confirmation.</p>
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
      <JobRetryRecovery jobs={jobs} projectSession={projectSession} onRefresh={refresh} />
      <CacheLifecycle assetPaths={assetPaths} hasProject={hasProject} projectSession={projectSession} rebuildAssetIds={rebuildAssetIds} onComplete={refresh} />
      <p className="settings-health-note" data-cut-health-confirmation>Recovery actions with ambiguous or destructive consequences stay in their owning workflow and require your confirmation.</p>
    </section>
  )
}
