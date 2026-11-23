import type {
  ProjectCacheRebuildEstimate,
  ProjectCacheRebuildResult,
  ProjectCacheRebuildSkip,
} from '../../lib/clientResults'

/** A skipped_assets entry for an asset that was NOT rebuilt although its
 * previews are not current. */
export type RebuildSkippedAsset = ProjectCacheRebuildSkip
  & { reason: Exclude<ProjectCacheRebuildSkip['reason'], 'already_current'> }

/** The assets that were really skipped. The engine also lists an asset whose
 * previews already match its source, with the enum reason `already_current`;
 * that asset is up to date (counted in `counts.fresh_assets`), not skipped.
 * The split is made on the reason value, never on its English text. */
export function rebuildSkips(result: ProjectCacheRebuildResult): RebuildSkippedAsset[] {
  return result.skipped_assets.filter((item): item is RebuildSkippedAsset => item.reason !== 'already_current')
}

/** The number of assets that need attention, counted from the per-asset list.
 * The summary sentence, the skipped-items heading and the
 * `data-cut-cache-rebuild-attention-count` attribute all read it, so the screen
 * and the attribute show one number. `counts.unowned_outputs` and
 * `counts.legacy_outputs` count OUTPUTS (a proxy and a filmstrip of one asset
 * make two), which is why they are not used here. */
export function attentionAssetCount(result: ProjectCacheRebuildResult): number {
  return new Set(rebuildSkips(result).map((item) => item.asset_id)).size
}

/** Human-facing, path-free cache lifecycle presentation kept outside the
 * request/polling component so result validation stays testable and compact. */
export function rebuildSummary(result: ProjectCacheRebuildResult, remainingAssets: number): string {
  const queued = result.scheduled_assets
  const upToDate = result.counts.fresh_assets
  const more = remainingAssets > 0
    ? ` ${remainingAssets} more item${remainingAssets === 1 ? '' : 's'} can be rebuilt after this batch completes.`
    : ''
  const attentionCount = attentionAssetCount(result)
  const attention = attentionCount > 0
    ? ` ${attentionCount} item${attentionCount === 1 ? ' needs' : 's need'} attention.`
    : ''
  if (result.status === 'already_queued') {
    return `${queued} item${queued === 1 ? '' : 's'} already queued.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}${attention}${more}`
  }
  if (result.status === 'not_needed') {
    if (attentionCount > 0) {
      return `No cache rebuild was scheduled.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}${attention}`
    }
    return `No rebuild was needed.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}`
  }
  if (result.status === 'estimated') {
    return `${queued} item${queued === 1 ? ' has' : 's have'} verified rebuild work.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}${attention}${more}`
  }
  return `${queued} item${queued === 1 ? '' : 's'} queued.${upToDate > 0 ? ` ${upToDate} already up to date.` : ''}${attention}${more}`
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
