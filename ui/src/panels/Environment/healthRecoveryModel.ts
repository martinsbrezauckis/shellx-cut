// Health & Recovery data and presentation model.
//
// This model keeps page aggregation separate from Settings rendering: project
// health is filesystem-backed and revision-bound, while the UI must never call
// a partial page a whole-project green result.

import type { VerbArgs, VerbResult } from '../../lib/client'
import type { JobRecord } from '../../lib/clientModel'
import type { DoctorReport } from '../../lib/doctor'
import type {
  JobsListResult,
  ProjectHealthAsset,
  ProjectHealthPageCounts,
  ProjectHealthResult,
} from '../../lib/clientResults'
import type { CaptureRecoveryInventory } from './captureRecoveryModel'

export type HealthState = 'healthy' | 'attention' | 'recoverable' | 'active-recovery' | 'unrecoverable' | 'unsupported' | 'checking'
export type HealthAction = 'assets' | 'recording' | 'toolchain' | null

export interface RecorderDoctor {
  cards: Array<{ name: string; status: 'ok' | 'missing' | 'degraded' | 'unknown' | string; detail: string }>
  ready: boolean
}

export interface HealthRow {
  id: 'journal' | 'media' | 'jobs' | 'capture' | 'toolchain'
  label: string
  state: HealthState
  summary: string
  detail?: string
  action: HealthAction
}

export interface AggregatedProjectHealth extends ProjectHealthResult {
  complete: boolean
  media: ProjectHealthResult['media'] & { assets: ProjectHealthAsset[]; page: ProjectHealthPageCounts }
}

export type ProjectHealthCall = (
  args: VerbArgs['project.health'],
) => Promise<VerbResult<ProjectHealthResult>>

function addCounts(left: ProjectHealthPageCounts, right: ProjectHealthPageCounts): ProjectHealthPageCounts {
  return {
    offline: left.offline + right.offline,
    proxy_available: left.proxy_available + right.proxy_available,
    proxy_missing: left.proxy_missing + right.proxy_missing,
    proxy_not_recorded: left.proxy_not_recorded + right.proxy_not_recorded,
    proxy_not_applicable: left.proxy_not_applicable + right.proxy_not_applicable,
    filmstrip_available: left.filmstrip_available + right.filmstrip_available,
    filmstrip_missing: left.filmstrip_missing + right.filmstrip_missing,
    filmstrip_not_recorded: left.filmstrip_not_recorded + right.filmstrip_not_recorded,
    filmstrip_not_applicable: left.filmstrip_not_applicable + right.filmstrip_not_applicable,
  }
}

/** Merge only contiguous pages from one durable revision. Any mismatch fails
 * closed so a fast project mutation cannot yield a mixed green media row. */
export function mergeProjectHealthPage(
  previous: AggregatedProjectHealth | null,
  next: ProjectHealthResult,
): AggregatedProjectHealth {
  if (!previous) {
    return {
      ...next,
      complete: next.media.status === 'ready' && !next.media.has_more,
      media: { ...next.media, assets: [...next.media.assets], page: { ...next.media.page } },
    }
  }
  if ((previous.project_revision ?? null) !== (next.project_revision ?? null)) {
    throw new Error('project.health pages changed revision; restart from the first page')
  }
  if ((previous.media.next_cursor ?? null) !== (next.media.cursor ?? null)) {
    throw new Error('project.health page cursor is not contiguous; restart from the first page')
  }
  if (previous.media.status !== 'ready' || next.media.status !== 'ready') {
    throw new Error('project.health unavailable pages cannot be merged')
  }
  const assets = new Map(previous.media.assets.map((asset) => [asset.asset, asset]))
  for (const asset of next.media.assets) assets.set(asset.asset, asset)
  return {
    ...next,
    complete: !next.media.has_more,
    media: {
      ...next.media,
      checked_count: previous.media.checked_count + next.media.checked_count,
      assets: [...assets.values()],
      page: addCounts(previous.media.page, next.media.page),
    },
  }
}

