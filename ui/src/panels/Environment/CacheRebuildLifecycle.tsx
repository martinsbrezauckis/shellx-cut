import type {
  ProjectCacheRebuildCounts,
  ProjectCacheRebuildResult,
} from '../../lib/clientResults'
import {
  rebuildCostSummary,
  rebuildSummary,
} from './cacheLifecyclePresentation'

interface CacheRebuildLifecycleProps {
  hasProject: boolean
  sessionCurrent: boolean
  hasCompleteInventory: boolean
  hasBatch: boolean
  remainingAssets: number
  activeJobLabel: string | null
  estimateLoading: boolean
  loading: boolean
  result: ProjectCacheRebuildResult | null
  success: string | null
  error: string | null
  onEstimate: () => Promise<void>
  onRebuild: () => Promise<void>
}

function refusedCount(counts: ProjectCacheRebuildCounts): number {
  return counts.source_changed
    + counts.source_unavailable
    + counts.unowned_outputs
    + counts.legacy_outputs
    + counts.unsupported_assets
}

function attentionReasons(counts: ProjectCacheRebuildCounts): string[] {
  const plural = (count: number, singular: string, pluralValue = `${singular}s`) => `${count} ${count === 1 ? singular : pluralValue}`
  return [
    counts.source_changed > 0 ? `${plural(counts.source_changed, 'source')} changed` : null,
    counts.source_unavailable > 0 ? `${plural(counts.source_unavailable, 'source')} unavailable` : null,
    counts.unowned_outputs > 0 ? `${plural(counts.unowned_outputs, 'existing cache entry')} not owned by Cut` : null,
    counts.legacy_outputs > 0 ? `${plural(counts.legacy_outputs, 'existing cache entry')} uses an older Cut format` : null,
    counts.unsupported_assets > 0 ? `${plural(counts.unsupported_assets, 'media item')} is not supported for cache rebuild` : null,
  ].filter((reason): reason is string => reason !== null)
}

export default function CacheRebuildLifecycle({
  hasProject,
  sessionCurrent,
  hasCompleteInventory,
  hasBatch,
  remainingAssets,
  activeJobLabel,
  estimateLoading,
  loading,
  result,
  success,
  error,
  onEstimate,
  onRebuild,
}: CacheRebuildLifecycleProps) {
  const disabled = !sessionCurrent || !hasProject || !hasCompleteInventory || !hasBatch || loading || estimateLoading || activeJobLabel !== null
  const unavailable = !sessionCurrent
    ? null
    : !hasProject
      ? 'Open a project to rebuild its editing cache.'
      : !hasCompleteInventory
        ? 'Finish the Health & Recovery check before rebuilding cache.'
        : !hasBatch
          ? 'This completed check found no missing cache to rebuild.'
          : activeJobLabel
            ? `${activeJobLabel} is in progress.`
            : null

  return (
    <section className="settings-cache-lifecycle-rebuild" aria-labelledby="settings-cache-rebuild-title" data-cut-cache-rebuild-section>
      <div>
        <strong id="settings-cache-rebuild-title">Editing cache</strong>
        <p>Rebuild only missing or stale Cut-owned proxies and base filmstrips. Original media stays unchanged.</p>
      </div>
      <div className="settings-cache-lifecycle-actions">
        <button
          type="button"
          className="env-btn env-btn--ghost"
          data-cut-cache-rebuild-estimate
          onClick={() => void onEstimate()}
          disabled={disabled}
          aria-describedby="cut-cache-rebuild-status"
        >
          {estimateLoading ? 'Verifying…' : 'Estimate rebuild work'}
        </button>
        <button
          type="button"
          className="env-btn env-btn--ghost"
          data-cut-cache-rebuild
          onClick={() => void onRebuild()}
          disabled={disabled}
          aria-describedby="cut-cache-rebuild-status"
        >
          {loading ? 'Scheduling…' : 'Rebuild missing cache'}
        </button>
        {unavailable && <span data-cut-cache-rebuild-disabled>{unavailable}</span>}
      </div>
      {result && (
        <div
          id="cut-cache-rebuild-status"
          data-cut-cache-rebuild-status
          data-cut-cache-rebuild-state={result.status}
          data-cut-cache-rebuild-queued={result.scheduled_assets}
          data-cut-cache-rebuild-up-to-date={result.counts.fresh_assets}
          data-cut-cache-rebuild-attention-count={refusedCount(result.counts)}
          aria-live="polite"
        >
          <p>{rebuildSummary(result, remainingAssets)}</p>
          <p data-cut-cache-rebuild-cost>{rebuildCostSummary(result.estimate)}. Source identity was verified for this estimate; Cut checks it again before publishing output.</p>
          {refusedCount(result.counts) > 0 && (
            <details className="settings-cache-lifecycle-attention" data-cut-cache-rebuild-attention>
              <summary data-cut-cache-rebuild-attention-toggle>{refusedCount(result.counts)} item{refusedCount(result.counts) === 1 ? ' needs' : 's need'} attention</summary>
              <ul>
                {attentionReasons(result.counts).map((reason) => <li key={reason}>{reason}</li>)}
              </ul>
            </details>
          )}
        </div>
      )}
      {success && <p className="settings-cache-lifecycle-success" data-cut-cache-rebuild-success aria-live="polite">{success}</p>}
      {error && <p className="settings-cache-lifecycle-error" data-cut-cache-rebuild-error role="alert">{error}</p>}
    </section>
  )
}
