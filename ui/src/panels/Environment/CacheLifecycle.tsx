import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import type { JobRecord } from '../../lib/clientModel'
import type { ProjectCachePreviewResult, ProjectCacheRebuildResult } from '../../lib/clientResults'
import { purgeReconciliation } from './cacheLifecyclePresentation'
import CacheRebuildLifecycle from './CacheRebuildLifecycle'

interface CacheLifecycleProps {
  hasProject: boolean
  projectSession: number
  /** Null until Health & Recovery has a complete revision-bound media aggregate. */
  rebuildAssetIds: string[] | null
  onComplete: () => Promise<void>
}

type CacheJobKind = 'cleanup' | 'rebuild'

interface ActiveCacheJob {
  id: string
  kind: CacheJobKind
  session: number
}

function jobLabel(kind: CacheJobKind): string {
  return kind === 'rebuild' ? 'Cache rebuild' : 'Cache cleanup'
}

function isTerminalJob(record: JobRecord): boolean {
  return record.state === 'done' || record.state === 'failed' || record.outcome === 'cancelled' || record.outcome === 'interrupted' || record.outcome === 'superseded'
}

export default function CacheLifecycle({ hasProject, projectSession, rebuildAssetIds, onComplete }: CacheLifecycleProps) {
  // Promise completions can arrive after the editor switches/reopens a project.
  // Cache plans and job ids are project-scoped, so never project one session's
  // result into another session's Health & Recovery surface.
  const currentSession = useRef(projectSession)
  currentSession.current = projectSession
  const [preview, setPreview] = useState<ProjectCachePreviewResult | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)
  const [confirmationOpen, setConfirmationOpen] = useState(false)
  const [activeJob, setActiveJob] = useState<ActiveCacheJob | null>(null)
  const [job, setJob] = useState<JobRecord | null>(null)
  const [jobSession, setJobSession] = useState<number | null>(null)
  const [cleanupError, setCleanupError] = useState<string | null>(null)
  const [cleanupSuccess, setCleanupSuccess] = useState<string | null>(null)
  const [cancelPending, setCancelPending] = useState(false)
  const [rebuildLoading, setRebuildLoading] = useState(false)
  const [rebuildEstimateLoading, setRebuildEstimateLoading] = useState(false)
  const [rebuildResult, setRebuildResult] = useState<ProjectCacheRebuildResult | null>(null)
  const [rebuildError, setRebuildError] = useState<string | null>(null)
  const [rebuildSuccess, setRebuildSuccess] = useState<string | null>(null)
  // Effects run after commit; this marker blocks prior-project values in that interim frame.
  const [stateSession, setStateSession] = useState(projectSession)

  useEffect(() => {
    setStateSession(projectSession)
    setPreview(null)
    setPreviewLoading(false)
    setConfirmationOpen(false)
    setActiveJob(null)
    setJob(null)
    setJobSession(null)
    setCleanupError(null)
    setCleanupSuccess(null)
    setCancelPending(false)
    setRebuildLoading(false)
    setRebuildEstimateLoading(false)
    setRebuildResult(null)
    setRebuildError(null)
    setRebuildSuccess(null)
  }, [projectSession])

  // Gate prior-project values before the reset effect can paint them.
  const stateIsCurrent = stateSession === projectSession
  const activeJobForSession = activeJob?.session === projectSession ? activeJob : null
  const jobForSession = jobSession === projectSession ? job : null
  const previewForSession = stateIsCurrent ? preview : null
  const previewLoadingForSession = stateIsCurrent && previewLoading
  const confirmationOpenForSession = stateIsCurrent && confirmationOpen
  const cleanupErrorForSession = stateIsCurrent ? cleanupError : null
  const cleanupSuccessForSession = stateIsCurrent ? cleanupSuccess : null
  const cancelPendingForSession = stateIsCurrent && cancelPending
  const rebuildLoadingForSession = stateIsCurrent && rebuildLoading
  const rebuildEstimateLoadingForSession = stateIsCurrent && rebuildEstimateLoading
  const rebuildResultForSession = stateIsCurrent ? rebuildResult : null
  const rebuildErrorForSession = stateIsCurrent ? rebuildError : null
  const rebuildSuccessForSession = stateIsCurrent ? rebuildSuccess : null

  const previewCache = useCallback(async () => {
    if (!stateIsCurrent || !hasProject || activeJobForSession) return
    const session = projectSession
    setPreviewLoading(true)
    setConfirmationOpen(false)
    setCleanupError(null)
    setCleanupSuccess(null)
    try {
      const response = await callVerb('project.cache_preview', {})
      if (currentSession.current !== session) return
      if (!response.ok || !response.result) {
        setPreview(null)
        setCleanupError(response.error?.message ?? 'Cache ownership could not be verified. Nothing was deleted.')
        return
      }
      setPreview(response.result)
    } catch {
      if (currentSession.current !== session) return
      setPreview(null)
      setCleanupError('Cache cleanup preview could not be read. Nothing was deleted.')
    } finally {
      if (currentSession.current === session) setPreviewLoading(false)
    }
  }, [activeJobForSession, hasProject, projectSession, stateIsCurrent])

  const rebuildBatch = rebuildAssetIds?.slice(0, 64) ?? []
  const remainingRebuildAssets = Math.max(0, (rebuildAssetIds?.length ?? 0) - rebuildBatch.length)

  const estimateRebuild = useCallback(async () => {
    if (!stateIsCurrent || !hasProject || rebuildBatch.length === 0 || rebuildEstimateLoading || activeJobForSession) return
    const session = projectSession
    setRebuildEstimateLoading(true)
    setRebuildError(null)
    setRebuildSuccess(null)
    try {
      const response = await callVerb('project.cache_rebuild', { asset_ids: rebuildBatch, estimate_only: true })
      if (currentSession.current !== session) return
      if (!response.ok || !response.result) {
        setRebuildResult(null)
        setRebuildError(response.error?.message ?? 'Cache rebuild work could not be verified.')
        return
      }
      setRebuildResult(response.result)
      if (response.result.status === 'not_needed') setRebuildSuccess('Editing cache is already up to date.')
    } catch {
      if (currentSession.current !== session) return
      setRebuildResult(null)
      setRebuildError('Cache rebuild work could not be verified.')
    } finally {
      if (currentSession.current === session) setRebuildEstimateLoading(false)
    }
  }, [activeJobForSession, hasProject, projectSession, rebuildBatch, rebuildEstimateLoading, stateIsCurrent])

  const rebuildCache = useCallback(async () => {
    if (!stateIsCurrent || !hasProject || rebuildBatch.length === 0 || rebuildLoading || activeJobForSession) return
    const session = projectSession
    setRebuildLoading(true)
    setRebuildError(null)
    setRebuildSuccess(null)
    try {
      const response = await callVerb('project.cache_rebuild', { asset_ids: rebuildBatch })
      if (currentSession.current !== session) return
      if (!response.ok || !response.result) {
        setRebuildResult(null)
        setRebuildError(response.error?.message ?? 'Cache rebuild could not be scheduled.')
        return
      }
      setRebuildResult(response.result)
      if (response.result.status === 'not_needed') {
        setRebuildSuccess('Editing cache is already up to date.')
      } else if (response.result.job_id) {
        setActiveJob({ id: response.result.job_id, kind: 'rebuild', session })
        setJob(null)
        setJobSession(null)
      } else {
        setRebuildError('Cache rebuild did not return a job to track.')
      }
    } catch {
      if (currentSession.current !== session) return
      setRebuildResult(null)
      setRebuildError('Cache rebuild could not be scheduled.')
    } finally {
      if (currentSession.current === session) setRebuildLoading(false)
    }
  }, [activeJobForSession, hasProject, projectSession, rebuildBatch, rebuildLoading, stateIsCurrent])

  const confirmPurge = useCallback(async () => {
    if (!previewForSession || previewForSession.purgeable.files === 0 || activeJobForSession) return
    const session = projectSession
    setCleanupError(null)
    setCleanupSuccess(null)
    try {
      const response = await callVerb('project.cache_purge', { plan_id: previewForSession.plan_id, confirm: true })
      if (currentSession.current !== session) return
      if (!response.ok || !response.result) {
        setCleanupError(response.error?.message ?? 'Cache cleanup did not start.')
        return
      }
      setActiveJob({ id: response.result.job_id, kind: 'cleanup', session })
      setJob(null)
      setJobSession(null)
      setConfirmationOpen(false)
      setPreview(null)
    } catch {
      if (currentSession.current !== session) return
      setCleanupError('Cache cleanup did not start.')
    }
  }, [activeJobForSession, previewForSession, projectSession])

  const cancelActiveJob = useCallback(async () => {
    if (!activeJob || activeJob.session !== projectSession) return
    const session = projectSession
    setCancelPending(true)
    if (activeJob.kind === 'rebuild') setRebuildError(null)
    else setCleanupError(null)
    try {
      const response = await callVerb('jobs.cancel', { job_id: activeJob.id })
      if (currentSession.current !== session) return
      if (!response.ok) {
        const message = response.error?.message ?? `${jobLabel(activeJob.kind)} could not be cancelled.`
        if (activeJob.kind === 'rebuild') setRebuildError(message)
        else setCleanupError(message)
      }
    } catch {
      if (currentSession.current !== session) return
      const message = `${jobLabel(activeJob.kind)} could not be cancelled.`
      if (activeJob.kind === 'rebuild') setRebuildError(message)
      else setCleanupError(message)
    } finally {
      if (currentSession.current === session) setCancelPending(false)
    }
  }, [activeJob, projectSession])

  useEffect(() => {
    if (!activeJob || activeJob.session !== projectSession) return
    const session = projectSession
    let disposed = false
    let timer: number | null = null
    const poll = async () => {
      try {
        const response = await callVerb('jobs.status', { job_id: activeJob.id })
        if (disposed || currentSession.current !== session) return
        if (!response.ok || !response.result || response.result.job_id !== activeJob.id) {
          const message = response.error?.message ?? `${jobLabel(activeJob.kind)} status could not be verified.`
          if (activeJob.kind === 'rebuild') setRebuildError(message)
          else setCleanupError(message)
          timer = window.setTimeout(() => { void poll() }, 800)
          return
        }
        const record = response.result
        setJob(record)
        setJobSession(session)
        if (isTerminalJob(record)) {
          setActiveJob(null)
          if (record.state === 'done') {
            if (activeJob.kind === 'rebuild') setRebuildSuccess('Cache rebuild completed. Editing cache status was refreshed.')
            else setCleanupSuccess('Cache cleanup completed. Recheck the preview before any later cleanup.')
          } else {
            const message = record.message ?? (record.outcome === 'cancelled'
              ? `${jobLabel(activeJob.kind)} was cancelled. You can safely try again.`
              : `${jobLabel(activeJob.kind)} did not finish. You can safely try again.`)
            if (activeJob.kind === 'rebuild') setRebuildError(message)
            else setCleanupError(message)
          }
          void onComplete()
          return
        }
        timer = window.setTimeout(() => { void poll() }, 400)
      } catch {
        if (!disposed && currentSession.current === session) {
          const message = `${jobLabel(activeJob.kind)} status could not be verified.`
          if (activeJob.kind === 'rebuild') setRebuildError(message)
          else setCleanupError(message)
          timer = window.setTimeout(() => { void poll() }, 800)
        }
      }
    }
    void poll()
    return () => {
      disposed = true
      if (timer !== null) window.clearTimeout(timer)
    }
  }, [activeJob, onComplete, projectSession])

  const cleanupDisabled = !stateIsCurrent || !hasProject || previewLoadingForSession || activeJobForSession !== null
  const completedPurgeReconciliation = jobForSession?.kind === 'cache_purge'
    ? purgeReconciliation(jobForSession.result)
    : null

  return (
    <aside className="settings-cache-lifecycle" data-cut-cache-lifecycle>
      <CacheRebuildLifecycle
        hasProject={hasProject}
        sessionCurrent={stateIsCurrent}
        hasCompleteInventory={rebuildAssetIds !== null}
        hasBatch={rebuildBatch.length > 0}
        remainingAssets={remainingRebuildAssets}
        activeJobLabel={activeJobForSession ? jobLabel(activeJobForSession.kind) : null}
        estimateLoading={rebuildEstimateLoadingForSession}
        loading={rebuildLoadingForSession}
        result={rebuildResultForSession}
        success={rebuildSuccessForSession}
        error={rebuildErrorForSession}
        onEstimate={estimateRebuild}
        onRebuild={rebuildCache}
      />

      <section className="settings-cache-lifecycle-cleanup" aria-labelledby="settings-cache-cleanup-title" data-cut-cache-cleanup>
        <div>
          <strong id="settings-cache-cleanup-title">Cleanup preview</strong>
          <p>Preview only checks ledger-owned, aged, unreferenced proxies and filmstrips. Sources, exports, captures, and receipts are never eligible.</p>
        </div>
        <div className="settings-cache-lifecycle-actions">
          <button type="button" className="env-btn env-btn--ghost" data-cut-cache-preview onClick={() => void previewCache()} disabled={cleanupDisabled}>
            {previewLoadingForSession ? 'Previewing…' : 'Preview cache cleanup'}
          </button>
          {previewForSession && <span data-cut-cache-preview-status>{previewForSession.purgeable.files} aged file{previewForSession.purgeable.files === 1 ? '' : 's'} / {previewForSession.purgeable.bytes.toLocaleString()} bytes eligible</span>}
        </div>
        {cleanupErrorForSession && <p className="settings-cache-lifecycle-error" data-cut-cache-error role="alert">{cleanupErrorForSession}</p>}
        {cleanupSuccessForSession && <p className="settings-cache-lifecycle-success" data-cut-cache-cleanup-success aria-live="polite">{cleanupSuccessForSession}</p>}
        {previewForSession && previewForSession.purgeable.files === 0 && <p data-cut-cache-empty>No ledger-owned cache files are eligible for cleanup.</p>}
        {previewForSession && previewForSession.purgeable.files > 0 && !confirmationOpenForSession && <button type="button" className="env-btn" data-cut-cache-purge onClick={() => setConfirmationOpen(true)} disabled={activeJobForSession !== null}>Review and confirm purge</button>}
        {previewForSession && previewForSession.purgeable.files > 0 && confirmationOpenForSession && (
          <div className="settings-cache-lifecycle-confirm" data-cut-cache-confirm>
            <p>Remove only the {previewForSession.purgeable.files} previewed ledger-owned cache file{previewForSession.purgeable.files === 1 ? '' : 's'}? This cannot delete source media, exports, captures, or receipts.</p>
            <div className="settings-cache-lifecycle-actions">
              <button type="button" className="env-btn env-btn--ghost" data-cut-cache-confirm-cancel onClick={() => setConfirmationOpen(false)}>Keep cache</button>
              <button type="button" className="env-btn" data-cut-cache-confirm-purge onClick={() => void confirmPurge()} disabled={activeJobForSession !== null}>Purge previewed cache</button>
            </div>
          </div>
        )}
      </section>

      {(activeJobForSession || jobForSession) && (
        <div data-cut-cache-job data-cut-cache-job-kind={activeJobForSession?.kind ?? (jobForSession?.kind === 'cache_rebuild' ? 'rebuild' : 'cleanup')} data-cut-cache-job-state={jobForSession?.state ?? 'queued'} aria-live="polite">
          <p>{jobForSession?.message ?? `${jobLabel(activeJobForSession?.kind ?? 'cleanup')} is queued.`} {jobForSession ? `${Math.round(jobForSession.progress * 100)}%` : ''}</p>
          {activeJobForSession && <button type="button" className="env-btn env-btn--ghost" data-cut-cache-cancel onClick={() => void cancelActiveJob()} disabled={cancelPendingForSession}>{cancelPendingForSession ? 'Cancelling…' : `Cancel ${activeJobForSession.kind}`}</button>}
          {!activeJobForSession && completedPurgeReconciliation && (() => {
            const { status, reconciliation } = completedPurgeReconciliation
            const {
              before,
              planned,
              removed,
              after,
              balanced,
              partial_progress,
              after_basis,
              ledger_recovery_required,
            } = reconciliation
            return <p data-cut-cache-reconciliation data-cut-cache-reconciliation-status={status} data-cut-cache-reconciliation-balanced={balanced ? 'true' : 'false'}>
              Before: {before.files} files / {before.bytes.toLocaleString()} bytes. Planned: {planned.files} / {planned.bytes.toLocaleString()}. Removed: {removed.files} / {removed.bytes.toLocaleString()}. After: {after.files} / {after.bytes.toLocaleString()} bytes. {balanced ? after_basis === 'strict_scan' ? 'Counts reconcile exactly.' : 'Counts reconcile through the exclusive cleanup lease.' : 'Counts did not reconcile; preview again before any later cleanup.'}{partial_progress ? ' Cancellation occurred after partial cleanup.' : ''}{ledger_recovery_required ? ' The ownership record needs recovery; later cleanup is blocked.' : ''}
            </p>
          })()}
          {!activeJobForSession && jobForSession?.kind === 'cache_purge' && <button type="button" className="env-btn env-btn--ghost" data-cut-cache-remeasure onClick={() => void previewCache()}>Remeasure cache</button>}
        </div>
      )}
    </aside>
  )
}