/** Page through one open project. The callback receives the partial aggregate,
 * so Settings can say “checking N of total” instead of showing a partial pass. */
export async function loadProjectHealth(
  call: ProjectHealthCall,
  onPage?: (health: AggregatedProjectHealth) => void,
): Promise<AggregatedProjectHealth | null> {
  let cursor: string | undefined
  let revision: string | undefined
  let aggregate: AggregatedProjectHealth | null = null
  const seenCursors = new Set<string>()
  for (let pages = 0; pages < 4_096; pages += 1) {
    const response = await call(cursor ? { cursor, revision, limit: 128 } : { limit: 128 })
    if (!response.ok || !response.result) return null
    aggregate = mergeProjectHealthPage(aggregate, response.result)
    onPage?.(aggregate)
    if (!aggregate.media.has_more) return aggregate
    revision = aggregate.project_revision
    cursor = aggregate.media.next_cursor ?? undefined
    if (!revision || !cursor || seenCursors.has(cursor)) {
      throw new Error('project.health returned an invalid continuation cursor')
    }
    seenCursors.add(cursor)
  }
  throw new Error('project.health exceeded the bounded page traversal limit')
}

function countJobs(jobs: JobRecord[], state: JobRecord['state']): number {
  return jobs.filter((job) => job.state === state).length
}

function labelledReasons(jobs: JobRecord[]): string {
  const reasons = new Set(jobs.map((job) => job.outcome_reason).filter(Boolean))
  return [...reasons].map((reason) => reason!.replaceAll('_', ' ')).join(', ')
}

function journalRow(projectHealth: AggregatedProjectHealth | null, hasProject: boolean, scanFailed: boolean): HealthRow {
  if (!hasProject) return unsupported('journal', 'Edit journal', 'Open a project to inspect its durable journal.')
  if (scanFailed) return attention('journal', 'Edit journal', 'The health check did not complete. Check again; close and reopen the project if its journal changed.')
  if (!projectHealth) return checking('journal', 'Edit journal', 'Checking journal identity and recovery evidence…')
  const { journal } = projectHealth
  if (journal.status === 'verified') return healthy('journal', 'Edit journal', `${journal.log_records ?? 0} durable records verified.`)
  if (journal.status === 'recovered') return recoverable('journal', 'Edit journal', 'Recovery evidence was recorded; review the notice before continuing.', journal.notices.map((notice) => notice.message).join(' '))
  return attention('journal', 'Edit journal', 'Journal identity is not current. Close and reopen the project before relying on it.', journal.notices.map((notice) => notice.message).join(' '))
}

function mediaRow(projectHealth: AggregatedProjectHealth | null, hasProject: boolean, scanFailed: boolean): HealthRow {
  if (!hasProject) return unsupported('media', 'Media, proxy & filmstrip', 'Open a project to verify registered media.')
  if (scanFailed) return attention('media', 'Media, proxy & filmstrip', 'The revision-bound media check did not complete. Check again from the first page.', undefined, 'assets')
  if (!projectHealth) return checking('media', 'Media, proxy & filmstrip', 'Checking registered media…', 'assets')
  const { media } = projectHealth
  if (media.status === 'unavailable') return attention('media', 'Media, proxy & filmstrip', 'Project membership is not current. Reopen before checking media.', undefined, 'assets')
  if (!projectHealth.complete) return checking('media', 'Media, proxy & filmstrip', `Checking ${media.checked_count} of ${media.asset_count} registered assets…`, 'assets')
  if (media.page.offline > 0) return recoverable('media', 'Media, proxy & filmstrip', `${media.page.offline} source file${plural(media.page.offline)} offline.`, undefined, 'assets')
  const missing = media.page.proxy_missing + media.page.filmstrip_missing
  if (missing > 0) return recoverable('media', 'Media, proxy & filmstrip', `${missing} recorded derived file${plural(missing)} missing.`, 'Open Assets to inspect or relink source media; no repair runs here.', 'assets')
  const unreported = media.page.proxy_not_recorded + media.page.filmstrip_not_recorded
  if (unreported > 0) return attention('media', 'Media, proxy & filmstrip', `${unreported} derived state${plural(unreported)} not recorded.`, 'This does not infer a pending job or a failed generation.', 'assets')
  return healthy('media', 'Media, proxy & filmstrip', `${media.asset_count} registered asset${plural(media.asset_count)} verified.`, 'assets')
}

