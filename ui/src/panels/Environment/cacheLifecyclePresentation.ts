import type {
  ProjectCacheRebuildEstimate,
  ProjectCacheRebuildResult,
} from '../../lib/clientResults'

/** Human-facing, path-free cache lifecycle presentation kept outside the
 * request/polling component so result validation stays testable and compact. */
export function rebuildSummary(result: ProjectCacheRebuildResult, remainingAssets: number): string {
  const queued = result.scheduled_assets
  const upToDate = result.counts.fresh_assets
  const more = remainingAssets > 0
    ? ` ${remainingAssets} more item${remainingAssets === 1 ? '' : 's'} can be rebuilt after this batch completes.`
    : ''
  if (result.status === 'already_queued') {
    return `${queued} item${queued === 1 ? '' : 's'} already queued.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}${more}`
  }
  if (result.status === 'not_needed') {
    return `No rebuild was needed.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}`
  }
  if (result.status === 'estimated') {
    return `${queued} item${queued === 1 ? '' : 's'} have verified rebuild work.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}${more}`
  }
  return `${queued} item${queued === 1 ? '' : 's'} queued.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}${more}`
}

export function rebuildCostSummary(estimate: ProjectCacheRebuildEstimate): string {
  const outputCount = estimate.proxy_outputs + estimate.filmstrip_outputs
  const duration = estimate.proxy_duration_ms > 0
    ? ` / ${Math.round(estimate.proxy_duration_ms / 1000).toLocaleString()} s of proxy media`
    : ''
  return `${estimate.assets} verified source item${estimate.assets === 1 ? '' : 's'} / ${estimate.verified_source_bytes.toLocaleString()} bytes / ${outputCount} derived output${outputCount === 1 ? '' : 's'}${duration}`
}

interface CacheMeasurement {
  files: number
  bytes: number
}

export interface CachePurgeReconciliation {
  schema: 'shellx-cut/cache-purge-reconciliation/1'
  status: 'completed' | 'cancelled' | 'failed'
  reconciliation: {
    before: CacheMeasurement
    planned: CacheMeasurement
    removed: CacheMeasurement
    after: CacheMeasurement
    balanced: boolean
    partial_progress: boolean
    after_basis: 'strict_scan' | 'exclusive_lease_delta'
    ledger_recovery_required: boolean
  }
}

function cacheMeasurement(value: unknown): CacheMeasurement | null {
  if (!value || typeof value !== 'object') return null
  const candidate = value as Record<string, unknown>
  return typeof candidate.files === 'number' && Number.isSafeInteger(candidate.files) && candidate.files >= 0
    && typeof candidate.bytes === 'number' && Number.isSafeInteger(candidate.bytes) && candidate.bytes >= 0
    ? { files: candidate.files, bytes: candidate.bytes }
    : null
}

/** Reject an untyped jobs.status result rather than guessing a cleanup outcome. */
export function purgeReconciliation(value: unknown): CachePurgeReconciliation | null {
  if (!value || typeof value !== 'object') return null
  const candidate = value as Record<string, unknown>
  const reconciliation = candidate.reconciliation
  if (candidate.schema !== 'shellx-cut/cache-purge-reconciliation/1'
    || (candidate.status !== 'completed' && candidate.status !== 'cancelled' && candidate.status !== 'failed')
    || !reconciliation || typeof reconciliation !== 'object') return null
  const counts = reconciliation as Record<string, unknown>
  const before = cacheMeasurement(counts.before)
  const planned = cacheMeasurement(counts.planned)
  const removed = cacheMeasurement(counts.removed)
  const after = cacheMeasurement(counts.after)
  if (!before || !planned || !removed || !after
    || typeof counts.balanced !== 'boolean'
    || typeof counts.partial_progress !== 'boolean'
    || (counts.after_basis !== 'strict_scan' && counts.after_basis !== 'exclusive_lease_delta')
    || typeof counts.ledger_recovery_required !== 'boolean') return null
  return {
    schema: candidate.schema,
    status: candidate.status,
    reconciliation: {
      before,
      planned,
      removed,
      after,
      balanced: counts.balanced,
      partial_progress: counts.partial_progress,
      after_basis: counts.after_basis,
      ledger_recovery_required: counts.ledger_recovery_required,
    },
  }
}