function jobsRow(jobsList: JobsListResult | null, scanFailed: boolean): HealthRow {
  if (scanFailed) return attention('jobs', 'Background jobs', 'The job status check did not complete. Check again before relying on recovery notices.')
  if (!jobsList) return checking('jobs', 'Background jobs', 'Checking persisted jobs and recovery notices…')
  const notices = jobsList.persistence_notices ?? []
  if (notices.length > 0) return recoverable('jobs', 'Background jobs', `${notices.length} corrupt job record${plural(notices.length)} quarantined.`, notices.map((notice) => notice.message).join(' '))
  const active = countJobs(jobsList.jobs, 'queued') + countJobs(jobsList.jobs, 'running')
  if (active > 0) return activeRecovery('jobs', 'Background jobs', `${active} background job${plural(active)} active.`)
  const failed = jobsList.jobs.filter((job) => job.state === 'failed')
  const trueFailures = failed.filter((job) => job.outcome_reason === 'true_failure')
  if (trueFailures.length > 0) return unrecoverable('jobs', 'Background jobs', `${trueFailures.length} job${plural(trueFailures.length)} ended in a true failure.`, 'Inspect the job error before choosing a retry.')
  if (failed.length > 0) return attention('jobs', 'Background jobs', `${failed.length} terminal job${plural(failed.length)} did not complete normally.`, labelledReasons(failed) || 'Legacy records did not report a terminal reason.')
  return healthy('jobs', 'Background jobs', 'No active jobs or persistence notices.')
}

function captureRow(input: {
  hasProject: boolean
  doctor: RecorderDoctor | null
  doctorFailed: boolean
  recovery: CaptureRecoveryInventory | null
  recoveryFailed: boolean
}): HealthRow {
  if (!input.hasProject) return unsupported('capture', 'Capture', 'Open a project to read capture recovery reported by this check.', undefined, 'recording')
  if (input.doctorFailed || input.recoveryFailed) {
    return attention('capture', 'Capture', 'The capture recovery check did not complete. Check again before relying on it.', undefined, 'recording')
  }
  if (!input.doctor || !input.recovery) return checking('capture', 'Capture', 'Checking capture readiness and recovery reported by this check…', 'recording')
  if (!input.doctor.ready) {
    return attention(
      'capture',
      'Capture',
      'Capture readiness is not verified on this machine.',
      input.doctor.cards.filter((card) => card.status !== 'ok').map((card) => card.detail).join(' '),
      'recording',
    )
  }
  const states = new Set(input.recovery.captures.map((capture) => capture.state))
  const attentionStates = ['corrupt', 'torn_journal', 'quarantined', 'owner_ambiguous'] as const
  const attentionStatesFound = attentionStates.filter((state) => states.has(state))
  if (attentionStatesFound.length > 0) {
    return attentionRow(
      'capture',
      'Capture',
      `${attentionStatesFound.length} capture recovery state${plural(attentionStatesFound.length)} needs attention in this check.`,
      `Reported: ${attentionStatesFound.map((state) => state.replaceAll('_', ' ')).join(', ')}.`,
      'recording',
    )
  }
  const recoverableStates = ['interrupted', 'recovered'] as const
  const recoverable = recoverableStates.filter((state) => states.has(state))
  if (recoverable.length > 0) {
    const count = input.recovery.captures.filter((capture) => recoverableStates.includes(capture.state as typeof recoverableStates[number])).length
    return recoverableRow(
      'capture',
      'Capture',
      `${count} capture${plural(count)} reported as recoverable in this check.`,
      `Reported: ${recoverable.map((state) => state.replaceAll('_', ' ')).join(', ')}.`,
      'recording',
    )
  }
  return healthy(
    'capture',
    'Capture',
    `${input.recovery.captures.length} complete capture recovery record${plural(input.recovery.captures.length)} reported in this check.`,
    'recording',
  )
}

function toolchainRow(report: DoctorReport | null, scanFailed: boolean): HealthRow {
  if (scanFailed) return attention('toolchain', 'Toolchain', 'The local tool check did not complete. Check again before relying on it.', undefined, 'toolchain')
  if (!report) return checking('toolchain', 'Toolchain', 'Checking local tools…', 'toolchain')
  const missing = report.cards.filter((card) => card.status === 'missing')
  if (missing.length > 0) return recoverable('toolchain', 'Toolchain', `${missing.length} required or optional tool${plural(missing.length)} missing.`, missing.map((card) => card.hint ?? card.id).join(' '), 'toolchain')
  const notHealthy = report.cards.filter((card) => card.status !== 'ok')
  if (notHealthy.length > 0) return attention('toolchain', 'Toolchain', `${notHealthy.length} tool check${plural(notHealthy.length)} needs attention.`, notHealthy.map((card) => card.hint ?? card.id).join(' '), 'toolchain')
  return healthy('toolchain', 'Toolchain', 'Local tool checks passed.', 'toolchain')
}

export function healthRecoveryRows(input: {
  hasProject: boolean
  projectHealth: AggregatedProjectHealth | null
  projectHealthScanFailed?: boolean
  jobs: JobsListResult | null
  jobsScanFailed?: boolean
  captureDoctor: RecorderDoctor | null
  captureDoctorFailed?: boolean
  captureRecovery: CaptureRecoveryInventory | null
  captureRecoveryFailed?: boolean
  toolchain: DoctorReport | null
  toolchainScanFailed?: boolean
}): HealthRow[] {
  return [
    journalRow(input.projectHealth, input.hasProject, input.projectHealthScanFailed === true),
    mediaRow(input.projectHealth, input.hasProject, input.projectHealthScanFailed === true),
    jobsRow(input.jobs, input.jobsScanFailed === true),
    captureRow({
      hasProject: input.hasProject,
      doctor: input.captureDoctor,
      doctorFailed: input.captureDoctorFailed === true,
      recovery: input.captureRecovery,
      recoveryFailed: input.captureRecoveryFailed === true,
    }),
    toolchainRow(input.toolchain, input.toolchainScanFailed === true),
  ]
}

function plural(count: number): string { return count === 1 ? '' : 's' }
function healthy(id: HealthRow['id'], label: string, summary: string, action: HealthAction = null): HealthRow { return { id, label, state: 'healthy', summary, action } }
function checking(id: HealthRow['id'], label: string, summary: string, action: HealthAction = null): HealthRow { return { id, label, state: 'checking', summary, action } }
function attention(id: HealthRow['id'], label: string, summary: string, detail?: string, action: HealthAction = null): HealthRow { return attentionRow(id, label, summary, detail, action) }
function attentionRow(id: HealthRow['id'], label: string, summary: string, detail?: string, action: HealthAction = null): HealthRow { return { id, label, state: 'attention', summary, detail, action } }
function recoverable(id: HealthRow['id'], label: string, summary: string, detail?: string, action: HealthAction = null): HealthRow { return recoverableRow(id, label, summary, detail, action) }
function recoverableRow(id: HealthRow['id'], label: string, summary: string, detail?: string, action: HealthAction = null): HealthRow { return { id, label, state: 'recoverable', summary, detail, action } }
function activeRecovery(id: HealthRow['id'], label: string, summary: string, action: HealthAction = null): HealthRow { return { id, label, state: 'active-recovery', summary, action } }
function unrecoverable(id: HealthRow['id'], label: string, summary: string, detail?: string): HealthRow { return { id, label, state: 'unrecoverable', summary, detail, action: null } }
function unsupported(id: HealthRow['id'], label: string, summary: string, detail?: string, action: HealthAction = null): HealthRow { return { id, label, state: 'unsupported', summary, detail, action } }
